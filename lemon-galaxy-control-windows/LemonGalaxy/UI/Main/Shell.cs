using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Shell;

namespace LemonGalaxy.UI;

/// <summary>根界面：登录页或主界面。主窗口与 selftest 都用它构建内容。</summary>
internal static class Shell
{
    public const double BarHeight = 38;

    public static FrameworkElement Root(AppModel app)
    {
        var root = new Grid { Background = Th.Content };
        TextElement.SetFontFamily(root, Th.UiFont);
        TextOptions.SetTextFormattingMode(root, TextFormattingMode.Display);
        var content = new Region(() => app.LoggedIn ? MainView(app) : LoginView.Build(app)).On(app.PhaseChanged);
        root.Children.Add(content);
        root.Children.Add(ModalLayer(app));
        root.Children.Add(ToastLayer(app));
        return root;
    }

    // ---------- 主界面 ----------

    private static FrameworkElement MainView(AppModel app)
    {
        var s = app.Settings;
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });

        var sidebar = Sidebar.Build(app);
        grid.Children.Add(sidebar);

        // 中间列：分区切换或显隐变化时才重建（里面的搜索框要保持焦点）
        bool ListVisible() => !s.ListHidden && !(s.AutoHideList && app.ActiveTabId != null);
        var middle = new Region(() =>
        {
            if (!ListVisible()) return null;
            var col = new Grid { Width = 284, Background = Th.List };
            col.Children.Add(MiddleFor(app));
            // 用 DockPanel 让中间列撑满整个高度（Flex 横排会把内容短的列垂直居中）
            var dock = new DockPanel();
            var line = K.VLine();
            DockPanel.SetDock(line, Dock.Left);
            dock.Children.Add(line);
            dock.Children.Add(col);
            return dock;
        }).Keyed(() => (app.Section, ListVisible(), app.HostsTab)).On(app.Nav, s);
        Grid.SetColumn(middle, 1);
        grid.Children.Add(middle);

        var mainArea = new DockPanel { Background = Th.Content, MinWidth = 480 };
        var leftLine = K.VLine();
        DockPanel.SetDock(leftLine, Dock.Left);
        mainArea.Children.Add(leftLine);
        var bar = TabBar(app);
        DockPanel.SetDock(bar, Dock.Top);
        mainArea.Children.Add(bar);
        mainArea.Children.Add(Workspace(app));
        Grid.SetColumn(mainArea, 2);
        grid.Children.Add(mainArea);
        return grid;
    }

    private static FrameworkElement MiddleFor(AppModel app) => app.Section switch
    {
        NavSection.Hosts => HostsViews.Middle(app),
        NavSection.Sftp => SftpViews.Middle(app),
        NavSection.Ai => AiViews.Middle(app),
        NavSection.Forwards => ForwardViews.Middle(app),
        NavSection.Snippets => SnippetViews.Middle(app),
        NavSection.Local => LocalViews.Middle(app),
        _ => SettingsViews.Middle(app),
    };

    /// <summary>中间列的标题栏（与主区标签栏同高）</summary>
    public static FrameworkElement ColumnHeader(string title, params UIElement?[] trailing)
    {
        var row = K.Row(4, new UIElement?[] { K.Grow(K.Semi(title, 14.5)) }.Concat(trailing).ToArray());
        return new Border { Height = BarHeight, Padding = new Thickness(14, 0, 8, 0), Child = row };
    }

    // ---------- 工作区：首页 + 各标签内容（标签视图常驻，切换只改可见性）----------

    private static FrameworkElement Workspace(AppModel app)
    {
        var host = new Grid { ClipToBounds = true };
        var home = new Region(() => HomeFor(app)).Keyed(() => app.Section).On(app.Nav);
        host.Children.Add(home);
        var tabLayer = new Grid();
        host.Children.Add(tabLayer);
        string lastIds = "";
        string? lastActive = null;
        void Sync()
        {
            var ids = string.Join(",", app.Tabs.Select(t => t.Id));
            if (ids != lastIds)
            {
                lastIds = ids;
                tabLayer.Children.Clear();
                foreach (var t in app.Tabs)
                {
                    var v = t.View;
                    Region.Detach(v);
                    tabLayer.Children.Add(v);
                }
            }
            home.Visibility = app.ActiveTabId == null ? Visibility.Visible : Visibility.Hidden;
            foreach (var t in app.Tabs) t.View.Visibility = t.Id == app.ActiveTabId ? Visibility.Visible : Visibility.Hidden;
            if (app.ActiveTabId != lastActive)
            {
                lastActive = app.ActiveTabId;
                app.ActiveTab?.Activated();
            }
        }
        Sync();
        app.Nav.Watch(host, () => Ui.Coalesce(host, Sync));
        return host;
    }

    private static FrameworkElement HomeFor(AppModel app) => app.Section switch
    {
        NavSection.Hosts => HostsViews.Home(app),
        NavSection.Sftp => SftpViews.Home(app),
        NavSection.Ai => AiViews.Home(app),
        NavSection.Forwards => ForwardViews.Home(app),
        NavSection.Snippets => SnippetViews.Home(app),
        NavSection.Local => LocalViews.Home(app),
        _ => SettingsViews.Home(app),
    };

    public static string SectionIcon(NavSection s) => s switch
    {
        NavSection.Hosts => "server",
        NavSection.Sftp => "folder",
        NavSection.Ai => "sparkles",
        NavSection.Forwards => "forward",
        NavSection.Snippets => "code",
        NavSection.Local => "display",
        _ => "gear",
    };

    public static string SectionKey(NavSection s) => "nav." + s switch
    {
        NavSection.Hosts => "hosts",
        NavSection.Sftp => "sftp",
        NavSection.Ai => "ai",
        NavSection.Forwards => "forwards",
        NavSection.Snippets => "snippets",
        NavSection.Local => "local",
        _ => "settings",
    };

    // ---------- 标签栏 ----------

    private static FrameworkElement TabBar(AppModel app)
    {
        var s = app.Settings;
        Region? tabsRegion = null;
        tabsRegion = new Region(() =>
        {
            var row = new StackPanel { Orientation = Orientation.Horizontal };
            row.Children.Add(HomeTab(app));
            foreach (var t in app.Tabs)
            {
                // 标题、状态点变化时重建标签栏（Watch 对同一个回调去重）
                if (tabsRegion != null) t.Watch(tabsRegion, tabsRegion.Invalidate);
                row.Children.Add(TabItem(app, t));
            }
            return row;
        }, immediate: false).On(app.Nav);
        tabsRegion.Refresh();
        var scroller = new ScrollViewer
        {
            Content = tabsRegion, HorizontalScrollBarVisibility = ScrollBarVisibility.Hidden, VerticalScrollBarVisibility = ScrollBarVisibility.Disabled, Focusable = false,
        };
        scroller.PreviewMouseWheel += (_, e) => { scroller.ScrollToHorizontalOffset(scroller.HorizontalOffset - e.Delta / 2.0); e.Handled = true; };
        var toggle = new Region(() => K.IconButton(s.ListHidden ? "sidebar" : "columns", L.T("nav.toggleList"), () => s.ListHidden = !s.ListHidden, size: 15)).On(s);
        var row2 = K.Row(0, K.Margin(toggle, 6, 0, 6, 0), K.Grow(scroller), new WindowButtons());
        return new Border { Height = BarHeight, Background = Th.TabBar, BorderBrush = Th.Separator, BorderThickness = new Thickness(0, 0, 0, 1), Child = row2 };
    }

    private static FrameworkElement HomeTab(AppModel app)
    {
        var active = app.ActiveTabId == null;
        var b = new Btn
        {
            Height = BarHeight - 1, Padding = new Thickness(14, 0, 14, 0), Normal = active ? Th.TabActive : Brushes.Transparent, HoverBrush = active ? Th.TabActive : Th.Hover,
            BorderBrush = active ? Th.Accent : Brushes.Transparent, BorderThickness = new Thickness(0, 2, 0, 0),
            Child = K.Row(7, K.Icon(SectionIcon(app.Section), 13, active ? Th.Text : Th.Secondary), K.Text(L.T(SectionKey(app.Section)), 13, active ? Th.Text : Th.Secondary, active ? FontWeights.SemiBold : FontWeights.Normal)),
        };
        b.UpdateLook();
        b.Click += () => app.ActivateTab(null);
        return b;
    }

    private static FrameworkElement TabItem(AppModel app, TabModel tab)
    {
        var active = app.ActiveTabId == tab.Id;
        var icon = new Grid { Width = 15, Height = 15 };
        icon.Children.Add(K.Icon(tab.Icon, 13, active ? Th.Text : Th.Secondary));
        if (tab.Live == false)
            icon.Children.Add(new System.Windows.Shapes.Ellipse { Width = 6, Height = 6, Fill = Th.Danger, HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Bottom });
        var close = K.IconButton("close", L.T("tab.close"), () => app.CloseTab(tab.Id), size: 10, w: 18, h: 18);
        close.CornerRadius = new CornerRadius(9);
        close.Opacity = active ? 1 : 0;
        var title = K.Text(tab.Title, 13, active ? Th.Text : Th.Secondary, active ? FontWeights.Medium : FontWeights.Normal);
        title.MaxWidth = 160;
        var b = new Btn
        {
            Height = BarHeight - 1, Padding = new Thickness(12, 0, 6, 0), Normal = active ? Th.TabActive : Brushes.Transparent, HoverBrush = active ? Th.TabActive : Th.Hover,
            BorderBrush = active ? Th.Accent : Brushes.Transparent, BorderThickness = new Thickness(0, 2, 0, 0),
            Child = K.Row(7, icon, title, close),
        };
        b.UpdateLook();
        b.MouseEnter += (_, _) => close.Opacity = 1;
        b.MouseLeave += (_, _) => close.Opacity = active ? 1 : 0;
        b.Click += () => app.ActivateTab(tab.Id);
        // 中键关闭
        b.MouseDown += (_, e) => { if (e.ChangedButton == MouseButton.Middle) { app.CloseTab(tab.Id); e.Handled = true; } };
        K.ContextMenuFor(b, () => new List<MenuEntry>
        {
            new(L.T("tab.close"), () => app.CloseTab(tab.Id)),
            new(L.T("tab.closeOthers"), () => { foreach (var t in app.Tabs.Where(x => x.Id != tab.Id).ToList()) app.CloseTab(t.Id); }),
        });
        return K.Row(0, K.VLine(), b);
    }

    // ---------- 模态卡片与提示条 ----------

    private static FrameworkElement ModalLayer(AppModel app) => new Region(() =>
    {
        var m = app.Modal;
        if (m == null) return null;
        var grid = new Grid();
        var scrim = new Border { Background = Th.Scrim };
        scrim.MouseLeftButtonDown += (_, e) =>
        {
            if (m.Dismissable && ReferenceEquals(app.Modal, m)) app.CloseModal(true);
            e.Handled = true;
        };
        grid.Children.Add(scrim);
        var card = new Border
        {
            Width = m.Width, Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(12),
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, Padding = new Thickness(20, 18, 20, 18),
            Effect = new System.Windows.Media.Effects.DropShadowEffect { BlurRadius = 30, ShadowDepth = 8, Direction = 270, Opacity = 0.28 },
            Child = m.Build(),
        };
        card.MouseLeftButtonDown += (_, e) => e.Handled = true;
        card.KeyDown += (_, e) => { if (e.Key == Key.Escape && m.Dismissable) { app.CloseModal(true); e.Handled = true; } };
        grid.Children.Add(card);
        // 卡片打开后把焦点放到第一个输入框
        card.Loaded += (_, _) => FocusFirstInput(card);
        return grid;
    }).Keyed(() => app.Modal).On(app.Overlay);

    private static void FocusFirstInput(DependencyObject root)
    {
        var count = VisualTreeHelper.GetChildrenCount(root);
        for (var i = 0; i < count; i++)
        {
            var c = VisualTreeHelper.GetChild(root, i);
            if (c is TextBox or PasswordBox && c is UIElement { IsVisible: true } u)
            {
                u.Focus();
                Keyboard.Focus(u as IInputElement);
                return;
            }
            FocusFirstInput(c);
            if (Keyboard.FocusedElement is TextBox or PasswordBox) return;
        }
    }

    private static FrameworkElement ToastLayer(AppModel app) => new Region(() =>
    {
        var t = app.CurrentToast;
        if (t == null) return null;
        Btn? action = null;
        if (t.ActionTitle != null && t.Action != null)
        {
            var act = t.Action;
            action = K.LinkButton(t.ActionTitle, () => { app.CurrentToast = null; app.Overlay.Notify(); act(); }, Th.Accent, 13);
            ((TextBlock)action.Child).FontWeight = FontWeights.SemiBold;
        }
        var b = new Border
        {
            Background = Th.Elevated, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(9), Padding = new Thickness(14, 9, action == null ? 14 : 8, 9),
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Bottom, Margin = new Thickness(0, 0, 0, 22), MaxWidth = 640,
            Effect = new System.Windows.Media.Effects.DropShadowEffect { BlurRadius = 18, ShadowDepth = 3, Direction = 270, Opacity = 0.2 },
            Child = K.Row(8, K.Icon(t.IsError ? "warning" : "checkCircle", 15, t.IsError ? Th.Danger : Th.Success), K.Grow(K.Text(t.Text, 13, Th.Text, wrap: true)), action),
            Cursor = Cursors.Hand,
        };
        b.MouseLeftButtonDown += (_, _) => { app.CurrentToast = null; app.Overlay.Notify(); };
        return b;
    }).Keyed(() => app.CurrentToast).On(app.Overlay);
}

/// <summary>自绘标题栏的最小化 / 最大化 / 关闭按钮</summary>
internal sealed class WindowButtons : StackPanel
{
    public WindowButtons()
    {
        Orientation = Orientation.Horizontal;
        Children.Add(Button("winMin", () => { var w = Window.GetWindow(this); if (w != null) w.WindowState = WindowState.Minimized; }, false));
        Children.Add(Button("winMax", () =>
        {
            var w = Window.GetWindow(this);
            if (w != null) w.WindowState = w.WindowState == WindowState.Maximized ? WindowState.Normal : WindowState.Maximized;
        }, false));
        Children.Add(Button("close", () => Window.GetWindow(this)?.Close(), true));
    }

    private static Btn Button(string icon, Action a, bool close)
    {
        var b = new Btn
        {
            Width = 46, Height = Shell.BarHeight - 1, HoverBrush = close ? Th.Hex("#E81123") : Th.Hover2, PressBrush = close ? Th.Hex("#F1707A") : Th.Hover2,
            Child = K.Center(K.Icon(icon, 11, Th.Secondary, 1.6)),
        };
        b.FocusVisualStyle = null;
        b.Focusable = false;
        b.UpdateLook();
        b.Click += a;
        if (close)
        {
            b.MouseEnter += (_, _) => b.Child = K.Center(K.Icon(icon, 11, Brushes.White, 1.6));
            b.MouseLeave += (_, _) => b.Child = K.Center(K.Icon(icon, 11, Th.Secondary, 1.6));
        }
        WindowChrome.SetIsHitTestVisibleInChrome(b, true);
        return b;
    }
}
