using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using LemonGalaxy.Core;

namespace LemonGalaxy.UI;

internal enum TerminalPhase { Connecting, Running, Ended }

/// <summary>终端标签：原始字节经内核 TerminalChannel / TerminalSink 往来，屏幕模型与绘制在 TerminalControl。</summary>
internal sealed class TerminalTab : TabModel
{
    public TerminalPhase Phase = TerminalPhase.Connecting;
    public string EndReason = "";
    public PeerConnection? Peer;
    public SshConnection? Ssh;
    public string SessionTitle = "";
    public readonly TerminalControl Term;
    private ICoreTerminal? channel;
    private TerminalSessionInfo? attach;
    private readonly string? themeOverride;
    private SinkAdapter? sink;

    public TerminalTab(AppModel app, string? deviceId, string? sshHostId, string title, TerminalSessionInfo? attach) : base(app, TabKind.Terminal, title)
    {
        DeviceId = deviceId;
        SshHostId = sshHostId;
        this.attach = attach;
        if (sshHostId != null && app.Entry(sshHostId)?.Data is VaultData.SshHost h) themeOverride = h.TerminalTheme;
        Term = new TerminalControl(app.Client.CreateScreen(24, 80, 10_000));
        Term.Input += data => channel?.Write(data);
        Term.Resized += (cols, rows) => channel?.Resize(cols, rows);
        Term.TitleChanged += t => { SessionTitle = t; Notify(); };
        ApplyAppearance();
        app.Settings.Watch(Term, ApplyAppearance);
    }

    public TerminalTheme Theme => TerminalTheme.Named(themeOverride ?? App.Settings.TerminalTheme);

    private void ApplyAppearance()
    {
        var s = App.Settings;
        Term.Theme = Theme;
        Term.CursorBlink = s.TerminalCursorBlink;
        Term.CopyOnSelect = s.TerminalCopyOnSelect;
        Term.RightClickPaste = s.TerminalRightClickPaste;
        Term.SetFont(TerminalFonts.Family(s.TerminalFont), s.TerminalFontSize);
        Term.InvalidateVisual();
        Notify();
    }

    public void Start()
    {
        Phase = TerminalPhase.Connecting;
        Live = null;
        Notify();
        var adapter = new SinkAdapter(this);
        sink = adapter;
        var opts = new TerminalOptions(attach?.Id, null, Term.Cols, Term.Rows, new[] { new EnvVar("TERM", "xterm-256color"), new EnvVar("COLORTERM", "truecolor") });
        Ui.Run(async () =>
        {
            try
            {
                if (DeviceId != null)
                {
                    var p = await App.Peer(DeviceId);
                    Peer = p;
                    channel = await p.Session.OpenTerminal(opts, adapter);
                    SessionTitle = attach?.Title ?? channel.Title();
                }
                else if (SshHostId != null)
                {
                    var s = await App.Ssh(SshHostId);
                    Ssh = s;
                    // SSH 没有断线继承
                    channel = await s.Session.OpenTerminal(opts with { AttachSession = null }, adapter);
                }
                // 打开期间窗口尺寸可能变了
                channel?.Resize(Term.Cols, Term.Rows);
                Phase = TerminalPhase.Running;
                Live = true;
                Notify();
                if (!App.SelfTest) Ui.After(50, () => Term.Focus());
            }
            catch (Exception e)
            {
                EndReason = Fmt.Error(e);
                Phase = TerminalPhase.Ended;
                Live = false;
                Notify();
            }
        });
    }

    public void Reconnect(TerminalSessionInfo? info = null)
    {
        try { channel?.Detach(); } catch { }
        channel = null;
        attach = info;
        Term.Screen.Feed(System.Text.Encoding.ASCII.GetBytes("\r\n"));
        Term.Changed();
        Start();
    }

    private void Exited(int? code)
    {
        EndReason = code is { } c ? L.T("terminal.exited.code", c) : L.T("terminal.exited");
        Phase = TerminalPhase.Ended;
        Live = false;
        channel = null;
        Notify();
    }

    public void SendText(string s) => channel?.Write(System.Text.Encoding.UTF8.GetBytes(s));

    public override void Close()
    {
        // 关闭标签只断开，被控端保留会话，下次可以接续（SSH 没有继承，直接结束）
        try { channel?.Detach(); } catch { }
        channel = null;
    }

    public void Kill()
    {
        try { channel?.Kill(); } catch { }
        channel = null;
        EndReason = L.T("terminal.killed");
        Phase = TerminalPhase.Ended;
        Live = false;
        Notify();
    }

    public override void Activated()
    {
        if (!App.SelfTest && Phase == TerminalPhase.Running) Ui.After(30, () => Term.Focus());
    }

    protected override FrameworkElement CreateView() => TerminalView.Build(App, this);

    /// <summary>内核线程 → 屏幕模型（线程安全）→ 合并到界面刷新</summary>
    private sealed class SinkAdapter : TerminalSink
    {
        private readonly WeakReference<TerminalTab> owner;
        public SinkAdapter(TerminalTab t) { owner = new WeakReference<TerminalTab>(t); }

        public void OnOutput(byte[] data)
        {
            if (!owner.TryGetTarget(out var t) || !ReferenceEquals(t.sink, this)) return;
            t.Term.Screen.Feed(data);
            t.Term.Changed();
        }

        public void OnExit(int? code) => Ui.Post(() =>
        {
            if (owner.TryGetTarget(out var t) && ReferenceEquals(t.sink, this)) t.Exited(code);
        });
    }
}

internal static class TerminalView
{
    public static FrameworkElement Build(AppModel app, TerminalTab tab)
    {
        var dock = new DockPanel();
        var header = new Region(() => Header(app, tab)).On(tab, app.Data);
        DockPanel.SetDock(header, Dock.Top);
        dock.Children.Add(header);
        var ended = new Region(() => tab.Phase == TerminalPhase.Ended ? EndedBar(app, tab) : null).On(tab);
        DockPanel.SetDock(ended, Dock.Bottom);
        dock.Children.Add(ended);
        var area = new Grid();
        var bg = new Region(() => new Border { Background = Th.B(tab.Theme.Background) }).On(tab);
        area.Children.Add(bg);
        Region.Detach(tab.Term);
        area.Children.Add(tab.Term);
        area.Children.Add(new Region(() => tab.Phase == TerminalPhase.Connecting ? K.Center(K.Spinner(20)) : null).On(tab));
        dock.Children.Add(area);
        return dock;
    }

    private static FrameworkElement Header(AppModel app, TerminalTab tab)
    {
        var platform = tab.DeviceId != null ? app.Device(tab.DeviceId)?.Platform ?? tab.Peer?.Info.Platform ?? Platform.Linux : Platform.Linux;
        var items = new List<UIElement?>
        {
            K.PlatformGlyph(platform, 14),
            K.Text(tab.Title, 13, Th.Text, FontWeights.SemiBold),
            tab.SessionTitle.Length > 0 ? K.Grow(K.Text(tab.SessionTitle, 12.5, Th.Tertiary)) : K.Spacer(),
        };
        if (tab.Peer != null) items.Add(PathBadge(tab.Peer));
        else if (tab.SshHostId != null) items.Add(SshBadge(app, tab));
        items.Add(K.MenuButton(K.Icon("code", 15), () =>
        {
            var list = app.Snippets.Where(e => e.Data is VaultData.Snippet).Select(e =>
            {
                var s = (VaultData.Snippet)e.Data;
                return new MenuEntry(s.Label, () => tab.SendText(s.Command + "\r"), tab.Phase == TerminalPhase.Running);
            }).ToList();
            if (list.Count == 0) list.Add(new MenuEntry(L.T("snippet.empty"), Enabled: false));
            return list;
        }, L.T("terminal.snippets")));
        items.Add(K.MenuButton(K.Icon("more", 16, Th.Secondary, 3), () =>
        {
            var list = new List<MenuEntry>
            {
                new(L.T("terminal.copy") + "\tCtrl+Shift+C", tab.Term.CopySelection, tab.Term.SelectedText().Length > 0),
                new(L.T("terminal.paste") + "\tCtrl+Shift+V", tab.Term.Paste, tab.Phase == TerminalPhase.Running),
                MenuEntry.Sep,
            };
            if (tab.DeviceId is { } id)
            {
                list.Add(new(L.T("terminal.newSession"), () => app.OpenTerminal(id)));
                list.Add(new(L.T("terminal.reattach"), () => ShowSessions(app, id)));
                list.Add(MenuEntry.Sep);
                list.Add(new(L.T("terminal.kill"), tab.Kill, tab.Phase == TerminalPhase.Running, Destructive: true));
            }
            else if (tab.SshHostId is { } h) list.Add(new(L.T("terminal.newSession"), () => app.OpenSshTerminal(h)));
            return list;
        }));
        return new Border
        {
            Height = 36, Padding = new Thickness(12, 0, 8, 0), Background = Th.Content, BorderBrush = Th.Separator, BorderThickness = new Thickness(0, 0, 0, 1),
            Child = K.Row(10, items),
        };
    }

    private static void ShowSessions(AppModel app, string deviceId) => Ui.Run(async () =>
    {
        var p = await app.Peer(deviceId);
        var list = (await p.Session.ListTerminals()).Where(s => !s.Attached).ToArray();
        app.ShowTerminalSessions(deviceId, list);
    });

    public static (string label, Color color, string icon) DescribePath(PathKind k) => k switch
    {
        PathKind.Lan => (L.T("path.lan"), Th.P.Success, "wifi"),
        PathKind.Direct => (L.T("path.direct"), Th.P.Info, "direct"),
        PathKind.RelayUdp => (L.T("path.relayUdp"), Th.P.Warning, "relay"),
        PathKind.RelayTcp => (L.T("path.relayTcp"), Th.P.Warning, "relay"),
        PathKind.Closed => (L.T("path.closed"), Th.P.Danger, "close"),
        _ => (L.T("path.connecting"), Th.P.TertiaryText, "more"),
    };

    /// <summary>连接路径（局域网 / 直连 / 中转）与延迟</summary>
    public static FrameworkElement PathBadge(PeerConnection peer) => new Region(() =>
    {
        var (label, color, icon) = DescribePath(peer.Path.Kind);
        var b = new Border
        {
            Background = Th.B(color, 0.12), CornerRadius = new CornerRadius(11), Padding = new Thickness(8, 3, 8, 3),
            Child = K.Row(5, K.Icon(icon, 11, Th.B(color), 2.4), K.Text(label, 11.5, Th.B(color), FontWeights.Medium), K.Text($"{peer.Path.RttMs} ms", 11.5, Th.Secondary)),
        };
        return K.Tip(b, peer.Path.RemoteAddr);
    }).On(peer);

    private static FrameworkElement SshBadge(AppModel app, TerminalTab tab)
    {
        var text = L.T("path.ssh");
        if (app.Entry(tab.SshHostId ?? "")?.Data is VaultData.SshHost h)
        {
            if (h.JumpDeviceId != null) text += " · " + L.T("ssh.via.device", app.DeviceName(h.JumpDeviceId));
            else if (h.JumpHostId != null) text += " · " + L.T("ssh.via.host", app.SshLabel(h.JumpHostId));
        }
        var color = Th.P.Info;
        return new Border
        {
            Background = Th.B(color, 0.12), CornerRadius = new CornerRadius(11), Padding = new Thickness(8, 3, 8, 3),
            Child = K.Row(5, K.Icon("lock", 11, Th.B(color), 2.4), K.Text(text, 11.5, Th.B(color), FontWeights.Medium)),
        };
    }

    private static FrameworkElement EndedBar(AppModel app, TerminalTab tab)
    {
        var items = new List<UIElement?> { K.Icon("power", 15, Th.Warning), K.Grow(K.Text(tab.EndReason, 13, Th.Text)) };
        if (tab.DeviceId is { } id) items.Add(K.SecondaryButton(L.T("terminal.reattach"), () => ShowSessions(app, id)));
        items.Add(K.PrimaryButton(L.T("terminal.reconnect"), () => tab.Reconnect()));
        return new Border
        {
            Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(0, 1, 0, 0), Padding = new Thickness(14, 8, 14, 8),
            Child = K.Row(10, items),
        };
    }
}
