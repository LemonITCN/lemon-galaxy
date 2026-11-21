using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Shapes;
using LemonGalaxy.Core;

namespace LemonGalaxy.UI;

internal enum ForwardState { Stopped, Starting, Running, Failed }

/// <summary>一条端口映射规则在本机的运行状态</summary>
internal sealed class ForwardRun
{
    public ForwardState State;
    /// <summary>实际监听的本机端口（规则里写 0 时由内核自动选）</summary>
    public ushort Port;
    public string? Error;
    public ICorePortForward? Handle;
    /// <summary>每次开始或停止加一：开到一半被停掉、退出登录时，迟到的结果直接停掉丢弃</summary>
    public int Gen;
}

/// <summary>
/// 端口映射：规则存在端到端加密库（port_forward 条目，各设备同步），运行状态只在本机。
/// 按规则开启走内核 start_port_forward（经由 SSH 主机时经 SecretPrompt 问密码、确认主机密钥）；
/// 登录后开启「自动开启」的规则（本次登录里手动停掉的不再自动开），退出登录时全部停止。
/// </summary>
internal sealed class PortForwardHub
{
    private readonly AppModel app;
    public readonly Observable Changed = new();
    private readonly Dictionary<string, ForwardRun> runs = new();
    /// <summary>本次登录里已经自动开启过的规则</summary>
    private readonly HashSet<string> autoTried = new();
    private bool autoStartDone;

    public PortForwardHub(AppModel app) { this.app = app; }

    public ForwardRun? Run(string id) => runs.TryGetValue(id, out var r) ? r : null;
    public ForwardState StateOf(string id) => Run(id)?.State ?? ForwardState.Stopped;
    public int RunningCount => runs.Values.Count(r => r.State == ForwardState.Running);

    public static string Address(ushort port) => "127.0.0.1:" + port;

    /// <summary>可复制的本机地址：运行中用实际端口；没运行时规则里写了固定端口也能给出，自动端口给不出</summary>
    public string? AddressOf(string id)
    {
        if (Run(id) is { State: ForwardState.Running } r) return Address(r.Port);
        return app.Entry(id)?.Data is VaultData.PortForward { LocalPort: > 0 } f ? Address(f.LocalPort) : null;
    }

    public void Start(string id, bool auto = false)
    {
        if (app.Entry(id)?.Data is not VaultData.PortForward rule) return;
        var cur = Run(id);
        if (cur is { State: ForwardState.Starting or ForwardState.Running }) return;
        var run = cur ?? new ForwardRun();
        runs[id] = run;
        var gen = ++run.Gen;
        run.State = ForwardState.Starting;
        run.Error = null;
        Changed.Notify();
        var name = ForwardViews.Name(rule);
        var promptTitle = rule.ViaSshHostId is { Length: > 0 } hid ? app.SshLabel(hid) : name;
        Ui.Run(async () =>
        {
            ICorePortForward? h = null;
            Exception? error = null;
            try { h = await app.Client.StartPortForward(id, new SecretPromptBridge(app, promptTitle)); }
            catch (Exception e) { error = e; }
            if (gen != run.Gen || !ReferenceEquals(Run(id), run))
            {
                // 等待期间被停掉了（或退出登录）：这一路不要了
                StopHandle(h);
                return;
            }
            if (h != null)
            {
                ushort port = 0;
                try { port = h.LocalPort(); }
                catch (Exception e) { Log.Error("读取端口映射的本机端口失败：" + e.Message); }
                run.Handle = h;
                run.Port = port;
                run.State = ForwardState.Running;
                Log.Info($"端口映射已开启：{name} {Address(port)} → {rule.RemoteHost}:{rule.RemotePort}");
            }
            else if (error is GalaxyException.Cancelled)
            {
                // 用户在问密码 / 主机密钥时取消了
                run.State = ForwardState.Stopped;
            }
            else
            {
                run.State = ForwardState.Failed;
                run.Error = error == null ? "" : Fmt.Error(error);
                Log.Info($"端口映射开启失败：{name}：{run.Error}");
                // 自动开启失败只在列表里标出来，不逐条弹提示
                if (!auto) app.ShowToast(L.T("forward.failed", name, run.Error), true);
            }
            Changed.Notify();
        });
    }

    public void Stop(string id)
    {
        if (!runs.TryGetValue(id, out var run)) return;
        run.Gen++;
        var h = run.Handle;
        run.Handle = null;
        run.State = ForwardState.Stopped;
        run.Error = null;
        run.Port = 0;
        StopHandle(h);
        Changed.Notify();
    }

    public void Toggle(string id)
    {
        if (StateOf(id) is ForwardState.Running or ForwardState.Starting) Stop(id);
        else Start(id);
    }

    /// <summary>按改过的规则重开</summary>
    public void Restart(string id)
    {
        Stop(id);
        Start(id);
    }

    /// <summary>退出登录 / 会话被撤销 / 关窗口：全部停止，下次登录重新自动开启</summary>
    public void StopAll()
    {
        foreach (var id in runs.Keys.ToList()) Stop(id);
        runs.Clear();
        autoTried.Clear();
        autoStartDone = false;
        Changed.Notify();
    }

    /// <summary>登录后（设备列表刷新后）开启「自动开启」的规则；之后同步来的新规则也补开一次</summary>
    public void AutoStart()
    {
        autoStartDone = true;
        foreach (var e in app.PortForwards)
            if (e.Data is VaultData.PortForward { AutoStart: true } && autoTried.Add(e.Id) && StateOf(e.Id) == ForwardState.Stopped)
                Start(e.Id, auto: true);
    }

    /// <summary>加密库变化：规则被删掉（本机或别的设备）就停掉</summary>
    public void VaultChanged()
    {
        var changed = false;
        foreach (var id in runs.Keys.ToList())
        {
            if (app.Entry(id)?.Data is VaultData.PortForward) continue;
            Stop(id);
            runs.Remove(id);
            changed = true;
        }
        if (autoStartDone && app.LoggedIn) AutoStart();
        if (changed) Changed.Notify();
    }

    private static void StopHandle(ICorePortForward? h)
    {
        if (h == null) return;
        try { h.Stop(); }
        catch (Exception e) { Log.Error("停止端口映射失败：" + e.Message); }
    }
}

/// <summary>端口映射规则的编辑草稿</summary>
internal sealed class ForwardDraft : Observable
{
    public string Id = "";
    public string? FolderId;
    public long Sort;
    /// <summary>经由："dev:" + 设备 id 或 "ssh:" + SSH 主机条目 id；空表示还没选</summary>
    public string Via = "";
    public string Label = "", RemoteHost = "", RemotePort = "", LocalPort = "", Note = "";
    public bool AutoStart;

    public static ForwardDraft From(AppModel app, string? id)
    {
        if (id == null || app.Entry(id) is not { Data: VaultData.PortForward f } e) return new ForwardDraft { RemoteHost = "127.0.0.1" };
        return new ForwardDraft
        {
            Id = e.Id, FolderId = e.FolderId, Sort = e.Sort, Label = f.Label,
            Via = f.ViaDeviceId is { Length: > 0 } d ? "dev:" + d : f.ViaSshHostId is { Length: > 0 } h ? "ssh:" + h : "",
            RemoteHost = f.RemoteHost, RemotePort = f.RemotePort.ToString(), LocalPort = f.LocalPort == 0 ? "" : f.LocalPort.ToString(),
            AutoStart = f.AutoStart, Note = f.Note,
        };
    }

    private static int? Num(string s) => int.TryParse(s.Trim(), System.Globalization.NumberStyles.None, System.Globalization.CultureInfo.InvariantCulture, out var v) ? v : null;

    /// <summary>目标端口 1–65535</summary>
    public ushort? RemotePortValue => Num(RemotePort) is { } p && p is >= 1 and <= 65535 ? (ushort)p : null;
    /// <summary>本机端口 0–65535，留空等于 0（每次自动选）</summary>
    public ushort? LocalPortValue => LocalPort.Trim().Length == 0 ? (ushort)0 : Num(LocalPort) is { } p && p is >= 0 and <= 65535 ? (ushort)p : null;
    public bool RemoteHostValid => RemoteHost.Trim().Length > 0 && !RemoteHost.Trim().Any(char.IsWhiteSpace);
    /// <summary>经由设备与经由 SSH 主机恰好一个</summary>
    public bool ViaValid => Via.Length > 4 && (Via.StartsWith("dev:", StringComparison.Ordinal) || Via.StartsWith("ssh:", StringComparison.Ordinal));

    public bool IsValid => ViaValid && RemoteHostValid && RemotePortValue != null && LocalPortValue != null;

    public VaultEntry Build()
    {
        var host = RemoteHost.Trim();
        var port = RemotePortValue ?? 0;
        var label = Label.Trim().Length > 0 ? Label.Trim() : $"{host}:{port}";
        var dev = Via.StartsWith("dev:", StringComparison.Ordinal) ? Via.Substring(4) : null;
        var ssh = Via.StartsWith("ssh:", StringComparison.Ordinal) ? Via.Substring(4) : null;
        return new VaultEntry(Id, FolderId, Sort, new VaultData.PortForward(label, dev, ssh, LocalPortValue ?? 0, host, port, AutoStart, Note), 0);
    }

    /// <summary>映射本身（经由、端口、目标）变了没有：运行中的规则改了这些要重开</summary>
    public static bool MappingChanged(VaultData.PortForward a, VaultData.PortForward b)
        => a.ViaDeviceId != b.ViaDeviceId || a.ViaSshHostId != b.ViaSshHostId || a.LocalPort != b.LocalPort || a.RemoteHost != b.RemoteHost || a.RemotePort != b.RemotePort;
}

/// <summary>「端口映射」分区：中间列是规则列表，首页是概览 / 规则详情 / 编辑</summary>
internal static class ForwardViews
{
    private static string query = "";

    public static string Name(VaultData.PortForward f) => f.Label.Trim().Length > 0 ? f.Label : Target(f);
    public static string Target(VaultData.PortForward f) => (f.RemoteHost.Contains(':') ? "[" + f.RemoteHost + "]" : f.RemoteHost) + ":" + f.RemotePort;

    public static string Via(AppModel app, VaultData.PortForward f)
        => f.ViaDeviceId is { Length: > 0 } d ? app.DeviceName(d) : f.ViaSshHostId is { Length: > 0 } h ? app.SshLabel(h) : "—";

    /// <summary>本机一侧：运行中显示实际地址，否则显示规则里的端口（0 为自动）</summary>
    public static string Local(AppModel app, string id, VaultData.PortForward f)
    {
        if (app.Forwards.Run(id) is { State: ForwardState.Running } r) return PortForwardHub.Address(r.Port);
        return f.LocalPort == 0 ? L.T("forward.local.auto") : PortForwardHub.Address(f.LocalPort);
    }

    private static string StateText(ForwardState s) => s switch
    {
        ForwardState.Running => L.T("forward.state.running"),
        ForwardState.Starting => L.T("forward.state.starting"),
        ForwardState.Failed => L.T("forward.state.failed"),
        _ => L.T("forward.state.stopped"),
    };

    private static Color StateColor(ForwardState s) => s switch
    {
        ForwardState.Running => Th.P.Success,
        ForwardState.Starting => Th.P.Warning,
        ForwardState.Failed => Th.P.Danger,
        _ => Th.P.TertiaryText,
    };

    private static FrameworkElement StateBadge(ForwardState s) => K.Badge(StateText(s), StateColor(s));

    /// <summary>规则图标：圆角方块 + 右下角状态点</summary>
    private static FrameworkElement RuleIcon(ForwardState s, double size, Brush ring)
    {
        var g = new Grid { Width = size, Height = size };
        var running = s == ForwardState.Running;
        g.Children.Add(new Border
        {
            CornerRadius = new CornerRadius(size * 0.22), Background = running ? Th.B(Th.P.Accent, 0.12) : Th.Elevated, BorderBrush = running ? null : Th.Border, BorderThickness = new Thickness(running ? 0 : 1),
            Child = K.Center(K.Icon("forward", size * 0.5, running ? Th.Accent : Th.Secondary, 1.9)),
        });
        if (s != ForwardState.Stopped)
        {
            var dot = size * 0.26;
            g.Children.Add(new Ellipse
            {
                Width = dot, Height = dot, Fill = Th.B(StateColor(s)), Stroke = ring, StrokeThickness = 1.5,
                HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Bottom, Margin = new Thickness(0, 0, -size * 0.08, -size * 0.08),
            });
        }
        return g;
    }

    private static void Select(AppModel app, string id)
    {
        app.SelectedForward = id;
        if (app.Editing is EditTarget.Forward) app.Editing = null;
        app.ActiveTabId = null;
        app.Nav.Notify();
    }

    public static void New(AppModel app)
    {
        app.SelectedForward = null;
        app.Editing = new EditTarget.Forward(null);
        app.SelectHome(NavSection.Forwards);
    }

    private static void Edit(AppModel app, string id)
    {
        app.SelectedForward = id;
        app.Editing = new EditTarget.Forward(id);
        app.SelectHome(NavSection.Forwards);
    }

    private static void CopyAddress(AppModel app, string id)
    {
        if (app.Forwards.AddressOf(id) is not { } addr) return;
        SysClipboard.SetText(addr);
        app.ShowToast(L.T("forward.copied", addr));
    }

    private static void AskDelete(AppModel app, string id, string name)
        => app.Confirm(L.T("forward.delete.title"), L.T("forward.delete.message", name), L.T("common.delete"), () =>
        {
            app.Forwards.Stop(id);
            Ui.Run(() => app.Delete(id));
        });

    public static List<MenuEntry> ContextMenu(AppModel app, string id)
    {
        if (app.Entry(id)?.Data is not VaultData.PortForward f) return new List<MenuEntry>();
        var on = app.Forwards.StateOf(id) is ForwardState.Running or ForwardState.Starting;
        return new List<MenuEntry>
        {
            new(on ? L.T("forward.stop") : L.T("forward.start"), () => app.Forwards.Toggle(id)),
            new(L.T("forward.copy"), () => CopyAddress(app, id), app.Forwards.AddressOf(id) != null),
            MenuEntry.Sep,
            new(L.T("hosts.edit"), () => Edit(app, id)),
            new(L.T("common.delete"), () => AskDelete(app, id, Name(f)), Destructive: true),
        };
    }

    // ---------- 中间列 ----------

    public static FrameworkElement Middle(AppModel app)
    {
        var list = new Region(() =>
        {
            var q = query.Trim().ToLowerInvariant();
            var items = new List<UIElement?>();
            foreach (var e in app.PortForwards)
            {
                if (e.Data is not VaultData.PortForward f) continue;
                var name = Name(f);
                if (q.Length > 0 && !$"{name} {Target(f)} {Via(app, f)} {f.LocalPort}".ToLowerInvariant().Contains(q)) continue;
                var id = e.Id;
                var run = app.Forwards.Run(id);
                var state = run?.State ?? ForwardState.Stopped;
                var sel = app.SelectedForward == id && app.ActiveTabId == null;
                var sub = state == ForwardState.Failed && run?.Error is { Length: > 0 } err
                    ? K.Text(err, 11.5, Th.Danger)
                    : K.Text($"{Local(app, id, f)} → {Target(f)}", 11.5, Th.Tertiary);
                var on = state is ForwardState.Running or ForwardState.Starting;
                var toggle = K.IconButton(on ? "stop" : "play", on ? L.T("forward.stop") : L.T("forward.start"), () => app.Forwards.Toggle(id), size: 10, color: on ? Th.Danger : Th.Secondary, w: 24, h: 24);
                var b = new Btn
                {
                    Height = 48, CornerRadius = new CornerRadius(6), Padding = new Thickness(8, 0, 6, 0),
                    Normal = sel ? Th.Selection : Brushes.Transparent, HoverBrush = sel ? Th.Selection : Th.Hover,
                    Child = K.Row(9, RuleIcon(state, 28, sel ? Th.Selection : Th.List), K.Grow(K.Col(2, K.Text(name, 13, Th.Text, FontWeights.Medium), sub)), toggle),
                };
                b.UpdateLook();
                b.Click += () => Select(app, id);
                b.DoubleClick += () => app.Forwards.Toggle(id);
                K.ContextMenuFor(b, () => ContextMenu(app, id));
                items.Add(b);
            }
            if (items.Count == 0)
                items.Add(K.Margin(K.Text(app.PortForwards.Count == 0 ? L.T("forward.empty") : L.T("hosts.noMatch"), 12.5, Th.Tertiary), 10, 10, 10, 10));
            return K.Margin(K.Col(1, items), 6);
        }).On(app.Data, app.Nav, app.Forwards.Changed);
        var search = K.SearchField(L.T("forward.search"), query, v => { query = v; list.Invalidate(); }, out _);
        var dock = new DockPanel();
        var top = K.Col(0,
            Shell.ColumnHeader(L.T("nav.forwards"), K.IconButton("plus", L.T("forward.new"), () => New(app), size: 15)),
            K.Margin(search, 12, 0, 12, 8), K.HLine());
        DockPanel.SetDock(top, Dock.Top);
        dock.Children.Add(top);
        dock.Children.Add(K.Scroll(list));
        return dock;
    }

    // ---------- 首页 ----------

    public static FrameworkElement Home(AppModel app)
    {
        EditTarget.Forward? Editing() => app.Editing as EditTarget.Forward;
        // 编辑面板只在目标变化时重建（输入框保持焦点）；概览与详情随数据、运行状态刷新
        return new Region(() =>
        {
            if (Editing() is { } target)
                return new Border { MaxWidth = 600, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 12, 0, 12), Child = Editor(app, target) };
            return new Region(() => app.SelectedForward is { } id && app.Entry(id)?.Data is VaultData.PortForward f ? Detail(app, id, f) : Overview(app))
                .On(app.Data, app.Nav, app.Forwards.Changed);
        }).Keyed(() => Editing()).On(app.Nav);
    }

    private static FrameworkElement Overview(AppModel app)
    {
        var rules = app.PortForwards;
        var header = K.Row(10,
            K.Semi(L.T("nav.forwards"), 21),
            K.Margin(K.Text(L.T("forward.runningCount", app.Forwards.RunningCount), 13, Th.Secondary), 0, 5, 0, 0),
            K.Info(L.T("forward.info")),
            K.Spacer(),
            K.SecondaryButton(L.T("forward.new"), () => New(app), icon: "plus"));
        if (rules.Count == 0)
        {
            var dock = new DockPanel();
            var top = K.Pad(header, 24, 24, 24, 0);
            DockPanel.SetDock(top, Dock.Top);
            dock.Children.Add(top);
            dock.Children.Add(K.EmptyState("forward", L.T("forward.empty"), (L.T("forward.new"), () => New(app))));
            return dock;
        }
        var grid = new AdaptiveGrid { MinItem = 280, Spacing = 12 };
        foreach (var e in rules)
            if (e.Data is VaultData.PortForward f) grid.Children.Add(Card(app, e.Id, f));
        return K.PageScroll(K.Col(18, header, grid), 1100);
    }

    private static FrameworkElement Card(AppModel app, string id, VaultData.PortForward f)
    {
        var run = app.Forwards.Run(id);
        var state = run?.State ?? ForwardState.Stopped;
        var on = state is ForwardState.Running or ForwardState.Starting;
        var toggle = K.IconButton(on ? "stop" : "play", on ? L.T("forward.stop") : L.T("forward.start"), () => app.Forwards.Toggle(id), size: 11, color: on ? Th.Danger : Th.Accent, w: 30, h: 30);
        toggle.CornerRadius = new CornerRadius(15);
        toggle.Normal = on ? Th.B(Th.P.Danger, 0.1) : Th.B(Th.P.Accent, 0.1);
        toggle.UpdateLook();
        var mapping = K.Text($"{Local(app, id, f)} → {Target(f)}", 12.5, Th.Text, mono: true);
        var via = state == ForwardState.Failed && run?.Error is { Length: > 0 } err
            ? K.Text(err, 12, Th.Danger)
            : K.Text(L.T("forward.viaName", Via(app, f)), 12, Th.Secondary);
        var b = new Btn
        {
            Padding = new Thickness(12), CornerRadius = new CornerRadius(10), BorderThickness = new Thickness(1),
            Normal = Th.Card, HoverBrush = Th.Elevated, NormalBorder = Th.Border, HoverBorder = Th.B(Th.P.Accent, 0.5),
            Child = K.Row(11, RuleIcon(state, 38, Th.Card),
                K.Grow(K.Col(4, K.Row(6, K.Grow(K.Semi(Name(f), 13.5)), f.AutoStart ? K.Tip(new Border { Background = Brushes.Transparent, Child = K.Icon("bolt", 12, Th.Tertiary) }, L.T("forward.autoStart")) : null, StateBadge(state)), mapping, via)),
                toggle),
        };
        b.UpdateLook();
        b.Click += () => Select(app, id);
        b.DoubleClick += () => app.Forwards.Toggle(id);
        K.ContextMenuFor(b, () => ContextMenu(app, id));
        return b;
    }

    private static FrameworkElement Detail(AppModel app, string id, VaultData.PortForward f)
    {
        var run = app.Forwards.Run(id);
        var state = run?.State ?? ForwardState.Stopped;
        var viaDevice = f.ViaDeviceId is { Length: > 0 };
        FrameworkElement action = state switch
        {
            ForwardState.Running => K.SecondaryButton(L.T("forward.stop"), () => app.Forwards.Stop(id), destructive: true, icon: "stop"),
            ForwardState.Starting => K.PrimaryButton(L.T("forward.state.starting"), () => { }, icon: "play", enabled: false),
            _ => K.PrimaryButton(L.T("forward.start"), () => app.Forwards.Start(id), icon: "play"),
        };
        var copy = K.SecondaryButton(L.T("forward.copy"), () => CopyAddress(app, id), icon: "copy", enabled: app.Forwards.AddressOf(id) != null);
        var col = new List<UIElement?>
        {
            K.Row(14, RuleIcon(state, 54, Th.Content),
                K.Grow(K.Col(4, K.Semi(Name(f), 21), K.Text(L.T("forward.viaName", Via(app, f)), 13, Th.Secondary))),
                K.SecondaryButton(L.T("hosts.edit"), () => Edit(app, id), icon: "pencil")),
            K.Row(8, action, copy, K.Spacer(), StateBadge(state)),
        };
        if (state == ForwardState.Failed && run?.Error is { Length: > 0 } err)
            col.Add(K.Box(K.Row(10, K.Top(K.Icon("warning", 16, Th.Danger)), K.Grow(K.Text(err, 13, Th.Text, wrap: true))),
                Th.B(Th.P.Danger, 0.08), 9, Th.B(Th.P.Danger, 0.35), 1, new Thickness(12)));
        col.Add(HostsViews.InfoGrid(new[]
        {
            (L.T("forward.local"), Local(app, id, f)),
            (L.T("forward.target"), Target(f)),
            (L.T("forward.via"), (viaDevice ? L.T("forward.via.device") : L.T("forward.via.ssh")) + " · " + Via(app, f)),
            (L.T("forward.autoStart"), f.AutoStart ? L.T("common.on") : L.T("common.off")),
        }));
        if (f.Note.Length > 0) col.Add(K.Col(6, K.SectionLabel(L.T("hosts.note")), K.Text(f.Note, 13, Th.Text, wrap: true)));
        col.Add(K.Row(0, K.Spacer(), K.SecondaryButton(L.T("common.delete"), () => AskDelete(app, id, Name(f)), destructive: true)));
        return K.PageScroll(K.Col(20, col), 900);
    }

    // ---------- 编辑 ----------

    private static FrameworkElement Labeled(string label, UIElement input, string? info = null)
        => K.Col(5, K.Row(4, K.Text(label, 12, Th.Secondary, FontWeights.Medium), info == null ? null : K.Info(info)), input);

    /// <summary>经由的选项：能被控的设备与 SSH 主机；当前值不在列表里（设备被移除）也保留，免得悄悄改掉</summary>
    private static List<(string, string)> ViaItems(AppModel app, string current)
    {
        var items = new List<(string, string)> { ("", L.T("forward.via.choose")) };
        items.AddRange(app.Devices.Where(x => !x.IsSelf && x.Kind != DeviceKind.Mobile).Select(x => ("dev:" + x.Id, L.T("forward.via.device") + " · " + app.DeviceName(x.Id))));
        items.AddRange(app.SshHosts.Select(e => ("ssh:" + e.Id, "SSH · " + app.SshLabel(e.Id))));
        if (current.Length > 0 && items.All(i => i.Item1 != current))
            items.Add((current, current.StartsWith("dev:", StringComparison.Ordinal) ? L.T("forward.via.device") + " · " + app.DeviceName(current.Substring(4)) : "SSH · " + app.SshLabel(current.Substring(4))));
        return items;
    }

    public static FrameworkElement Editor(AppModel app, EditTarget.Forward target)
    {
        var d = ForwardDraft.From(app, target.Id);
        var isNew = d.Id.Length == 0;
        Field Text(string value, Action<string> set, string placeholder = "", bool mono = false)
            => K.TextField(placeholder, value, v => { set(v); d.Notify(); }, mono);

        var label = Text(d.Label, v => d.Label = v, L.T("forward.label.placeholder"));
        var host = Text(d.RemoteHost, v => d.RemoteHost = v, "127.0.0.1", mono: true);
        var remotePort = Text(d.RemotePort, v => d.RemotePort = v, "5432", mono: true);
        remotePort.Width = 96;
        var localPort = Text(d.LocalPort, v => d.LocalPort = v, L.T("forward.local.auto"), mono: true);
        localPort.Width = 140;
        void Validate()
        {
            host.Invalid = d.RemoteHost.Length > 0 && !d.RemoteHostValid;
            remotePort.Invalid = d.RemotePort.Length > 0 && d.RemotePortValue == null;
            localPort.Invalid = d.LocalPortValue == null;
        }
        d.Watch(host, Validate);
        Validate();

        var fields = K.Col(14,
            Labeled(L.T("editor.label"), label),
            Labeled(L.T("forward.via"), K.Dropdown(ViaItems(app, d.Via), d.Via, v => { d.Via = v; d.Notify(); }), L.T("forward.via.info")),
            K.Row(8, K.Grow(Labeled(L.T("forward.target.host"), host, L.T("forward.target.info"))), Labeled(L.T("forward.target.port"), remotePort)),
            Labeled(L.T("forward.local.port"), K.Left(localPort), L.T("forward.local.info")),
            K.Row(6, K.Text(L.T("forward.autoStart"), 13), K.Info(L.T("forward.autoStart.info")), K.Spacer(), K.Switch(d.AutoStart, v => { d.AutoStart = v; d.Notify(); })),
            Labeled(L.T("hosts.note"), K.TextArea("", d.Note, 64, v => { d.Note = v; d.Notify(); })));

        var saving = false;
        Btn? save = null;
        void UpdateSave() { if (save != null) save.IsEnabled = d.IsValid && !saving; }
        save = K.PrimaryButton(L.T("common.save"), () =>
        {
            if (!d.IsValid || saving) return;
            saving = true;
            UpdateSave();
            var before = isNew ? null : app.Entry(d.Id)?.Data as VaultData.PortForward;
            Ui.Run(async () =>
            {
                var saved = await app.Save(d.Build());
                if (saved != null)
                {
                    app.SelectedForward = saved.Id;
                    app.Editing = null;
                    app.Nav.Notify();
                    app.ShowToast(L.T("common.saved"));
                    // 正在运行的规则改了映射：按新规则重开
                    if (before != null && saved.Data is VaultData.PortForward after && app.Forwards.StateOf(saved.Id) == ForwardState.Running && ForwardDraft.MappingChanged(before, after))
                        app.Forwards.Restart(saved.Id);
                }
                saving = false;
                UpdateSave();
            });
        });
        d.Watch(save, UpdateSave);
        UpdateSave();

        var footer = new List<UIElement?>();
        if (!isNew) footer.Add(K.SecondaryButton(L.T("common.delete"), () => AskDelete(app, d.Id, d.Label.Length > 0 ? d.Label : d.RemoteHost), destructive: true));
        footer.Add(K.Spacer());
        footer.Add(K.SecondaryButton(L.T("common.cancel"), () => { app.Editing = null; app.Nav.Notify(); }));
        footer.Add(save);

        var dock = new DockPanel();
        var head = new Border
        {
            Height = 46, Padding = new Thickness(16, 0, 10, 0), BorderBrush = Th.Separator, BorderThickness = new Thickness(0, 0, 0, 1),
            Child = K.Row(6, K.Grow(K.Semi(isNew ? L.T("forward.editor.new") : L.T("forward.editor.edit"), 14.5)), K.IconButton("close", L.T("common.close"), () => { app.Editing = null; app.Nav.Notify(); }, size: 12)),
        };
        DockPanel.SetDock(head, Dock.Top);
        dock.Children.Add(head);
        var foot = new Border { Padding = new Thickness(12), BorderBrush = Th.Separator, BorderThickness = new Thickness(0, 1, 0, 0), Child = K.Row(8, footer) };
        DockPanel.SetDock(foot, Dock.Bottom);
        dock.Children.Add(foot);
        dock.Children.Add(K.Scroll(K.Pad(fields, 16)));
        return dock;
    }
}
