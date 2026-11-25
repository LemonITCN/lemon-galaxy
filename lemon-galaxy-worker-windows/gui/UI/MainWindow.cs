using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using LemonGalaxy.Setup;

namespace LemonGalaxy.UI;

internal enum Overall { NotRunning, Connecting, Disabled, Ai, Controlled, NoAccount, Online, Offline }

internal static class States
{
    public static Overall Of(AppModel app)
    {
        var s = app.Status;
        if (s == null) return app.Service is ServiceState.Running or ServiceState.Starting || !app.ConnectGraceOver ? Overall.Connecting : Overall.NotRunning;
        if (!s.RemoteEnabled) return Overall.Disabled;
        if (s.AiActive) return Overall.Ai;
        if (s.Controlled) return Overall.Controlled;
        if (s.AccountCount == 0) return Overall.NoAccount;
        return s.Online ? Overall.Online : Overall.Offline;
    }

    public static string Text(Overall o) => L.T(o switch
    {
        Overall.NotRunning => "state.notRunning",
        Overall.Connecting => "state.connecting",
        Overall.Disabled => "state.disabled",
        Overall.Ai => "state.ai",
        Overall.Controlled => "state.controlled",
        Overall.NoAccount => "state.noAccount",
        Overall.Online => "state.online",
        _ => "state.offline",
    });

    public static Color Color(Overall o) => o switch
    {
        Overall.Online => Th.P.Success,
        Overall.Controlled or Overall.Ai => Th.P.Warning,
        Overall.Disabled or Overall.NotRunning => Th.P.Danger,
        Overall.Offline => Th.P.Warning,
        _ => Th.P.TertiaryText,
    };
}

/// <summary>
/// 主窗口：系统原生标题栏（深色主题时标题栏也变深），左侧导航 + 右侧页面；服务没运行时整页换成安装 / 启动服务。
/// 关窗口只是藏到托盘，界面进程与被控服务都继续运行；从托盘菜单「退出界面」才退出。
/// </summary>
internal sealed class MainWindow : Window
{
    private readonly AppModel app;
    private readonly Border host = new();
    public bool ReallyClose;

    public static ImageSource? AppIcon
    {
        get
        {
            try
            {
                var path = IOPath.Combine(AppInfo.ResourcesDir, "LemonGalaxyWorker.ico");
                return System.IO.File.Exists(path) ? BitmapFrame.Create(new Uri(path, UriKind.Absolute)) : null;
            }
            catch { return null; }
        }
    }

    public MainWindow(AppModel app)
    {
        this.app = app;
        Title = L.T("app.name");
        var s = app.Settings;
        Width = Math.Max(880, s.WindowWidth);
        Height = Math.Max(580, s.WindowHeight);
        MinWidth = 820;
        MinHeight = 540;
        WindowStartupLocation = WindowStartupLocation.CenterScreen;
        if (AppIcon is { } icon) Icon = icon;
        Content = host;
        Rebuild();
        s.LookChanged.Watch(this, () => Ui.Coalesce(this, () =>
        {
            Th.Apply(s);
            L.Apply(s.Language);
            Title = L.T("app.name");
            ApplyDarkTitleBar();
            Rebuild();
        }));
        SourceInitialized += (_, _) => ApplyDarkTitleBar();
        Closing += (_, e) =>
        {
            s.WindowWidth = ActualWidth;
            s.WindowHeight = ActualHeight;
            s.Save();
            if (ReallyClose) return;
            // 关窗口：藏到托盘，进程与服务都继续
            e.Cancel = true;
            Hide();
            if (!s.ToldAboutTray)
            {
                s.ToldAboutTray = true;
                s.Save();
                Tray.Instance?.Balloon(L.T("tray.stillRunning.title"), L.T("tray.stillRunning.body"), null);
            }
        };
    }

    public void ShowPage(Page? p = null)
    {
        if (p != null) app.Go(p.Value);
        if (!IsVisible)
        {
            // 藏着（或从没显示过）时界面不跟着状态刷新：显示前按当前状态重建
            Rebuild();
            Show();
        }
        if (WindowState == WindowState.Minimized) WindowState = WindowState.Normal;
        Activate();
        Topmost = true;
        Topmost = false;
        Focus();
    }

    private void Rebuild()
    {
        Background = Th.Content;
        host.Child = Shell.Root(app);
    }

    [DllImport("dwmapi.dll")]
    private static extern int DwmSetWindowAttribute(IntPtr hwnd, int attr, ref int value, int size);

    /// <summary>Windows 10 1809 起：深色主题时标题栏也用深色</summary>
    private void ApplyDarkTitleBar()
    {
        try
        {
            var h = new WindowInteropHelper(this).Handle;
            if (h == IntPtr.Zero) return;
            var v = Th.P.IsDark ? 1 : 0;
            if (DwmSetWindowAttribute(h, 20, ref v, 4) != 0) DwmSetWindowAttribute(h, 19, ref v, 4);
        }
        catch { }
    }
}

/// <summary>窗口内容：导航 + 页面 + 提示条 + 弹层</summary>
internal static class Shell
{
    public static FrameworkElement Root(AppModel app)
    {
        var grid = new Grid();
        var body = new Region(() => Body(app)).On(app.StatusChanged).Keyed(() => app.Connected ? "main" : (object)(States.Of(app), app.Service, app.Busy));
        grid.Children.Add(body);
        grid.Children.Add(new Region(() => ToastBar(app)).On(app.ToastChanged));
        grid.Children.Add(new Region(() => ModalLayer(app)).On(app.ModalChanged));
        return grid;
    }

    private static FrameworkElement Body(AppModel app)
    {
        if (!app.Connected) return Pages.ServiceDown(app);
        var g = new Grid();
        g.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(220) });
        g.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1) });
        g.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var side = Sidebar(app);
        g.Children.Add(side);
        var line = new Border { Background = Th.Separator };
        Grid.SetColumn(line, 1);
        g.Children.Add(line);
        var page = new Region(() => Pages.Build(app)).On(app.PageChanged);
        Grid.SetColumn(page, 2);
        g.Children.Add(page);
        return g;
    }

    private static FrameworkElement Sidebar(AppModel app)
    {
        var head = new Region(() =>
        {
            var o = States.Of(app);
            return K.Pad(K.Row(10, AppMark(30), K.Grow(K.Col(2,
                K.Semi(L.T("app.name"), 13),
                K.Row(5, new System.Windows.Shapes.Ellipse { Width = 7, Height = 7, Fill = Th.B(States.Color(o)) }, K.Grow(K.Text(States.Text(o), 11.5, Th.Secondary)))))), 16, 18, 12, 14);
        }).On(app.StatusChanged);

        var nav = new Region(() =>
        {
            var items = new List<UIElement?>();
            foreach (var (p, icon) in new[] { (Page.Overview, "home"), (Page.Accounts, "users"), (Page.Assist, "hand"), (Page.Settings, "sliders"), (Page.Events, "list"), (Page.About, "info") })
            {
                if (p == Page.About) items.Add(K.Margin(K.HLine(), 6, 6, 6, 6));
                items.Add(NavItem(app, p, icon));
            }
            return K.Pad(K.Col(2, items), 8, 0, 8, 0);
        }).On(app.PageChanged, app.StatusChanged);

        var remote = new Region(() => RemoteCard(app)).On(app.StatusChanged).Keyed(() => (app.Status?.RemoteEnabled, app.Rights));
        var dock = new DockPanel { Background = Th.Sidebar, LastChildFill = true };
        DockPanel.SetDock(head, Dock.Top);
        dock.Children.Add(head);
        var bottom = K.Pad(remote, 12);
        DockPanel.SetDock(bottom, Dock.Bottom);
        dock.Children.Add(bottom);
        dock.Children.Add(K.Scroll(nav));
        return dock;
    }

    private static FrameworkElement NavItem(AppModel app, Page p, string icon)
    {
        var sel = app.Page == p;
        UIElement? badge = null;
        var s = app.Status;
        if (p == Page.Overview && s is { Sessions.Count: > 0 })
            badge = new Border { Background = Th.B(s.AiActive ? Th.P.Warning : Th.P.Danger), CornerRadius = new CornerRadius(8), Padding = new Thickness(6, 0, 6, 1), Child = K.Text(s.Sessions.Count.ToString(), 11, Brushes.White, FontWeights.SemiBold) };
        else if (p == Page.Accounts && s is { NeedsRelogin: > 0 })
            badge = K.Icon("alertCircle", 13, Th.Warning);
        else if (p == Page.Assist && s?.Assist != null)
            badge = new System.Windows.Shapes.Ellipse { Width = 7, Height = 7, Fill = Th.Success };
        var b = new Btn
        {
            Height = 32, CornerRadius = new CornerRadius(7), Padding = new Thickness(10, 0, 10, 0),
            Normal = sel ? Th.SidebarSelection : Brushes.Transparent, HoverBrush = sel ? Th.SidebarSelection : Th.Hover2,
            Child = K.Row(10, K.Icon(icon, 15, sel ? Th.Accent : Th.Secondary), K.Grow(K.Text(L.T("page." + p.ToString().ToLowerInvariant()), 13, Th.SidebarText, sel ? FontWeights.SemiBold : FontWeights.Normal)), badge),
        };
        b.UpdateLook();
        b.Click += () => app.Go(p);
        return b;
    }

    /// <summary>导航底部：允许远程开关（禁止远程屏幕前的人就能做；重新允许要管理员）</summary>
    private static FrameworkElement RemoteCard(AppModel app)
    {
        var s = app.Status;
        var on = s?.RemoteEnabled ?? false;
        var r = app.Rights;
        var enabled = s != null && (r.CanManage || (r.Operate && on));
        var sw = W.Switch(on, async v => await app.Do(Ipc.Req.SetRemoteEnabled(v)) != null, enabled, shield: !on && r.NeedsElevation);
        return new Border
        {
            Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(9), Padding = new Thickness(12, 10, 12, 10),
            Child = K.Row(8, K.Grow(K.Col(2, K.Text(L.T("overview.allowRemote"), 12.5, Th.Text, FontWeights.Medium),
                K.Text(on ? L.T("common.on") : L.T("common.off"), 11, on ? Th.Success : Th.Danger))), sw),
        };
    }

    /// <summary>被控端标志：深青底的星轨弧线（与 Mac 被控端相同，便于和主控端区分）</summary>
    public static FrameworkElement AppMark(double size)
    {
        var g = new Grid { Width = size, Height = size };
        g.Children.Add(new Border { CornerRadius = new CornerRadius(size * 0.24), Background = new LinearGradientBrush(Palettes.Hex("#15363A"), Palettes.Hex("#0A1719"), 90) });
        var pad = size * 0.2;
        var inner = size - pad * 2;
        var c = new Point(pad + inner / 2, pad + inner / 2);
        var geo = new PathGeometry();
        void Arc(double r, double from, double to)
        {
            Point P(double deg) => new(c.X + r * Math.Cos(deg * Math.PI / 180), c.Y + r * Math.Sin(deg * Math.PI / 180));
            var span = ((to - from) % 360 + 360) % 360;
            var f = new PathFigure { StartPoint = P(from), IsClosed = false };
            f.Segments.Add(new ArcSegment(P(to), new Size(r, r), 0, span > 180, SweepDirection.Clockwise, true));
            geo.Figures.Add(f);
        }
        Arc(inner * 0.5, 200, 340);
        Arc(inner * 0.5, 20, 160);
        Arc(inner * 0.3, 290, 70);
        g.Children.Add(new System.Windows.Shapes.Path
        {
            Data = geo, StrokeThickness = size * 0.055, StrokeStartLineCap = PenLineCap.Round, StrokeEndLineCap = PenLineCap.Round,
            Stroke = new LinearGradientBrush(Palettes.Hex("#F5D90A"), Palettes.Hex("#7BD35A"), new Point(0, 0), new Point(1, 1)),
        });
        var dot = size * 0.13;
        g.Children.Add(new System.Windows.Shapes.Ellipse { Width = dot, Height = dot, Fill = Th.Hex("#F5D90A"), HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center });
        return g;
    }

    private static FrameworkElement ToastBar(AppModel app)
    {
        if (app.Toast == null) return new Border { IsHitTestVisible = false };
        var bar = new Border
        {
            Background = Th.Elevated, BorderBrush = app.ToastError ? Th.B(Th.P.Danger, 0.5) : Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(18),
            Padding = new Thickness(14, 8, 8, 8), Margin = new Thickness(0, 14, 0, 0), HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Top, MaxWidth = 560,
            Effect = new System.Windows.Media.Effects.DropShadowEffect { BlurRadius = 12, ShadowDepth = 3, Opacity = Th.P.IsDark ? 0.4 : 0.14, Direction = 270 },
            Child = K.Row(8, K.Icon(app.ToastError ? "alertCircle" : "checkCircle", 14, app.ToastError ? Th.Danger : Th.Success), K.Text(app.Toast, 12.5, Th.Text, wrap: true),
                K.IconButton("close", L.T("common.close"), () => { app.Toast = null; app.ToastChanged.Notify(); }, size: 11, w: 22, h: 22)),
        };
        return bar;
    }

    private static FrameworkElement ModalLayer(AppModel app)
    {
        if (app.Modals.Count == 0) return new Border { IsHitTestVisible = false };
        var g = new Grid { Background = Th.Scrim };
        foreach (var m in app.Modals)
        {
            Region.Detach(m);
            g.Children.Add(m);
        }
        return g;
    }
}
