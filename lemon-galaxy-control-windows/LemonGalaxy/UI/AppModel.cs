using System.Diagnostics;
using System.IO;
using System.Text;
using System.Windows;
using LemonGalaxy.Core;

namespace LemonGalaxy.UI;

internal enum NavSection { Hosts, Sftp, Ai, Mcp, Forwards, Snippets, Local, Settings }
internal enum HostsListTab { Hosts, Credentials, KnownHosts }
internal enum SettingsPage { Account, Appearance, Terminal, Connection, Ai, About }
internal enum TabKind { Terminal, Sftp, Remote }

/// <summary>右侧编辑面板的目标</summary>
internal abstract record EditTarget
{
    public sealed record Vault(string Id) : EditTarget;
    public sealed record NewSsh(string? Folder) : EditTarget;
    public sealed record NewRdp(string? Folder) : EditTarget;
    public sealed record NewCredential : EditTarget;
    public sealed record Device(string Id) : EditTarget;
    public sealed record NewSnippet : EditTarget;
    /// <summary>端口映射规则；Id 为空表示新建</summary>
    public sealed record Forward(string? Id) : EditTarget;
}

/// <summary>界面内的模态卡片（画在主窗口里，不用系统对话框，selftest 也能离屏渲染出来）</summary>
internal sealed class ModalSpec
{
    public string Id = Guid.NewGuid().ToString("N");
    public Func<FrameworkElement> Build = () => new System.Windows.Controls.Border();
    public double Width = 400;
    public bool Dismissable = true;
    /// <summary>点遮罩或关闭按钮关掉时调用（SSH 问密码要给内核回 null）</summary>
    public Action? OnDismiss;
}

/// <summary>提示条；可带一个操作（如「在资源管理器中显示」）</summary>
internal sealed record Toast(string Text, bool IsError, string? ActionTitle = null, Action? Action = null);

/// <summary>主窗口的全部状态。界面只通过它和内核打交道。</summary>
internal sealed class AppModel
{
    public static AppModel? Current;

    public readonly AppSettings Settings;
    public readonly ICoreClient Client;
    public readonly bool SelfTest;
    public readonly string? CoreFallbackReason;

    /// <summary>登录状态、设备、加密库、账号</summary>
    public readonly Observable Data = new();
    /// <summary>导航：分区、选择、编辑、标签页</summary>
    public readonly Observable Nav = new();
    /// <summary>模态卡片与提示条</summary>
    public readonly Observable Overlay = new();
    /// <summary>登录页 / 主界面切换</summary>
    public readonly Observable PhaseChanged = new();

    public bool LoggedIn;
    public ClientState ClientState = ClientState.LoggedOut;
    public UserInfo? User;
    public ServerInfo? Server;
    public DeviceInfo[] Devices = Array.Empty<DeviceInfo>();
    public VaultEntry[] Vault = Array.Empty<VaultEntry>();

    public NavSection Section = NavSection.Hosts;
    public HostsListTab HostsTab = HostsListTab.Hosts;
    public string? Selection;
    public EditTarget? Editing;
    public string SearchText = "";
    public string? TagFilter;
    public SettingsPage SettingsPage = SettingsPage.Account;
    public string? SelectedSnippet;
    public string? SelectedForward;

    public readonly List<TabModel> Tabs = new();
    public string? ActiveTabId;
    public ModalSpec? Modal;
    public Toast? CurrentToast;

    public readonly TransferQueue Transfers = new();
    public readonly AiHub Ai;
    public readonly McpHub Mcp;
    public readonly WorkerServiceModel Worker;
    public readonly Updater Updater;
    public readonly PortForwardHub Forwards;

    private readonly Dictionary<string, Task<PeerConnection>> connecting = new();
    private readonly Dictionary<string, PeerConnection> peers = new();
    private readonly Dictionary<string, SshConnection> sshSessions = new();
    private readonly Dictionary<string, Task<SshConnection>> sshConnecting = new();
    private readonly Dictionary<string, string> assistNames = new();

    public AppModel(AppSettings settings, bool selfTest, bool forceMock, MockOptions? mockOptions = null)
    {
        Current = this;
        Settings = settings;
        SelfTest = selfTest;
        foreach (var d in new[] { AppInfo.DataDir, AppInfo.CacheDir })
            try { Directory.CreateDirectory(d); } catch { }
        var adapter = new ClientListenerAdapter(this);
        // 模拟内核不碰系统的安全存储
        SecureStore store = selfTest || forceMock ? new MemoryStore() : new DpapiStore(IOPath.Combine(AppInfo.DataDir, "secure"));
        var config = new CoreConfig(AppInfo.DataDir, AppInfo.CacheDir, "info", Platform.Windows, DeviceKind.Control, AppInfo.Version,
            AppInfo.OsVersion, AppInfo.Hostname, L.Pack.CoreLocale,
            // 本机被控交给系统服务 LemonGalaxyWorker，内核不在进程内跑被控
            HostEnabled: false);
        (Client, CoreFallbackReason) = CoreProvider.Make(config, store, adapter, forceMock, mockOptions);
        Ai = new AiHub(this);
        Mcp = new McpHub(this);
        Forwards = new PortForwardHub(this);
        Worker = new WorkerServiceModel(this);
        Updater = new Updater(this);
        if (!selfTest && Client.Restore()) DidLogin();
    }

    // ---------- 登录状态 ----------

    public void DidLogin()
    {
        User = Client.CurrentUser();
        Server = Client.Server();
        ClientState = Client.State();
        Devices = Client.Devices();
        Vault = Client.Vault();
        LoggedIn = true;
        DisableBuiltInHost();
        Ai.ReloadSummaries();
        PhaseChanged.Notify();
        Data.Notify();
        // 本机 MCP 服务：开关开着就开启（退出登录不关服务，再登录时已在运行就不动）
        Mcp.OnLogin();
        Ui.Run(async () =>
        {
            try { await Client.RefreshDevices(); } catch { }
            // 设备列表刷新后再开自动开启的端口映射（经由的设备要先知道在不在线）
            if (LoggedIn) Forwards.AutoStart();
        });
    }

    /// <summary>Windows 上被控交给 lemon-galaxy-worker 系统服务；内核里 control 客户端自带的被控服务关掉，免得同一台机器出现两个可连的设备。</summary>
    private bool builtInHostOff;

    private void DisableBuiltInHost()
    {
        if (builtInHostOff) return;
        try { builtInHostOff = Client.DisableBuiltInHost(); }
        catch (Exception e) { Log.Error("关闭内核自带的被控服务失败：" + e.Message); }
    }

    public void SetLanDiscovery(bool active)
    {
        if (!LoggedIn) return;
        try { Client.SetLanDiscovery(active); } catch { }
    }

    public async Task Logout()
    {
        CloseAllTabs();
        Forwards.StopAll();
        try { await Client.Logout(); } catch { }
        Mcp.OnLogout();
        LoggedIn = false;
        builtInHostOff = false;
        User = null;
        Server = null;
        Devices = Array.Empty<DeviceInfo>();
        Vault = Array.Empty<VaultEntry>();
        PhaseChanged.Notify();
        Data.Notify();
    }

    public void HandleState(ClientState s)
    {
        ClientState = s;
        if (s == ClientState.LoggedOut && LoggedIn)
        {
            LoggedIn = false;
            Forwards.StopAll();
            Mcp.OnLogout();
            PhaseChanged.Notify();
        }
        if (s == ClientState.Online && LoggedIn)
        {
            User = Client.CurrentUser();
            Server = Client.Server();
            DisableBuiltInHost();
        }
        Data.Notify();
    }

    public void HandleSessionRevoked()
    {
        CloseAllTabs();
        Forwards.StopAll();
        Mcp.OnLogout();
        LoggedIn = false;
        PhaseChanged.Notify();
        ShowToast(L.T("login.revoked"), true);
    }

    public void ReloadDevices()
    {
        Devices = Client.Devices();
        if (LoggedIn) DisableBuiltInHost();
        Data.Notify();
    }
    public void ReloadVault()
    {
        Vault = Client.Vault();
        Forwards.VaultChanged();
        Data.Notify();
    }

    // ---------- 加密库辅助 ----------

    public VaultEntry? Entry(string id) => Vault.FirstOrDefault(e => e.Id == id);
    public DeviceInfo? Device(string id) => Devices.FirstOrDefault(d => d.Id == id);
    public VaultEntry? MetaFor(string deviceId) => Vault.FirstOrDefault(e => e.Data is VaultData.DeviceMeta m && m.DeviceId == deviceId);

    public string DeviceName(string id)
    {
        if (assistNames.TryGetValue(id, out var n)) return n;
        if (MetaFor(id)?.Data is VaultData.DeviceMeta { Alias: { Length: > 0 } alias }) return alias;
        return Device(id)?.Hostname ?? id;
    }

    public string SshLabel(string id) => Entry(id)?.Data is VaultData.SshHost h ? h.Label : id;

    public List<VaultEntry> Credentials => Vault.Where(e => e.Data is VaultData.Credential).OrderBy(e => e.Sort).ToList();
    public List<VaultEntry> Snippets => Vault.Where(e => e.Data is VaultData.Snippet).OrderBy(e => e.Sort).ToList();
    public List<VaultEntry> KnownHosts => Vault.Where(e => e.Data is VaultData.KnownHost).ToList();
    public List<VaultEntry> SshHosts => Vault.Where(e => e.Data is VaultData.SshHost).OrderBy(e => e.Sort).ThenBy(e => SshLabel(e.Id)).ToList();
    public List<VaultEntry> PortForwards => Vault.Where(e => e.Data is VaultData.PortForward).OrderBy(e => e.Sort).ThenBy(e => ((VaultData.PortForward)e.Data).Label, StringComparer.CurrentCultureIgnoreCase).ToList();

    public List<string> AllTags
    {
        get
        {
            var set = new SortedSet<string>(StringComparer.Ordinal);
            foreach (var e in Vault)
            {
                var tags = e.Data switch
                {
                    VaultData.DeviceMeta m => m.Tags,
                    VaultData.SshHost s => s.Tags,
                    VaultData.RdpHost r => r.Tags,
                    _ => Array.Empty<string>(),
                };
                foreach (var t in tags) set.Add(t);
            }
            return set.ToList();
        }
    }

    public async Task<VaultEntry?> Save(VaultEntry entry)
    {
        // 先在本地更新，界面立即响应；内核保存后以返回值为准
        if (entry.Id.Length > 0)
        {
            var i = Array.FindIndex(Vault, e => e.Id == entry.Id);
            if (i >= 0) { Vault = (VaultEntry[])Vault.Clone(); Vault[i] = entry; Data.Notify(); }
        }
        try
        {
            var saved = await Client.VaultSave(entry);
            var list = Vault.ToList();
            var j = list.FindIndex(e => e.Id == saved.Id);
            if (j >= 0) list[j] = saved; else list.Add(saved);
            Vault = list.ToArray();
            Data.Notify();
            return saved;
        }
        catch (Exception e)
        {
            ShowError(e);
            ReloadVault();
            return null;
        }
    }

    public async Task Delete(string id)
    {
        try
        {
            await Client.VaultDelete(id);
            ReloadVault();
            if (Selection == id) Selection = null;
            if (SelectedForward == id) SelectedForward = null;
            if (Editing is EditTarget.Vault v && v.Id == id) Editing = null;
            if (Editing is EditTarget.Forward f && f.Id == id) Editing = null;
            Nav.Notify();
        }
        catch (Exception e) { ShowError(e); }
    }

    /// <summary>设备的别名、标签、收藏、所在文件夹存在 DeviceMeta 里；没有就新建一条</summary>
    public async Task UpdateDeviceMeta(string deviceId, Func<VaultData.DeviceMeta, string?, (VaultData.DeviceMeta, string?)> change)
    {
        var e = MetaFor(deviceId) ?? new VaultEntry("", null, 0, new VaultData.DeviceMeta(deviceId, null, Array.Empty<string>(), "", false), 0);
        if (e.Data is not VaultData.DeviceMeta meta) return;
        var (m, folder) = change(meta, e.FolderId);
        await Save(e with { Data = m, FolderId = folder });
    }

    // ---------- 提示 ----------

    public void ShowToast(string text, bool error = false, string? actionTitle = null, Action? action = null)
    {
        var t = new Toast(text, error, actionTitle, action);
        CurrentToast = t;
        Overlay.Notify();
        if (SelfTest) return;
        // 带操作的提示多停一会儿，给用户时间点
        Ui.After(action != null ? 8000 : error ? 5000 : 2500, () =>
        {
            if (!ReferenceEquals(CurrentToast, t)) return;
            CurrentToast = null;
            Overlay.Notify();
        });
    }

    public void ShowError(Exception e) => ShowToast(Fmt.Error(e), true);

    public void ShowModal(ModalSpec m)
    {
        var old = Modal;
        Modal = m;
        Overlay.Notify();
        if (old != null && !ReferenceEquals(old, m)) old.OnDismiss?.Invoke();
    }

    public void CloseModal(bool dismissed = false)
    {
        var m = Modal;
        Modal = null;
        Overlay.Notify();
        if (dismissed) m?.OnDismiss?.Invoke();
    }

    public void Confirm(string title, string message, string button, Action action, bool destructive = true)
    {
        ShowModal(new ModalSpec { Width = 400, Build = () => Modals.ConfirmCard(this, title, message, button, destructive, action) });
    }

    // ---------- 连接池（同一台设备的终端、文件、桌面、AI 共用一条端到端会话）----------

    public Task<PeerConnection> Peer(string deviceId)
    {
        if (peers.TryGetValue(deviceId, out var p) && p.ClosedReason == null && !p.Session.IsClosed()) return Task.FromResult(p);
        if (connecting.TryGetValue(deviceId, out var pending)) return pending;
        var t = ConnectPeer(deviceId);
        // 同步就完成了（立即失败、模拟内核）时 ConnectPeer 的 finally 已经先跑过：不能再记进去，否则之后断线重连一直拿到这个旧结果
        if (!t.IsCompleted) connecting[deviceId] = t;
        return t;
    }

    private async Task<PeerConnection> ConnectPeer(string deviceId)
    {
        try
        {
            var session = await Client.Connect(deviceId);
            var p = new PeerConnection(deviceId, session);
            peers[deviceId] = p;
            return p;
        }
        finally { connecting.Remove(deviceId); }
    }

    public Task<SshConnection> Ssh(string hostId)
    {
        if (sshSessions.TryGetValue(hostId, out var s) && !s.Session.IsClosed()) return Task.FromResult(s);
        if (sshConnecting.TryGetValue(hostId, out var pending)) return pending;
        var t = ConnectSsh(hostId);
        sshConnecting[hostId] = t;
        return t;
    }

    private async Task<SshConnection> ConnectSsh(string hostId)
    {
        try
        {
            var session = await Client.ConnectSsh(hostId, new SecretPromptBridge(this, SshLabel(hostId)));
            var c = new SshConnection(hostId, session);
            sshSessions[hostId] = c;
            return c;
        }
        finally { sshConnecting.Remove(hostId); }
    }

    /// <summary>临时协助连上的设备不在本账号设备列表里：用合成 id 放进连接池，标签页照常按 id 取会话</summary>
    public string RegisterAssistPeer(ICorePeer s)
    {
        var info = s.Info();
        var id = "assist:" + info.DeviceId;
        peers[id] = new PeerConnection(id, s);
        assistNames[id] = info.Hostname + " · " + L.T("assist.badge");
        return id;
    }

    public PeerInfo? AssistInfo(string id) => id.StartsWith("assist:", StringComparison.Ordinal) && peers.TryGetValue(id, out var p) ? p.Info : null;

    private void ReleaseIfUnused(string? deviceId, string? sshHostId)
    {
        if (deviceId != null && !Tabs.Any(t => t.DeviceId == deviceId) && peers.TryGetValue(deviceId, out var p))
        {
            try { p.Session.Disconnect(); } catch { }
            peers.Remove(deviceId);
        }
        if (sshHostId != null && !Tabs.Any(t => t.SshHostId == sshHostId) && sshSessions.TryGetValue(sshHostId, out var s))
        {
            try { s.Session.Disconnect(); } catch { }
            sshSessions.Remove(sshHostId);
        }
    }

    // ---------- 标签页 ----------

    public TabModel? ActiveTab => Tabs.FirstOrDefault(t => t.Id == ActiveTabId);

    public void AddTab(TabModel tab)
    {
        Tabs.Add(tab);
        ActiveTabId = tab.Id;
        Nav.Notify();
    }

    public void ActivateTab(string? id)
    {
        ActiveTabId = id;
        Nav.Notify();
    }

    public void CloseTab(string id)
    {
        var i = Tabs.FindIndex(t => t.Id == id);
        if (i < 0) return;
        var tab = Tabs[i];
        try { tab.Close(); } catch (Exception e) { Log.Error("关闭标签出错：" + e.Message); }
        Tabs.RemoveAt(i);
        if (ActiveTabId == id) ActiveTabId = Tabs.Count == 0 ? null : Tabs[Math.Min(i, Tabs.Count - 1)].Id;
        ReleaseIfUnused(tab.DeviceId, tab.SshHostId);
        Nav.Notify();
    }

    public void CloseAllTabs()
    {
        foreach (var t in Tabs) { try { t.Close(); } catch { } }
        Tabs.Clear();
        ActiveTabId = null;
        foreach (var p in peers.Values) { try { p.Session.Disconnect(); } catch { } }
        peers.Clear();
        foreach (var s in sshSessions.Values) { try { s.Session.Disconnect(); } catch { } }
        sshSessions.Clear();
        Nav.Notify();
    }

    public void SelectHome(NavSection? s = null)
    {
        if (s is { } sec) Section = sec;
        ActiveTabId = null;
        Nav.Notify();
    }

    // ---------- 打开连接 ----------

    public void OpenTerminal(string deviceId, TerminalSessionInfo? attach = null)
    {
        var tab = new TerminalTab(this, deviceId, null, DeviceName(deviceId), attach);
        AddTab(tab);
        tab.Start();
    }

    public void OpenSshTerminal(string hostId)
    {
        var tab = new TerminalTab(this, null, hostId, SshLabel(hostId), null);
        AddTab(tab);
        tab.Start();
    }

    /// <summary>先查被控端保留的会话：有就让用户选接续还是新开</summary>
    public void OpenTerminalAsking(string deviceId)
    {
        if (!Settings.ReattachTerminals) { OpenTerminal(deviceId); return; }
        Ui.Run(async () =>
        {
            try
            {
                var p = await Peer(deviceId);
                var list = (await p.Session.ListTerminals()).Where(s => !s.Attached).ToArray();
                if (list.Length == 0) OpenTerminal(deviceId);
                else ShowTerminalSessions(deviceId, list);
            }
            catch (Exception e) { ShowError(e); }
        });
    }

    public void ShowTerminalSessions(string deviceId, TerminalSessionInfo[] sessions)
        => ShowModal(new ModalSpec { Width = 460, Build = () => Modals.TerminalSessionsCard(this, deviceId, sessions) });

    public void OpenSftp(string? deviceId = null, string? sshHostId = null)
    {
        var tab = new SftpTab(this);
        if (deviceId != null) { tab.Title = DeviceName(deviceId); tab.DeviceId = deviceId; }
        if (sshHostId != null) { tab.Title = SshLabel(sshHostId); tab.SshHostId = sshHostId; }
        AddTab(tab);
        tab.Start(deviceId != null ? FileEndpoint.ForDevice(deviceId) : sshHostId != null ? FileEndpoint.ForSsh(sshHostId) : null);
    }

    public void OpenRemoteDesktop(string deviceId)
    {
        var tab = new RemoteTab(this, deviceId, DeviceName(deviceId));
        AddTab(tab);
        tab.Start();
    }

    /// <summary>设备、主机页的「AI」：切到 AI 页开一个新对话，输入框里先引用这台设备或主机</summary>
    public void OpenAi(params AgentTarget[] targets)
    {
        Section = NavSection.Ai;
        ActiveTabId = null;
        Ai.StartWith(targets.Select(t => t.Kind == AgentTargetKind.Device ? AiHub.DeviceRef(this, t.Id) : AiHub.SshRef(this, t.Id)));
        Nav.Notify();
    }

    /// <summary>切到 MCP 页（新的待批准通知点开时）</summary>
    public void OpenMcp()
    {
        Section = NavSection.Mcp;
        ActiveTabId = null;
        Nav.Notify();
    }

    /// <summary>RDP 主机：内核准备好地址（经由设备时开本机转发），交给系统的远程桌面连接（mstsc）</summary>
    public void OpenRdp(string hostId)
    {
        if (Entry(hostId)?.Data is not VaultData.RdpHost) return;
        Ui.Run(async () =>
        {
            var target = await Client.OpenRdp(hostId);
            RdpLauncher.Open(this, target);
        });
    }

    /// <summary>双击设备：弹连接方式选择</summary>
    public void ActivateDevice(string id)
    {
        var d = Device(id);
        if (d == null || d.IsSelf || d.Kind == DeviceKind.Mobile) return;
        ShowModal(new ModalSpec { Width = 460, Build = () => Modals.ConnectChooser(this, id) });
    }

    public bool AiEnabled => User?.AiEnabled ?? false;

    // ---------- 远程开机 ----------

    private readonly HashSet<string> waking = new();

    /// <summary>离线、能被控、上报过网卡地址的设备显示「远程开机」</summary>
    public bool CanWake(string deviceId)
    {
        if (!LoggedIn) return false;
        try { return Client.CanWake(deviceId); }
        catch (Exception e) { Log.Error("查询能否远程开机失败：" + e.Message); return false; }
    }

    public bool IsWaking(string deviceId) => waking.Contains(deviceId);

    public void WakeDevice(string deviceId)
    {
        if (!waking.Add(deviceId)) return;
        Data.Notify();
        Ui.Run(async () =>
        {
            try
            {
                var via = await Client.WakeDevice(deviceId);
                var names = string.Join(L.T("wake.viaSeparator"), via.Select(v => v.Length == 0 ? L.T("wake.viaSelf") : v).Distinct());
                ShowToast(names.Length == 0 ? L.T("wake.sentPlain") : L.T("wake.sent", names));
            }
            catch (Exception e) { ShowToast(L.T("wake.failed", Fmt.Error(e)), true); }
            finally
            {
                waking.Remove(deviceId);
                Data.Notify();
            }
        });
    }

    // ---------- 会话录像 ----------

    /// <summary>录像保存目录：用户「视频」文件夹下的 LemonGalaxy（selftest 用临时目录，不碰用户目录）</summary>
    public string RecordingsDir
    {
        get
        {
            if (SelfTest) return IOPath.Combine(IOPath.GetTempPath(), "lemongalaxy-selftest-recordings");
            var videos = Environment.GetFolderPath(Environment.SpecialFolder.MyVideos);
            if (string.IsNullOrEmpty(videos)) videos = IOPath.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), "Videos");
            return IOPath.Combine(videos, "LemonGalaxy");
        }
    }

    /// <summary>在资源管理器里打开文件所在文件夹并选中它</summary>
    public void RevealInExplorer(string path)
    {
        if (SelfTest) return;
        try
        {
            if (File.Exists(path)) Process.Start(new ProcessStartInfo("explorer.exe", "/select,\"" + path + "\"") { UseShellExecute = true });
            else if (IOPath.GetDirectoryName(path) is { } dir && Directory.Exists(dir)) Process.Start(new ProcessStartInfo("explorer.exe", "\"" + dir + "\"") { UseShellExecute = true });
            else ShowToast(L.T("remote.record.missing"), true);
        }
        catch (Exception e) { ShowError(e); }
    }
}

/// <summary>一条与被控设备的端到端会话</summary>
internal sealed class PeerConnection : Observable
{
    public readonly string DeviceId;
    public readonly ICorePeer Session;
    public PathInfo Path;
    public PeerInfo Info;
    public string? ClosedReason;
    public readonly List<ICorePortForward> Forwards = new();

    public PeerConnection(string deviceId, ICorePeer session)
    {
        DeviceId = deviceId;
        Session = session;
        Path = session.Path();
        Info = session.Info();
        session.SetListener(new PeerListenerAdapter(this));
    }

    private sealed class PeerListenerAdapter : PeerListener
    {
        private readonly WeakReference<PeerConnection> owner;
        public PeerListenerAdapter(PeerConnection o) { owner = new WeakReference<PeerConnection>(o); }
        public void OnPathChanged(PathInfo path) => Ui.Post(() => { if (owner.TryGetTarget(out var o)) { o.Path = path; o.Notify(); } });
        public void OnClosed(string reason) => Ui.Post(() => { if (owner.TryGetTarget(out var o)) { o.ClosedReason = reason; o.Notify(); } });
    }
}

internal sealed class SshConnection
{
    public readonly string HostId;
    public readonly ICoreSsh Session;
    public readonly SshInfo Info;
    public SshConnection(string hostId, ICoreSsh session) { HostId = hostId; Session = session; Info = session.Info(); }
}

/// <summary>内核回调（任意线程）→ 界面线程</summary>
internal sealed class ClientListenerAdapter : ClientListener
{
    private readonly AppModel model;
    public ClientListenerAdapter(AppModel m) { model = m; }
    public void OnState(ClientState state) => Ui.Post(() => model.HandleState(state));
    public void OnDevicesChanged() => Ui.Coalesce("devices", model.ReloadDevices);
    public void OnVaultChanged() => Ui.Coalesce("vault", model.ReloadVault);
    public void OnSessionRevoked() => Ui.Post(model.HandleSessionRevoked);
}

/// <summary>
/// SSH 直连时向用户要密码、私钥口令、键盘交互答案，以及确认主机密钥。内核在自己的线程上同步调用，
/// 这里切到界面线程弹卡片，当前（内核）线程等用户回答；不能在界面线程上等界面线程。
/// </summary>
internal sealed class SecretPromptBridge : SecretPrompt
{
    private readonly AppModel model;
    private readonly string title;
    public SecretPromptBridge(AppModel model, string title) { this.model = model; this.title = title; }

    public string? Ask(string prompt, bool echo)
    {
        if (Ui.OnUiThread) return null;
        string? answer = null;
        using var done = new ManualResetEventSlim(false);
        Ui.Dispatcher.Invoke(() => model.ShowModal(new ModalSpec
        {
            Width = 400, Dismissable = true,
            OnDismiss = () => { answer = null; done.Set(); },
            Build = () => Modals.SecretPromptCard(model, title, prompt, echo, v => { answer = v; done.Set(); }),
        }));
        done.Wait();
        return answer;
    }

    public bool ConfirmHostKey(string host, ushort port, string keyType, string fingerprint, bool changed)
    {
        if (Ui.OnUiThread) return false;
        var ok = false;
        using var done = new ManualResetEventSlim(false);
        Ui.Dispatcher.Invoke(() => model.ShowModal(new ModalSpec
        {
            Width = 460, Dismissable = true,
            OnDismiss = () => { ok = false; done.Set(); },
            Build = () => Modals.HostKeyCard(model, host, port, keyType, fingerprint, changed, v => { ok = v; done.Set(); }),
        }));
        done.Wait();
        return ok;
    }
}

/// <summary>
/// RDP 直连：生成 .rdp 文件交给系统自带的远程桌面连接（mstsc）打开。加密库里存了密码时用 cmdkey 临时放进
/// Windows 凭据管理器（TERMSRV/主机），mstsc 退出后删除；经由设备时 mstsc 退出后关掉本机端口转发。嵌入式 RDP 控件后续再做。
/// </summary>
internal static class RdpLauncher
{
    public static void Open(AppModel app, RdpTarget t)
    {
        if (app.SelfTest) return;
        var address = $"{t.Host}:{t.Port}";
        var user = (t.Domain is { Length: > 0 } ? t.Domain + "\\" : "") + t.Username;
        var sb = new StringBuilder();
        sb.Append("full address:s:").Append(address).Append("\r\n");
        sb.Append("username:s:").Append(user).Append("\r\n");
        sb.Append("prompt for credentials:i:").Append(string.IsNullOrEmpty(t.Password) ? 1 : 0).Append("\r\n");
        sb.Append("screen mode id:i:1\r\nuse multimon:i:0\r\nredirectclipboard:i:1\r\nauthentication level:i:2\r\n");
        var safe = string.Concat(t.Label.Select(c => IOPath.GetInvalidFileNameChars().Contains(c) ? '-' : c));
        var path = IOPath.Combine(IOPath.GetTempPath(), "LemonGalaxy-" + safe + ".rdp");
        var credTarget = "TERMSRV/" + t.Host;
        var storedCred = false;
        try
        {
            File.WriteAllText(path, sb.ToString(), Encoding.Unicode);
            if (!string.IsNullOrEmpty(t.Password))
                storedCred = RunHidden("cmdkey.exe", $"/generic:{Quote(credTarget)} /user:{Quote(user)} /pass:{Quote(t.Password!)}");
            var p = Process.Start(new ProcessStartInfo("mstsc.exe", Quote(path)) { UseShellExecute = false });
            // mstsc 关闭后清理：删除临时凭据、关掉端口转发、删掉 .rdp 文件
            Task.Run(() =>
            {
                try { p?.WaitForExit(); } catch { }
                if (storedCred) RunHidden("cmdkey.exe", $"/delete:{Quote(credTarget)}");
                if (t.Forwarded) { try { app.Client.CloseRdp(t.ForwardId); } catch { } }
                try { File.Delete(path); } catch { }
            });
        }
        catch (Exception e)
        {
            if (storedCred) RunHidden("cmdkey.exe", $"/delete:{Quote(credTarget)}");
            if (t.Forwarded) { try { app.Client.CloseRdp(t.ForwardId); } catch { } }
            app.ShowToast(L.T("rdp.noclient.title") + "：" + e.Message, true);
        }
    }

    private static string Quote(string s) => "\"" + s.Replace("\"", "") + "\"";

    private static bool RunHidden(string exe, string args)
    {
        try
        {
            using var p = Process.Start(new ProcessStartInfo(exe, args) { UseShellExecute = false, CreateNoWindow = true, WindowStyle = ProcessWindowStyle.Hidden });
            p?.WaitForExit(5000);
            return p is { HasExited: true, ExitCode: 0 };
        }
        catch { return false; }
    }
}
