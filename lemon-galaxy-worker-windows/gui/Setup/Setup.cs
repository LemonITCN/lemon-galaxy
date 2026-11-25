using System.Diagnostics;
using System.IO;
using System.ServiceProcess;
using System.Text;
using Microsoft.Win32;

namespace LemonGalaxy.Setup;

internal enum ServiceState { Unknown, NotInstalled, Stopped, Starting, Running }

/// <summary>
/// 安装、卸载、启动服务（docs/spec.md 3.0.2）。界面以普通权限运行，这些都在以管理员身份运行的子命令里做：
///   --install [--no-autostart] [--no-tray]   复制到 Program Files\LemonGalaxy\Worker、装服务、所有用户登录后显示托盘、开始菜单、「应用和功能」里的卸载入口
///   --uninstall                               停掉并删除服务、删除程序文件与入口（账号与设置保留在 ProgramData）
///   --start-service                           启动已安装的服务
/// </summary>
internal static class Installer
{
    public const string UninstallKey = @"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\LemonGalaxyWorker";
    public const string RunKey = @"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
    public const string RunValue = "LemonGalaxyWorker";

    public static ServiceState QueryService()
    {
        try
        {
            using var sc = new ServiceController(AppInfo.ServiceName);
            return sc.Status switch
            {
                ServiceControllerStatus.Running => ServiceState.Running,
                ServiceControllerStatus.StartPending or ServiceControllerStatus.ContinuePending => ServiceState.Starting,
                _ => ServiceState.Stopped,
            };
        }
        catch (InvalidOperationException) { return ServiceState.NotInstalled; }
        catch { return ServiceState.Unknown; }
    }

    private static RegistryKey Hklm => RegistryKey.OpenBaseKey(RegistryHive.LocalMachine, Environment.Is64BitOperatingSystem ? RegistryView.Registry64 : RegistryView.Default);

    /// <summary>安装程序写了「所有用户登录后显示托盘」</summary>
    public static bool MachineTrayEntry
    {
        get
        {
            try
            {
                using var k = Hklm.OpenSubKey(RunKey);
                return k?.GetValue(RunValue) is string;
            }
            catch { return false; }
        }
    }

    /// <summary>
    /// 本用户登录后显示托盘：装好的程序由 HKLM\…\Run 对所有用户启动（--tray），本用户关掉时启动后马上退出（设置里记着）；
    /// 没装（直接从解压目录运行）时写本用户的 HKCU\…\Run。
    /// </summary>
    public static void SetUserTray(bool on)
    {
        var s = AppSettings.Shared;
        s.TrayAtLogin = on;
        s.Save();
        try
        {
            using var k = Registry.CurrentUser.CreateSubKey(RunKey);
            if (on && !MachineTrayEntry) k.SetValue(RunValue, "\"" + AppInfo.ExePath + "\" --tray");
            else k.DeleteValue(RunValue, false);
        }
        catch (Exception e) { Log.Error("改登录启动项失败：" + e.Message); }
    }

    // ---------------- 以管理员身份运行的部分 ----------------

    private static readonly string[] Files = { "LemonGalaxyWorker.exe", "LemonGalaxyWorker.exe.config", "lemon-galaxy-worker.exe", "LICENSE.txt" };

    public static int InstallMain(bool autostart, bool tray, string? result)
    {
        try
        {
            Install(autostart, tray);
            return 0;
        }
        catch (Exception e)
        {
            Log.Error("安装失败：" + e);
            Ipc.Elevation.WriteResult(result, e.Message);
            return 1;
        }
    }

    private static void Install(bool autostart, bool tray)
    {
        var src = AppInfo.BaseDir;
        var dst = AppInfo.InstallDir;
        Directory.CreateDirectory(dst);
        if (!AppInfo.SamePath(src, dst))
        {
            StopService();
            foreach (var f in Files)
            {
                var from = IOPath.Combine(src, f);
                if (File.Exists(from)) CopyReplacing(from, IOPath.Combine(dst, f));
            }
            var res = IOPath.Combine(src, "Resources");
            if (Directory.Exists(res)) CopyDir(res, IOPath.Combine(dst, "Resources"));
        }
        // 服务：install-service 发现程序已在 Program Files 时不再复制，只建 / 改服务并启动
        var worker = IOPath.Combine(dst, "lemon-galaxy-worker.exe");
        if (!File.Exists(worker)) throw new FileNotFoundException(L.T("setup.noWorker"), worker);
        var (code, output) = RunHidden(worker, "install-service");
        if (code != 0) throw new Exception(output.Length > 0 ? output : $"install-service: {code}");
        if (!autostart) RunHidden(IOPath.Combine(Environment.SystemDirectory, "sc.exe"), $"config {AppInfo.ServiceName} start= demand");
        var exe = IOPath.Combine(dst, "LemonGalaxyWorker.exe");
        using (var run = Hklm.CreateSubKey(RunKey))
        {
            if (tray) run.SetValue(RunValue, "\"" + exe + "\" --tray");
            else run.DeleteValue(RunValue, false);
        }
        var lnk = CreateShortcut(exe);
        using (var u = Hklm.CreateSubKey(UninstallKey))
        {
            if (lnk != null) u.SetValue("LemonGalaxyShortcut", lnk);
            u.SetValue("DisplayName", L.T("app.name"));
            u.SetValue("DisplayVersion", AppInfo.Version);
            u.SetValue("Publisher", "LemonIT.CN");
            u.SetValue("InstallLocation", dst);
            u.SetValue("DisplayIcon", exe + ",0");
            u.SetValue("UninstallString", "\"" + exe + "\" --uninstall");
            u.SetValue("URLInfoAbout", AppInfo.RepoUrl);
            u.SetValue("NoModify", 1, RegistryValueKind.DWord);
            u.SetValue("NoRepair", 1, RegistryValueKind.DWord);
            var size = Directory.GetFiles(dst, "*", SearchOption.AllDirectories).Sum(f => new FileInfo(f).Length) / 1024;
            u.SetValue("EstimatedSize", (int)size, RegistryValueKind.DWord);
        }
        Log.Info("已安装到 " + dst);
    }

    public static int StartServiceMain(string? result)
    {
        try
        {
            using var sc = new ServiceController(AppInfo.ServiceName);
            if (sc.Status != ServiceControllerStatus.Running)
            {
                sc.Start();
                sc.WaitForStatus(ServiceControllerStatus.Running, TimeSpan.FromSeconds(20));
            }
            return 0;
        }
        catch (Exception e)
        {
            Ipc.Elevation.WriteResult(result, e.InnerException?.Message ?? e.Message);
            return 1;
        }
    }

    /// <param name="keep">不关的界面进程（发起卸载的那个，它自己提示后退出）</param>
    public static int UninstallMain(string? result, int keep)
    {
        try
        {
            // 别的用户、别的会话里开着的界面一起关掉（文件要删）
            foreach (var p in Process.GetProcessesByName("LemonGalaxyWorker"))
            {
                if (p.Id == Process.GetCurrentProcess().Id || p.Id == keep) continue;
                try { p.Kill(); p.WaitForExit(3000); } catch { }
            }
            var dst = AppInfo.InstallDir;
            var worker = IOPath.Combine(dst, "lemon-galaxy-worker.exe");
            if (File.Exists(worker)) RunHidden(worker, "uninstall-service");
            else RunHidden(IOPath.Combine(Environment.SystemDirectory, "sc.exe"), $"delete {AppInfo.ServiceName}");
            using (var run = Hklm.OpenSubKey(RunKey, true)) run?.DeleteValue(RunValue, false);
            try { using var k = Registry.CurrentUser.OpenSubKey(RunKey, true); k?.DeleteValue(RunValue, false); } catch { }
            string? lnk;
            using (var u = Hklm.OpenSubKey(UninstallKey)) lnk = u?.GetValue("LemonGalaxyShortcut") as string;
            Hklm.DeleteSubKeyTree(UninstallKey, false);
            if (lnk != null && lnk.EndsWith(".lnk", StringComparison.OrdinalIgnoreCase)) { try { File.Delete(lnk); } catch { } }
            // 自己正在运行，删不掉自己：交给延时执行的 cmd（等本进程退出后删整个目录）
            if (Directory.Exists(dst))
            {
                var cmd = IOPath.Combine(Environment.SystemDirectory, "cmd.exe");
                Process.Start(new ProcessStartInfo(cmd, $"/c ping 127.0.0.1 -n 5 >nul & rd /s /q \"{dst}\"") { CreateNoWindow = true, UseShellExecute = false, WindowStyle = ProcessWindowStyle.Hidden });
            }
            Log.Info("已卸载");
            return 0;
        }
        catch (Exception e)
        {
            Ipc.Elevation.WriteResult(result, e.Message);
            return 1;
        }
    }

    private static void StopService()
    {
        try
        {
            using var sc = new ServiceController(AppInfo.ServiceName);
            if (sc.Status != ServiceControllerStatus.Stopped)
            {
                sc.Stop();
                sc.WaitForStatus(ServiceControllerStatus.Stopped, TimeSpan.FromSeconds(20));
            }
        }
        catch (InvalidOperationException) { }
        catch (System.ServiceProcess.TimeoutException) { }
    }

    /// <summary>目标文件正被运行（旧版界面开着、会话代理还没退）时先改名挪开，运行中的程序照常跑完</summary>
    private static void CopyReplacing(string from, string to)
    {
        try
        {
            File.Copy(from, to, true);
        }
        catch (IOException)
        {
            var old = to.EndsWith(".exe", StringComparison.OrdinalIgnoreCase) ? to.Substring(0, to.Length - 4) + ".old.exe" : to + ".old";
            try { File.Delete(old); } catch { }
            File.Move(to, old);
            File.Copy(from, to, true);
        }
    }

    private static void CopyDir(string from, string to)
    {
        Directory.CreateDirectory(to);
        foreach (var f in Directory.GetFiles(from)) CopyReplacing(f, IOPath.Combine(to, IOPath.GetFileName(f)));
        foreach (var d in Directory.GetDirectories(from)) CopyDir(d, IOPath.Combine(to, IOPath.GetFileName(d)));
    }

    /// <summary>上次升级挪开的旧文件（启动时顺手删）</summary>
    public static void CleanOldFiles()
    {
        try
        {
            foreach (var f in Directory.GetFiles(AppInfo.BaseDir, "*.old*")) { try { File.Delete(f); } catch { } }
        }
        catch { }
    }

    public static (int code, string output) RunHidden(string exe, string args)
    {
        var psi = new ProcessStartInfo(exe, args)
        {
            UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardError = true,
            StandardOutputEncoding = Encoding.UTF8, StandardErrorEncoding = Encoding.UTF8,
        };
        psi.EnvironmentVariables["LEMON_GALAXY_LANG"] = L.IsChinese ? "zh-CN" : "en";
        using var p = Process.Start(psi)!;
        var err = p.StandardError.ReadToEndAsync();
        var outText = p.StandardOutput.ReadToEnd();
        p.WaitForExit();
        var all = (outText + "\n" + err.Result).Trim();
        Log.Info($"{IOPath.GetFileName(exe)} {args} → {p.ExitCode} {all}");
        return (p.ExitCode, all);
    }

    /// <summary>开始菜单（所有用户）里的快捷方式，名字按安装时的界面语言；返回路径，卸载时按它删</summary>
    private static string? CreateShortcut(string exe)
    {
        try
        {
            var path = IOPath.Combine(Environment.GetFolderPath(Environment.SpecialFolder.CommonPrograms), L.T("app.name") + ".lnk");
            var t = Type.GetTypeFromProgID("WScript.Shell");
            if (t == null) return null;
            dynamic shell = Activator.CreateInstance(t)!;
            dynamic lnk = shell.CreateShortcut(path);
            lnk.TargetPath = exe;
            lnk.WorkingDirectory = IOPath.GetDirectoryName(exe);
            lnk.IconLocation = exe + ",0";
            lnk.Description = L.T("app.name");
            lnk.Save();
            return path;
        }
        catch (Exception e)
        {
            Log.Error("建开始菜单快捷方式失败：" + e.Message);
            return null;
        }
    }
}
