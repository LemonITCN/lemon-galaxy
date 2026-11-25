using System.Diagnostics;
using System.IO;
using System.Security.Cryptography;
using System.Text;
using System.Windows;
using LemonGalaxy.Ipc;
using LemonGalaxy.Setup;
using LemonGalaxy.UI;

namespace LemonGalaxy;

/// <summary>
/// 入口。
///   LemonGalaxyWorker.exe                      打开主窗口（已在运行就把窗口拉到前面）
///   LemonGalaxyWorker.exe --tray               登录后自启：只放托盘图标（本用户关了「登录后在托盘显示」时直接退出）
///   LemonGalaxyWorker.exe --replace            替换同一位置正在运行的旧实例（升级后用）
///   LemonGalaxyWorker.exe --selftest &lt;目录&gt; 离屏渲染各页面成 PNG、核对协议与语言包，跑完退出；退出码 0 成功
///       可选 --live：再连真的被控服务（环境变量 LEMON_GALAXY_CONTROL_SOCKET 指定管道）走一遍
/// 以管理员身份运行的子命令（界面经 UAC 调用，见 Control/Elevation.cs、Setup/Setup.cs）：
///   --elevate &lt;钥匙&gt; --pipe &lt;管道&gt;    把钥匙交给服务，授权界面那个连接几分钟内直接改配置
///   --install [--no-autostart] [--no-tray]     安装（复制到 Program Files、装服务、托盘自启、开始菜单、卸载入口）
///   --start-service                            启动服务
///   --uninstall                                卸载（「应用和功能」里调用；没提权时自己弹 UAC）
/// </summary>
internal static class Program
{
    private static Mutex? single;
    private static EventWaitHandle? showEvent, quitEvent;
    private static Application? app;
    private static MainWindow? window;

    [STAThread]
    private static int Main(string[] args)
    {
        Ui.Init();
        string? Arg(string name)
        {
            var i = Array.IndexOf(args, name);
            return i >= 0 && i + 1 < args.Length ? args[i + 1] : null;
        }
        bool Has(string name) => Array.IndexOf(args, name) >= 0;

        var settings = AppSettings.Shared;
        if (Has("--selftest"))
        {
            var dir = Arg("--selftest") is { } d && !d.StartsWith("--", StringComparison.Ordinal) ? d : "selftest";
            return SelfTest.Run(dir, args);
        }
        try { Directory.CreateDirectory(AppInfo.DataDir); } catch { }
        Log.FilePath = IOPath.Combine(AppInfo.DataDir, "ui.log");
        AppDomain.CurrentDomain.UnhandledException += (_, e) => Log.Error("未处理的异常：" + e.ExceptionObject);
        settings.Load();
        L.LoadPacks();
        L.Apply(settings.Language);

        // 以管理员身份运行的子命令：没有窗口，做完就退
        var result = Arg("--result");
        if (Arg("--elevate") is { } key) return Elevation.ElevateMain(key, Arg("--pipe") ?? ControlClient.SystemPipe, result);
        if (Has("--install")) return Installer.InstallMain(!Has("--no-autostart"), !Has("--no-tray"), result);
        if (Has("--start-service")) return Installer.StartServiceMain(result);
        if (Has("--uninstall")) return Uninstall(result, Has("--confirmed"), int.TryParse(Arg("--keep"), out var keep) ? keep : 0);

        var tray = Has("--tray");
        if (tray && !settings.TrayAtLogin) return 0;
        // 单实例（按程序位置区分：解压目录里的新版本与已安装的旧版本可以同时开，安装后由新的那份 --replace 接替）
        var tag = Tag(AppInfo.ExePath);
        single = new Mutex(false, @"Local\LemonGalaxyWorkerUI." + tag);
        showEvent = new EventWaitHandle(false, EventResetMode.AutoReset, @"Local\LemonGalaxyWorkerUI.show." + tag);
        quitEvent = new EventWaitHandle(false, EventResetMode.AutoReset, @"Local\LemonGalaxyWorkerUI.quit." + tag);
        bool mine;
        try { mine = single.WaitOne(0); }
        catch (AbandonedMutexException) { mine = true; }
        if (!mine)
        {
            if (Has("--replace"))
            {
                quitEvent.Set();
                try { mine = single.WaitOne(8000); }
                catch (AbandonedMutexException) { mine = true; }
            }
            if (!mine)
            {
                // 已经在运行：再打开一次就把窗口切到前面；登录自启的那份不打扰
                if (!tray) showEvent.Set();
                return 0;
            }
        }
        Installer.CleanOldFiles();
        Region.RequireAttached = true;
        app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
        app.DispatcherUnhandledException += (_, e) =>
        {
            Log.Error("界面线程未处理的异常：" + e.Exception);
            AppModel.Current?.ShowError(e.Exception);
            e.Handled = true;
        };
        // 注销、关机：窗口直接关，不藏到托盘（否则会挡住注销）
        app.SessionEnding += (_, _) =>
        {
            if (window != null) window.ReallyClose = true;
        };
        Th.Apply(settings);
        Styles.Install(app);
        var model = new AppModel(settings, selfTest: false);
        window = new MainWindow(model);
        app.MainWindow = window;
        var trayIcon = new Tray(model, window);
        Listen(showEvent, () => window.ShowPage());
        Listen(quitEvent, Exit);
        if (!tray) window.ShowPage();
        var code = app.Run();
        trayIcon.Dispose();
        model.Client?.Dispose();
        try { single.ReleaseMutex(); } catch { }
        return code;
    }

    /// <summary>后台线程等命名事件，触发后在界面线程执行</summary>
    private static void Listen(EventWaitHandle ev, Action a)
    {
        new Thread(() =>
        {
            while (true)
            {
                ev.WaitOne();
                Ui.Post(a);
            }
        }) { IsBackground = true, Name = "single-instance" }.Start();
    }

    private static string Tag(string path)
    {
        using var sha = SHA256.Create();
        var h = sha.ComputeHash(Encoding.UTF8.GetBytes(path.ToLowerInvariant()));
        return string.Concat(h.Take(8).Select(b => b.ToString("x2")));
    }

    /// <summary>退出界面（被控服务照常运行）</summary>
    public static void Exit()
    {
        if (window != null) window.ReallyClose = true;
        try { window?.Close(); } catch { }
        app?.Shutdown();
    }

    /// <summary>安装好了：打开装好的那份（普通权限，接替同位置的旧实例），自己退出</summary>
    public static void LaunchInstalledAndExit()
    {
        try
        {
            if (!AppInfo.RunningInstalled)
                Process.Start(new ProcessStartInfo(AppInfo.InstalledExe, "--replace") { UseShellExecute = true, WorkingDirectory = AppInfo.InstallDir });
            else
                return;
        }
        catch (Exception e)
        {
            AppModel.Current?.ShowError(e);
            return;
        }
        Exit();
    }

    /// <summary>
    /// 卸载。「应用和功能」里调用时先确认（没提权就自己弹 UAC 再做）；界面「关于」页调用时已确认过（--confirmed），
    /// 并带上界面自己的进程号（--keep），卸载时不关它，由它自己提示后退出。
    /// </summary>
    private static int Uninstall(string? result, bool confirmed, int keep)
    {
        if (!confirmed)
        {
            var ok = MessageBox.Show(L.T("about.uninstall.body", AppInfo.ServiceDataDir), L.T("about.uninstall.title"), MessageBoxButton.OKCancel, MessageBoxImage.Question);
            if (ok != MessageBoxResult.OK) return 1;
        }
        if (Elevation.IsAdmin)
        {
            var code = Installer.UninstallMain(result, keep);
            if (!confirmed) MessageBox.Show(code == 0 ? L.T("about.uninstall.done") : L.T("setup.failed", ""), L.T("app.name"), MessageBoxButton.OK, code == 0 ? MessageBoxImage.Information : MessageBoxImage.Error);
            return code;
        }
        var r = Elevation.RunSync(new[] { "--uninstall", "--confirmed", "--keep", Process.GetCurrentProcess().Id.ToString() });
        if (r.Ok) MessageBox.Show(L.T("about.uninstall.done"), L.T("app.name"), MessageBoxButton.OK, MessageBoxImage.Information);
        else if (!r.Cancelled) MessageBox.Show(L.T("setup.failed", r.Error ?? ""), L.T("app.name"), MessageBoxButton.OK, MessageBoxImage.Error);
        return r.Ok ? 0 : 1;
    }
}
