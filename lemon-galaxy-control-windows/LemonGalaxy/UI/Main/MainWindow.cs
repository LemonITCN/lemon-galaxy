using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Shell;

namespace LemonGalaxy.UI;

/// <summary>主窗口：自绘标题栏（WindowChrome），内容由 Shell.Root 构建；外观或语言变化时整体重建。</summary>
internal sealed class MainWindow : Window
{
    private readonly AppModel app;
    private readonly System.Windows.Controls.Border host = new();

    public static ImageSource? AppIcon
    {
        get
        {
            try
            {
                var path = IOPath.Combine(AppInfo.ResourcesDir, "LemonGalaxy.ico");
                return System.IO.File.Exists(path) ? BitmapFrame.Create(new Uri(path, UriKind.Absolute)) : null;
            }
            catch { return null; }
        }
    }

    public MainWindow(AppModel app)
    {
        this.app = app;
        Title = "LemonGalaxy";
        var s = app.Settings;
        Width = Math.Max(960, s.WindowWidth);
        Height = Math.Max(600, s.WindowHeight);
        MinWidth = 900;
        MinHeight = 560;
        WindowStartupLocation = WindowStartupLocation.CenterScreen;
        if (AppIcon is { } icon) Icon = icon;
        WindowChrome.SetWindowChrome(this, new WindowChrome
        {
            CaptionHeight = Shell.BarHeight,
            ResizeBorderThickness = new Thickness(6),
            GlassFrameThickness = new Thickness(0, 0, 0, 1),
            CornerRadius = new CornerRadius(0),
            UseAeroCaptionButtons = false,
        });
        Content = host;
        Rebuild();
        s.LookChanged.Watch(this, () => Ui.Coalesce(this, () =>
        {
            Th.Apply(s);
            L.Apply(s.Language);
            try { app.Client.SetLocale(L.Pack.CoreLocale); } catch { }
            Rebuild();
        }));
        StateChanged += (_, _) => FixMaximizedMargin();
        // 窗口级快捷键：只处理终端、远程画面没有占用的按键（它们自己标记了 Handled）
        KeyDown += (_, e) =>
        {
            var mods = System.Windows.Input.Keyboard.Modifiers;
            var ctrl = mods.HasFlag(System.Windows.Input.ModifierKeys.Control);
            if (ctrl && e.Key == System.Windows.Input.Key.W && app.ActiveTabId is { } id)
            {
                app.CloseTab(id);
                e.Handled = true;
            }
            else if (ctrl && e.Key == System.Windows.Input.Key.Tab && app.LoggedIn)
            {
                // 首页 + 各标签循环切换
                var ids = new List<string?> { null };
                ids.AddRange(app.Tabs.Select(t => (string?)t.Id));
                var i = ids.IndexOf(app.ActiveTabId);
                var step = mods.HasFlag(System.Windows.Input.ModifierKeys.Shift) ? -1 : 1;
                app.ActivateTab(ids[((i + step) % ids.Count + ids.Count) % ids.Count]);
                e.Handled = true;
            }
        };
        if (s.WindowMaximized) WindowState = WindowState.Maximized;
        Closing += (_, _) =>
        {
            s.WindowMaximized = WindowState == WindowState.Maximized;
            if (WindowState == WindowState.Normal)
            {
                s.WindowWidth = (int)ActualWidth;
                s.WindowHeight = (int)ActualHeight;
            }
            app.CloseAllTabs();
            app.Forwards.StopAll();
        };
        // 系统切换深浅色时跟随（外观设为「跟随系统」时）
        Microsoft.Win32.SystemEvents.UserPreferenceChanged += (_, e) =>
        {
            if (e.Category == Microsoft.Win32.UserPreferenceCategory.General && s.Appearance == "system")
                Dispatcher.BeginInvoke(new Action(() => s.LookChanged.Notify()));
        };
    }

    /// <summary>外观或语言变化：常驻控件（终端、画面）保留在标签模型里，其余全部重建</summary>
    private void Rebuild()
    {
        Background = Th.Content;
        foreach (var t in app.Tabs) t.ResetView();
        host.Child = Shell.Root(app);
    }

    /// <summary>WindowChrome 最大化时窗口会超出屏幕一圈边框，内容要缩进回来</summary>
    private void FixMaximizedMargin()
    {
        if (WindowState != WindowState.Maximized)
        {
            host.Margin = new Thickness(0);
            return;
        }
        double px = 8;
        try { px = GetSystemMetrics(32) + GetSystemMetrics(92); } catch { }
        var scale = 1.0;
        try { scale = VisualTreeHelper.GetDpi(this).DpiScaleX; } catch { }
        host.Margin = new Thickness(px / scale);
    }

    [DllImport("user32.dll")]
    private static extern int GetSystemMetrics(int index);

    protected override void OnSourceInitialized(EventArgs e)
    {
        base.OnSourceInitialized(e);
        FixMaximizedMargin();
        // Windows 11：圆角窗口
        try
        {
            var hwnd = new WindowInteropHelper(this).Handle;
            var pref = 2;
            DwmSetWindowAttribute(hwnd, 33, ref pref, sizeof(int));
        }
        catch { }
    }

    [DllImport("dwmapi.dll", PreserveSig = true)]
    private static extern int DwmSetWindowAttribute(IntPtr hwnd, int attr, ref int value, int size);
}
