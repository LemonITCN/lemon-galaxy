using System.IO;
using System.Windows;
using LemonGalaxy.Core;
using LemonGalaxy.UI;

namespace LemonGalaxy;

/// <summary>
/// 入口。
///   LemonGalaxy.exe                       正常运行（exe 同目录有 galaxy_ffi.dll 时用 Rust 内核，否则退回模拟内核）
///   LemonGalaxy.exe --mock                用模拟内核（假设备、假文件、假 AI），用于界面开发与演示
///   LemonGalaxy.exe --selftest &lt;目录&gt;   用模拟内核把主要界面离屏渲染成 PNG，跑完退出；退出码 0 成功
///       可选 --scale 1.5、--only 名称片段、--lang zh-Hans|en、--theme light|dark
/// </summary>
internal static class Program
{
    [STAThread]
    private static int Main(string[] args)
    {
        Ui.Init();
        var selftest = Array.IndexOf(args, "--selftest");
        if (selftest >= 0)
        {
            var dir = selftest + 1 < args.Length && !args[selftest + 1].StartsWith("--", StringComparison.Ordinal) ? args[selftest + 1] : "selftest";
            return SelfTest.Run(dir, args);
        }
        try { Directory.CreateDirectory(AppInfo.DataDir); } catch { }
        Log.FilePath = IOPath.Combine(AppInfo.DataDir, "ui.log");
        AppDomain.CurrentDomain.UnhandledException += (_, e) => Log.Error("未处理的异常：" + e.ExceptionObject);
        Region.RequireAttached = true;
        // 旧版本远程桌面复制粘贴文件的下载缓存（现在粘贴时才传，不再写入）：一天前的在后台删掉
        ThreadPool.QueueUserWorkItem(_ => ClipFiles.CleanStale(AppInfo.ClipboardDir));
        var app = new Application { ShutdownMode = ShutdownMode.OnMainWindowClose };
        app.DispatcherUnhandledException += (_, e) =>
        {
            Log.Error("界面线程未处理的异常：" + e.Exception);
            AppModel.Current?.ShowError(e.Exception);
            e.Handled = true;
        };
        var settings = AppSettings.Shared;
        settings.Load();
        L.LoadPacks();
        L.Apply(settings.Language);
        Th.Apply(settings);
        Styles.Install(app);
        var model = new AppModel(settings, selfTest: false, forceMock: CoreProvider.WantsMock(args));
        if (model.CoreFallbackReason != null)
            MessageBox.Show(L.T("about.mock.fallback", model.CoreFallbackReason), "LemonGalaxy", MessageBoxButton.OK, MessageBoxImage.Warning);
        var window = new MainWindow(model);
        app.MainWindow = window;
        var code = app.Run(window);
        // 剪贴板里放着的对方文件随本进程失效，退出前清掉
        VirtualClipboard.Shutdown();
        return code;
    }
}
