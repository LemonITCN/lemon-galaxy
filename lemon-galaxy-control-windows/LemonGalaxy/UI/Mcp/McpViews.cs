using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using LemonGalaxy.Core;

namespace LemonGalaxy.UI;

/// <summary>
/// 「MCP」分区（docs/mcp.md 第 5 节）。中间列：概览入口、已连接的客户端、服务状态；
/// 首页自上而下：待批准（有才显示）、服务、连接方式、权限、已连接的客户端、操作记录。
/// 待批准卡片与操作记录按条目各自重绘（卡片里的输入框不会因为别的事件到来而丢焦点）。
/// </summary>
internal static class McpViews
{
    private const double LabelWidth = 72;

    // ---------- 中间列 ----------

    public static FrameworkElement Middle(AppModel app)
    {
        var m = app.Mcp;
        var nav = new Region(() =>
        {
            var sel = app.ActiveTabId == null;
            var b = new Btn
            {
                Height = 34, CornerRadius = new CornerRadius(7), Padding = new Thickness(10, 0, 8, 0),
                Normal = sel ? Th.Selection : Brushes.Transparent, HoverBrush = sel ? Th.Selection : Th.Hover,
                Child = K.Row(10, K.Icon("plug", 15, Th.Accent), K.Grow(K.Text(L.T("mcp.page.overview"), 13.5, Th.Text, FontWeights.SemiBold)),
                    m.PendingCount > 0 ? K.Badge(m.PendingCount.ToString(CultureInfo.InvariantCulture), Th.P.Danger) : null),
            };
            b.UpdateLook();
            b.Click += () => app.ActivateTab(null);
            return b;
        }).On(app.Nav, m.Changed);
        var clients = new Region(() => MiddleClients(app)).On(m.Changed);
        var status = new Region(() => StatusCard(app)).On(m.Changed, app.Settings);
        var dock = new DockPanel();
        var top = K.Col(0, Shell.ColumnHeader(L.T("nav.mcp")), K.HLine(), K.Margin(nav, 8, 8, 8, 0));
        DockPanel.SetDock(top, Dock.Top);
        dock.Children.Add(top);
        DockPanel.SetDock(status, Dock.Bottom);
        dock.Children.Add(status);
        dock.Children.Add(K.Scroll(clients));
        return dock;
    }

    private static FrameworkElement? MiddleClients(AppModel app)
    {
        var m = app.Mcp;
        if (m.Clients.Length == 0) return null;
        var items = new List<UIElement?> { K.Margin(K.SectionLabel(L.T("mcp.clients")), 10, 14, 0, 6) };
        foreach (var c in m.Clients)
        {
            UIElement? trailing = c.PendingApprovals > 0 ? K.Badge(c.PendingApprovals.ToString(CultureInfo.InvariantCulture), Th.P.Danger)
                : c.Running ? K.Tip(K.Spinner(11), L.T("mcp.client.running")) : null;
            var name = K.Text(c.Name.Length > 0 ? c.Name : c.Key, 13, Th.Text);
            items.Add(K.Tip(new Border
            {
                Height = 32, Padding = new Thickness(10, 0, 10, 0),
                Child = K.Row(9, K.StatusDot(c.PendingApprovals > 0 ? DotState.Busy : DotState.Online, 7, Th.List), K.Grow(name), trailing),
            }, c.Version.Length > 0 ? c.Name + " " + c.Version : c.Name));
        }
        return K.Margin(K.Col(0, items), 8, 0, 8, 8);
    }

    private static (string text, Color color, DotState dot) StateOf(McpHub m)
        => m.Busy ? (L.T("mcp.state.starting"), Th.P.Warning, DotState.Busy)
            : m.Running ? (L.T("mcp.state.runningPort", m.Status.Port), Th.P.Success, DotState.Online)
            : m.ShownError != null ? (L.T("mcp.state.error"), Th.P.Danger, DotState.Offline)
            : (L.T("mcp.state.off"), Th.P.TertiaryText, DotState.Offline);

    private static FrameworkElement StatusCard(AppModel app)
    {
        var (text, color, dot) = StateOf(app.Mcp);
        return new Border
        {
            Margin = new Thickness(12), Padding = new Thickness(12), Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(9),
            Child = K.Row(10, K.StatusDot(dot, 8, Th.Card), K.Grow(K.Col(2, K.Text(L.T("mcp.service.toggle"), 13, Th.Text, FontWeights.Medium), K.Text(text, 11.5, Th.B(color))))),
        };
    }

    // ---------- 首页 ----------

    public static FrameworkElement Home(AppModel app)
    {
        var m = app.Mcp;
        // 重新进页面时补齐操作记录（界面一直在收事件，正常不会漏）
        m.SyncHistory();
        // 各块带名字：selftest 按名字滚到对应位置截图
        var col = K.Col(22,
            new Region(() => Header(app)).Keyed(() => (m.Busy, m.Running, m.ShownError)).On(m.Changed),
            PendingSection(app).Also(e => e.Name = "McpPending"),
            new Region(() => Service(app)) { Name = "McpService" }.Keyed(() => (m.Enabled, m.Running, m.Busy, m.ShownError, m.EffectivePort, m.Status.Port)).On(m.Changed, app.Settings),
            new Region(() => Connection(app)) { Name = "McpConnection" }.Keyed(() => (m.Url, m.Token, m.TokenVisible, m.Running)).On(m.Changed),
            new Region(() => Permissions(app)) { Name = "McpPermissions" }.Keyed(() => (m.Mode, m.TrustMinutes, m.TrustUntilMs)).On(m.Changed, app.Settings),
            new Region(() => Clients(app)) { Name = "McpClients" }.Keyed(() => ClientsKey(m)).On(m.Changed),
            RecordsSection(app).Also(e => e.Name = "McpRecords"));
        var grid = new Grid { Background = Th.Content };
        grid.Children.Add(K.PageScroll(col, 860).Also(e => e.Name = "McpPage"));
        grid.Children.Add(new Region(() => Zoom(m)).Keyed(() => m.ZoomImage).On(m.Changed));
        return grid;
    }

    /// <summary>客户端列表的内容摘要：没变就不重建（标题旁的说明弹层不会被关掉）</summary>
    private static string ClientsKey(McpHub m)
        => m.Running + "|" + string.Join("|", m.Clients.Select(c => $"{c.Key},{c.Name},{c.Version},{c.Running},{c.PendingApprovals},{c.LastActiveMs}"));

    private static FrameworkElement Header(AppModel app)
        => K.Row(8, K.Semi(L.T("nav.mcp"), 21), K.Info(L.T("mcp.info")), K.Spacer(), StateBadge(app.Mcp));

    private static FrameworkElement StateBadge(McpHub m)
    {
        if (m.Busy) return K.Row(6, K.Spinner(13), K.Text(L.T("mcp.state.starting"), 12.5, Th.Secondary));
        if (m.Running) return K.Badge(L.T("mcp.state.running"), Th.P.Success, "checkCircle");
        if (m.ShownError != null) return K.Badge(L.T("mcp.state.error"), Th.P.Danger, "alertCircle");
        return K.Badge(L.T("mcp.state.off"), Th.P.TertiaryText);
    }

    private static FrameworkElement Section(string title, string? info, UIElement? trailing, params UIElement?[] content)
        => K.Col(10, K.Row(5, K.SectionLabel(title), info == null ? null : K.Info(info), K.Spacer(), trailing), K.Card(K.Col(14, content)));

    private static FrameworkElement FormRow(string label, UIElement content)
        => K.Row(12, K.Width(K.Text(label, 13, Th.Secondary), LabelWidth), K.Grow(content));

    /// <summary>可选中的等宽文字：滚轮交给页面</summary>
    private static TextBox Mono(string text, double size = 13, bool wrap = false)
        => K.SelectableText(text, size, Th.Text, mono: true, wrap: wrap).Also(Wheel.PassToParent);

    private static FrameworkElement CodeBox(string text)
        => K.Box(Mono(text, 12, wrap: true), Th.CodeBg, 7, null, 0, new Thickness(10, 8, 10, 8));

    private static Btn CopyIcon(McpHub m, string text, bool enabled = true)
    {
        var b = K.IconButton("copy", L.T("common.copy"), () => m.Copy(text), size: 13);
        b.IsEnabled = enabled;
        return b;
    }

    /// <summary>客户端名标签（卡片右上角）</summary>
    private static FrameworkElement ClientChip(string name)
    {
        var t = K.Text(name, 11.5, Th.Secondary, FontWeights.Medium);
        t.MaxWidth = 180;
        return new Border
        {
            Height = 22, CornerRadius = new CornerRadius(6), Padding = new Thickness(6, 0, 8, 0),
            Background = Th.B(Th.P.IsDark ? Palettes.A(Colors.White, 0.08) : Palettes.A(Colors.Black, 0.05)), BorderBrush = Th.Border, BorderThickness = new Thickness(1),
            Child = K.Row(5, K.Icon("plug", 11, Th.Tertiary, 2), t),
        };
    }

    // ---------- 待批准 ----------

    /// <summary>卡片按条目缓存：新的待批准到来时已有卡片（和里面输入了一半的密码、拒绝理由）原样保留</summary>
    private static FrameworkElement PendingSection(AppModel app)
    {
        var m = app.Mcp;
        var cards = new Dictionary<object, FrameworkElement>();
        var title = new Border();
        var stack = new Flex { Spacing = 10 };
        var root = K.Col(10, title, stack);
        void Sync()
        {
            var items = m.PendingItems();
            root.Visibility = items.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
            title.Child = K.Row(6, K.SectionLabel(L.T("mcp.pending")), K.Badge(items.Count.ToString(CultureInfo.InvariantCulture), Th.P.Danger, null, 10.5));
            foreach (var k in cards.Keys.Where(k => !items.Contains(k)).ToList()) cards.Remove(k);
            stack.Children.Clear();
            foreach (var it in items)
            {
                if (!cards.TryGetValue(it, out var card))
                {
                    card = it switch
                    {
                        McpCall c => new Region(() => ApprovalCard(app, c)).Keyed(() => c.Denying).On(c),
                        McpSecret s => SecretCard(app, s),
                        _ => K.Gap(0),
                    };
                    cards[it] = card;
                }
                Region.Detach(card);
                stack.Children.Add(card);
            }
        }
        Sync();
        m.Changed.Watch(root, () => Ui.Coalesce(root, Sync));
        return root;
    }

    private static string TargetIcon(AppModel app, string targetId)
        => app.Device(targetId) is { } d ? K.PlatformIcon(d.Platform) : targetId == AiHub.LocalId ? "display" : "server";

    /// <summary>
    /// 需要批准：客户端、目标、操作摘要、完整命令 / 改动、风险级别与原因、外部智能体给的理由；
    /// 按钮「允许一次」「本客户端同类都允许」「拒绝」（可填理由）。use_target / use_app 只有允许与拒绝。
    /// </summary>
    private static FrameworkElement ApprovalCard(AppModel app, McpCall c)
    {
        var m = app.Mcp;
        var call = c.Call;
        var client = m.ClientName(c.ClientKey);
        var rc = c.IsSimpleApproval ? Th.P.Accent : Img.RiskColor(call.Risk);
        var col = new List<UIElement?>();
        if (c.IsUseTarget)
        {
            col.Add(K.Row(9, K.Icon(TargetIcon(app, call.TargetId), 16, Th.Text, 1.9), K.Grow(K.Semi(L.T("mcp.useTarget.title", client, call.TargetName), 14)), ClientChip(client)));
            if (call.Detail.Length > 0) col.Add(K.Text(call.Detail, 12.5, Th.Secondary, wrap: true));
        }
        else if (c.IsUseApp)
        {
            col.Add(K.Row(9, K.Icon("appWindow", 16, Th.Text, 1.9), K.Semi(call.Summary.Length > 0 ? call.Summary : AiViews.ToolLabel(call.Tool), 14),
                call.TargetName.Length > 0 ? K.Row(4, K.Icon("arrowRight", 11, Th.Tertiary), K.Text(call.TargetName, 13, Th.Secondary)) : null,
                K.Spacer(), ClientChip(client)));
            if (call.Detail.Length > 0) col.Add(K.Text(call.Detail, 12.5, Th.Secondary, wrap: true));
        }
        else
        {
            col.Add(K.Row(8, K.Icon(AiViews.ToolIcon(call.Tool), 15, Th.Text, 1.9), K.Semi(AiViews.ToolLabel(call.Tool), 13.5),
                call.TargetName.Length > 0 ? K.Row(4, K.Icon("arrowRight", 11, Th.Tertiary), K.Text(call.TargetName, 13, Th.Secondary)) : null,
                K.Spacer(), ClientChip(client), K.Badge(L.T("ai.risk." + Img.RiskKey(call.Risk)), rc, Img.RiskIcon(call.Risk), 11)));
            col.Add(AiViews.CommandBox(call.Tool, call.Detail.Length > 0 ? call.Detail : call.Summary));
        }
        if (call.Image != null) col.Add(Thumb(m, call.Image));
        if (call.RiskReasons.Length > 0)
            col.Add(K.Col(3, call.RiskReasons.Select(r => (UIElement?)K.Row(7, K.Top(K.Margin(K.Icon(c.IsSimpleApproval ? "info" : Img.RiskIcon(call.Risk), 12, Th.B(rc)), 0, 2, 0, 0)),
                K.Grow(K.Text(r, 12.5, Th.Text, wrap: true))))));
        if (call.ModelReason.Length > 0) col.Add(ReasonRow(call.ModelReason));
        col.Add(ApprovalButtons(app, c));
        return new Border
        {
            Background = Th.Card, CornerRadius = new CornerRadius(12), BorderThickness = new Thickness(1.5), BorderBrush = Th.B(rc, 0.7),
            Padding = new Thickness(16, 13, 14, 13), Child = K.Col(11, col),
        };
    }

    private static FrameworkElement ReasonRow(string reason)
        => K.Tip(K.Row(7, K.Top(K.Margin(K.Icon("chatText", 12, Th.Tertiary), 0, 2, 0, 0)), K.Grow(K.Text(reason, 12.5, Th.Secondary, wrap: true))), L.T("mcp.reason"));

    private static FrameworkElement ApprovalButtons(AppModel app, McpCall c)
    {
        var m = app.Mcp;
        if (c.Denying)
        {
            string? Reason() => c.DenyReason.Trim().Length == 0 ? null : c.DenyReason.Trim();
            void Cancel() { c.Denying = false; c.Notify(); }
            var f = K.TextField(L.T("mcp.deny.reason"), c.DenyReason, v => c.DenyReason = v);
            f.Submitted += () => m.Decide(c, new ApprovalDecision.Deny(Reason()));
            f.Escaped += Cancel;
            f.Loaded += (_, _) => { if (!app.SelfTest) f.FocusInput(); };
            return K.Col(8, f, K.Row(8, K.Spacer(), K.SecondaryButton(L.T("common.cancel"), Cancel),
                K.PrimaryButton(L.T("ai.approve.deny"), () => m.Decide(c, new ApprovalDecision.Deny(Reason())), destructive: true)));
        }
        var deny = K.SecondaryButton(L.T("ai.approve.deny"), () => { c.Denying = true; c.Notify(); }, destructive: true);
        if (c.IsSimpleApproval)
            return K.Row(8, K.PrimaryButton(L.T("ai.approve.allow"), () => m.Decide(c, new ApprovalDecision.AllowOnce())), K.Spacer(), deny);
        return K.Row(8,
            K.PrimaryButton(L.T("ai.approve.once"), () => m.Decide(c, new ApprovalDecision.AllowOnce())),
            K.SecondaryButton(L.T("mcp.approve.similar"), () => m.Decide(c, new ApprovalDecision.AllowSimilar())),
            K.Spacer(), deny);
    }

    /// <summary>request_secret：安全输入框，内容由内核直接送到设备，不经过外部智能体</summary>
    private static FrameworkElement SecretCard(AppModel app, McpSecret s)
    {
        var m = app.Mcp;
        var target = s.TargetId.Length == 0 ? "" : app.Ai.WhereName(s.TargetId);
        var head = K.Row(8, K.Top(K.Margin(K.Icon("key", 15, Th.Warning), 0, 1, 0, 0)), K.Grow(K.Text(s.Prompt, 13.5, Th.Text, FontWeights.Medium, wrap: true)),
            ClientChip(m.ClientName(s.ClientKey)), K.Info(L.T("mcp.secret.info")));
        var pwd = K.PasswordField(L.T("ai.secret.placeholder"));
        void Submit()
        {
            if (pwd.Text.Length == 0) return;
            var v = pwd.Text;
            pwd.Text = "";
            m.ProvideSecret(s, v);
        }
        pwd.Submitted += Submit;
        pwd.Loaded += (_, _) => { if (!app.SelfTest) pwd.FocusInput(); };
        return new Border
        {
            Background = Th.Card, CornerRadius = new CornerRadius(12), Padding = new Thickness(16, 13, 14, 13), BorderThickness = new Thickness(1.5), BorderBrush = Th.B(Th.P.Warning, 0.6),
            Child = K.Col(11, head,
                target.Length == 0 ? null : K.Row(6, K.Icon("arrowRight", 12, Th.Tertiary), K.Text(L.T("ai.secret.target", target), 12, Th.Tertiary)),
                K.Row(8, K.Grow(pwd), K.Tip(K.SecondaryButton(L.T("common.cancel"), () => m.CancelSecret(s)), L.T("mcp.secret.cancel")), K.PrimaryButton(L.T("ai.secret.send"), Submit))),
        };
    }

    // ---------- 服务 ----------

    private static FrameworkElement Service(AppModel app)
    {
        var m = app.Mcp;
        var (text, color, dot) = StateOf(m);
        var tile = new Border
        {
            Width = 36, Height = 36, CornerRadius = new CornerRadius(9), Background = m.Running ? Th.B(Th.P.Accent, 0.12) : Th.Elevated,
            Child = K.Center(K.Icon("plug", 17, m.Running ? Th.Accent : Th.Secondary, 1.9)),
        };
        var head = K.Row(12, tile,
            K.Grow(K.Col(3, K.Text(L.T("mcp.service.toggle"), 13.5, Th.Text, FontWeights.Medium), K.Row(6, K.StatusDot(dot, 6, Th.Card), K.Text(text, 12, Th.B(color))))),
            m.Busy ? K.Spinner(14) : null, K.Switch(m.Enabled, m.SetEnabled));

        // 端口：改了点「应用」即重启
        var apply = K.SecondaryButton(L.T("common.apply"), m.ApplyPort);
        var field = K.TextField(m.DefaultPort.ToString(CultureInfo.InvariantCulture), m.PortDraft, mono: true, width: 96);
        void Update()
        {
            field.Invalid = !m.PortDraftValid;
            apply.IsEnabled = m.PortDraftChanged && !m.Busy;
        }
        field.Changed += v =>
        {
            m.PortDraft = v;
            Update();
        };
        field.Submitted += () => { if (m.PortDraftChanged && !m.Busy) m.ApplyPort(); };
        Update();
        var reset = m.EffectivePort != m.DefaultPort ? K.LinkButton(L.T("mcp.port.default", m.DefaultPort), m.ResetPort, Th.Accent) : null;
        var rows = new List<UIElement?> { head, K.HLine(), FormRow(L.T("mcp.port"), K.Row(8, field, apply, reset, K.Info(L.T("mcp.port.info")))) };
        if (m.ShownError is { } err)
            rows.Add(K.Row(8, K.Top(K.Margin(K.Icon("alertCircle", 14, Th.Danger), 0, 1, 0, 0)), K.Grow(K.Text(err, 12.5, Th.Danger, wrap: true))));
        return Section(L.T("mcp.service"), null, null, rows.ToArray());
    }

    // ---------- 连接方式 ----------

    private static FrameworkElement Connection(AppModel app)
    {
        var m = app.Mcp;
        var url = m.Url;
        var token = m.Token;
        var hasToken = token.Length > 0;
        var shown = !hasToken ? L.T("mcp.token.placeholder") : m.TokenVisible ? token : McpConfigs.Mask(token);

        var address = FormRow(L.T("mcp.address"), K.Row(6, K.Grow(Mono(url)), CopyIcon(m, url)));
        UIElement tokenValue;
        if (!hasToken) tokenValue = K.Row(6, K.Grow(K.Text(L.T("mcp.token.none"), 13, Th.Tertiary)));
        else
        {
            var eye = K.IconButton(m.TokenVisible ? "eyeOff" : "eye", m.TokenVisible ? L.T("mcp.token.hide") : L.T("mcp.token.show"), () =>
            {
                m.TokenVisible = !m.TokenVisible;
                m.Changed.Notify();
            }, size: 14);
            var value = m.TokenVisible ? (UIElement)Mono(token) : K.Text(shown, 13, Th.Text, mono: true);
            tokenValue = K.Row(6, K.Grow(value), eye, CopyIcon(m, token), K.Gap(2), K.SecondaryButton(L.T("mcp.token.regenerate"), m.AskRegenerateToken, icon: "refresh"));
        }
        var rows = new List<UIElement?> { address, FormRow(L.T("mcp.token"), tokenValue), K.HLine() };

        var exe = McpConfigs.ExePath;
        var configs = new (string title, string info, string show, string copy, bool needsToken)[]
        {
            (L.T("mcp.config.claudeCode"), L.T("mcp.config.claudeCode.info"), McpConfigs.ClaudeCode(url, shown), McpConfigs.ClaudeCode(url, token), true),
            (L.T("mcp.config.codex"), L.T("mcp.config.codex.info"), McpConfigs.Codex(url, shown), McpConfigs.Codex(url, token), true),
            (L.T("mcp.config.cursor"), L.T("mcp.config.cursor.info"), McpConfigs.Cursor(url, shown), McpConfigs.Cursor(url, token), true),
            (L.T("mcp.config.stdio"), L.T("mcp.config.stdio.info"), McpConfigs.Stdio(exe), McpConfigs.Stdio(exe), false),
        };
        foreach (var (title, info, show, copy, needsToken) in configs)
        {
            var btn = K.TintButton(L.T("common.copy"), () => m.Copy(copy), Th.P.Accent, "copy", 26);
            btn.IsEnabled = !needsToken || hasToken;
            rows.Add(K.Col(7, K.Row(6, K.Text(title, 13, Th.Text, FontWeights.Medium), K.Info(info), K.Spacer(), btn), CodeBox(show)));
        }
        return Section(L.T("mcp.connect"), L.T("mcp.connect.info"), null, rows.ToArray());
    }

    // ---------- 权限 ----------

    private static FrameworkElement Permissions(AppModel app)
    {
        var m = app.Mcp;
        var rows = new List<UIElement?> { AiViews.ModePicker(m.Mode, AiMode.FullTrust, mode => m.SetMode(mode, m.TrustMinutes)) };
        if (m.Mode == AiMode.FullTrust)
        {
            var seg = K.Segmented(new[] { (15, L.T("ai.trust.15m")), (60, L.T("ai.trust.1h")), (0, L.T("mcp.trust.unlimited")) }, m.TrustMinutes, v => m.SetMode(AiMode.FullTrust, v));
            var until = m.TrustMinutes > 0 && m.TrustUntilMs is { } ms && ms > Clock.NowMs
                ? K.Badge(L.T("ai.trust.until", Fmt.Local(ms).ToString("HH:mm", CultureInfo.InvariantCulture)), Th.P.Danger, "bolt", 11)
                : null;
            rows.Add(K.Row(10, K.Text(L.T("mcp.trust"), 13, Th.Secondary), seg, until));
        }
        return Section(L.T("mcp.mode"), L.T("mcp.mode.info"), null, rows.ToArray());
    }

    // ---------- 已连接的客户端 ----------

    private static FrameworkElement Clients(AppModel app)
    {
        var m = app.Mcp;
        var list = m.Clients;
        if (list.Length == 0)
            return Section(L.T("mcp.clients"), L.T("mcp.clients.info"), null, K.Text(m.Running ? L.T("mcp.clients.none") : L.T("mcp.clients.off"), 13, Th.Tertiary));
        var rows = new List<UIElement?>();
        for (var i = 0; i < list.Length; i++)
        {
            var c = list[i];
            var key = c.Key;
            var name = c.Name.Length > 0 ? c.Name : c.Key;
            var meta = new List<UIElement?> { K.Text(L.T("mcp.client.active", Fmt.Relative(c.LastActiveMs)), 12, Th.Tertiary) };
            if (c.Running) meta.Add(K.Row(4, K.Spinner(11), K.Text(L.T("mcp.client.running"), 12, Th.Secondary)));
            if (c.PendingApprovals > 0) meta.Add(K.Badge(L.T("mcp.client.pending", c.PendingApprovals), Th.P.Warning, "hand", 10.5));
            var stop = K.SecondaryButton(L.T("mcp.client.stop"), () => m.StopClient(key), destructive: true, icon: "stop");
            stop.IsEnabled = c.Running || c.PendingApprovals > 0;
            K.Tip(stop, L.T("mcp.client.stop.info"));
            rows.Add(new Border
            {
                Padding = new Thickness(14, 11, 12, 11),
                Child = K.Row(12, K.Avatar(name, 32),
                    K.Grow(K.Col(4, K.Row(6, K.Text(name, 13.5, Th.Text, FontWeights.Medium), c.Version.Length > 0 ? K.Text(c.Version, 12, Th.Tertiary, mono: true) : null), K.Row(10, meta))),
                    stop),
            });
            if (i < list.Length - 1) rows.Add(K.Margin(K.HLine(), 14, 0, 0, 0));
        }
        return K.Col(10, K.Row(5, K.SectionLabel(L.T("mcp.clients")), K.Info(L.T("mcp.clients.info"))), K.Card(K.Col(0, rows), 0));
    }

    // ---------- 操作记录 ----------

    /// <summary>最新在上，最多 200 条；每行按自己的状态重绘，列表只在增删时重排</summary>
    private static FrameworkElement RecordsSection(AppModel app)
    {
        var m = app.Mcp;
        var header = new Region(() => K.Row(5, K.SectionLabel(L.T("mcp.log")), K.Info(L.T("mcp.log.info")), K.Spacer(),
            m.Records.Count > 0 ? K.Text(L.T("mcp.log.count", m.Records.Count), 11.5, Th.Tertiary) : null)).On(m.LogChanged);
        var rows = new Dictionary<McpCall, FrameworkElement>();
        var list = new StackPanel();
        var card = new Border { Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(10), SnapsToDevicePixels = true, ClipToBounds = true };
        void Sync()
        {
            var items = m.Records;
            var alive = new HashSet<McpCall>(items);
            foreach (var k in rows.Keys.Where(k => !alive.Contains(k)).ToList()) rows.Remove(k);
            list.Children.Clear();
            for (var i = 0; i < items.Count; i++)
            {
                var c = items[i];
                if (!rows.TryGetValue(c, out var row))
                {
                    row = new Region(() => RecordRow(app, c)).On(c);
                    rows[c] = row;
                }
                if (i > 0) list.Children.Add(K.Margin(K.HLine(), 14, 0, 0, 0));
                Region.Detach(row);
                list.Children.Add(row);
            }
            card.Child = items.Count == 0 ? K.Pad(K.Text(L.T("mcp.log.none"), 13, Th.Tertiary), 16) : list;
        }
        Sync();
        m.LogChanged.Watch(card, () => Ui.Coalesce(card, Sync));
        return K.Col(10, header, card);
    }

    private static string Title(McpCall c)
    {
        var call = c.Call;
        if (c.IsUseTarget) return L.T("ai.useTarget.row", call.TargetName);
        return call.Summary.Length > 0 ? call.Summary.Replace('\n', ' ') : AiViews.ToolLabel(call.Tool);
    }

    private static FrameworkElement Outcome(McpCall c)
    {
        if (c.Phase == McpPhase.Pending && c.Result == null) return K.Badge(L.T("mcp.result.pending"), Th.P.Warning, "hand", 10.5);
        return c.Outcome switch
        {
            McpOutcome.Success => K.Badge(L.T("mcp.result.success"), Th.P.Success, null, 10.5),
            McpOutcome.Failed => K.Badge(L.T("mcp.result.failed"), Th.P.Danger, null, 10.5),
            McpOutcome.Denied => K.Badge(L.T("mcp.result.denied"), Th.P.Warning, null, 10.5),
            McpOutcome.Cancelled => K.Badge(L.T("mcp.result.cancelled"), Th.P.TertiaryText, null, 10.5),
            _ => K.Row(4, K.Spinner(11), K.Text(L.T("mcp.result.running"), 11.5, Th.Secondary)),
        };
    }

    private static FrameworkElement RecordRow(AppModel app, McpCall c)
    {
        var m = app.Mcp;
        var call = c.Call;
        var time = K.Text(c.Time is { } t ? Fmt.Time(t) : "—", 12, Th.Tertiary, mono: true);
        time.Width = 62;
        var title = K.Text(Title(c), 12.5, c.Outcome is McpOutcome.Failed ? Th.Danger : Th.Text, mono: call.Tool == "run_command");
        UIElement? target = null;
        if (call.TargetName.Length > 0 && !c.IsUseTarget)
        {
            var tt = K.Text(call.TargetName, 11.5, Th.Tertiary);
            tt.MaxWidth = 130;
            target = tt;
        }
        var client = K.Text(m.ClientName(c.ClientKey), 11.5, Th.Tertiary);
        client.MaxWidth = 120;
        var head = new Btn
        {
            Padding = new Thickness(14, 8, 12, 8), HoverBrush = Th.Hover, HorizontalAlignment = HorizontalAlignment.Stretch,
            Child = K.Row(10, time, K.Icon(AiViews.ToolIcon(call.Tool), 13, Th.Tertiary, 1.9), K.Grow(title), target, client, Outcome(c),
                K.Icon(c.Expanded ? "chevronDown" : "chevronRight", 10, Th.Tertiary, 2.4)),
        };
        head.UpdateLook();
        K.Tip(head, AiViews.ToolLabel(call.Tool));
        head.Click += () =>
        {
            c.Expanded = !c.Expanded;
            c.Notify();
        };
        if (!c.Expanded) return head;
        // 详情与标题左对齐：左内边距 14 + 时间 62 + 间距 10 + 图标 13 + 间距 10
        return K.Col(0, head, K.Margin(RecordDetail(m, c), 109, 0, 14, 12));
    }

    /// <summary>展开后的详情：完整命令或参数、风险原因、外部智能体的理由、输出、截图、结果说明</summary>
    private static FrameworkElement RecordDetail(McpHub m, McpCall c)
    {
        var call = c.Call;
        var col = new List<UIElement?>();
        var command = call.Detail.Length > 0 ? call.Detail : call.Summary;
        if (command.Length > 0 && !c.IsSimpleApproval) col.Add(AiViews.CommandBox(call.Tool, command));
        if (c.IsSimpleApproval && call.Detail.Length > 0) col.Add(K.Text(call.Detail, 12.5, Th.Secondary, wrap: true));
        if (call.RiskReasons.Length > 0 || call.Risk is RiskLevel.Dangerous or RiskLevel.Unknown or RiskLevel.Forbidden) col.Add(AiViews.RiskRow(call));
        if (call.ModelReason.Length > 0) col.Add(ReasonRow(call.ModelReason));
        if (call.Image != null) col.Add(Thumb(m, call.Image));
        var outcome = c.Outcome;
        if (outcome == McpOutcome.Denied)
        {
            // 用户拒绝时内核把理由放在 output 里
            var why = c.Result?.Output.Trim() ?? "";
            col.Add(K.Row(6, K.Icon("xCircle", 12, Th.Warning), K.Grow(K.Text(why.Length > 0 ? L.T("mcp.denied.reason", why) : c.Result?.Summary ?? "", 12, Th.Secondary, wrap: true))));
        }
        else if (outcome == McpOutcome.Cancelled)
            col.Add(K.Row(6, K.Icon("ban", 12, Th.Tertiary), K.Grow(K.Text(L.T("mcp.result.cancelled.info"), 12, Th.Tertiary, wrap: true))));
        else
        {
            if (c.Output.Length > 0)
            {
                var trimmed = c.Output.Trim('\n', '\r');
                var lines = trimmed.Split('\n');
                const int max = 12;
                var shown = c.OutputExpanded || lines.Length <= max ? trimmed : string.Join("\n", lines.Skip(lines.Length - max));
                var box = K.SelectableText(shown, 12, Th.Secondary, mono: true, wrap: false).Also(Wheel.PassToParent);
                box.HorizontalScrollBarVisibility = ScrollBarVisibility.Auto;
                var outCol = new List<UIElement?> { K.Box(box, Th.CodeBg, 7, null, 0, new Thickness(10, 8, 10, 8)) };
                if (lines.Length > max)
                    outCol.Add(K.Left(K.LinkButton(c.OutputExpanded ? L.T("ai.output.less") : L.T("ai.output.all", lines.Length), () =>
                    {
                        c.OutputExpanded = !c.OutputExpanded;
                        c.Notify();
                    }, Th.Accent)));
                col.Add(K.Col(4, outCol));
            }
            if (c.Result?.Image is { } img) col.Add(Thumb(m, img));
            if (c.Result is { Summary.Length: > 0 } res)
                col.Add(K.Row(6, K.Icon(res.Ok ? "checkCircle" : "xCircle", 12, res.Ok ? Th.Success : Th.Danger), K.Grow(K.Text(res.Summary, 12, Th.Secondary, wrap: true))));
        }
        if (col.Count == 0) col.Add(K.Text(AiViews.ToolLabel(call.Tool), 12, Th.Tertiary));
        return K.Col(8, col);
    }

    private static FrameworkElement Thumb(McpHub m, ImageData img)
    {
        var src = Img.From(img);
        if (src == null) return K.Gap(0);
        var image = new Image { Source = src, Stretch = Stretch.Uniform, MaxWidth = 340, MaxHeight = 200, HorizontalAlignment = HorizontalAlignment.Left };
        var b = new Btn { Child = new Border { CornerRadius = new CornerRadius(6), BorderBrush = Th.Border, BorderThickness = new Thickness(1), Child = image, ClipToBounds = true }, HorizontalAlignment = HorizontalAlignment.Left };
        K.Tip(b, L.T("ai.enlarge"));
        b.Click += () =>
        {
            m.ZoomImage = img;
            m.Changed.Notify();
        };
        return b;
    }

    private static FrameworkElement? Zoom(McpHub m)
    {
        if (m.ZoomImage is not { } img || Img.From(img) is not { } src) return null;
        var g = new Grid { Background = Th.B(Colors.Black, 0.8), Cursor = Cursors.Hand };
        g.Children.Add(new Image { Source = src, Stretch = Stretch.Uniform, Margin = new Thickness(30) });
        g.MouseLeftButtonDown += (_, _) =>
        {
            m.ZoomImage = null;
            m.Changed.Notify();
        };
        return g;
    }
}
