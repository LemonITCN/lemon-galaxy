using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using LemonGalaxy.Core;

namespace LemonGalaxy.UI;

/// <summary>
/// AI 聊天页面（docs/ai-chat-ui.md）：左侧对话历史，右侧消息流 + 输入框。
/// 左侧用主界面的中间列（可收起），右侧是 AI 分区的首页。
/// </summary>
internal static class AiViews
{
    public const double ListWidth = 240;
    private const double ContentWidth = 760;

    // ---------- 左侧：对话历史 ----------

    public static FrameworkElement Middle(AppModel app)
    {
        var ai = app.Ai;
        ai.ReloadSummaries();
        var newChat = new Btn
        {
            Height = 32, CornerRadius = new CornerRadius(7), Padding = new Thickness(10, 0, 10, 0), HoverBrush = Th.Hover2,
            Child = K.Row(9, K.Icon("squarePen", 14, Th.Text), K.Grow(K.Text(L.T("ai.new"), 13, Th.Text, FontWeights.Medium)), K.Text("Ctrl+N", 11, Th.Tertiary)),
        };
        newChat.UpdateLook();
        newChat.Click += ai.NewChat;
        Region? list = null;
        list = new Region(() => HistoryList(app, list!), immediate: false).On(ai.Changed, app.Nav);
        list.Refresh();
        var search = K.SearchField(L.T("ai.search"), ai.Search, v => { ai.Search = v; list.Invalidate(); }, out _);
        var dock = new DockPanel();
        var top = K.Col(6, Shell.ColumnHeader(L.T("nav.ai")), K.Margin(newChat, 8, 0, 8, 0), K.Margin(search, 10, 2, 10, 6));
        DockPanel.SetDock(top, Dock.Top);
        dock.Children.Add(top);
        dock.Children.Add(K.Scroll(list));
        return dock;
    }

    private static FrameworkElement HistoryList(AppModel app, Region list)
    {
        var ai = app.Ai;
        foreach (var c in ai.Live) c.Watch(list, list.Invalidate);
        var rows = ai.Rows();
        var items = new List<UIElement?>();
        if (rows.Count == 0)
        {
            items.Add(K.Margin(K.Text(ai.Search.Trim().Length > 0 ? L.T("ai.pick.noMatch") : L.T("ai.noConversations"), 12.5, Th.Tertiary), 12, 10, 12, 10));
            return K.Margin(K.Col(1, items), 6, 0, 6, 8);
        }
        var today = Fmt.Local(Clock.NowMs).Date;
        string Group(long ms)
        {
            var d = Fmt.Local(ms).Date;
            if (d >= today) return "ai.group.today";
            if (d >= today.AddDays(-1)) return "ai.group.yesterday";
            if (d >= today.AddDays(-7)) return "ai.group.week";
            return "ai.group.earlier";
        }
        string? last = null;
        foreach (var r in rows)
        {
            var g = Group(r.UpdatedAt);
            if (g != last)
            {
                items.Add(K.Margin(K.Text(L.T(g), 11.5, Th.Tertiary, FontWeights.SemiBold), 10, last == null ? 4 : 12, 0, 4));
                last = g;
            }
            items.Add(HistoryRow(app, r));
        }
        return K.Margin(K.Col(1, items), 6, 0, 6, 8);
    }

    private static FrameworkElement HistoryRow(AppModel app, AiRow r)
    {
        var ai = app.Ai;
        var selected = ai.Current?.Id == r.Id && app.ActiveTabId == null;
        FrameworkElement? state = r.Live switch
        {
            { Running: true } c when c.HasPendingApproval => K.Tip(K.StatusDot(DotState.Busy, 6, selected ? Th.Selection : Th.List), L.T("ai.waitingApproval")),
            { Running: true } => K.Tip(K.Spinner(11), L.T("ai.running")),
            _ => null,
        };
        var title = K.Text(r.Title.Length == 0 ? L.T("ai.newConversation") : r.Title, 13, Th.Text, selected ? FontWeights.Medium : FontWeights.Normal);
        var time = Fmt.Local(r.UpdatedAt);
        var when = time.Date == Fmt.Local(Clock.NowMs).Date ? time.ToString("HH:mm", CultureInfo.InvariantCulture) : time.ToString("MM-dd", CultureInfo.InvariantCulture);
        var b = new Btn
        {
            Height = 34, CornerRadius = new CornerRadius(7), Padding = new Thickness(10, 0, 8, 0),
            Normal = selected ? Th.Selection : Brushes.Transparent, HoverBrush = selected ? Th.Selection : Th.Hover,
            Child = K.Row(8, K.Grow(title), state ?? K.Text(when, 11, Th.Tertiary)),
        };
        b.UpdateLook();
        K.Tip(b, r.Title);
        var id = r.Id;
        b.Click += () =>
        {
            if (app.ActiveTabId != null) app.ActivateTab(null);
            ai.Select(id);
        };
        K.ContextMenuFor(b, () => new List<MenuEntry>
        {
            new(L.T("common.delete"), () => app.Confirm(L.T("ai.delete.title"), L.T("ai.delete.message", r.Title), L.T("common.delete"), () => ai.Delete(id)), Destructive: true),
        });
        return b;
    }

    // ---------- 右侧 ----------

    public static FrameworkElement Home(AppModel app)
    {
        var ai = app.Ai;
        ai.LoadModels();
        return new Region(() => Pane(app)).Keyed(() => (ai.Current, ai.Current == null ? ai.Availability : AiAvailability.Ready)).On(ai.Changed, app.Data);
    }

    private static FrameworkElement Pane(AppModel app)
    {
        var ai = app.Ai;
        var chat = ai.Current;
        if (chat != null) return ChatView(app, chat);
        if (ai.Availability != AiAvailability.Ready) return new Grid { Background = Th.Content, Children = { StateView(app) } };
        // 新对话：问候 + 输入框居中
        var col = K.Col(22, K.Center(K.Text(L.T("ai.greeting"), 24, Th.Text, FontWeights.SemiBold)), Composer(app, null));
        col.MaxWidth = ContentWidth;
        col.Margin = new Thickness(24, 0, 24, 60);
        col.VerticalAlignment = VerticalAlignment.Center;
        return new Grid { Background = Th.Content, Children = { col } };
    }

    /// <summary>模型列表加载中 / 失败 / 没开通 AI / 没有可用模型</summary>
    private static FrameworkElement StateView(AppModel app)
    {
        var ai = app.Ai;
        FrameworkElement Message(string icon, string text, bool retry)
        {
            var col = K.Col(14,
                K.Center(K.Icon(icon, 30, Th.Tertiary, 1.5)),
                K.Text(text, 14, Th.Secondary, wrap: true).Also(t => t.TextAlignment = TextAlignment.Center),
                retry ? K.Center(K.SecondaryButton(L.T("common.retry"), ai.Retry, icon: "refresh")) : null);
            col.MaxWidth = 440;
            col.HorizontalAlignment = HorizontalAlignment.Center;
            col.VerticalAlignment = VerticalAlignment.Center;
            col.Margin = new Thickness(24, 0, 24, 40);
            return col;
        }
        return ai.Availability switch
        {
            AiAvailability.Loading => K.Center(K.Spinner(20)),
            AiAvailability.Failed => Message("alertCircle", ai.ModelsError ?? "", true),
            AiAvailability.Disabled => Message("ban", L.T("ai.unavailable.disabled"), false),
            _ => Message("sparkles", L.T("ai.unavailable.noModels"), true),
        };
    }

    // ---------- 对话 ----------

    private static FrameworkElement ChatView(AppModel app, AiChat chat)
    {
        var ai = app.Ai;
        var dock = new DockPanel { Background = Th.Content };
        var header = new Region(() => Header(app, chat)).On(chat, ai.Changed);
        DockPanel.SetDock(header, Dock.Top);
        dock.Children.Add(header);
        // 输入框：模型列表不可用时换成一行提示（还能看历史消息）
        var bottom = new Region(() =>
        {
            var state = ai.Availability;
            if (state == AiAvailability.Ready) return K.Margin(Composer(app, chat), 24, 4, 24, 16).Also(c => c.MaxWidth = ContentWidth);
            var text = state switch
            {
                AiAvailability.Loading => null,
                AiAvailability.Failed => ai.ModelsError,
                AiAvailability.Disabled => L.T("ai.unavailable.disabled"),
                _ => L.T("ai.unavailable.noModels"),
            };
            var row = text == null ? K.Center(K.Spinner(14))
                : K.Row(10, K.Icon("alertCircle", 15, Th.Warning), K.Grow(K.Text(text, 12.5, Th.Secondary, wrap: true)),
                    state is AiAvailability.Failed or AiAvailability.NoModels ? K.SecondaryButton(L.T("common.retry"), ai.Retry, icon: "refresh") : null);
            return new Border
            {
                Margin = new Thickness(24, 4, 24, 16), MaxWidth = ContentWidth, Padding = new Thickness(14, 10, 12, 10), CornerRadius = new CornerRadius(12),
                Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(1), Child = row,
            };
        }).Keyed(() => ai.Availability).On(ai.Changed);
        DockPanel.SetDock(bottom, Dock.Bottom);
        dock.Children.Add(bottom);

        // 条目只追加：已有条目各自按自己的状态重绘，卡片里的输入框不会因为别的消息到来而丢焦点
        var stack = new StackPanel { Margin = new Thickness(24, 20, 24, 12), MaxWidth = ContentWidth };
        var rendered = 0;
        var scroll = new ScrollViewer
        {
            Content = stack, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, Focusable = false,
        };
        var progress = new Region(() => ShowProgress(chat) ? K.Margin(K.Left(K.Spinner(14)), 0, 2, 0, 0) : null).On(chat, chat.ItemsChanged);
        var toBottom = new Btn
        {
            Height = 30, CornerRadius = new CornerRadius(15), Padding = new Thickness(12, 0, 14, 0), Normal = Th.Elevated, HoverBrush = Th.B(K.Mix(Th.P.Elevated, Th.P.Text, 0.06)),
            BorderThickness = new Thickness(1), NormalBorder = Th.Border, HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Bottom,
            Margin = new Thickness(0, 0, 22, 12), Visibility = Visibility.Collapsed,
            Effect = new System.Windows.Media.Effects.DropShadowEffect { BlurRadius = 10, ShadowDepth = 2, Direction = 270, Opacity = 0.18 },
            Child = K.Row(6, K.Icon("arrowDown", 13, Th.Text, 2.2), K.Text(L.T("ai.toBottom"), 12.5, Th.Text, FontWeights.Medium)),
        };
        toBottom.UpdateLook();
        toBottom.Click += () => scroll.ScrollToEnd();
        // 新消息到达时自动滚到底；用户往上翻时不动，右下角给「回到底部」
        var stick = true;
        scroll.ScrollChanged += (_, e) =>
        {
            if (e.ExtentHeightChange == 0 || !stick) stick = scroll.VerticalOffset >= scroll.ScrollableHeight - 24;
            if (stick && e.ExtentHeightChange != 0) scroll.ScrollToEnd();
            toBottom.Visibility = stick || scroll.ScrollableHeight <= 0 ? Visibility.Collapsed : Visibility.Visible;
        };
        void Sync()
        {
            stack.Children.Remove(progress);
            for (; rendered < chat.Items.Count; rendered++)
            {
                var item = chat.Items[rendered];
                var gap = item is ToolItem && rendered + 1 < chat.Items.Count && chat.Items[rendered + 1] is ToolItem ? 4 : 16;
                stack.Children.Add(K.Margin(ItemView(app, chat, item), 0, 0, 0, gap));
            }
            stack.Children.Add(progress);
        }
        Sync();
        chat.ItemsChanged.Watch(stack, () => Ui.Coalesce(stack, Sync));
        var area = new Grid();
        area.Children.Add(scroll);
        area.Children.Add(new Region(() => chat.Opening ? K.Center(K.Spinner(20)) : null).On(chat));
        area.Children.Add(toBottom);
        area.Children.Add(new Region(() => Zoom(chat)).On(chat));
        dock.Children.Add(area);
        dock.Loaded += (_, _) => scroll.ScrollToEnd();
        return dock;
    }

    /// <summary>任务进行中、但眼下没有别的进度可看时（等模型回话），消息流末尾一个小转圈</summary>
    private static bool ShowProgress(AiChat chat)
    {
        if (!chat.Running || chat.Opening) return false;
        return chat.Items.LastOrDefault() switch
        {
            ReplyItem { Done: false } => false,
            ThinkingItem { Active: true } => false,
            ToolItem t when t.Decision == ToolDecision.Pending || t.Phase == ToolPhase.Running => false,
            AskItem { Answer: null } => false,
            SecretItem { Done: false, Cancelled: false } => false,
            _ => true,
        };
    }

    private static FrameworkElement Header(AppModel app, AiChat chat)
    {
        var title = K.Semi(chat.Title.Length == 0 ? L.T("ai.newConversation") : chat.Title, 14);
        var effective = chat.EffectiveMode is { } eff && eff != chat.Mode ? K.Tip(K.Badge(L.T(eff.LabelKey()), Th.P.Warning, "shieldAlert", 10.5), L.T("ai.mode.effective")) : null;
        var trust = chat.Mode == AiMode.FullTrust && chat.TrustUntil is { } until
            ? K.Tip(K.Badge(L.T("ai.trust.until", until.ToString("HH:mm", CultureInfo.InvariantCulture)), Th.P.Danger, "bolt", 10.5), L.T("ai.mode.fullTrust"))
            : null;
        var tokens = chat.PromptTokens + chat.CompletionTokens > 0
            ? K.Tip(K.Row(4, K.Icon("hash", 11, Th.Tertiary), K.Text(L.T("ai.tokens", Fmt.Kilo(chat.PromptTokens), Fmt.Kilo(chat.CompletionTokens)), 11.5, Th.Tertiary)), L.T("ai.tokens.help"))
            : null;
        return new Border
        {
            Height = 44, Padding = new Thickness(20, 0, 16, 0), BorderBrush = Th.Separator, BorderThickness = new Thickness(0, 0, 0, 1),
            Child = K.Row(10, K.Grow(title), effective, trust, tokens),
        };
    }

    // ---------- 输入框 ----------

    private static FrameworkElement Composer(AppModel app, AiChat? chat)
    {
        var ai = app.Ai;
        var draft = chat?.Draft ?? ai.NewDraft;

        var box = new TextBox
        {
            Text = draft.Text, AcceptsReturn = true, AcceptsTab = false, TextWrapping = TextWrapping.Wrap, BorderThickness = new Thickness(0), Background = Brushes.Transparent,
            Foreground = Th.Text, CaretBrush = Th.Text, SelectionBrush = Th.Accent, FontFamily = Th.UiFont, FontSize = 14, Padding = new Thickness(0),
            MinHeight = 22, MaxHeight = 8 * 20 + 4, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, FocusVisualStyle = null,
        };
        TextBlock.SetLineHeight(box, 20);
        var placeholder = new TextBlock
        {
            Text = L.T("ai.composer.placeholder"), FontSize = 14, Foreground = Th.Tertiary, IsHitTestVisible = false, FontFamily = Th.UiFont, Margin = new Thickness(2, 0, 0, 0),
            Visibility = draft.Text.Length == 0 ? Visibility.Visible : Visibility.Collapsed,
        };
        var input = new Grid { Children = { placeholder, box }, Margin = new Thickness(4, 0, 4, 0) };
        var mention = new Mention(app, box, draft);

        void Send()
        {
            if (mention.IsOpen) return;
            if (chat == null) ai.SendNew();
            else ai.Send(chat);
        }
        box.TextChanged += (_, _) =>
        {
            draft.Text = box.Text;
            placeholder.Visibility = box.Text.Length == 0 ? Visibility.Visible : Visibility.Collapsed;
            draft.Notify();
        };
        box.PreviewKeyDown += (_, e) =>
        {
            if (mention.HandleKey(e)) { e.Handled = true; return; }
            // Enter 发送，Shift+Enter 换行
            if (e.Key == Key.Enter && !Keyboard.Modifiers.HasFlag(ModifierKeys.Shift))
            {
                e.Handled = true;
                Send();
            }
        };
        // 发出后草稿清空：输入框跟着清空
        draft.Watch(box, () => { if (box.Text != draft.Text) box.Text = draft.Text; });

        var refs = new Region(() =>
        {
            if (draft.Refs.Count == 0) return null;
            var flow = new FlowPanel { Spacing = 6 };
            foreach (var r in draft.Refs.ToList()) flow.Children.Add(RefChip(app, r, () => draft.RemoveRef(r)));
            return K.Margin(flow, 0, 0, 0, 8);
        }).Keyed(() => string.Join("\n", draft.Refs.Select(r => $"{r.Kind}|{r.TargetId}|{r.Id}"))).On(draft);

        var tools = new Region(() => Toolbar(app, chat, draft, () => Send()))
            .Keyed(() => (draft.IsEmpty, draft.Sending, chat?.Running, chat?.Conversation != null, chat?.ModelKey ?? ai.DraftModel, chat?.Mode ?? ai.DraftMode,
                chat?.TrustMinutes ?? ai.DraftTrustMinutes, ai.DraftPlanFirst, ai.Models))
            .On(ai.Changed, chat, draft);

        var card = new Border
        {
            Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(16), Padding = new Thickness(12, 12, 10, 8),
            Effect = new System.Windows.Media.Effects.DropShadowEffect { BlurRadius = 16, ShadowDepth = 2, Direction = 270, Opacity = Th.P.IsDark ? 0.3 : 0.07 },
            Child = K.Col(6, refs, input, tools), Cursor = Cursors.IBeam,
        };
        box.GotKeyboardFocus += (_, _) => card.BorderBrush = Th.B(Th.P.Accent, 0.6);
        box.LostKeyboardFocus += (_, _) => card.BorderBrush = Th.Border;
        // 点卡片空白处也把焦点放进文本框
        card.MouseLeftButtonDown += (_, e) => { if (!box.IsKeyboardFocusWithin) { box.Focus(); box.CaretIndex = box.Text.Length; } };
        card.Loaded += (_, _) =>
        {
            if (app.SelfTest) return;
            box.Focus();
            box.CaretIndex = box.Text.Length;
        };
        var error = new Region(() => draft.Error == null ? null
            : K.Margin(K.Row(6, K.Top(K.Icon("alertCircle", 14, Th.Danger)), K.Grow(K.Text(draft.Error, 12.5, Th.Danger, wrap: true))), 6, 0, 6, 8)).Keyed(() => draft.Error).On(draft);
        return K.Col(0, error, card);
    }

    private static FrameworkElement Toolbar(AppModel app, AiChat? chat, ComposerDraft draft, Action send)
    {
        var ai = app.Ai;
        var plus = new Btn { Width = 30, Height = 30, CornerRadius = new CornerRadius(15), BorderThickness = new Thickness(1), NormalBorder = Th.Border, HoverBrush = Th.Hover2, Child = K.Center(K.Icon("plus", 15, Th.Secondary, 2)) };
        plus.UpdateLook();
        K.Tip(plus, L.T("ai.plus"));
        plus.Click += () => OpenMenu(plus, PlusMenu(app, draft));

        var modelKey = chat?.ModelKey ?? ai.DraftModel;
        var vision = ai.Model(modelKey)?.Vision == true;
        var model = Pill(K.Row(5, K.Text(ai.ModelName(modelKey), 12.5, Th.Secondary, FontWeights.Medium), vision ? K.Icon("eye", 12, Th.Tertiary) : null, K.Icon("chevronDown", 10, Th.Tertiary, 2.4)), L.T("ai.model"));
        model.Click += () => OpenMenu(model, ModelMenu(app, chat));

        var mode = chat?.Mode ?? ai.DraftMode;
        var modeFg = mode == AiMode.FullTrust ? Th.Danger : Th.Secondary;
        var modeBtn = Pill(K.Row(5, K.Icon(ModeIcon(mode), 13, modeFg, 1.9), K.Text(L.T(mode.LabelKey()), 12.5, modeFg, FontWeights.Medium), K.Icon("chevronDown", 10, Th.Tertiary, 2.4)), L.T("ai.mode"));
        modeBtn.Click += () => OpenMenu(modeBtn, ModeMenu(app, chat));

        Btn action;
        if (chat is { Running: true })
        {
            action = new Btn { Width = 30, Height = 30, CornerRadius = new CornerRadius(15), Normal = Th.Text, HoverBrush = Th.B(K.Mix(Th.P.Text, Th.P.Content, 0.2)), Child = K.Center(K.Icon("Fillstop", 11, Th.Content)) };
            K.Tip(action, L.T("ai.stop"));
            action.Click += chat.Stop;
        }
        else if (draft.Sending)
        {
            action = new Btn { Width = 30, Height = 30, CornerRadius = new CornerRadius(15), Normal = Th.Hover2, Child = K.Center(K.Spinner(14)) };
        }
        else
        {
            var enabled = !draft.IsEmpty && (chat == null || chat.Conversation != null);
            action = new Btn
            {
                Width = 30, Height = 30, CornerRadius = new CornerRadius(15), Normal = enabled ? Th.Text : Th.Hover2, HoverBrush = enabled ? Th.B(K.Mix(Th.P.Text, Th.P.Content, 0.2)) : Th.Hover2,
                Child = K.Center(K.Icon("arrowUp", 15, enabled ? Th.Content : Th.Tertiary, 2.6)),
            };
            K.Tip(action, L.T("ai.send"));
            action.Click += send;
            action.IsEnabled = enabled;
            // 置灰用底色区分，不再叠半透明
            action.Opacity = 1;
            action.IsEnabledChanged += (_, _) => action.Opacity = 1;
        }
        action.UpdateLook();
        return K.Row(4, plus, K.Gap(2), model, modeBtn, K.Spacer(), action);
    }

    private static Btn Pill(UIElement content, string tip)
    {
        var b = new Btn { Height = 30, CornerRadius = new CornerRadius(15), Padding = new Thickness(10, 0, 9, 0), HoverBrush = Th.Hover2, Child = content };
        b.UpdateLook();
        K.Tip(b, tip);
        return b;
    }

    public static string ModeIcon(AiMode m) => m switch
    {
        AiMode.ReadOnly => "eye",
        AiMode.ConfirmAll => "hand",
        AiMode.ConfirmDangerous => "shieldAlert",
        AiMode.FullTrust => "bolt",
        _ => "ban",
    };

    // ---------- 弹出菜单（主题化的 ContextMenu，Esc 关闭、方向键选择） ----------

    private static void OpenMenu(FrameworkElement anchor, ContextMenu menu)
    {
        menu.PlacementTarget = anchor;
        menu.Placement = PlacementMode.Top;
        menu.VerticalOffset = -4;
        menu.IsOpen = true;
    }

    private static MenuItem Item(UIElement header, Action? click, bool enabled = true, bool isChecked = false)
    {
        var mi = new MenuItem { Header = header, IsEnabled = enabled, IsChecked = isChecked };
        if (click != null) mi.Click += (_, _) => click();
        return mi;
    }

    private static MenuItem GroupHeader(string text) => new()
    {
        Header = K.Text(text, 11.5, Th.Tertiary, FontWeights.SemiBold), Focusable = false, IsHitTestVisible = false,
    };

    private static ContextMenu PlusMenu(AppModel app, ComposerDraft draft)
    {
        UIElement Row(string icon, string key) => K.Row(9, K.Icon(icon, 14, Th.Secondary, 1.9), K.Text(L.T(key), 13));
        var m = new ContextMenu();
        m.Items.Add(Item(Row("server", "ai.plus.targets"), () => AiPickers.PickTargets(app, draft)));
        m.Items.Add(Item(Row("folder", "ai.plus.files"), () => AiPickers.PickFile(app, draft)));
        m.Items.Add(Item(Row("code", "ai.plus.snippets"), () => AiPickers.PickSnippets(app, draft)));
        m.Items.Add(Item(Row("forward", "ai.plus.forwards"), () => AiPickers.PickForwards(app, draft)));
        m.Items.Add(Item(Row("clock", "ai.plus.activity"), () => draft.AddRef(AiPickers.ActivityRef())));
        return m;
    }

    /// <summary>模型菜单：按供应商分组，能看图的带「看图」标记</summary>
    private static ContextMenu ModelMenu(AppModel app, AiChat? chat)
    {
        var ai = app.Ai;
        var current = chat?.ModelKey ?? ai.DraftModel;
        var m = new ContextMenu();
        var first = true;
        foreach (var g in (ai.Models?.Items ?? Array.Empty<AiModel>()).GroupBy(x => x.ProviderId))
        {
            if (!first) m.Items.Add(new Separator());
            first = false;
            m.Items.Add(GroupHeader(g.First().ProviderName));
            foreach (var model in g)
            {
                var row = K.Row(8, K.Grow(K.Text(model.Name, 13)), model.Vision ? K.Badge(L.T("ai.vision"), Th.P.Accent, "eye", 10.5) : null);
                row.MinWidth = 200;
                var mm = model;
                m.Items.Add(Item(row, () => ai.SetModel(chat, mm), isChecked: AiHub.Key(model) == current));
            }
        }
        return m;
    }

    /// <summary>权限档位：四项（高于管理员上限的置灰），完全信任再选时限；底部「先出计划」</summary>
    private static ContextMenu ModeMenu(AppModel app, AiChat? chat)
    {
        var ai = app.Ai;
        var current = chat?.Mode ?? ai.DraftMode;
        var trust = chat?.TrustMinutes ?? ai.DraftTrustMinutes;
        var max = ai.MaxMode;
        var m = new ContextMenu();
        foreach (var mode in AiModes.Selectable)
        {
            var allowed = mode.Rank() <= max.Rank();
            var row = K.Row(9, K.Icon(ModeIcon(mode), 14, mode == AiMode.FullTrust ? Th.Danger : Th.Secondary, 1.9), K.Text(L.T(mode.LabelKey()), 13));
            row.MinWidth = 180;
            if (mode == AiMode.FullTrust)
            {
                var full = Item(row, null, allowed, current == mode);
                foreach (var (minutes, key) in new[] { (15, "ai.trust.15m"), (60, "ai.trust.1h"), (0, "ai.trust.conversation") })
                {
                    var min = minutes;
                    full.Items.Add(Item(K.Text(L.T(key), 13), () => ai.SetMode(chat, AiMode.FullTrust, min), isChecked: current == AiMode.FullTrust && trust == minutes));
                }
                m.Items.Add(full);
            }
            else
            {
                var mm = mode;
                m.Items.Add(Item(row, () => ai.SetMode(chat, mm, 0), allowed, current == mode));
            }
        }
        m.Items.Add(new Separator());
        // 先出计划只对新对话生效（创建对话时交给内核）
        var plan = chat?.PlanFirst ?? ai.DraftPlanFirst;
        var planItem = Item(K.Row(6, K.Text(L.T("ai.planFirst"), 13)), () => ai.SetPlanFirst(!plan), chat == null, plan);
        planItem.ToolTip = L.T("ai.planFirst.info");
        m.Items.Add(planItem);
        return m;
    }

    // ---------- 引用标签 ----------

    public static FrameworkElement RefChip(AppModel app, ContextRef r, Action? remove)
    {
        var label = K.Text(r.Label, 12, Th.Text);
        label.MaxWidth = 220;
        UIElement? x = null;
        if (remove != null)
        {
            var xb = new Btn { Width = 16, Height = 16, CornerRadius = new CornerRadius(8), HoverBrush = Th.Hover2, Focusable = false, Child = K.Center(K.Icon("close", 8.5, Th.Tertiary, 2.4)) };
            xb.UpdateLook();
            K.Tip(xb, L.T("ai.ref.remove"));
            xb.Click += remove;
            x = xb;
        }
        var chip = new Border
        {
            Height = 24, CornerRadius = new CornerRadius(6), Padding = new Thickness(7, 0, remove == null ? 8 : 3, 0),
            Background = Th.B(Th.P.IsDark ? Palettes.A(Colors.White, 0.08) : Palettes.A(Colors.Black, 0.05)), BorderBrush = Th.Border, BorderThickness = new Thickness(1),
            Child = K.Row(5, K.Icon(AiPickers.RefIcon(app, r), 12, Th.Secondary, 1.9), label, x), Cursor = Cursors.Arrow,
        };
        K.Tip(chip, AiPickers.RefTip(app, r));
        return chip;
    }

    // ---------- 消息流 ----------

    private static FrameworkElement ItemView(AppModel app, AiChat chat, ChatItem item)
    {
        switch (item)
        {
            case UserItem u:
            {
                var col = new List<UIElement?>();
                if (u.Refs.Length > 0)
                {
                    // 靠右换行：WrapPanel 的宽度跟内容走，整体靠右
                    var flow = new WrapPanel { HorizontalAlignment = HorizontalAlignment.Right, Margin = new Thickness(0, 0, 0, -6) };
                    foreach (var r in u.Refs) flow.Children.Add(K.Margin(RefChip(app, r, null), 6, 0, 0, 6));
                    col.Add(flow);
                }
                if (u.Text.Length > 0)
                    col.Add(new Border
                    {
                        HorizontalAlignment = HorizontalAlignment.Right, Background = Th.B(Th.P.IsDark ? Palettes.A(Colors.White, 0.09) : Palettes.A(Colors.Black, 0.055)),
                        CornerRadius = new CornerRadius(16), Padding = new Thickness(14, 9, 14, 9), Child = K.SelectableText(u.Text, 14).Also(Wheel.PassToParent),
                    });
                var c = K.Col(6, col);
                c.HorizontalAlignment = HorizontalAlignment.Right;
                c.MaxWidth = 600;
                c.Margin = new Thickness(80, 0, 0, 0);
                return c;
            }
            case ThinkingItem t:
                return new Region(() => ThinkingView(t)).On(t);
            case ReplyItem r:
                return ReplyView(r);
            case ToolItem { IsPlan: true } plan:
                return new Region(() => PlanView(plan)).Keyed(() => (plan.Expanded, plan.PlanSuperseded)).On(plan);
            case ToolItem c:
                return new Region(() => ToolView(app, chat, c)).Keyed(() => (c.Decision, c.Editing, c.Denying, c.Phase, c.Expanded, c.OutputExpanded, c.Output.Length, c.Result != null)).On(c);
            case AskItem a:
                return new Region(() => AskView(chat, a)).Keyed(() => a.Answer).On(a);
            case SecretItem s:
                return new Region(() => SecretView(app, chat, s)).Keyed(() => (s.Done, s.Cancelled)).On(s);
            case ErrorItem e:
                return new Region(() => ErrorView(chat, e)).Keyed(() => CanContinue(chat, e)).On(chat, chat.ItemsChanged);
        }
        return K.Gap(0);
    }

    /// <summary>连续执行太多步暂停：提示里让回复「继续」，只在最后一条、没在运行时旁边给个按钮</summary>
    private static bool CanContinue(AiChat chat, ErrorItem e)
        => e.Text.Contains("「继续」") && chat.Items.Count > 0 && chat.Items[chat.Items.Count - 1] == e && !chat.Running && chat.Conversation != null;

    private static FrameworkElement ErrorView(AiChat chat, ErrorItem e)
    {
        UIElement? go = null;
        if (CanContinue(chat, e))
        {
            var b = K.TintButton(L.T("ai.continue"), () => chat.SendQuick("继续"), Th.P.Accent, "play", 22);
            b.VerticalAlignment = VerticalAlignment.Top;
            b.Margin = new Thickness(0, -2, 0, 0);
            go = b;
        }
        return K.Row(8, K.Top(K.Margin(K.Icon("alertCircle", 15, Th.Danger), 0, 1, 0, 0)), K.Grow(K.Text(e.Text, 13, Th.Danger, wrap: true)), go);
    }

    // ---------- 任务清单（update_plan） ----------

    internal enum PlanStatus { Done, Active, Pending }

    /// <summary>detail 每行一项，前缀 "[x] " 已完成、"[>] " 进行中、"[ ] " 待做</summary>
    internal static List<(PlanStatus Status, string Text)> ParsePlan(string detail)
    {
        var list = new List<(PlanStatus, string)>();
        foreach (var raw in detail.Replace("\r", "").Split('\n'))
        {
            var line = raw.Trim();
            if (line.Length == 0) continue;
            if (line.StartsWith("[x]", StringComparison.OrdinalIgnoreCase)) list.Add((PlanStatus.Done, line.Substring(3).Trim()));
            else if (line.StartsWith("[>]", StringComparison.Ordinal)) list.Add((PlanStatus.Active, line.Substring(3).Trim()));
            else if (line.StartsWith("[ ]", StringComparison.Ordinal)) list.Add((PlanStatus.Pending, line.Substring(3).Trim()));
            else list.Add((PlanStatus.Pending, line));
        }
        return list;
    }

    /// <summary>任务清单卡片：一轮里只有最新的一份展开，之前的收成一行摘要（点开可看）</summary>
    private static FrameworkElement PlanView(ToolItem c)
    {
        var open = c.PlanSuperseded ? c.Expanded : !c.Expanded;
        var items = ParsePlan(c.Call.Detail);
        var title = c.Call.Summary.Length > 0 ? c.Call.Summary.Replace('\n', ' ') : ToolLabel(c.Call.Tool);
        var head = new Btn
        {
            CornerRadius = new CornerRadius(7), Padding = new Thickness(8, 5, 8, 5), HoverBrush = Th.Hover, HorizontalAlignment = HorizontalAlignment.Stretch,
            Child = K.Row(8,
                K.Icon("checklist", 13, Th.Tertiary, 1.9),
                K.Grow(K.Text(title, 12.5, open ? Th.Text : Th.Secondary, open ? FontWeights.Medium : FontWeights.Normal)),
                K.Icon(open ? "chevronDown" : "chevronRight", 10, Th.Tertiary, 2.4)),
        };
        head.UpdateLook();
        K.Tip(head, ToolLabel(c.Call.Tool));
        head.Click += () => { c.Expanded = !c.Expanded; c.Notify(); };
        if (!open || items.Count == 0) return head;
        var rows = new List<UIElement?>();
        foreach (var (st, text) in items)
        {
            var icon = st switch
            {
                PlanStatus.Done => K.Icon("checkCircle", 14, Th.Success, 2),
                PlanStatus.Active => K.Icon("circleDot", 14, Th.Accent, 2),
                _ => K.Icon("circle", 14, Th.Tertiary, 1.8),
            };
            var t = K.Text(text, 12.5, st == PlanStatus.Pending ? Th.Secondary : st == PlanStatus.Done ? Th.Tertiary : Th.Text,
                st == PlanStatus.Active ? FontWeights.Medium : FontWeights.Normal, wrap: true);
            if (st == PlanStatus.Done) t.TextDecorations = TextDecorations.Strikethrough;
            rows.Add(K.Row(9, K.Top(K.Margin(icon, 0, 1, 0, 0)), K.Grow(t)));
        }
        var body = new Border
        {
            Background = Th.Card, CornerRadius = new CornerRadius(9), BorderBrush = Th.Border, BorderThickness = new Thickness(1),
            Padding = new Thickness(12, 10, 12, 10), Margin = new Thickness(29, 2, 0, 4), Child = K.Col(7, rows),
        };
        return K.Col(2, head, body);
    }

    private static FrameworkElement ReplyView(ReplyItem r)
    {
        var view = Md.View();
        void Render() => view.Document = Md.Document(r.Text, !r.Done);
        Render();
        r.Watch(view, () => Ui.Coalesce(view, Render));
        return view;
    }

    private static FrameworkElement ThinkingView(ThinkingItem t)
    {
        var head = new Btn
        {
            HoverBrush = Th.Hover, CornerRadius = new CornerRadius(6), Padding = new Thickness(6, 3, 8, 3), HorizontalAlignment = HorizontalAlignment.Left,
            Child = t.Active
                ? K.Row(6, K.Spinner(12, Th.Tertiary), K.Text(L.T("ai.thinking.active"), 12.5, Th.Tertiary), K.Icon(t.Expanded ? "chevronDown" : "chevronRight", 10, Th.Tertiary, 2.4))
                : K.Row(6, K.Icon("bulb", 13, Th.Tertiary), K.Text(L.T("ai.thinking"), 12.5, Th.Tertiary), K.Icon(t.Expanded ? "chevronDown" : "chevronRight", 10, Th.Tertiary, 2.4)),
        };
        head.UpdateLook();
        head.Click += () => { t.Expanded = !t.Expanded; t.Notify(); };
        if (!t.Expanded) return head;
        var body = new Border
        {
            Margin = new Thickness(12, 2, 0, 0), Padding = new Thickness(12, 2, 0, 2), BorderBrush = Th.Separator, BorderThickness = new Thickness(2, 0, 0, 0),
            Child = K.SelectableText(t.Text, 12.5, Th.Tertiary).Also(Wheel.PassToParent),
        };
        return K.Col(4, head, body);
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
        "use_target" => "link",
        "use_app" or "app_list" or "app_state" or "app_click" or "app_set_value" or "app_type" or "app_key" or "app_scroll" or "app_action" or "app_open" => "appWindow",
        "web_fetch" => "globe",
        "system_info" => "chip",
        "update_plan" => "checklist",
        "list_devices" => "server",
        "list_snippets" => "code",
        "list_port_forwards" => "forward",
        "recent_activity" => "clock",
        "ask_user" => "ask",
        "request_secret" => "key",
        _ => "wrench",
    };

    /// <summary>工具的显示名；内核以后加了界面还不认识的工具时直接显示工具名</summary>
    private static string ToolLabel(string tool)
    {
        var key = "ai.tool." + tool;
        return L.Pack.Strings.ContainsKey(key) ? L.T(key) : tool;
    }

    private static string ToolTitle(ToolItem c) => c.IsUseTarget
        ? L.T("ai.useTarget.row", c.Call.TargetName)
        : c.Call.Summary.Length > 0 ? c.Call.Summary.Replace('\n', ' ') : ToolLabel(c.Call.Tool);

    private static FrameworkElement Thumb(AiChat chat, ImageData img)
    {
        var src = Img.From(img);
        if (src == null) return K.Gap(0);
        var image = new Image { Source = src, Stretch = Stretch.Uniform, MaxWidth = 340, MaxHeight = 200, HorizontalAlignment = HorizontalAlignment.Left };
        var b = new Btn { Child = new Border { CornerRadius = new CornerRadius(6), BorderBrush = Th.Border, BorderThickness = new Thickness(1), Child = image, ClipToBounds = true }, HorizontalAlignment = HorizontalAlignment.Left };
        K.Tip(b, L.T("ai.enlarge"));
        b.Click += () => { chat.ZoomImage = img; chat.Notify(); };
        return b;
    }

    /// <summary>工具调用：一行紧凑条目，点开看详情；需要确认时展开成醒目的卡片</summary>
    private static FrameworkElement ToolView(AppModel app, AiChat chat, ToolItem c)
    {
        if (c.Decision == ToolDecision.Pending) return ApprovalCard(app, chat, c);
        var call = c.Call;
        FrameworkElement status = c.Phase switch
        {
            ToolPhase.Running => K.Spinner(12),
            ToolPhase.Finished when c.Failed => K.Icon("xCircle", 13, Th.Danger),
            ToolPhase.Finished when c.Result != null => K.Icon("checkCircle", 13, Th.Success),
            ToolPhase.Finished => K.Gap(0),
            _ => c.Decision == ToolDecision.Denied ? K.Icon("xCircle", 13, Th.Danger) : K.Spinner(12),
        };
        var row = new Btn
        {
            CornerRadius = new CornerRadius(7), Padding = new Thickness(8, 5, 8, 5), HoverBrush = Th.Hover, HorizontalAlignment = HorizontalAlignment.Stretch,
            Child = K.Row(8,
                K.Icon(ToolIcon(call.Tool), 13, Th.Tertiary, 1.9),
                K.Grow(K.Text(ToolTitle(c), 12.5, c.Failed ? Th.Danger : Th.Secondary, mono: call.Tool == "run_command")),
                call.TargetName.Length > 0 && !c.IsUseTarget ? K.Text(call.TargetName, 11.5, Th.Tertiary) : null,
                status,
                K.Icon(c.Expanded ? "chevronDown" : "chevronRight", 10, Th.Tertiary, 2.4)),
        };
        row.UpdateLook();
        K.Tip(row, ToolLabel(call.Tool));
        row.Click += () => { c.Expanded = !c.Expanded; c.Notify(); };
        if (!c.Expanded) return row;
        return K.Col(4, row, K.Margin(ToolDetail(chat, c), 29, 2, 0, 6));
    }

    /// <summary>展开后的详情：完整命令或参数、模型理由、风险原因、输出、截图</summary>
    private static FrameworkElement ToolDetail(AiChat chat, ToolItem c)
    {
        var call = c.Call;
        var col = new List<UIElement?>();
        var command = c.EditedCommand ?? (call.Detail.Length > 0 ? call.Detail : call.Summary);
        if (command.Length > 0 && !c.IsSimpleApproval) col.Add(CommandBox(call.Tool, command));
        if (c.IsUseApp && call.Detail.Length > 0) col.Add(K.Text(call.Detail, 12.5, Th.Secondary, wrap: true));
        if (call.ModelReason.Length > 0) col.Add(K.Row(7, K.Top(K.Margin(K.Icon("chatText", 12, Th.Tertiary), 0, 2, 0, 0)), K.Grow(K.Text(call.ModelReason, 12.5, Th.Secondary, wrap: true))));
        if (call.RiskReasons.Length > 0 || call.Risk is RiskLevel.Dangerous or RiskLevel.Unknown or RiskLevel.Forbidden) col.Add(RiskRow(call));
        if (call.Image != null) col.Add(Thumb(chat, call.Image));
        if (c.Decision == ToolDecision.ApprovedSimilar) col.Add(K.Row(5, K.Icon("checkCircle", 12, Th.Tertiary), K.Text(L.T("ai.decision.similar"), 12, Th.Tertiary)));
        if (c.Decision == ToolDecision.Edited) col.Add(K.Row(5, K.Icon("pencil", 12, Th.Tertiary), K.Text(L.T("ai.decision.edited"), 12, Th.Tertiary)));
        if (c.Decision == ToolDecision.Denied && c.DenyReasonSent is { } reason)
            col.Add(K.Row(5, K.Icon("xCircle", 12, Th.Danger), K.Text(L.T("ai.decision.denied") + (L.IsChinese ? "：" : ": ") + reason, 12, Th.Danger)));
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
                outCol.Add(K.LinkButton(c.OutputExpanded ? L.T("ai.output.less") : L.T("ai.output.all", lines.Length), () => { c.OutputExpanded = !c.OutputExpanded; c.Notify(); }, Th.Accent));
            col.Add(K.Col(4, outCol));
        }
        if (c.Result?.Image is { } img) col.Add(Thumb(chat, img));
        if (c.Result is { } res && res.Summary.Length > 0)
            col.Add(K.Row(6, K.Icon(c.Failed ? "xCircle" : "checkCircle", 12, c.Failed ? Th.Danger : Th.Success), K.Grow(K.Text(res.Summary, 12, Th.Secondary, wrap: true))));
        if (col.Count == 0) col.Add(K.Text(ToolLabel(call.Tool), 12, Th.Tertiary));
        return K.Col(8, col);
    }

    /// <summary>命令、参数、文件改动：等宽字体；edit_file 的原文 / 改为按删除 / 新增着色</summary>
    private static FrameworkElement CommandBox(string tool, string text)
    {
        if (tool == "edit_file" && text.StartsWith("--- ", StringComparison.Ordinal))
        {
            var lines = text.Split('\n');
            var col = new List<UIElement?>();
            var added = false;
            foreach (var l in lines)
            {
                if (l.StartsWith("+++ ", StringComparison.Ordinal)) { added = true; col.Add(K.Text(l.Substring(4), 11.5, Th.Tertiary, FontWeights.SemiBold)); continue; }
                if (l.StartsWith("--- ", StringComparison.Ordinal)) { col.Add(K.Text(l.Substring(4), 11.5, Th.Tertiary, FontWeights.SemiBold)); continue; }
                var color = added ? Th.P.Success : Th.P.Danger;
                col.Add(new Border
                {
                    Background = Th.B(color, 0.1), Padding = new Thickness(6, 0, 6, 0),
                    Child = K.Text((added ? "+ " : "- ") + l, 12, Th.Text, mono: true),
                });
            }
            return K.Box(K.Col(1, col), Th.CodeBg, 7, null, 0, new Thickness(8));
        }
        var box = K.SelectableText(text, 12.5, Th.Text, mono: true).Also(Wheel.PassToParent);
        return K.Box(box, Th.CodeBg, 7, null, 0, new Thickness(10, 8, 10, 8));
    }

    private static FrameworkElement RiskRow(ToolCallView call)
    {
        var rc = Img.RiskColor(call.Risk);
        var reasons = call.RiskReasons.Select(r => (UIElement?)K.Text(r, 12.5, Th.Text, wrap: true)).ToList();
        return K.Row(8, K.Top(K.Badge(L.T("ai.risk." + Img.RiskKey(call.Risk)), rc, Img.RiskIcon(call.Risk), 11)), K.Grow(K.Col(3, reasons)));
    }

    /// <summary>需要确认：目标、完整命令 / 改动 / 点击位置、风险级别与原因、模型理由；use_target 只有允许与拒绝</summary>
    private static FrameworkElement ApprovalCard(AppModel app, AiChat chat, ToolItem c)
    {
        var call = c.Call;
        var rc = c.IsSimpleApproval ? Th.P.Accent : Img.RiskColor(call.Risk);
        var col = new List<UIElement?>();
        if (c.IsUseTarget)
        {
            col.Add(K.Row(9, K.Icon(app.Device(call.TargetId) is { } d ? K.PlatformIcon(d.Platform) : call.TargetId == AiHub.LocalId ? "display" : "server", 16, Th.Text, 1.9),
                K.Grow(K.Semi(L.T("ai.useTarget.title", call.TargetName), 14))));
        }
        else if (c.IsUseApp)
        {
            // 第一次操作某个应用：标题就是内核给的摘要（「AI 想操作「Safari」」），下面是说明
            col.Add(K.Row(9, K.Icon("appWindow", 16, Th.Text, 1.9), K.Semi(call.Summary.Length > 0 ? call.Summary : ToolLabel(call.Tool), 14),
                call.TargetName.Length > 0 ? K.Row(4, K.Icon("arrowRight", 11, Th.Tertiary), K.Text(call.TargetName, 13, Th.Secondary)) : null));
            if (call.Detail.Length > 0) col.Add(K.Text(call.Detail, 12.5, Th.Secondary, wrap: true));
        }
        else
        {
            col.Add(K.Row(8, K.Icon(ToolIcon(call.Tool), 15, Th.Text, 1.9), K.Semi(ToolLabel(call.Tool), 13.5),
                call.TargetName.Length > 0 ? K.Row(4, K.Icon("arrowRight", 11, Th.Tertiary), K.Text(call.TargetName, 13, Th.Secondary)) : null,
                K.Spacer(), K.Badge(L.T("ai.risk." + Img.RiskKey(call.Risk)), rc, Img.RiskIcon(call.Risk), 11)));
            col.Add(CommandBox(call.Tool, call.Detail.Length > 0 ? call.Detail : call.Summary));
        }
        if (call.Image != null) col.Add(Thumb(chat, call.Image));
        if (call.RiskReasons.Length > 0)
            col.Add(K.Col(3, call.RiskReasons.Select(r => (UIElement?)K.Row(7, K.Top(K.Margin(K.Icon(c.IsSimpleApproval ? "info" : Img.RiskIcon(call.Risk), 12, Th.B(rc)), 0, 2, 0, 0)), K.Grow(K.Text(r, 12.5, Th.Text, wrap: true))))));
        if (call.ModelReason.Length > 0)
            col.Add(K.Row(7, K.Top(K.Margin(K.Icon("chatText", 12, Th.Tertiary), 0, 2, 0, 0)), K.Grow(K.Text(call.ModelReason, 12.5, Th.Secondary, wrap: true))));
        col.Add(ApprovalButtons(chat, c));
        return new Border
        {
            Background = Th.Card, CornerRadius = new CornerRadius(12), BorderThickness = new Thickness(1.5), BorderBrush = Th.B(rc, 0.7),
            Padding = new Thickness(16, 13, 14, 13), Child = K.Col(11, col),
        };
    }

    private static FrameworkElement ApprovalButtons(AiChat chat, ToolItem c)
    {
        if (c.IsSimpleApproval)
            return K.Row(8, K.PrimaryButton(L.T("ai.approve.allow"), () => chat.Decide(c, new ApprovalDecision.AllowOnce())),
                K.SecondaryButton(L.T("ai.approve.deny"), () => chat.Decide(c, new ApprovalDecision.Deny(null)), destructive: true));
        if (c.Editing)
        {
            var f = K.TextArea("", c.EditText, 70, v => c.EditText = v, mono: true, fontSize: 12.5);
            return K.Col(8, f, K.Row(8, K.Spacer(), K.SecondaryButton(L.T("common.cancel"), () => { c.Editing = false; c.Notify(); }),
                K.PrimaryButton(L.T("ai.approve.runEdited"), () => chat.Decide(c, new ApprovalDecision.EditAndRun(c.EditText)))));
        }
        if (c.Denying)
        {
            string? Reason() => c.DenyReason.Trim().Length == 0 ? null : c.DenyReason.Trim();
            var f = K.TextField(L.T("ai.deny.reason"), c.DenyReason, v => c.DenyReason = v);
            f.Submitted += () => chat.Decide(c, new ApprovalDecision.Deny(Reason()));
            f.Loaded += (_, _) => { if (AppModel.Current?.SelfTest != true) f.FocusInput(); };
            return K.Col(8, f, K.Row(8, K.Spacer(), K.SecondaryButton(L.T("common.cancel"), () => { c.Denying = false; c.Notify(); }),
                K.PrimaryButton(L.T("ai.approve.deny"), () => chat.Decide(c, new ApprovalDecision.Deny(Reason())), destructive: true)));
        }
        return K.Row(8,
            K.PrimaryButton(L.T("ai.approve.once"), () => chat.Decide(c, new ApprovalDecision.AllowOnce())),
            K.SecondaryButton(L.T("ai.approve.similar"), () => chat.Decide(c, new ApprovalDecision.AllowSimilar())),
            c.Call.Tool == "run_command" ? K.SecondaryButton(L.T("ai.approve.edit"), () => { c.Editing = true; c.Notify(); }) : null,
            K.Spacer(),
            K.SecondaryButton(L.T("ai.approve.deny"), () => { c.Denying = true; c.Notify(); }, destructive: true));
    }

    private static FrameworkElement AskView(AiChat chat, AskItem a)
    {
        var col = new List<UIElement?> { K.Row(8, K.Top(K.Margin(K.Icon("ask", 15, Th.Accent), 0, 1, 0, 0)), K.Grow(K.Text(a.Question, 13.5, Th.Text, FontWeights.Medium, wrap: true))) };
        if (a.Answer != null) col.Add(K.Row(6, K.Icon("reply", 13, Th.Secondary), K.Text(a.Answer, 13, Th.Secondary)));
        else
        {
            var row = new FlowPanel { Spacing = 8 };
            for (var i = 0; i < a.Options.Length; i++)
            {
                var o = a.Options[i];
                row.Children.Add(i == 0 ? K.PrimaryButton(o, () => chat.Answer(a, o)) : K.SecondaryButton(o, () => chat.Answer(a, o)));
            }
            col.Add(row);
            var other = K.TextField(L.T("ai.ask.other"), a.FreeText, v => a.FreeText = v);
            void Submit() { if (a.FreeText.Trim().Length > 0) chat.Answer(a, a.FreeText.Trim()); }
            other.Submitted += Submit;
            col.Add(K.Row(8, K.Grow(other), K.SecondaryButton(L.T("ai.ask.send"), Submit)));
        }
        return new Border
        {
            Background = Th.Card, CornerRadius = new CornerRadius(12), Padding = new Thickness(16, 13, 14, 13), BorderThickness = new Thickness(a.Answer == null ? 1.5 : 1),
            BorderBrush = a.Answer == null ? Th.B(Th.P.Accent, 0.6) : Th.Border, Child = K.Col(11, col),
        };
    }

    /// <summary>要密码：卡片里一个不回显的输入框，内容走 provide_secret，不进消息流</summary>
    private static FrameworkElement SecretView(AppModel app, AiChat chat, SecretItem s)
    {
        var target = s.TargetId.Length == 0 ? "" : app.Ai.WhereName(s.TargetId);
        var head = K.Row(8, K.Top(K.Margin(K.Icon("key", 15, Th.Warning), 0, 1, 0, 0)), K.Grow(K.Text(s.Prompt, 13.5, Th.Text, FontWeights.Medium, wrap: true)), K.Info(L.T("ai.secret.info")));
        if (s.Done || s.Cancelled)
        {
            return new Border
            {
                Background = Th.Card, CornerRadius = new CornerRadius(12), Padding = new Thickness(16, 11, 14, 11), BorderThickness = new Thickness(1), BorderBrush = Th.Border,
                Child = K.Row(8, K.Icon("key", 14, Th.Tertiary), K.Grow(K.Text(s.Prompt, 13, Th.Secondary)),
                    s.Done ? K.Row(4, K.Icon("check", 12, Th.Success, 2.6), K.Text(L.T("ai.secret.sent"), 12, Th.Success)) : K.Text(L.T("ai.secret.cancelled"), 12, Th.Tertiary)),
            };
        }
        var pwd = K.PasswordField(L.T("ai.secret.placeholder"));
        void Submit()
        {
            if (pwd.Text.Length == 0) return;
            var v = pwd.Text;
            pwd.Text = "";
            chat.ProvideSecret(s, v);
        }
        pwd.Submitted += Submit;
        pwd.Escaped += () => chat.CancelSecret(s);
        pwd.Loaded += (_, _) => { if (!app.SelfTest) pwd.FocusInput(); };
        return new Border
        {
            Background = Th.Card, CornerRadius = new CornerRadius(12), Padding = new Thickness(16, 13, 14, 13), BorderThickness = new Thickness(1.5), BorderBrush = Th.B(Th.P.Warning, 0.6),
            Child = K.Col(11, head,
                target.Length == 0 ? null : K.Row(6, K.Icon("arrowRight", 12, Th.Tertiary), K.Text(L.T("ai.secret.target", target), 12, Th.Tertiary)),
                K.Row(8, K.Grow(pwd), K.SecondaryButton(L.T("common.cancel"), () => chat.CancelSecret(s)), K.PrimaryButton(L.T("ai.secret.send"), Submit))),
        };
    }

    private static FrameworkElement? Zoom(AiChat chat)
    {
        if (chat.ZoomImage is not { } img || Img.From(img) is not { } src) return null;
        var g = new Grid { Background = Th.B(Colors.Black, 0.8), Cursor = Cursors.Hand };
        g.Children.Add(new Image { Source = src, Stretch = Stretch.Uniform, Margin = new Thickness(30) });
        g.MouseLeftButtonDown += (_, _) => { chat.ZoomImage = null; chat.Notify(); };
        return g;
    }

    /// <summary>设置页「新对话的默认档位」：分段选择，超过上限的置灰</summary>
    public static FrameworkElement ModePicker(AiMode mode, AiMode max, Action<AiMode> pick)
    {
        var grid = new Grid();
        var col = 0;
        foreach (var m in AiModes.Selectable)
        {
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
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
            Grid.SetColumn(b, col++);
            grid.Children.Add(b);
        }
        return new Border { Padding = new Thickness(3), Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(9), MaxWidth = 580, HorizontalAlignment = HorizontalAlignment.Left, Child = grid };
    }
}

/// <summary>
/// 输入框里的 @ 引用：光标前是「@ + 过滤文字」时在光标上方浮出设备与 SSH 主机候选（平台图标、名称、在线状态），
/// 方向键选择、Enter / Tab 选中、Esc 或点空白处关闭；选中后删掉 @ 与过滤文字，加一个引用标签。
/// </summary>
internal sealed class Mention
{
    private readonly AppModel app;
    private readonly TextBox box;
    private readonly ComposerDraft draft;
    private readonly Popup popup;
    private readonly StackPanel list = new();
    private List<ContextRef> items = new();
    private int at = -1, sel;
    private bool picking;

    public Mention(AppModel app, TextBox box, ComposerDraft draft)
    {
        this.app = app;
        this.box = box;
        this.draft = draft;
        var card = new Border
        {
            Margin = new Thickness(0, 0, 10, 10), Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(9),
            Padding = new Thickness(4), Width = 280,
            Effect = new System.Windows.Media.Effects.DropShadowEffect { BlurRadius = 14, ShadowDepth = 3, Opacity = 0.22, Direction = 270 },
            Child = list,
        };
        popup = new Popup
        {
            PlacementTarget = box, Placement = PlacementMode.Top, StaysOpen = false, AllowsTransparency = true, Focusable = false, Child = card, VerticalOffset = -4,
        };
        popup.Closed += (_, _) => at = -1;
        box.TextChanged += (_, _) => Update();
        box.SelectionChanged += (_, _) => { if (popup.IsOpen) Update(); };
        box.LostKeyboardFocus += (_, _) => Close();
    }

    public bool IsOpen => popup.IsOpen;

    private void Close()
    {
        popup.IsOpen = false;
        at = -1;
    }

    /// <summary>光标前的 @ 片段：@ 在开头或空白之后，中间没有空白</summary>
    public static (int at, string query)? Find(string text, int caret)
    {
        var i = caret - 1;
        while (i >= 0 && text[i] != '@' && !char.IsWhiteSpace(text[i])) i--;
        if (i < 0 || text[i] != '@' || i > 0 && !char.IsWhiteSpace(text[i - 1])) return null;
        return (i, text.Substring(i + 1, caret - i - 1));
    }

    private void Update()
    {
        if (picking) return;
        var hit = Find(box.Text, box.CaretIndex);
        if (hit == null) { Close(); return; }
        at = hit.Value.at;
        items = app.Ai.TargetCandidates(hit.Value.query).Take(8).ToList();
        sel = Math.Min(sel, Math.Max(0, items.Count - 1));
        Render();
        var r = box.GetRectFromCharacterIndex(at);
        if (!r.IsEmpty) popup.PlacementRectangle = new Rect(r.X, r.Y, 1, r.Height);
        popup.IsOpen = true;
    }

    private void Render()
    {
        list.Children.Clear();
        if (items.Count == 0)
        {
            list.Children.Add(K.Margin(K.Text(L.T("ai.mention.none"), 12.5, Th.Tertiary), 10, 8, 10, 8));
            return;
        }
        for (var i = 0; i < items.Count; i++)
        {
            var r = items[i];
            var d = r.Kind == ContextRefKind.Device ? app.Device(r.Id) : null;
            var active = i == sel;
            var b = new Btn
            {
                Height = 32, CornerRadius = new CornerRadius(6), Padding = new Thickness(8, 0, 10, 0), Focusable = false,
                Normal = active ? Th.Hover2 : Brushes.Transparent, HoverBrush = Th.Hover2,
                Child = K.Row(9, K.Icon(AiPickers.RefIcon(app, r), 14, Th.Secondary, 1.8), K.Grow(K.Text(r.Label, 13)),
                    d != null ? K.StatusDot(K.DotOf(d), 7, Th.Card) : AiHub.IsLocal(r) ? K.StatusDot(DotState.Online, 7, Th.Card) : K.Text("SSH", 10, Th.Tertiary, FontWeights.SemiBold)),
            };
            b.UpdateLook();
            var pick = r;
            b.Click += () => Pick(pick);
            list.Children.Add(b);
        }
    }

    private void Pick(ContextRef r)
    {
        if (at < 0) return;
        picking = true;
        var caret = Math.Max(at, box.CaretIndex);
        box.Text = box.Text.Remove(at, caret - at);
        box.CaretIndex = at;
        picking = false;
        draft.AddRef(r);
        Close();
        box.Focus();
    }

    public bool HandleKey(KeyEventArgs e)
    {
        if (!popup.IsOpen) return false;
        switch (e.Key)
        {
            case Key.Down when items.Count > 0:
                sel = (sel + 1) % items.Count;
                Render();
                return true;
            case Key.Up when items.Count > 0:
                sel = (sel - 1 + items.Count) % items.Count;
                Render();
                return true;
            case Key.Enter or Key.Tab when items.Count > 0:
                Pick(items[sel]);
                return true;
            case Key.Escape:
                Close();
                return true;
        }
        return false;
    }
}
