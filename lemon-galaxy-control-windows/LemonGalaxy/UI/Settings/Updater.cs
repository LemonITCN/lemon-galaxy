using System.IO;
using System.Windows;

namespace LemonGalaxy.UI;

internal enum UpdateState { Idle, Checking, UpToDate, Available, Downloading, Downloaded, Installing, Failed }

/// <summary>
/// 软件更新（设置 → 关于，加后台检查）。
/// 后台：登录后 20 秒查一次，之后每 6 小时查一次（发布版才查，开发版与模拟内核不查）；有新版本就在后台下载好
/// （内核已验发布签名、核对 SHA-256），侧栏「设置」与「关于」上出现标记，并提示一次（带「立即更新」）。不点不装。
/// 点「立即更新」后一路自动：没下载好先下载 → 退出 → 替换 → 重新打开并回到原来的界面，中间不弹任何窗口（不提权、不确认）。
/// 程序目录当前用户不可写时改装到 %LOCALAPPDATA%\Programs\LemonGalaxy。安装细节见 App/UpdateInstall.cs。
/// </summary>
internal sealed class Updater : Observable
{
    private const int FirstCheckDelayMs = 20_000;
    private static readonly TimeSpan CheckInterval = TimeSpan.FromHours(6);

    private readonly AppModel app;
    private readonly UpdateEnv env;
    public UpdateState State = UpdateState.Idle;
    public UpdateInfo? Info;
    public string? Error;
    public ulong Done, Total;
    /// <summary>下载进度（只刷新进度条，不惊动侧栏）</summary>
    public readonly Observable Progress = new();
    public string? FilePath;
    /// <summary>用户点了「立即更新」：下载完接着装</summary>
    public bool InstallRequested;
    /// <summary>上一次没装上（辅助程序写的结果）；同一个包后台不再提示</summary>
    public UpdateResult? LastFailure;
    private DateTime lastCheck = DateTime.MinValue;
    private int generation;
    private bool retriedCorrupt;
    /// <summary>失败在下载这一步（安装失败不自动重下）</summary>
    private bool downloadFailed;

    public Updater(AppModel app)
    {
        this.app = app;
        if (app.SelfTest)
        {
            env = new UpdateEnv();
            return;
        }
        env = UpdateEnv.Default;
        // 模拟内核下载的是假文件：放到临时目录，不和真的更新混在一起
        if (app.Client.IsMock) env.UpdatesDir = IOPath.Combine(IOPath.GetTempPath(), "LemonGalaxy-mock-updates");
        LastFailure = UpdateJson.Load<UpdateResult>(env.ResultFile) is { Ok: false } r ? r : null;
        app.PhaseChanged.Watch(this, () => { if (app.LoggedIn) Ui.After(FirstCheckDelayMs, Tick); });
        Ui.Every(30 * 60_000, Tick);
    }

    public string LogFile => env.LogFile;

    /// <summary>有新版本（侧栏「设置」与「关于」上的标记）</summary>
    public bool HasUpdate => Info != null && State is UpdateState.Available or UpdateState.Downloading or UpdateState.Downloaded or UpdateState.Installing or UpdateState.Failed;

    public bool Busy => State is UpdateState.Checking or UpdateState.Downloading or UpdateState.Installing;

    /// <summary>后台自动检查：发布版、真内核才查</summary>
    private bool AutoCheck => !app.SelfTest && !app.Client.IsMock && AppInfo.Build != "dev";

    private void Tick()
    {
        if (!AutoCheck || !app.LoggedIn || Busy || InstallRequested) return;
        // 后台下载失败过（网络断了等）：半小时后再下
        if (State == UpdateState.Failed && downloadFailed && Info != null && DateTime.UtcNow - lastCheck < CheckInterval)
        {
            StartDownload();
            return;
        }
        if (DateTime.UtcNow - lastCheck < CheckInterval) return;
        Check(true);
    }

    /// <summary>「检查更新」按钮</summary>
    public void Check() => Check(false);

    private void Check(bool background)
    {
        if (Busy) return;
        var before = State;
        State = UpdateState.Checking;
        Error = null;
        Notify();
        Ui.Run(async () =>
        {
            UpdateInfo? info;
            try
            {
                // Windows 10 以前的系统用兼容版（Win7 目标编的内核库）
                var platform = Environment.OSVersion.Version.Major < 10 ? "windows7" : "windows";
                info = await app.Client.CheckUpdate("control", platform, "x86_64", AppInfo.Version);
            }
            catch (Exception e)
            {
                if (background)
                {
                    // 后台查不到不打扰，半小时后再试
                    Log.Info("后台检查更新失败：" + Fmt.Error(e));
                    State = before == UpdateState.Checking ? UpdateState.Idle : before;
                }
                else
                {
                    Error = Fmt.Error(e);
                    State = UpdateState.Failed;
                }
                Notify();
                return;
            }
            lastCheck = DateTime.UtcNow;
            Info = info;
            if (info == null)
            {
                State = UpdateState.UpToDate;
                ClearDownloads(null);
                Notify();
                return;
            }
            if (ExistingDownload(info) is { } file)
            {
                FilePath = file;
                State = UpdateState.Downloaded;
                Notify();
                Announce();
                return;
            }
            State = UpdateState.Available;
            Notify();
            // 预先在后台下载好，点「立即更新」就能马上装（模拟内核只在点了之后才「下载」）
            if (!app.Client.IsMock) StartDownload();
        });
    }

    /// <summary>「立即更新」：没下载好就先下载，下载好立即安装并重新打开</summary>
    public void InstallNow()
    {
        if (Info == null || State is UpdateState.Checking or UpdateState.Installing) return;
        if (BlockedByUnsaved()) return;
        InstallRequested = true;
        if (State == UpdateState.Downloaded && FilePath != null && File.Exists(FilePath))
        {
            Install();
            return;
        }
        if (State == UpdateState.Downloading)
        {
            Notify();
            return;
        }
        StartDownload();
    }

    private void StartDownload()
    {
        if (Info is not { } info) return;
        try { Directory.CreateDirectory(env.UpdatesDir); } catch { }
        var name = string.Concat(info.FileName.Select(c => IOPath.GetInvalidFileNameChars().Contains(c) ? '_' : c));
        FilePath = IOPath.Combine(env.UpdatesDir, name);
        UpdateFiles.TryDelete(FilePath);
        State = UpdateState.Downloading;
        Done = 0;
        Total = info.Size;
        Error = null;
        Notify();
        var gen = ++generation;
        try { app.Client.DownloadUpdate(info, FilePath, new Listener(this, gen, info)); }
        catch (Exception e) { DownloadFinished(gen, info, Fmt.Error(e)); }
    }

    private sealed class Listener : TransferListener
    {
        private readonly Updater u;
        private readonly int gen;
        private readonly UpdateInfo info;
        private ulong d, t;
        public Listener(Updater u, int gen, UpdateInfo info) { this.u = u; this.gen = gen; this.info = info; }

        public void OnProgress(ulong done, ulong total)
        {
            lock (this) { d = done; t = total; }
            Ui.Coalesce(this, () =>
            {
                if (gen != u.generation || u.State != UpdateState.Downloading) return;
                lock (this) { u.Done = d; u.Total = t; }
                u.Progress.Notify();
            });
        }

        public void OnFinished(string? error) => Ui.Post(() => u.DownloadFinished(gen, info, error));
    }

    private void DownloadFinished(int gen, UpdateInfo info, string? error)
    {
        if (gen != generation) return;
        downloadFailed = error != null;
        if (error != null)
        {
            Error = error;
            State = UpdateState.Failed;
            if (InstallRequested) app.ShowToast(L.T("update.downloadFailed", error), true);
            InstallRequested = false;
            Notify();
            return;
        }
        var file = FilePath!;
        Done = Total;
        Ui.Run(async () =>
        {
            // 下载完记下这个包的校验值：下次启动认得出已经下载好的，安装前再核对一次
            try
            {
                var sha = await Task.Run(() => UpdateFiles.Sha256(file));
                UpdateJson.Save(env.MarkerFile, new DownloadMarker { File = IOPath.GetFileName(file), Sha256 = sha, InfoSha256 = info.Sha256, Version = info.Version });
                ClearDownloads(file);
            }
            catch (Exception e) { Log.Info("记录下载的更新包失败：" + e.Message); }
            if (gen != generation) return;
            State = UpdateState.Downloaded;
            Notify();
            if (InstallRequested) Install();
            else Announce();
        });
    }

    /// <summary>后台下载好了：提示一次（同一个版本只提示一次；上次装失败的包不提示）</summary>
    private void Announce()
    {
        if (InstallRequested || app.SelfTest || Info is not { } info) return;
        if (LastFailure != null && LastFailure.Sha256 == info.Sha256) return;
        if (app.Settings.UpdateAnnounced == info.Version) return;
        app.Settings.UpdateAnnounced = info.Version;
        app.ShowToast(L.T("update.readyToast", info.Version), false, L.T("update.now"), InstallNow);
    }

    private string? ExistingDownload(UpdateInfo info)
    {
        var m = UpdateJson.Load<DownloadMarker>(env.MarkerFile);
        if (m == null || m.InfoSha256 != info.Sha256 || m.File.Length == 0) return null;
        var file = IOPath.Combine(env.UpdatesDir, m.File);
        return File.Exists(file) ? file : null;
    }

    /// <summary>删掉别的版本的下载（keep 之外的 .zip / .part）</summary>
    private void ClearDownloads(string? keep)
    {
        try
        {
            if (!Directory.Exists(env.UpdatesDir)) return;
            foreach (var f in Directory.GetFiles(env.UpdatesDir))
            {
                var ext = IOPath.GetExtension(f).ToLowerInvariant();
                if (ext is ".zip" or ".part" or ".exe" or ".msi" && !UpdateFiles.Same(f, keep)) UpdateFiles.TryDelete(f);
            }
            if (keep == null) UpdateFiles.TryDelete(env.MarkerFile);
        }
        catch { }
    }

    /// <summary>解压、核对、选位置、交给辅助程序，然后退出（辅助程序换好文件后重新打开）</summary>
    private void Install()
    {
        InstallRequested = false;
        if (BlockedByUnsaved())
        {
            Notify();
            return;
        }
        var file = FilePath;
        if (file == null || !File.Exists(file))
        {
            InstallRequested = true;
            StartDownload();
            return;
        }
        if (app.Client.IsMock)
        {
            app.ShowToast(L.T("update.mockInstall"));
            Notify();
            return;
        }
        if (!file.EndsWith(".zip", StringComparison.OrdinalIgnoreCase))
        {
            Fail("package", L.T("update.unknownPackage"));
            return;
        }
        State = UpdateState.Installing;
        Error = null;
        Notify();
        var marker = UpdateJson.Load<DownloadMarker>(env.MarkerFile);
        var redownload = !retriedCorrupt;
        var zipSha = marker != null && string.Equals(marker.File, IOPath.GetFileName(file), StringComparison.OrdinalIgnoreCase) ? marker.Sha256 : "";
        var infoSha = Info?.Sha256 ?? "";
        var exe = UpdateEnv.CurrentExe;
        var log = new UpdateLog(env.LogFile);
        Ui.Run(async () =>
        {
            UpdatePreparer.Prepared p;
            try { p = await Task.Run(() => UpdatePreparer.Prepare(env, file, zipSha, infoSha, exe, AppInfo.Version, log)); }
            catch (Exception e)
            {
                log.Write("准备更新失败：" + e);
                var (code, _) = UpdateFailure.Classify(e);
                if (code == "corrupt" && redownload)
                {
                    // 下载好的包在磁盘上坏了：删掉重新下载（只重下一次），下载完接着装
                    retriedCorrupt = true;
                    UpdateFiles.TryDelete(file);
                    UpdateFiles.TryDelete(env.MarkerFile);
                    InstallRequested = true;
                    StartDownload();
                    return;
                }
                Fail(code, e is UpdateFailure ? e.Message : Fmt.Error(e));
                return;
            }
            try
            {
                UpdateResume.Capture(app).Save(env.ResumeFile);
                using (NativeProcess.Start(p.HelperExe, UpdateStartup.JoinArgs(new[] { "--apply-update", p.PlanPath }), env.UpdatesDir)) { }
            }
            catch (Exception e)
            {
                log.Write("启动辅助程序失败：" + e);
                Fail("other", Fmt.Error(e));
                return;
            }
            log.Write("已交给辅助程序，退出");
            NativeProcess.AllowForeground();
            Application.Current?.Shutdown();
        });
    }

    /// <summary>文本编辑窗口里有没保存的改动：不能替它决定存不存，提示保存后再点（退出时会弹「是否保存」）</summary>
    private bool BlockedByUnsaved()
    {
        var names = TextEditorWindow.Unsaved;
        if (names.Length == 0) return false;
        app.ShowToast(L.T("update.unsaved", string.Join(L.T("update.unsavedSeparator"), names)), true);
        return true;
    }

    private void Fail(string code, string detail)
    {
        Error = L.T("update.installFailed", L.T("update.fail." + code)) + (detail.Length > 0 ? "\n" + detail : "");
        State = UpdateState.Failed;
        Notify();
        app.ShowToast(L.T("update.installFailed", L.T("update.fail." + code)), true, L.T("update.viewLog"), () => app.RevealInExplorer(env.LogFile));
    }

    /// <summary>
    /// 本进程是辅助程序打开的（--update-token）：先回到原来的分区，界面画出来后报到（辅助程序据此确认更新成功），
    /// 提示结果，登录上线后重新打开原来的标签页（设备终端接回原来的会话）。
    /// </summary>
    public void AfterRelaunch(Window window, string token, bool failed)
    {
        var resume = UpdateResume.Take(env.ResumeFile);
        resume?.ApplyNav(app);
        var done = false;
        window.ContentRendered += (_, _) =>
        {
            if (done) return;
            done = true;
            UpdateStartup.Signal(env, token);
            try
            {
                window.Activate();
                window.Topmost = true;
                window.Topmost = false;
            }
            catch { }
            if (failed)
            {
                var code = LastFailure?.Code is { Length: > 0 } c ? c : "other";
                app.ShowToast(L.T("update.restored", L.T("update.fail." + code)), true, L.T("update.viewLog"), () => app.RevealInExplorer(env.LogFile));
            }
            else app.ShowToast(L.T("update.done", AppInfo.Version));
            if (resume is { HasSessions: true }) Ui.Run(() => resume.RestoreSessions(app));
        };
    }

    /// <summary>selftest：直接摆出某个状态</summary>
    public void Fake(UpdateState state, UpdateInfo? info, ulong done = 0, string? error = null, bool requested = false)
    {
        State = state;
        Info = info;
        Done = done;
        Total = info?.Size ?? 0;
        Error = error;
        InstallRequested = requested;
        Notify();
    }
}

/// <summary>下载好的更新包（updates\download.json）</summary>
internal sealed class DownloadMarker
{
    public string File = "";
    /// <summary>下载完时算的</summary>
    public string Sha256 = "";
    /// <summary>服务器给的（认出是不是同一个包）</summary>
    public string InfoSha256 = "";
    public string Version = "";
}

/// <summary>
/// 更新前的界面状态（updates\resume.json），重新打开后恢复：所在分区、设置页、打开的标签页（设备终端记下会话，接回去）、
/// 开着的端口映射。窗口位置与大小本来就存在设置里。临时协助的设备、要问密码的 SSH 主机不恢复（不能弹东西让用户点）。
/// </summary>
internal sealed class UpdateResume
{
    public string Section = "";
    public string SettingsPage = "";
    /// <summary>当时在前台的标签（Tabs 里的序号），-1 表示在首页</summary>
    public int ActiveTab = -1;
    public List<ResumeTab> Tabs = new();
    /// <summary>开着的端口映射（规则 ID）</summary>
    public List<string> Forwards = new();
    public long Time;

    public static UpdateResume Capture(AppModel app)
    {
        var r = new UpdateResume { Section = app.Section.ToString(), SettingsPage = app.SettingsPage.ToString(), Time = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() };
        foreach (var t in app.Tabs)
        {
            if (t.DeviceId is { } d && d.StartsWith("assist:", StringComparison.Ordinal)) continue;
            if (t.DeviceId == null && t.SshHostId == null && t.Kind != TabKind.Sftp) continue;
            if (t.Id == app.ActiveTabId) r.ActiveTab = r.Tabs.Count;
            r.Tabs.Add(new ResumeTab { Kind = t.Kind.ToString(), Device = t.DeviceId, Ssh = t.SshHostId, Session = (t as TerminalTab)?.SessionId });
        }
        r.Forwards = app.PortForwards.Where(e => app.Forwards.StateOf(e.Id) is ForwardState.Running or ForwardState.Starting).Select(e => e.Id).ToList();
        return r;
    }

    public void Save(string path) => UpdateJson.Save(path, this);

    /// <summary>读出并删掉；半小时前的不算</summary>
    public static UpdateResume? Take(string path)
    {
        var r = UpdateJson.Load<UpdateResume>(path);
        UpdateFiles.TryDelete(path);
        if (r == null || DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() - r.Time > 30 * 60_000) return null;
        return r;
    }

    public void ApplyNav(AppModel app)
    {
        if (Enum.TryParse<NavSection>(Section, out var s)) app.Section = s;
        if (Enum.TryParse<SettingsPage>(SettingsPage, out var p)) app.SettingsPage = p;
        app.Nav.Notify();
    }

    public bool HasSessions => Tabs.Count > 0 || Forwards.Count > 0;

    /// <summary>登录并上线后（最多等 1 分钟）重新打开标签页、开回端口映射</summary>
    public async Task RestoreSessions(AppModel app)
    {
        for (var i = 0; i < 120 && !(app.LoggedIn && app.ClientState == ClientState.Online); i++) await Task.Delay(500);
        if (!app.LoggedIn || app.ClientState != ClientState.Online)
        {
            Log.Info("更新后一分钟内没有上线，不恢复标签页与端口映射");
            return;
        }
        foreach (var id in Forwards)
        {
            if (app.Entry(id)?.Data is not VaultData.PortForward f || app.Forwards.StateOf(id) != ForwardState.Stopped) continue;
            if (f.ViaSshHostId is { } via && !SshNeedsNoPrompt(app, via)) continue;
            app.Forwards.Start(id);
        }
        var opened = new List<TabModel?>();
        foreach (var t in Tabs)
        {
            var before = app.Tabs.Count;
            try { await Reopen(app, t); }
            catch (Exception e) { Log.Info("更新后恢复标签页失败：" + e.Message); }
            opened.Add(app.Tabs.Count > before ? app.Tabs[app.Tabs.Count - 1] : null);
        }
        if (ActiveTab >= 0 && ActiveTab < opened.Count && opened[ActiveTab] is { } active) app.ActivateTab(active.Id);
        else app.SelectHome();
    }

    private static async Task Reopen(AppModel app, ResumeTab t)
    {
        if (t.Device is { } dev && app.Device(dev) == null) return;
        if (t.Ssh is { } ssh && !SshNeedsNoPrompt(app, ssh)) return;
        switch (t.Kind)
        {
            case nameof(TabKind.Terminal) when t.Device is { } id:
                TerminalSessionInfo? attach = null;
                if (t.Session is { Length: > 0 } sid)
                {
                    try
                    {
                        var p = await app.Peer(id);
                        attach = (await p.Session.ListTerminals()).FirstOrDefault(s => s.Id == sid && !s.Attached);
                    }
                    catch { }
                }
                app.OpenTerminal(id, attach);
                break;
            case nameof(TabKind.Terminal) when t.Ssh is { } id:
                app.OpenSshTerminal(id);
                break;
            case nameof(TabKind.Sftp):
                app.OpenSftp(t.Device, t.Ssh);
                break;
            case nameof(TabKind.Remote) when t.Device is { } id:
                app.OpenRemoteDesktop(id);
                break;
        }
    }

    /// <summary>SSH 主机存了密码、私钥或用代理认证：重连不用问</summary>
    private static bool SshNeedsNoPrompt(AppModel app, string hostId)
        => app.Entry(hostId)?.Data is VaultData.SshHost h && h.Auth switch
        {
            SshAuth.Password p => !string.IsNullOrEmpty(p.PasswordValue),
            SshAuth.PrivateKey or SshAuth.Agent => true,
            _ => false,
        };
}

internal sealed class ResumeTab
{
    public string Kind = "";
    public string? Device;
    public string? Ssh;
    /// <summary>设备终端的会话（被控端保留着，接回去）</summary>
    public string? Session;
}
