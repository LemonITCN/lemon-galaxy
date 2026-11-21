using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace LemonGalaxy.UI;

/// <summary>片段：常用命令，端到端加密同步；终端顶栏可一键输入</summary>
internal static class SnippetViews
{
    private static string query = "";

    public static FrameworkElement Middle(AppModel app)
    {
        var list = new Region(() =>
        {
            var q = query.ToLowerInvariant();
            var items = new List<UIElement?>();
            foreach (var e in app.Snippets)
            {
                if (e.Data is not VaultData.Snippet s) continue;
                if (q.Length > 0 && !s.Label.ToLowerInvariant().Contains(q) && !s.Command.ToLowerInvariant().Contains(q) && !s.Tags.Any(t => t.ToLowerInvariant().Contains(q))) continue;
                var sel = app.SelectedSnippet == e.Id && app.ActiveTabId == null;
                var cmd = K.Text(s.Command, 11.5, Th.Tertiary, mono: true);
                var b = new Btn
                {
                    CornerRadius = new CornerRadius(7), Padding = new Thickness(10, 7, 10, 7), Normal = sel ? Th.Selection : Brushes.Transparent, HoverBrush = sel ? Th.Selection : Th.Hover,
                    Child = K.Col(3, K.Row(6, K.Grow(K.Text(s.Label, 13, Th.Text, FontWeights.Medium)), s.Tags.Length > 0 ? K.TagChip(s.Tags[0]) : null), cmd),
                };
                b.UpdateLook();
                var id = e.Id;
                b.Click += () => { app.SelectedSnippet = id; app.Editing = null; app.ActiveTabId = null; app.Nav.Notify(); };
                b.DoubleClick += () => SendToActiveTerminal(app, s.Command);
                items.Add(b);
            }
            return K.Margin(K.Col(1, items), 6);
        }).On(app.Data, app.Nav);
        var search = K.SearchField(L.T("snippet.search"), query, v => { query = v; list.Invalidate(); }, out _);
        var dock = new DockPanel();
        var top = K.Col(0,
            Shell.ColumnHeader(L.T("nav.snippets"), K.IconButton("plus", L.T("snippet.new"), () => { app.SelectedSnippet = null; app.Editing = new EditTarget.NewSnippet(); app.SelectHome(NavSection.Snippets); }, size: 15)),
            K.Margin(search, 12, 0, 12, 8), K.HLine());
        DockPanel.SetDock(top, Dock.Top);
        dock.Children.Add(top);
        dock.Children.Add(K.Scroll(list));
        return dock;
    }

    private static void SendToActiveTerminal(AppModel app, string command)
    {
        var t = app.Tabs.OfType<TerminalTab>().FirstOrDefault(x => x.Phase == TerminalPhase.Running);
        if (t == null) { app.ShowToast(L.T("snippet.noTerminal"), true); return; }
        t.SendText(command + "\r");
        app.ActivateTab(t.Id);
    }

    public static FrameworkElement Home(AppModel app)
    {
        EditTarget? Editing() => app.Editing switch
        {
            EditTarget.NewSnippet n => n,
            EditTarget.Vault v when v.Id == app.SelectedSnippet && app.Entry(v.Id)?.Data is VaultData.Snippet => v,
            _ => null,
        };
        // 编辑面板只在目标变化时重建（输入框保持焦点）；详情随数据刷新
        return new Region(() =>
        {
            if (Editing() is { } target)
                return new Border { MaxWidth = 600, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 12, 0, 12), Child = Editors.Panel(app, target) };
            return new Region(() => Detail(app)).On(app.Data, app.Nav);
        }).Keyed(() => Editing()).On(app.Nav);
    }

    private static FrameworkElement Detail(AppModel app)
    {
        if (app.SelectedSnippet is not { } id || app.Entry(id)?.Data is not VaultData.Snippet s)
            return K.EmptyState("code", L.T("snippet.empty"), (L.T("snippet.new"), () => { app.Editing = new EditTarget.NewSnippet(); app.Nav.Notify(); }));
        var tags = new FlowPanel { Spacing = 5 };
        foreach (var t in s.Tags) tags.Children.Add(K.TagChip(t));
        var code = K.Box(K.SelectableText(s.Command, 13.5, Th.Text, mono: true), Th.CodeBg, 9, null, 0, new Thickness(14));
        var send = K.MenuButton(K.Row(6, K.Icon("terminal", 13, Th.Text), K.Text(L.T("snippet.sendTo"), 13, Th.Text, FontWeights.Medium), K.Icon("chevronDown", 11, Th.Tertiary)), () =>
        {
            var terms = app.Tabs.OfType<TerminalTab>().Where(t => t.Phase == TerminalPhase.Running).ToList();
            if (terms.Count == 0) return new List<MenuEntry> { new(L.T("snippet.noTerminal"), Enabled: false) };
            return terms.Select(t => new MenuEntry(t.Title, () => { t.SendText(s.Command + "\r"); app.ActivateTab(t.Id); })).ToList();
        }, height: 30);
        send.BorderThickness = new Thickness(1);
        send.NormalBorder = Th.Border;
        send.Normal = Th.Elevated;
        send.Padding = new Thickness(12, 0, 10, 0);
        send.CornerRadius = new CornerRadius(7);
        send.UpdateLook();
        return K.PageScroll(K.Col(16,
            K.Row(10, K.Grow(K.Semi(s.Label, 21)), K.SecondaryButton(L.T("hosts.edit"), () => { app.Editing = new EditTarget.Vault(id); app.Nav.Notify(); }, icon: "pencil")),
            s.Tags.Length > 0 ? tags : null,
            code,
            K.Row(8, send, K.SecondaryButton(L.T("common.copy"), () => { SysClipboard.SetText(s.Command); app.ShowToast(L.T("common.copied")); }, icon: "copy"), K.Info(L.T("snippet.info")))), 860);
    }
}
