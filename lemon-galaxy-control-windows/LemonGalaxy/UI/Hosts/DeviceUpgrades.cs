namespace LemonGalaxy.UI;

internal enum UpgradePhase { Requesting, Started, Offline, Done, Failed }

/// <summary>一台被控端的远程升级进度（界面线程读写）</summary>
internal sealed class UpgradeProgress
{
    public UpgradePhase Phase = UpgradePhase.Requesting;
    /// <summary>点击时对方的版本</summary>
    public string FromVersion = "";
    /// <summary>要升级到的版本：点击时取设备列表里的新版本，对方开始升级后以它报的为准</summary>
    public string Target = "";
    /// <summary>结束后对方现在的版本</summary>
    public string? Version;
    public string? Error;
    /// <summary>对方查了一遍已是最新（没开始升级就结束）</summary>
    public bool AlreadyLatest;

    public bool Running => Phase is UpgradePhase.Requesting or UpgradePhase.Started or UpgradePhase.Offline;
}

/// <summary>
/// 设备详情里的「升级被控端」（docs/auto-update.md）：点了直接调内核，不再确认；对方自己下载、替换、重启，内核等它以新版本重新上线。
/// 进度按设备记在这里，离开详情再回来还在；结束后刷新设备列表，不在看这台设备的详情时再弹一条提示。
/// 状态变化通知 app.Data（设备列表、概览、详情都订阅了它）。
/// </summary>
internal sealed class UpgradeHub
{
    private readonly AppModel app;
    private readonly Dictionary<string, UpgradeProgress> items = new();

    public UpgradeHub(AppModel app) { this.app = app; }

    /// <summary>
    /// 能远程升级的设备：被控端（worker），或 macOS 客户端（自带本机被控）；不含本机与手机。
    /// Windows 客户端不算：它的被控由同一台机器上的被控端系统服务负责，那个服务作为单独的被控端出现在列表里。
    /// </summary>
    public static bool Upgradable(DeviceInfo d) => !d.IsSelf && (d.Kind == DeviceKind.Worker || (d.Kind == DeviceKind.Control && d.Platform == Platform.Macos));

    /// <summary>服务器上有比它新的版本：设备列表标「可升级」，详情里显示「升级被控端」</summary>
    public static bool HasUpdate(DeviceInfo d) => Upgradable(d) && d.UpdateVersion.Length > 0;

    public UpgradeProgress? Of(string deviceId) => items.TryGetValue(deviceId, out var p) ? p : null;

    public bool IsRunning(string deviceId) => Of(deviceId)?.Running == true;

    /// <summary>现在能不能点「升级被控端」：有新版本、在线、没有正在进行的升级</summary>
    public bool CanStart(DeviceInfo d) => app.LoggedIn && HasUpdate(d) && d.Online && !IsRunning(d.Id);

    public void Start(string deviceId)
    {
        if (app.Device(deviceId) is not { } d || !CanStart(d)) return;
        var p = new UpgradeProgress { FromVersion = d.AppVersion, Target = d.UpdateVersion };
        items[deviceId] = p;
        app.Data.Notify();
        try { app.Client.UpgradeDevice(deviceId, new Listener(this, deviceId, p)); }
        catch (Exception e) { Finish(deviceId, p, null, Fmt.Error(e)); }
    }

    /// <summary>收起已结束的结果</summary>
    public void Dismiss(string deviceId)
    {
        if (Of(deviceId) is not { Running: false }) return;
        items.Remove(deviceId);
        app.Data.Notify();
    }

    /// <summary>退出登录：丢掉所有进度（之后到来的回调不再理会）</summary>
    public void Clear() => items.Clear();

    private bool Current(string id, UpgradeProgress p) => items.TryGetValue(id, out var cur) && ReferenceEquals(cur, p) && p.Running;

    private void Started(string id, UpgradeProgress p, string version)
    {
        if (!Current(id, p)) return;
        p.Phase = UpgradePhase.Started;
        if (version.Length > 0) p.Target = version;
        app.Data.Notify();
    }

    private void Offline(string id, UpgradeProgress p)
    {
        if (!Current(id, p)) return;
        p.Phase = UpgradePhase.Offline;
        app.Data.Notify();
    }

    private void Finish(string id, UpgradeProgress p, string? version, string? error)
    {
        if (!Current(id, p)) return;
        var name = app.DeviceName(id);
        string toast;
        if (!string.IsNullOrEmpty(error))
        {
            p.Phase = UpgradePhase.Failed;
            p.Error = error;
            toast = L.T("upgrade.toast.failed", name, error!);
        }
        else
        {
            p.AlreadyLatest = p.Phase == UpgradePhase.Requesting;
            p.Phase = UpgradePhase.Done;
            p.Version = string.IsNullOrEmpty(version) ? (p.AlreadyLatest ? p.FromVersion : p.Target) : version;
            toast = p.AlreadyLatest ? L.T("upgrade.toast.latest", name) : L.T("upgrade.toast.done", name, p.Version!);
        }
        app.Data.Notify();
        // 正在看这台设备的详情时结果已经就地显示，不再弹提示
        var viewing = app.Section == NavSection.Hosts && app.ActiveTabId == null && app.Selection == "dev:" + id;
        if (!viewing) app.ShowToast(toast, p.Phase == UpgradePhase.Failed);
        // 版本号、「可升级」标记跟着刷新
        Ui.Run(async () =>
        {
            try { await app.Client.RefreshDevices(); }
            catch (Exception e) { Log.Info("升级后刷新设备列表失败：" + Fmt.Error(e)); }
        });
    }

    /// <summary>内核线程回调，回界面线程再改状态</summary>
    private sealed class Listener : UpgradeListener
    {
        private readonly UpgradeHub hub;
        private readonly string id;
        private readonly UpgradeProgress p;
        public Listener(UpgradeHub hub, string id, UpgradeProgress p) { this.hub = hub; this.id = id; this.p = p; }
        public void OnStarted(string version) => Ui.Post(() => hub.Started(id, p, version));
        public void OnOffline() => Ui.Post(() => hub.Offline(id, p));
        public void OnFinished(string? version, string? error) => Ui.Post(() => hub.Finish(id, p, version, error));
    }
}
