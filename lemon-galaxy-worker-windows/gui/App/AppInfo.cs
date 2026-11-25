using System.Diagnostics;
using System.IO;
using System.Reflection;

namespace LemonGalaxy;

/// <summary>路径与版本。程序目录里有 lemon-galaxy-worker.exe（被控服务）与 Resources\。</summary>
internal static class AppInfo
{
    public const string ServiceName = "LemonGalaxyWorker";
    public const string RepoUrl = "https://github.com/LemonITCN/lemon-galaxy";
    public const string LicenseUrl = "https://github.com/LemonITCN/lemon-galaxy/blob/main/LICENSE";

    public static string ExePath => Assembly.GetExecutingAssembly().Location;
    public static string BaseDir => AppDomain.CurrentDomain.BaseDirectory;
    public static string ResourcesDir => IOPath.Combine(BaseDir, "Resources");
    public static string WorkerExe => IOPath.Combine(BaseDir, "lemon-galaxy-worker.exe");

    /// <summary>安装位置：与 lemon-galaxy-worker install-service 复制到的目录相同</summary>
    public static string InstallDir => IOPath.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), "LemonGalaxy", "Worker");
    public static string InstalledExe => IOPath.Combine(InstallDir, "LemonGalaxyWorker.exe");

    /// <summary>被控服务的数据目录（账号、设置、日志），卸载时保留</summary>
    public static string ServiceDataDir => IOPath.Combine(Environment.GetFolderPath(Environment.SpecialFolder.CommonApplicationData), "LemonGalaxy", "worker");

    /// <summary>界面自己的设置与日志（每个 Windows 用户一份）</summary>
    public static string DataDir => IOPath.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "LemonGalaxy", "worker-ui");

    public static bool RunningInstalled => SamePath(BaseDir, InstallDir);

    public static bool SamePath(string a, string b)
    {
        try { return string.Equals(IOPath.GetFullPath(a).TrimEnd('\\'), IOPath.GetFullPath(b).TrimEnd('\\'), StringComparison.OrdinalIgnoreCase); }
        catch { return false; }
    }

    /// <summary>这一份界面按哪个 .NET Framework 编（Windows 10 起的包 4.8，老系统兼容版 4.0）</summary>
#if NET40
    public const string Framework = ".NET Framework 4.0";
#else
    public const string Framework = ".NET Framework 4.8";
#endif

    public static string Version
    {
        get
        {
            var a = Attribute.GetCustomAttribute(Assembly.GetExecutingAssembly(), typeof(AssemblyInformationalVersionAttribute)) as AssemblyInformationalVersionAttribute;
            var v = a?.InformationalVersion ?? "0.1.0";
            var plus = v.IndexOf('+');
            return plus > 0 ? v.Substring(0, plus) : v;
        }
    }

    /// <summary>已安装那份界面的版本（没装为 null）</summary>
    public static string? InstalledVersion
    {
        get
        {
            try
            {
                if (!File.Exists(InstalledExe)) return null;
                var v = FileVersionInfo.GetVersionInfo(InstalledExe).ProductVersion ?? "";
                var plus = v.IndexOf('+');
                return plus > 0 ? v.Substring(0, plus) : v;
            }
            catch { return null; }
        }
    }

    public static IEnumerable<string> ResourceFiles(string sub, string ext)
    {
        var dir = IOPath.Combine(ResourcesDir, sub);
        try { return Directory.Exists(dir) ? Directory.GetFiles(dir, "*." + ext).OrderBy(f => f, StringComparer.Ordinal).ToArray() : new string[0]; }
        catch { return new string[0]; }
    }

    public static string OsName
    {
        get
        {
            try
            {
                using var k = Microsoft.Win32.Registry.LocalMachine.OpenSubKey(@"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
                var name = k?.GetValue("ProductName") as string ?? "Windows";
                var build = int.TryParse(k?.GetValue("CurrentBuild") as string, out var b) ? b : 0;
                if (build >= 22000) name = name.Replace("Windows 10", "Windows 11");
                var disp = k?.GetValue("DisplayVersion") as string ?? k?.GetValue("ReleaseId") as string ?? "";
                return disp.Length > 0 ? $"{name} {disp} ({build})" : $"{name} ({build})";
            }
            catch { return "Windows"; }
        }
    }
}

/// <summary>毫秒时间戳（DateTimeOffset 的 Unix 时间换算是 4.6 起才有的，老系统版按 4.0 编，自己算）</summary>
internal static class Clock
{
    private static readonly DateTime Epoch = new(1970, 1, 1, 0, 0, 0, DateTimeKind.Utc);
    public static long NowMs => (long)(DateTime.UtcNow - Epoch).TotalMilliseconds;
    public static DateTime Local(long ms) => Epoch.AddMilliseconds(ms).ToLocalTime();
}
