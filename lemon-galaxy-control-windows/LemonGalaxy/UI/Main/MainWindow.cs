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
        StateChanged += (_, _) =>
        {
            FixMaximizedMargin();
            if (WindowState != WindowState.Minimized) restoreState = WindowState;
        };
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
            else if (ctrl && e.Key == System.Windows.Input.Key.N && app.LoggedIn && app.Section == NavSection.Ai)
            {
                // AI 页：新对话
                app.SelectHome(NavSection.Ai);
                app.Ai.NewChat();
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
        app.Mcp.Attention += OnMcpAttention;
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
            app.Mcp.Attention -= OnMcpAttention;
            tray?.Dispose();
            tray = null;
        };
        // 系统切换深浅色时跟随（外观设为「跟随系统」时）
        Microsoft.Win32.SystemEvents.UserPreferenceChanged += (_, e) =>
        {
            if (e.Category == Microsoft.Win32.UserPreferenceCategory.General && s.Appearance == "system")
                Dispatcher.BeginInvoke(new Action(() => s.LookChanged.Notify()));
        };
    }

    // ---------- MCP 待批准提醒 ----------

    private System.Windows.Forms.NotifyIcon? tray;
    private McpAttention? lastAttention;
    private readonly object attentionKey = new();

    /// <summary>
    /// 外部智能体有新的待批准：窗口不在前台时闪任务栏按钮并发系统通知（托盘气泡，Windows 10 起显示为通知横幅），
    /// 在前台但不在 MCP 页时在窗口里提示；点开都跳到 MCP 页。
    /// </summary>
    private void OnMcpAttention(McpAttention a)
    {
        var onPage = app.Section == NavSection.Mcp && app.ActiveTabId == null && app.LoggedIn;
        if (IsActive && onPage) return;
        lastAttention = a;
        // 一下子来好几个只提醒一次
        Ui.Coalesce(attentionKey, () =>
        {
            if (lastAttention is not { } x) return;
            lastAttention = null;
            if (!IsActive || WindowState == WindowState.Minimized)
            {
                FlashTaskbar();
                ShowTrayNotice(x);
            }
            else app.ShowToast(NoticeText(x), false, L.T("mcp.notify.view"), app.OpenMcp);
        });
    }

    private static string NoticeText(McpAttention a)
        => a.Pending > 1 ? L.T("mcp.notify.many", a.Pending) : L.T("mcp.notify.body", a.ClientName, a.What);

    [StructLayout(LayoutKind.Sequential)]
    private struct FlashInfo
    {
        public uint Size;
        public IntPtr Hwnd;
        public uint Flags;
        public uint Count;
        public uint Timeout;
    }

    [DllImport("user32.dll")]
    private static extern bool FlashWindowEx(ref FlashInfo info);

    /// <summary>闪任务栏按钮，直到窗口回到前台（FLASHW_ALL | FLASHW_TIMERNOFG）</summary>
    private void FlashTaskbar()
    {
        try
        {
            var hwnd = new WindowInteropHelper(this).Handle;
            if (hwnd == IntPtr.Zero) return;
            var info = new FlashInfo { Size = (uint)Marshal.SizeOf(typeof(FlashInfo)), Hwnd = hwnd, Flags = 3 | 12, Count = uint.MaxValue, Timeout = 0 };
            FlashWindowEx(ref info);
        }
        catch (Exception e) { Log.Error("闪任务栏按钮失败：" + e.Message); }
    }

    /// <summary>托盘图标只在有通知时出现：点通知或图标把窗口带到前台并打开 MCP 页，通知收起后图标随之隐藏</summary>
    private void ShowTrayNotice(McpAttention a)
    {
        try
        {
            if (tray == null)
            {
                tray = new System.Windows.Forms.NotifyIcon { Text = "LemonGalaxy" };
                try
                {
                    var ico = IOPath.Combine(AppInfo.ResourcesDir, "LemonGalaxy.ico");
                    tray.Icon = System.IO.File.Exists(ico) ? new System.Drawing.Icon(ico) : System.Drawing.SystemIcons.Application;
                }
                catch { tray.Icon = System.Drawing.SystemIcons.Application; }
                tray.BalloonTipClicked += (_, _) => Dispatcher.BeginInvoke(new Action(BringToMcp));
                tray.MouseClick += (_, _) => Dispatcher.BeginInvoke(new Action(BringToMcp));
                tray.BalloonTipClosed += (_, _) => Dispatcher.BeginInvoke(new Action(() => { if (tray != null) tray.Visible = false; }));
            }
            tray.Visible = true;
            tray.ShowBalloonTip(8000, L.T("mcp.notify.title"), NoticeText(a), System.Windows.Forms.ToolTipIcon.Info);
        }
        catch (Exception e) { Log.Error("显示 MCP 通知失败：" + e.Message); }
    }

    private void BringToMcp()
    {
        if (tray != null) tray.Visible = false;
        if (WindowState == WindowState.Minimized) WindowState = restoreState;
        Show();
        Activate();
        // 别的程序在前台时 Activate 可能只闪按钮：临时置顶一下把窗口带到最前
        Topmost = true;
        Topmost = false;
        Focus();
        app.OpenMcp();
    }

    private WindowState restoreState = WindowState.Normal;

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
