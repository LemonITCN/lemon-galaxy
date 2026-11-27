using System.Diagnostics;
using System.IO;
using System.Windows.Threading;

namespace LemonGalaxy;

/// <summary>
/// 被控服务自动更新换了整个桌面端（docs/auto-update.md）后，界面自己换成新版本：装在 Program Files 的那份界面
/// 发现自己的程序文件变成了别的版本，就以同样的方式（窗口开着 / 只在托盘）打开新版本接替自己（--replace），
/// 旧进程收到「退出」信号后退出。不弹任何提示。服务换回旧版本时同样会换回来。
/// </summary>
internal static class SelfUpdate
{
    private static string? pending;
    private static int seen;
    private static int failures;
    private static bool launched;

    public static void Start(Func<bool> windowVisible)
    {
        // 只有装好的那份会被服务换掉（从解压目录直接运行的不管）
        if (!AppInfo.RunningInstalled) return;
        var t = new DispatcherTimer { Interval = TimeSpan.FromSeconds(10) };
        t.Tick += (_, _) =>
        {
            if (launched) return;
            var v = DiskVersion();
            if (v == null || v == AppInfo.Version)
            {
                pending = null;
                seen = 0;
                return;
            }
            if (v != pending)
            {
                pending = v;
                seen = 1;
                return;
            }
            // 连着两次（10 秒以上）都是这个版本：文件换完了，再换
            if (++seen < 2) return;
            Relaunch(windowVisible(), v, t);
        };
        t.Start();
    }

    /// <summary>磁盘上现在那份界面程序的版本（换文件的一瞬间读不到时为 null）</summary>
    private static string? DiskVersion()
    {
        try
        {
            // 按安装位置找（运行中的程序被改名挪开后，那个位置上就是新版本）
            if (!File.Exists(AppInfo.InstalledExe)) return null;
            var v = FileVersionInfo.GetVersionInfo(AppInfo.InstalledExe).ProductVersion ?? "";
            var plus = v.IndexOf('+');
            v = (plus > 0 ? v.Substring(0, plus) : v).Trim();
            return v.Length > 0 ? v : null;
        }
        catch { return null; }
    }

    private static void Relaunch(bool visible, string version, DispatcherTimer timer)
    {
        try
        {
            Log.Info($"界面程序已更新到 {version}，换成新版本（{(visible ? "窗口" : "托盘")}）");
            // 新版本发「退出」信号，本进程在 Program.Exit 里退出；没开窗口的只放托盘（--background 不看「登录后在托盘显示」）
            Process.Start(new ProcessStartInfo(AppInfo.InstalledExe, visible ? "--replace" : "--replace --background") { UseShellExecute = false, WorkingDirectory = AppInfo.InstallDir });
            launched = true;
            // 新版本没接替（起不来）：一分钟后照常继续看着
            var back = new DispatcherTimer { Interval = TimeSpan.FromSeconds(60) };
            back.Tick += (_, _) =>
            {
                back.Stop();
                launched = false;
                seen = 0;
                pending = null;
            };
            back.Start();
        }
        catch (Exception e)
        {
            Log.Error("打开新版本的界面失败：" + e.Message);
            seen = 0;
            pending = null;
            if (++failures >= 3) timer.Stop();
        }
    }
}
