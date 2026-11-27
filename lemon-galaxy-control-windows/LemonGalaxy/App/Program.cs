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
///   LemonGalaxy.exe --mcp-stdio           MCP stdio 桥接（Claude Desktop 等只支持 stdio 的外部智能体启动它）：不建窗口，
///                                         读标准输入、写标准输出，转给本机已开启的 MCP 服务，输入结束后以内核给的退出码退出
///   LemonGalaxy.exe --apply-update &lt;plan.json&gt;   更新辅助程序（复制到更新目录里运行）：不建窗口，换文件后打开新程序（App/UpdateInstall.cs）
///   --update-token / --updated-from / --update-failed   辅助程序打开新程序或换回旧程序时带的参数（见 App/UpdateInstall.cs 开头的约定）
/// </summary>
internal static class Program
{
    [STAThread]
    private static int Main(string[] args)
    {
        // 最早判断：标准输入输出是 MCP 协议通道（句柄由父进程继承，内核直接读写），这里不建任何窗口、不碰 Console
        if (Array.IndexOf(args, "--mcp-stdio") >= 0) return McpStdio.Run();
        if (UpdateStartup.Arg(args, "--apply-update") is { } plan) return UpdateHelper.Run(plan);
        Ui.Init();
        var selftest = Array.IndexOf(args, "--selftest");
        if (selftest >= 0)
        {
            var dir = selftest + 1 < args.Length && !args[selftest + 1].StartsWith("--", StringComparison.Ordinal) ? args[selftest + 1] : "selftest";
            return SelfTest.Run(dir, args);
        }
        var updateToken = UpdateStartup.Arg(args, "--update-token");
        if (updateToken != null && Array.IndexOf(args, "--update-probe") >= 0) return UpdateProbe(updateToken, Array.IndexOf(args, "--update-failed") >= 0);
        try { Directory.CreateDirectory(AppInfo.DataDir); } catch { }
        Log.FilePath = IOPath.Combine(AppInfo.DataDir, "ui.log");
        AppDomain.CurrentDomain.UnhandledException += (_, e) => Log.Error("未处理的异常：" + e.ExceptionObject);
        var updateEnv = UpdateEnv.Default;
        // 更新正在装：等它装完，辅助程序会打开新程序，这一个不用再开。
        // 辅助程序打开的（带令牌）：它还拿着更新互斥量等本进程报到，不等、不恢复、不转去别处
        var running = updateToken == null ? UpdateStartup.WaitForRunningUpdate(updateEnv, 300_000) : UpdateStartup.RunningUpdate.StillRunning;
        if (running == UpdateStartup.RunningUpdate.Finished) return 0;
        if (running == UpdateStartup.RunningUpdate.None)
        {
            // 程序目录里有换到一半的文件（断电、辅助程序被结束）：换回旧文件后重新打开
            var recoverLog = new UpdateLog(updateEnv.LogFile);
            if (FileSwap.RecoverAll(AppInfo.BaseDir, recoverLog))
            {
                ProcessLauncher.Instance.Start(UpdateEnv.CurrentExe, UpdateStartup.JoinArgs(args), AppInfo.BaseDir, recoverLog);
                return 0;
            }
            // 原目录不可写时更新装到了用户目录：从旧位置（旧快捷方式）打开时转去新位置（开发版不转）
            if (AppInfo.Build != "dev" && InstallPointer.RedirectTarget(updateEnv, UpdateEnv.CurrentExe, AppInfo.Version) is { } moved)
            {
                Log.Info("程序已装到 " + moved + "，转去打开那一份");
                if (ProcessLauncher.Instance.Start(moved, UpdateStartup.JoinArgs(args), IOPath.GetDirectoryName(moved)!, recoverLog)) return 0;
            }
        }
        ThreadPool.QueueUserWorkItem(_ => UpdateStartup.Cleanup(updateEnv, AppInfo.BaseDir));
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
        var updateFailed = Array.IndexOf(args, "--update-failed") >= 0;
        if (model.CoreFallbackReason != null)
        {
            // 刚更新上来的新版本连内核都加载不了：不弹框，告诉辅助程序换回旧版本
            if (updateToken != null && !updateFailed)
            {
                UpdateStartup.ReportStartupFailure(updateEnv, updateToken, model.CoreFallbackReason);
                return 3;
            }
            MessageBox.Show(L.T("about.mock.fallback", model.CoreFallbackReason), "LemonGalaxy", MessageBoxButton.OK, MessageBoxImage.Warning);
        }
        var window = new MainWindow(model);
        if (updateToken != null) model.Updater.AfterRelaunch(window, updateToken, updateFailed);
        app.MainWindow = window;
        var code = app.Run(window);
        // 剪贴板里放着的对方文件随本进程失效，退出前清掉
        VirtualClipboard.Shutdown();
        return code;
    }

    /// <summary>
    /// --update-probe（build.sh 的更新端到端检查用）：不开窗口、不碰数据目录，只加载同目录的内核库（核对绑定校验），
    /// 成功就像正常启动那样报到并退出；失败写下原因、以退出码 3 退出（辅助程序据此换回旧版本）。
    /// </summary>
    private static int UpdateProbe(string token, bool failed)
    {
        var env = UpdateEnv.Default;
        try
        {
            var v = GalaxyFfiMethods.CoreVersion();
            new UpdateLog(env.LogFile).Write($"检查：{AppInfo.Version} 的内核 {v} 加载正常");
            UpdateStartup.Signal(env, token);
            return 0;
        }
        catch (Exception e)
        {
            var inner = e is TypeInitializationException { InnerException: { } ie } ? ie : e;
            if (!failed) UpdateStartup.ReportStartupFailure(env, token, inner.GetType().Name + ": " + inner.Message);
            return 3;
        }
    }
}

/// <summary>--mcp-stdio：调用内核的 stdio 桥接（docs/mcp.md 第 5 节），数据目录与界面版相同（从那里读 mcp-endpoint.json）</summary>
internal static class McpStdio
{
    public static int Run()
    {
        try { return GalaxyFfiMethods.McpStdioBridge(AppInfo.DataDir); }
        catch (Exception e)
        {
            // 标准输出是协议通道，不能写；原因记到数据目录里（多半是 exe 旁边没有 galaxy_ffi.dll）
            try
            {
                Directory.CreateDirectory(AppInfo.DataDir);
                File.AppendAllText(IOPath.Combine(AppInfo.DataDir, "mcp-stdio.log"), $"{DateTime.Now:yyyy-MM-dd HH:mm:ss} {e}{Environment.NewLine}");
            }
            catch { }
            return 1;
        }
    }
}
