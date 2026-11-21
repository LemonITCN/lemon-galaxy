using System.IO;
using System.Reflection;
using Microsoft.Win32;

namespace LemonGalaxy;

/// <summary>应用版本、系统信息、数据目录。</summary>
internal static class AppInfo
{
    /// <summary>发布版本（CI 构建为「版本.构建序号」，与 Release 名一致），取程序集信息版本「+」前面的部分</summary>
    public static string Version
    {
        get
        {
            var info = Assembly.GetExecutingAssembly().GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion ?? "";
            var i = info.IndexOf('+');
            var v = i >= 0 ? info.Substring(0, i) : info;
            if (v.Length > 0) return v;
            var a = Assembly.GetExecutingAssembly().GetName().Version;
            return a == null ? "0.1.0" : $"{a.Major}.{a.Minor}.{a.Build}";
        }
    }

    /// <summary>构建号：程序集信息版本里「+」后面的部分（CI 用 -p:InformationalVersion=0.1.0+123 传入），没有时为 dev。</summary>
    public static string Build
    {
        get
        {
            var info = Assembly.GetExecutingAssembly().GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion ?? "";
            var i = info.IndexOf('+');
            if (i < 0 || i + 1 >= info.Length) return "dev";
            var b = info.Substring(i + 1);
            return b.Length > 12 ? b.Substring(0, 12) : b;
        }
    }

    /// <summary>「Windows 11 23H2（10.0.22631）」这类描述。读注册表，读不到时退回 Environment.OSVersion。</summary>
    public static string OsVersion
    {
        get
        {
            try
            {
                using var k = Registry.LocalMachine.OpenSubKey(@"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
                if (k != null)
                {
                    var name = k.GetValue("ProductName") as string ?? "Windows";
                    var build = k.GetValue("CurrentBuildNumber") as string ?? k.GetValue("CurrentBuild") as string ?? "";
                    var display = k.GetValue("DisplayVersion") as string ?? k.GetValue("ReleaseId") as string ?? "";
                    // Windows 11 的 ProductName 仍写着 Windows 10，按构建号纠正
                    if (int.TryParse(build, out var b) && b >= 22000) name = name.Replace("Windows 10", "Windows 11");
                    return (name + (display.Length > 0 ? " " + display : "")).Trim();
                }
            }
            catch { }
            return Environment.OSVersion.VersionString;
        }
    }

    public static string Hostname => Environment.MachineName;

    /// <summary>界面设置等本机偏好：%APPDATA%\LemonGalaxy</summary>
    public static string ConfigDir => IOPath.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "LemonGalaxy");

    /// <summary>内核数据（设备身份、加密库缓存、会话）：%LOCALAPPDATA%\LemonGalaxy\control</summary>
    public static string DataDir => IOPath.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "LemonGalaxy", "control");

    public static string CacheDir => IOPath.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "LemonGalaxy", "cache");

    /// <summary>exe 所在目录</summary>
    public static string BaseDir => AppDomain.CurrentDomain.BaseDirectory;

    /// <summary>资源目录：exe 同目录的 Resources\；环境变量 LEMONGALAXY_RESOURCES 可覆盖。</summary>
    public static string ResourcesDir
    {
        get
        {
            var env = Environment.GetEnvironmentVariable("LEMONGALAXY_RESOURCES");
            if (!string.IsNullOrEmpty(env)) return env!;
            return IOPath.Combine(BaseDir, "Resources");
        }
    }

    /// <summary>同目录附带的 Windows 被控端</summary>
    public static string WorkerExe => IOPath.Combine(BaseDir, "lemon-galaxy-worker.exe");

    public static string[] ResourceFiles(string dir, string ext)
    {
        var d = IOPath.Combine(ResourcesDir, dir);
        try
        {
            if (!Directory.Exists(d)) return Array.Empty<string>();
            return Directory.GetFiles(d, "*." + ext).OrderBy(f => f, StringComparer.Ordinal).ToArray();
        }
        catch { return Array.Empty<string>(); }
    }
}

/// <summary>统一的「现在」（毫秒）。selftest 冻结时间，截图里的时间戳才稳定。</summary>
internal static class Clock
{
    public static long? FrozenMs;
    public static long NowMs => FrozenMs ?? DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
}
