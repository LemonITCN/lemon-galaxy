using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;

namespace LemonGalaxy.UI;

/// <summary>AI 智能体：对话列表、新建对话、对话视图（流式消息、工具确认卡片、提问、密码请求、权限档位）</summary>
internal static class AiViews
{
    // ---------- 中间列：对话列表 ----------

    public static FrameworkElement Middle(AppModel app)
    {
        var ai = app.Ai;
        ai.LoadModels();
        ai.ReloadSummaries();
        var list = new Region(() =>
        {
            var items = new List<UIElement?>();
            if (ai.Conversations.Count > 0)
            {
                items.Add(K.Margin(K.SectionLabel(L.T("ai.open")), 10, 8, 0, 4));
                foreach (var c in ai.Conversations) items.Add(ConversationRow(app, c));
            }
            if (ai.Summaries.Length > 0)
            {
                items.Add(K.Margin(K.SectionLabel(L.T("ai.history")), 10, 12, 0, 4));
                foreach (var s in ai.Summaries)
                {
                    var b = new Btn
                    {
                        CornerRadius = new CornerRadius(7), Padding = new Thickness(10, 6, 10, 6), HoverBrush = Th.Hover,
                        Child = K.Col(2, K.Text(s.Title, 13), K.Text(Fmt.Date(s.UpdatedAt) + " · " + string.Join(", ", s.TargetNames), 11.5, Th.Tertiary)),
                    };
                    b.UpdateLook();
                    var sum = s;
                    b.Click += () => ai.Reopen(sum);
                    K.ContextMenuFor(b, () => new List<MenuEntry>
                    {
                        new(L.T("sftp.open"), () => ai.Reopen(sum)),
                        new(L.T("common.delete"), () => app.Confirm(L.T("common.delete"), sum.Title, L.T("common.delete"), () => ai.Delete(sum)), Destructive: true),
                    });
                    items.Add(b);
                }
            }
            if (items.Count == 0) items.Add(K.Margin(K.Text(L.T("ai.noConversations"), 12.5, Th.Tertiary), 14, 14, 14, 14));
            return K.Margin(K.Col(2, items), 6);
        }).On(ai.Changed, app.Nav);
        foreach (var c in ai.Conversations) c.Watch(list, list.Invalidate);
        var dock = new DockPanel();
        var top = K.Col(0, Shell.ColumnHeader(L.T("nav.ai"), K.IconButton("squarePen", L.T("ai.new"), () => app.SelectHome(NavSection.Ai), size: 15)), K.HLine());
        DockPanel.SetDock(top, Dock.Top);
        dock.Children.Add(top);
        dock.Children.Add(K.Scroll(list));
        return dock;
    }

    private static FrameworkElement ConversationRow(AppModel app, AiConversationTab c)
    {
        var selected = app.ActiveTabId == c.Id;
        FrameworkElement? state = c.Running ? K.Spinner(12) : c.HasPendingApproval ? K.StatusDot(DotState.Busy, 7) : null;
        var b = new Btn
        {
            CornerRadius = new CornerRadius(7), Padding = new Thickness(10, 7, 10, 7), Normal = selected ? Th.Selection : Brushes.Transparent, HoverBrush = selected ? Th.Selection : Th.Hover,
            Child = K.Row(9, K.Top(K.Margin(K.Icon("chat", 13), 0, 2, 0, 0)),
                K.Grow(K.Col(3, K.Text(c.Title, 13, Th.Text, FontWeights.Medium), K.Text(c.Model.Name + " · " + string.Join(", ", c.TargetNames), 11.5, Th.Tertiary))), state),
        };
        b.UpdateLook();
        b.Click += () => app.Ai.Focus(c);
        return b;
    }

    // ---------- 新建对话 ----------

    public static FrameworkElement Home(AppModel app)
    {
        var ai = app.Ai;
        ai.LoadModels();
        var prompt = K.TextArea(L.T("ai.task.placeholder"), ai.DraftPrompt, 120, null);
        prompt.CtrlEnterSubmits = true;
        Btn? start = null;
        void UpdateStart() { if (start != null) start.IsEnabled = ai.CanStart; }
        start = K.PrimaryButton(L.T("ai.start") + "  Ctrl+Enter", () => ai.Start(), icon: "arrowUp");
        prompt.Changed += v => { ai.DraftPrompt = v; UpdateStart(); };
        prompt.Submitted += () => { if (ai.CanStart) ai.Start(); };
        ai.Changed.Watch(start, () =>
        {
            UpdateStart();
            if (prompt.Text != ai.DraftPrompt) prompt.Text = ai.DraftPrompt;
        });
        UpdateStart();
        var sections = new Region(() =>
        {
            if (app.User?.AiEnabled == false || ai.Models?.Enabled == false)
                return K.Card(K.Row(8, K.Icon("ban", 15), K.Text(L.T("ai.disabled"), 13, Th.Secondary)));
            return K.Col(22, ModelSection(app), TargetSection(app), ModeSection(app));
        }).On(ai.Changed, app.Data);
        // 任务输入框所在区域只在 AI 开关变化时重建（输入中不丢焦点）
        bool Disabled() => app.User?.AiEnabled == false || ai.Models?.Enabled == false;
        var promptSection = new Region(() => Disabled() ? null
            : K.Col(10, K.Semi(L.T("ai.task"), 13.5), prompt, K.Row(0, K.Spacer(), start))).Keyed(() => Disabled()).On(ai.Changed, app.Data);
        return K.PageScroll(K.Col(22,
            K.Row(10, K.Semi(L.T("ai.new"), 21), K.Info(L.T("ai.new.info")), K.Spacer()),
            sections, promptSection), 860);
    }

    private static FrameworkElement Title(string t, string? info = null) => K.Row(5, K.Semi(t, 13.5), info == null ? null : K.Info(info));

    private static FrameworkElement ModelSection(AppModel app)
    {
        var ai = app.Ai;
        var col = new List<UIElement?> { Title(L.T("ai.model"), L.T("ai.model.info")) };
        if (ai.LoadingModels) col.Add(K.Spinner(16));
        if (ai.ModelsError != null) col.Add(K.Text(ai.ModelsError, 12.5, Th.Danger, wrap: true));
        if (ai.Models != null)
        {
            var grid = new AdaptiveGrid { MinItem = 250, Spacing = 10 };
            foreach (var m in ai.Models.Items)
            {
                var sel = ai.DraftModel == AiHub.Key(m);
                var radio = new Border
                {
                    Width = 16, Height = 16, CornerRadius = new CornerRadius(8), BorderThickness = new Thickness(sel ? 5 : 1.5), BorderBrush = sel ? Th.Accent : Th.Tertiary,
                };
                var b = new Btn
                {
                    Padding = new Thickness(11, 10, 11, 10), CornerRadius = new CornerRadius(9), BorderThickness = new Thickness(sel ? 1.5 : 1), Normal = Th.Card,
                    NormalBorder = sel ? Th.Accent : Th.Border, HoverBorder = sel ? Th.Accent : Th.B(Th.P.Accent, 0.5), HoverBrush = Th.Card,
                    Child = K.Row(10, radio, K.Grow(K.Col(3, K.Semi(m.Name, 13), K.Text($"{m.ProviderName} · {m.ContextLength / 1000}K", 11.5, Th.Tertiary))),
                        m.Vision ? K.Badge(L.T("ai.vision"), Th.P.Accent, "eye", 10.5) : null),
                };
                b.UpdateLook();
                var key = AiHub.Key(m);
                b.Click += () => { ai.DraftModel = key; ai.Changed.Notify(); };
                grid.Children.Add(b);
            }
            col.Add(grid);
            if (ai.SelectedModel is { Vision: false } && ai.DraftTargets.Any(t => IsGui(app, t)))
                col.Add(K.Row(6, K.Icon("info", 14, Th.Warning), K.Grow(K.Text(L.T("ai.noVisionHint"), 12.5, Th.Warning, wrap: true))));
        }
        return K.Col(10, col);
    }

    private static bool IsGui(AppModel app, AgentTarget t) => t.Kind == AgentTargetKind.Device && app.Device(t.Id) is { } d && d.Platform is Platform.Windows or Platform.Macos;

    private static FrameworkElement TargetSection(AppModel app)
    {
        var ai = app.Ai;
        var flow = new FlowPanel { Spacing = 6 };
        foreach (var d in app.Devices.Where(x => !x.IsSelf && x.Kind != DeviceKind.Mobile))
            flow.Children.Add(Chip(app, new AgentTarget(AgentTargetKind.Device, d.Id), app.DeviceName(d.Id), d.Online || d.LanSeen, K.PlatformIcon(d.Platform)));
        foreach (var e in app.SshHosts)
            flow.Children.Add(Chip(app, new AgentTarget(AgentTargetKind.SshHost, e.Id), app.SshLabel(e.Id), true, null));
        return K.Col(10, Title(L.T("ai.targets"), L.T("ai.targets.info")), flow);
    }

    private static FrameworkElement Chip(AppModel app, AgentTarget t, string name, bool enabled, string? icon)
    {
        var ai = app.Ai;
        var sel = ai.DraftTargets.Contains(t);
        var fg = sel ? Th.AccentText : enabled ? Th.Text : Th.Tertiary;
        UIElement glyph = icon != null ? K.Icon(icon, 12, sel ? Th.AccentText : Th.Secondary, 1.8)
            : new TextBlock { Text = "SSH", FontSize = 8.5, FontWeight = FontWeights.Bold, Foreground = sel ? Th.AccentText : Th.Secondary, VerticalAlignment = VerticalAlignment.Center };
        var b = new Btn
        {
            Height = 28, Padding = new Thickness(10, 0, 10, 0), CornerRadius = new CornerRadius(14), BorderThickness = new Thickness(sel ? 0 : 1),
            Normal = sel ? Th.Accent : Th.Card, HoverBrush = sel ? Th.Accent : Th.Elevated, NormalBorder = Th.Border,
            Child = K.Row(6, glyph, K.Text(name, 12.5, fg), sel ? K.Icon("check", 10, Th.AccentText, 3) : null),
        };
        b.UpdateLook();
        b.IsEnabled = enabled;
        b.Click += () => ai.ToggleTarget(t);
        return b;
    }

    private static string ModeIcon(AiMode m) => m switch
    {
        AiMode.ReadOnly => "eye",
        AiMode.ConfirmAll => "hand",
        AiMode.ConfirmDangerous => "shieldAlert",
        AiMode.FullTrust => "bolt",
        _ => "ban",
    };

    /// <summary>权限档位分段选择；超过管理员上限的档位置灰</summary>
    public static FrameworkElement ModePicker(AiMode mode, AiMode max, Action<AiMode> pick)
    {
        var grid = new UniformGridLite(AiModes.Selectable.Length);
        foreach (var m in AiModes.Selectable)
        {
            var sel = mode == m;
            var allowed = m.Rank() <= max.Rank();
            var color = m == AiMode.FullTrust ? Th.P.Danger : Th.P.Accent;
            var fg = sel ? (m == AiMode.FullTrust ? Brushes.White : Th.AccentText) : allowed ? Th.Text : Th.Tertiary;
            var b = new Btn
            {
                Height = 54, CornerRadius = new CornerRadius(7), Normal = sel ? Th.B(color) : Brushes.Transparent, HoverBrush = sel ? Th.B(color) : Th.Hover2,
                Child = K.Center(K.Col(4, K.Center(K.Icon(ModeIcon(m), 15, fg, 1.8)), K.Center(K.Text(L.T(m.LabelKey()), 12, fg, sel ? FontWeights.SemiBold : FontWeights.Normal)))),
            };
            b.UpdateLook();
            b.IsEnabled = allowed;
            var mm = m;
            b.Click += () => pick(mm);
            grid.Add(b);
        }
        return new Border { Padding = new Thickness(3), Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(9), MaxWidth = 580, HorizontalAlignment = HorizontalAlignment.Left, Child = grid.Panel };
    }

    private static FrameworkElement ModeSection(AppModel app)
    {
        var ai = app.Ai;
        var col = new List<UIElement?>
        {
            Title(L.T("ai.mode"), L.T("ai.mode.info")),
            ModePicker(ai.DraftMode, ai.Models?.MaxMode ?? AiMode.FullTrust, m => { ai.DraftMode = m; ai.Changed.Notify(); }),
        };
        if (ai.DraftMode == AiMode.FullTrust)
            col.Add(K.Row(8, K.Text(L.T("ai.trust.limit"), 13, Th.Secondary), K.Dropdown(new[]
            {
                (0, L.T("ai.trust.none")), (15, L.T("ai.trust.minutes", 15)), (60, L.T("ai.trust.hours", 1)), (240, L.T("ai.trust.hours", 4)),
            }, ai.DraftTrustMinutes, v => { ai.DraftTrustMinutes = v; ai.Changed.Notify(); }, 140)));
        col.Add(K.Width(K.Row(6, K.Text(L.T("ai.planFirst"), 13), K.Info(L.T("ai.planFirst.info")), K.Spacer(), K.Switch(ai.DraftPlanFirst, v => ai.DraftPlanFirst = v)), 360).Also(r => r.HorizontalAlignment = HorizontalAlignment.Left));
        return K.Col(10, col);
    }

    // ---------- 对话视图 ----------

    public static FrameworkElement Conversation(AppModel app, AiConversationTab tab)
    {
        var dock = new DockPanel { Background = Th.Content };
        var header = new Region(() => Header(tab)).On(tab);
        DockPanel.SetDock(header, Dock.Top);
        dock.Children.Add(header);
        dock.Children.Add(Composer(tab).Also(c => DockPanel.SetDock(c, Dock.Bottom)));

        // 条目只追加：已有条目各自按自己的状态重绘，输入框（拒绝理由、修改命令）不会因为别的消息到来而丢焦点
        var stack = new StackPanel { Margin = new Thickness(24, 18, 24, 18), MaxWidth = 860 };
        var rendered = 0;
        var scroll = new ScrollViewer { Content = stack, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, Focusable = false };
        var running = new Region(() => tab.Running
            ? K.Row(8, K.Spinner(14), K.Text(tab.Preparing ? L.T("state.connecting") : L.T("ai.working", tab.Step), 12.5, Th.Tertiary))
            : null).On(tab);
        var stickBottom = true;
        scroll.ScrollChanged += (_, e) =>
        {
            if (e.ExtentHeightChange == 0) stickBottom = scroll.VerticalOffset >= scroll.ScrollableHeight - 24;
            else if (stickBottom) scroll.ScrollToEnd();
        };
        void Sync()
        {
            stack.Children.Remove(running);
            for (; rendered < tab.Items.Count; rendered++)
                stack.Children.Add(K.Margin(ItemView(app, tab, tab.Items[rendered]), 0, 0, 0, 14));
            stack.Children.Add(running);
        }
        Sync();
        tab.ItemsChanged.Watch(stack, () => Ui.Coalesce(stack, Sync));
        var area = new Grid();
        area.Children.Add(scroll);
        area.Children.Add(new Region(() => Overlay(app, tab)).On(tab));
        dock.Children.Add(area);
        return dock;
    }

    private static FrameworkElement Header(AiConversationTab tab)
    {
        var names = new StackPanel { Orientation = Orientation.Horizontal };
        foreach (var n in tab.TargetNames) names.Children.Add(K.Margin(K.TagChip(n), 0, 0, 4, 0));
        var info = K.Col(3, K.Semi(tab.Title, 14), K.Row(6, K.Text(tab.Model.Name, 11.5, Th.Secondary), tab.Model.Vision ? K.Icon("eye", 11) : null, names));
        var modeLabel = L.T(tab.Mode.LabelKey()) + (tab.Mode == AiMode.FullTrust && tab.TrustMinutes > 0 ? $" · {tab.TrustMinutes} min" : "");
        var modeColor = tab.Mode == AiMode.FullTrust ? Th.Danger : Th.Text;
        var mode = K.MenuButton(K.Row(5, K.Icon(tab.Mode == AiMode.FullTrust ? "bolt" : "shield", 13, modeColor), K.Text(modeLabel, 12.5, modeColor, FontWeights.Medium), K.Icon("chevronDown", 10, Th.Tertiary, 2.4)), () =>
            AiModes.Selectable.Select(m => m == AiMode.FullTrust
                ? new MenuEntry(L.T(m.LabelKey()), Checked: tab.Mode == m, Sub: new List<MenuEntry>
                {
                    new(L.T("ai.trust.none"), () => tab.ChangeMode(AiMode.FullTrust, 0)),
                    new(L.T("ai.trust.minutes", 15), () => tab.ChangeMode(AiMode.FullTrust, 15)),
                    new(L.T("ai.trust.hours", 1), () => tab.ChangeMode(AiMode.FullTrust, 60)),
                })
                : new MenuEntry(L.T(m.LabelKey()), () => tab.ChangeMode(m, 0), Checked: tab.Mode == m)).ToList(), L.T("ai.mode"));
        var effective = tab.EffectiveMode is { } eff && eff != tab.Mode ? K.Tip(K.Badge(L.T(eff.LabelKey()), Th.P.Warning, "shieldAlert", 10.5), L.T("ai.mode.effective")) : null;
        var tokens = K.Tip(K.Row(4, K.Icon("hash", 11, Th.Tertiary), K.Text(L.T("ai.tokens", Fmt.Kilo(tab.PromptTokens), Fmt.Kilo(tab.CompletionTokens)), 11.5, Th.Tertiary)), L.T("ai.tokens.help"));
        var stop = tab.Running ? K.TintButton(L.T("ai.stop"), tab.Stop, Th.P.Danger, "stop") : null;
        return new Border
        {
            Height = 56, Padding = new Thickness(16, 0, 16, 0), BorderBrush = Th.Separator, BorderThickness = new Thickness(0, 0, 0, 1),
            Child = K.Row(10, K.Grow(info), effective, mode, tokens, stop),
        };
    }

    private static FrameworkElement Composer(AiConversationTab tab)
    {
        var f = K.TextArea(L.T("ai.composer.placeholder"), tab.Composer, 40);
        f.Height = double.NaN;
        f.MinHeight = 40;
        f.MaxHeight = 120;
        f.CtrlEnterSubmits = true;
        Btn? send = null;
        void Update() { if (send != null) send.IsEnabled = !tab.Running && tab.Composer.Trim().Length > 0; }
        void Send()
        {
            if (tab.Running || tab.Composer.Trim().Length == 0) return;
            var t = tab.Composer;
            tab.Composer = "";
            f.Text = "";
            tab.Send(t);
        }
        send = new Btn { Width = 34, Height = 34, CornerRadius = new CornerRadius(17), Normal = Th.Accent, HoverBrush = Th.B(K.Mix(Th.P.Accent, Colors.Black, 0.1)), Child = K.Center(K.Icon("arrowUp", 15, Th.AccentText, 2.6)) };
        send.UpdateLook();
        K.Tip(send, "Ctrl+Enter");
        send.Click += Send;
        f.Changed += v => { tab.Composer = v; Update(); };
        f.Submitted += Send;
        tab.Watch(send, Update);
        Update();
        var row = K.Row(8, K.Grow(f), K.Top(send).Also(s => s.VerticalAlignment = VerticalAlignment.Bottom));
        return new Border
        {
            Background = Th.Content, BorderBrush = Th.Separator, BorderThickness = new Thickness(0, 1, 0, 0), Padding = new Thickness(24, 12, 24, 12),
            Child = new Border { MaxWidth = 860, Child = row },
        };
    }

    private static FrameworkElement ItemView(AppModel app, AiConversationTab tab, ChatItem item)
    {
        switch (item)
        {
            case ChatUser u:
                return new Border
                {
                    HorizontalAlignment = HorizontalAlignment.Right, MaxWidth = 640, Margin = new Thickness(80, 0, 0, 0), Background = Th.B(Th.P.Accent, 0.14), CornerRadius = new CornerRadius(12),
                    Padding = new Thickness(12, 8, 12, 8), Child = K.SelectableText(u.Text, 13.5),
                };
            case ChatText t when t.Thinking:
                return new Region(() =>
                {
                    var head = new Btn
                    {
                        HoverBrush = Th.Hover, CornerRadius = new CornerRadius(5), Padding = new Thickness(4, 2, 6, 2), HorizontalAlignment = HorizontalAlignment.Left,
                        Child = K.Row(5, K.Icon(t.Expanded ? "chevronDown" : "chevronRight", 10, Th.Tertiary, 2.4), K.Icon("bulb", 12, Th.Tertiary), K.Text(L.T("ai.thinking"), 12, Th.Tertiary)),
                    };
                    head.UpdateLook();
                    head.Click += () => { t.Expanded = !t.Expanded; t.Notify(); };
                    return K.Col(4, head, t.Expanded ? K.Margin(K.Text(t.Text, 12.5, Th.Tertiary, wrap: true).Also(x => x.FontStyle = FontStyles.Italic), 22, 0, 0, 0) : null);
                }).On(t);
            case ChatText t:
                return new Region(() => K.SelectableText(t.Text, 13.5)).On(t);
            case ToolCard c:
                return new Region(() => ToolCardView(app, tab, c)).Keyed(() => (c.Decision, c.Editing, c.Denying, c.Phase, c.Expanded, c.Output.Length, c.Result != null)).On(c);
            case AskItem a:
                return new Region(() => AskView(tab, a)).Keyed(() => a.Answer).On(a);
            case SecretItem s:
                return new Region(() => SecretRow(tab, s)).On(s);
            case ChatError e:
                return K.Row(6, K.Top(K.Icon("xCircle", 15, Th.Danger)), K.Grow(K.Text(e.Text, 13, Th.Danger, wrap: true)));
        }
        return K.Gap(0);
    }

    private static string ToolIcon(string tool) => tool switch
    {
        "run_command" => "terminal",
        "screenshot" => "camera",
        "mouse" => "cursor",
        "keyboard" => "keyboard",
        "read_file" or "list_dir" or "search_files" => "fileText",
        "write_file" or "edit_file" => "squarePen",
        "ui_tree" => "list",
        "terminal_open" or "terminal_send" or "terminal_read" or "terminal_close" => "terminal",
        "upload_file" => "upload",
        "download_file" => "download",
        "wait" => "clock",
        _ => "wrench",
    };

    private static FrameworkElement Thumb(AiConversationTab tab, ImageData img)
    {
        var src = Img.From(img);
        if (src == null) return K.Gap(0);
        var image = new Image { Source = src, Stretch = Stretch.Uniform, MaxWidth = 340, MaxHeight = 200, HorizontalAlignment = HorizontalAlignment.Left };
        var b = new Btn { Child = new Border { CornerRadius = new CornerRadius(6), BorderBrush = Th.Border, BorderThickness = new Thickness(1), Child = image, ClipToBounds = true }, HorizontalAlignment = HorizontalAlignment.Left, Cursor = Cursors.Hand };
        K.Tip(b, L.T("ai.enlarge"));
        b.Click += () => { tab.ZoomImage = img; tab.Notify(); };
        return b;
    }

    /// <summary>工具调用卡片：工具、目标、命令、风险级别与原因、模型说明、输出、截图；需要确认时带操作按钮</summary>
    private static FrameworkElement ToolCardView(AppModel app, AiConversationTab tab, ToolCard c)
    {
        var call = c.Call;
        var risk = call.Risk;
        var rc = Img.RiskColor(risk);
        var head = K.Row(8,
            K.Icon(ToolIcon(call.Tool), 14),
            K.Semi(L.T("ai.tool." + call.Tool), 12.5),
            K.Text(call.Tool, 11.5, Th.Tertiary, mono: true),
            K.Text("→", 12, Th.Tertiary),
            K.Grow(K.Text(call.TargetName, 12.5, Th.Secondary)),
            K.Badge(L.T("ai.risk." + Img.RiskKey(risk)), rc, Img.RiskIcon(risk), 11));
        var body = new List<UIElement?>
        {
            K.Box(K.SelectableText(c.EditedCommand ?? call.Summary, 12.5, Th.Text, mono: true), Th.CodeBg, 7, null, 0, new Thickness(10, 8, 10, 8)),
        };
        if (call.ModelReason.Length > 0) body.Add(K.Row(6, K.Top(K.Margin(K.Icon("chatText", 12, Th.Tertiary), 0, 2, 0, 0)), K.Grow(K.Text(call.ModelReason, 12.5, Th.Secondary, wrap: true))));
        if (call.RiskReasons.Length > 0 && (risk is RiskLevel.Dangerous or RiskLevel.Unknown or RiskLevel.Forbidden || c.Decision == ToolDecision.Pending))
            body.Add(K.Col(3, call.RiskReasons.Select(r => (UIElement?)K.Row(6, K.Top(K.Margin(K.Icon(Img.RiskIcon(risk), 11, Th.B(rc)), 0, 3, 0, 0)), K.Grow(K.Text(r, 12.5, Th.Text, wrap: true))))));
        if (call.Image != null) body.Add(Thumb(tab, call.Image));
        if (c.Decision == ToolDecision.Pending) body.Add(ApprovalArea(tab, c));
        body.Add(StatusRow(c, rc));
        if (c.Output.Length > 0 || c.Result?.Image != null) body.Add(OutputArea(tab, c));
        var card = new Border
        {
            Background = Th.Card, CornerRadius = new CornerRadius(10), BorderThickness = new Thickness(c.Decision == ToolDecision.Pending ? 1.5 : 1),
            BorderBrush = c.Decision == ToolDecision.Pending ? Th.B(rc, 0.75) : Th.Border,
            Child = K.Col(10, K.Margin(head, 0, 0, 0, 0), K.Col(10, body)),
            Padding = new Thickness(14, 10, 12, 12),
        };
        var g = new Grid();
        g.Children.Add(card);
        g.Children.Add(new Border { Width = 3, CornerRadius = new CornerRadius(2), Background = Th.B(rc), HorizontalAlignment = HorizontalAlignment.Left, Margin = new Thickness(0, 10, 0, 10) });
        return g;
    }

    private static FrameworkElement ApprovalArea(AiConversationTab tab, ToolCard c)
    {
        if (c.Editing)
        {
            var f = K.TextArea("", c.EditText, 70, v => c.EditText = v, mono: true, fontSize: 12.5);
            return K.Col(8, f, K.Row(8, K.Spacer(), K.SecondaryButton(L.T("common.cancel"), () => { c.Editing = false; c.Notify(); }),
                K.PrimaryButton(L.T("ai.approve.runEdited"), () => tab.Decide(c, new ApprovalDecision.EditAndRun(c.EditText)))));
        }
        if (c.Denying)
        {
            var f = K.TextField(L.T("ai.deny.reason"), c.DenyReason, v => c.DenyReason = v);
            f.Submitted += () => tab.Decide(c, new ApprovalDecision.Deny(c.DenyReason.Trim().Length == 0 ? null : c.DenyReason.Trim()));
            return K.Col(8, f, K.Row(8, K.Spacer(), K.SecondaryButton(L.T("common.cancel"), () => { c.Denying = false; c.Notify(); }),
                K.PrimaryButton(L.T("ai.approve.deny"), () => tab.Decide(c, new ApprovalDecision.Deny(c.DenyReason.Trim().Length == 0 ? null : c.DenyReason.Trim())), destructive: true)));
        }
        return K.Row(8,
            K.PrimaryButton(L.T("ai.approve.once"), () => tab.Decide(c, new ApprovalDecision.AllowOnce())),
            K.SecondaryButton(L.T("ai.approve.similar"), () => tab.Decide(c, new ApprovalDecision.AllowSimilar())),
            c.Call.Tool == "run_command" ? K.SecondaryButton(L.T("ai.approve.edit"), () => { c.Editing = true; c.Notify(); }) : null,
            K.Spacer(),
            K.SecondaryButton(L.T("ai.approve.deny"), () => { c.Denying = true; c.Notify(); }, destructive: true));
    }

    private static FrameworkElement StatusRow(ToolCard c, Color rc)
    {
        var left = new List<UIElement?>();
        switch (c.Decision)
        {
            case ToolDecision.ApprovedSimilar: left.Add(K.Row(4, K.Icon("checkCircle", 13, Th.Tertiary), K.Text(L.T("ai.decision.similar"), 12, Th.Tertiary))); break;
            case ToolDecision.Edited: left.Add(K.Row(4, K.Icon("pencil", 12, Th.Tertiary), K.Text(L.T("ai.decision.edited"), 12, Th.Tertiary))); break;
            case ToolDecision.Denied when c.Result?.Denied != true || c.DenyReasonSent != null || c.Result == null:
                left.Add(K.Row(4, K.Icon("xCircle", 13, Th.Danger), K.Text(L.T("ai.decision.denied") + (c.DenyReasonSent is { } r ? (L.IsChinese ? "：" : ": ") + r : ""), 12, Th.Danger)));
                break;
        }
        var right = new List<UIElement?>();
        switch (c.Phase)
        {
            case ToolPhase.Running:
                right.Add(K.Spinner(12));
                right.Add(K.Text(L.T("ai.tool.running"), 12, Th.Tertiary));
                break;
            case ToolPhase.Finished when c.Result is { } res:
                if (!res.Denied)
                {
                    right.Add(K.Icon(res.Ok ? "checkCircle" : "xCircle", 13, res.Ok ? Th.Success : Th.Danger));
                    right.Add(K.Text(res.Summary, 12, Th.Secondary));
                }
                else if (c.Decision is ToolDecision.Auto or ToolDecision.Denied && c.DenyReasonSent == null)
                {
                    right.Add(K.Icon("ban", 13, Th.Danger));
                    right.Add(K.Text(res.Summary, 12, Th.Danger));
                }
                break;
            case ToolPhase.Proposed when c.Decision == ToolDecision.Pending:
                right.Add(K.Icon("hourglass", 12, Th.B(rc)));
                right.Add(K.Text(L.T("ai.waitingApproval"), 12, Th.B(rc), FontWeights.Medium));
                break;
        }
        if (left.Count == 0 && right.Count == 0) return K.Gap(0);
        return K.Row(6, new UIElement?[] { K.Row(6, left), K.Spacer() }.Concat(right).ToArray());
    }

    private static FrameworkElement OutputArea(AiConversationTab tab, ToolCard c)
    {
        var col = new List<UIElement?>();
        if (c.Output.Length > 0)
        {
            var trimmed = c.Output.Trim('\n', '\r');
            var lines = trimmed.Split('\n');
            var head = new Btn
            {
                HoverBrush = Th.Hover, CornerRadius = new CornerRadius(5), Padding = new Thickness(2, 2, 6, 2), HorizontalAlignment = HorizontalAlignment.Left,
                Child = K.Row(4, K.Icon(c.Expanded ? "chevronDown" : "chevronRight", 10, Th.Secondary, 2.4), K.Text(L.T("ai.output", lines.Length), 12, Th.Secondary)),
            };
            head.UpdateLook();
            head.Click += () => { c.Expanded = !c.Expanded; c.Notify(); };
            var shown = c.Expanded ? trimmed : string.Join("\n", lines.Skip(Math.Max(0, lines.Length - (c.Phase == ToolPhase.Running ? 4 : 3))));
            col.Add(head);
            col.Add(K.Box(K.SelectableText(shown, 12, Th.Secondary, mono: true), Th.CodeBg, 6, null, 0, new Thickness(8)));
        }
        if (c.Result?.Image is { } img) col.Add(Thumb(tab, img));
        return K.Col(6, col);
    }

    private static FrameworkElement AskView(AiConversationTab tab, AskItem a)
    {
        var col = new List<UIElement?> { K.Row(6, K.Icon("ask", 15, Th.Accent), K.Grow(K.Text(a.Question, 13.5, Th.Text, FontWeights.Medium, wrap: true))) };
        if (a.Answer != null) col.Add(K.Row(6, K.Icon("reply", 13, Th.Secondary), K.Text(a.Answer, 12.5, Th.Secondary)));
        else
        {
            var row = new FlowPanel { Spacing = 8 };
            for (var i = 0; i < a.Options.Length; i++)
            {
                var o = a.Options[i];
                row.Children.Add(i == 0 ? K.PrimaryButton(o, () => tab.Answer(a, o)) : K.SecondaryButton(o, () => tab.Answer(a, o)));
            }
            var other = K.TextField(L.T("ai.ask.other"), a.FreeText, v => a.FreeText = v, width: 220);
            other.Submitted += () => { if (a.FreeText.Trim().Length > 0) tab.Answer(a, a.FreeText.Trim()); };
            row.Children.Add(other);
            col.Add(row);
        }
        return new Border
        {
            Background = Th.Card, CornerRadius = new CornerRadius(10), Padding = new Thickness(12), BorderThickness = new Thickness(a.Answer == null ? 1.5 : 1),
            BorderBrush = a.Answer == null ? Th.B(Th.P.Accent, 0.6) : Th.Border, Child = K.Col(10, col),
        };
    }

    private static FrameworkElement SecretRow(AiConversationTab tab, SecretItem s)
    {
        UIElement right = s.Done ? K.Row(4, K.Icon("check", 12, Th.Success, 2.6), K.Text(L.T("ai.secret.sent"), 12, Th.Success))
            : s.Cancelled ? K.Text(L.T("ai.secret.cancelled"), 12, Th.Tertiary)
            : K.PrimaryButton(L.T("ai.secret.enter"), () => { tab.ActiveSecret = s; tab.Notify(); });
        return new Border
        {
            Background = Th.B(Th.P.Warning, 0.08), BorderBrush = Th.B(Th.P.Warning, 0.4), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(10), Padding = new Thickness(12),
            Child = K.Row(8, K.Icon("key", 15, Th.Warning), K.Grow(K.Text(s.Prompt, 13, Th.Text, wrap: true)), right),
        };
    }

    /// <summary>对话内的浮层：request_secret 的安全输入框、截图放大</summary>
    private static FrameworkElement? Overlay(AppModel app, AiConversationTab tab)
    {
        if (tab.ActiveSecret is { } req)
        {
            var pwd = K.PasswordField();
            void Submit()
            {
                if (pwd.Text.Length == 0) return;
                var v = pwd.Text;
                pwd.Text = "";
                tab.ProvideSecret(req, v);
            }
            pwd.Submitted += Submit;
            pwd.Escaped += () => tab.CancelSecret(req);
            var kind = app.Device(req.TargetId) != null ? AgentTargetKind.Device : AgentTargetKind.SshHost;
            var card = new Border
            {
                Width = 420, Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(12), Padding = new Thickness(20, 18, 20, 18),
                HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center,
                Effect = new System.Windows.Media.Effects.DropShadowEffect { BlurRadius = 26, ShadowDepth = 6, Direction = 270, Opacity = 0.25 },
                Child = K.Col(14,
                    K.Row(8, K.Icon("shieldCheck", 19, Th.Accent), K.Semi(L.T("ai.secret.title"), 15), K.Info(L.T("ai.secret.info"))),
                    K.Text(req.Prompt, 13, Th.Secondary, wrap: true),
                    K.Row(6, K.Icon("arrowRight", 12, Th.Tertiary), K.Text(L.T("ai.secret.target", app.Ai.TargetName(new AgentTarget(kind, req.TargetId))), 12, Th.Tertiary)),
                    pwd,
                    Modals.Buttons(K.SecondaryButton(L.T("common.cancel"), () => tab.CancelSecret(req)), K.PrimaryButton(L.T("ai.secret.send"), Submit))),
            };
            card.Loaded += (_, _) => pwd.FocusInput();
            return new Grid { Children = { new Border { Background = Th.Scrim }, card } };
        }
        if (tab.ZoomImage is { } img && Img.From(img) is { } src)
        {
            var g = new Grid { Background = Th.B(Colors.Black, 0.8), Cursor = Cursors.Hand };
            g.Children.Add(new Image { Source = src, Stretch = Stretch.Uniform, Margin = new Thickness(30) });
            g.MouseLeftButtonDown += (_, _) => { tab.ZoomImage = null; tab.Notify(); };
            return g;
        }
        return null;
    }
}

/// <summary>等宽多列（档位选择用）</summary>
internal sealed class UniformGridLite
{
    public readonly Grid Panel = new();
    private int count;

    public UniformGridLite(int columns)
    {
        for (var i = 0; i < columns; i++) Panel.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
    }

    public void Add(UIElement e)
    {
        Grid.SetColumn(e, count++);
        Panel.Children.Add(e);
    }
}
