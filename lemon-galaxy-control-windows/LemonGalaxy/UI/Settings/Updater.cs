using System.Diagnostics;
using System.IO;
using System.IO.Compression;
using System.Text;
using System.Windows;

namespace LemonGalaxy.UI;

internal enum UpdateState { Idle, Checking, UpToDate, Available, Downloading, Downloaded, Failed }

/// <summary>
/// 自动更新（设置 → 关于）：向自己的服务器查询（内核已验发布签名）→ 下载并核对 SHA-256 → 安装。
/// 安装：.zip 解压到暂存目录，退出本程序后由一段批处理把文件覆盖到程序目录再重新启动（目录不可写时以管理员身份执行）；
/// .exe / .msi 直接运行安装程序。被控服务正在运行时不覆盖 lemon-galaxy-worker.exe（它按自己的更新流程走）。
/// </summary>
internal sealed class Updater : Observable
{
    private readonly AppModel app;
    public UpdateState State = UpdateState.Idle;
    public UpdateInfo? Info;
    public string? Error;
    public ulong Done, Total;
    public string? FilePath;

    public Updater(AppModel app) { this.app = app; }

    private static string Dir => IOPath.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "LemonGalaxy", "updates");

    public void Check()
    {
        if (State is UpdateState.Checking or UpdateState.Downloading) return;
        State = UpdateState.Checking;
        Error = null;
        Notify();
        Ui.Run(async () =>
        {
            try
            {
                Info = await app.Client.CheckUpdate("control", "windows", "x86_64", AppInfo.Version);
                State = Info == null ? UpdateState.UpToDate : UpdateState.Available;
            }
            catch (Exception e)
            {
                Error = Fmt.Error(e);
                State = UpdateState.Failed;
            }
            Notify();
        });
    }

    public void Download()
    {
        if (Info is not { } info) return;
        try { Directory.CreateDirectory(Dir); } catch { }
        var name = string.Concat(info.FileName.Select(c => IOPath.GetInvalidFileNameChars().Contains(c) ? '_' : c));
        FilePath = IOPath.Combine(Dir, name);
        State = UpdateState.Downloading;
        Done = 0;
        Total = info.Size;
        Notify();
        try { app.Client.DownloadUpdate(info, FilePath, new Listener(this)); }
        catch (Exception e)
        {
            Error = Fmt.Error(e);
            State = UpdateState.Failed;
            Notify();
        }
    }

    private sealed class Listener : TransferListener
    {
        private readonly Updater u;
        private ulong d, t;
        public Listener(Updater u) { this.u = u; }

        public void OnProgress(ulong done, ulong total)
        {
            lock (this) { d = done; t = total; }
            Ui.Coalesce(this, () =>
            {
                lock (this) { u.Done = d; u.Total = t; }
                if (u.State == UpdateState.Downloading) u.Notify();
            });
        }

        public void OnFinished(string? error) => Ui.Post(() =>
        {
            if (error != null) { u.Error = error; u.State = UpdateState.Failed; }
            else { u.State = UpdateState.Downloaded; u.Done = u.Total; }
            u.Notify();
        });
    }

    public void Install()
    {
        if (FilePath == null || !File.Exists(FilePath)) return;
        if (app.Client.IsMock) { app.ShowToast(L.T("update.mockInstall")); return; }
        try
        {
            var ext = IOPath.GetExtension(FilePath).ToLowerInvariant();
            if (ext is ".exe" or ".msi")
            {
                Process.Start(new ProcessStartInfo(FilePath) { UseShellExecute = true });
                Application.Current?.Shutdown();
                return;
            }
            if (ext != ".zip") { app.ShowToast(L.T("update.unknownPackage"), true); return; }
            var staging = IOPath.Combine(Dir, "staging-" + Info?.Version);
            if (Directory.Exists(staging)) Directory.Delete(staging, true);
            ZipFile.ExtractToDirectory(FilePath, staging);
            // 压缩包里只有一个文件夹时以它为准
            var src = staging;
            if (!File.Exists(IOPath.Combine(src, "LemonGalaxy.exe")))
            {
                var sub = Directory.GetDirectories(staging).FirstOrDefault(d => File.Exists(IOPath.Combine(d, "LemonGalaxy.exe")));
                if (sub == null) { app.ShowToast(L.T("update.unknownPackage"), true); return; }
                src = sub;
            }
            var target = AppInfo.BaseDir.TrimEnd('\\');
            var workerRunning = app.Worker.State is WorkerState.Running or WorkerState.Starting;
            var script = IOPath.Combine(Dir, "install.cmd");
            var pid = Process.GetCurrentProcess().Id;
            var sb = new StringBuilder();
            sb.AppendLine("@echo off");
            sb.AppendLine("chcp 65001 >nul");
            sb.AppendLine(":wait");
            sb.AppendLine($"tasklist /FI \"PID eq {pid}\" 2>nul | find \"{pid}\" >nul && (timeout /t 1 /nobreak >nul & goto wait)");
            sb.AppendLine($"robocopy \"{src}\" \"{target}\" /E /R:5 /W:1 /NFL /NDL /NJH /NJS{(workerRunning ? " /XF lemon-galaxy-worker.exe" : "")}");
            sb.AppendLine($"start \"\" \"{IOPath.Combine(target, "LemonGalaxy.exe")}\"");
            File.WriteAllText(script, sb.ToString(), new UTF8Encoding(false));
            var psi = new ProcessStartInfo("cmd.exe", "/c \"" + script + "\"") { UseShellExecute = true, WindowStyle = ProcessWindowStyle.Hidden };
            if (!Writable(target)) psi.Verb = "runas";
            Process.Start(psi);
            Application.Current?.Shutdown();
        }
        catch (Exception e) { app.ShowError(e); }
    }

    private static bool Writable(string dir)
    {
        try
        {
            var probe = IOPath.Combine(dir, ".lg-write-test");
            File.WriteAllText(probe, "");
            File.Delete(probe);
            return true;
        }
        catch { return false; }
    }
}
