namespace LemonGalaxy.Core;

// 真内核：把 uniffi 生成的类包装成 CoreApi.cs 里的接口。只做转发，不加逻辑。

internal sealed class RealClient : ICoreClient
{
    private readonly GalaxyClient c;
    public RealClient(GalaxyClient client) { c = client; }

    public bool IsMock => false;
    public string CoreVersion => GalaxyFfiMethods.CoreVersion();

    public string DeviceId() => c.DeviceId();
    public ClientState State() => c.State();
    public UserInfo? CurrentUser() => c.CurrentUser();
    public ServerInfo? Server() => c.Server();
    public bool Restore() => c.Restore();
    public Task<LoginResult> Login(string server, string username, string password, string? totp) => c.Login(server, username, password, totp);
    public Task CompleteFirstLogin(string newPassword) => c.CompleteFirstLogin(newPassword);
    public Task ChangePassword(string oldPassword, string newPassword, string? totp) => c.ChangePassword(oldPassword, newPassword, totp);
    public Task Logout() => c.Logout();
    public Task<TotpSetup> TotpSetup() => c.TotpSetup();
    public Task TotpVerify(string code) => c.TotpVerify(code);
    public Task TotpDisable(string code) => c.TotpDisable(code);
    public DeviceInfo[] Devices() => c.Devices();
    public Task RefreshDevices() => c.RefreshDevices();
    public Task RemoveDevice(string deviceId) => c.RemoveDevice(deviceId);
    public void SetLanDiscovery(bool active) => c.SetLanDiscovery(active);
    public VaultEntry[] Vault() => c.Vault();
    public Task<VaultEntry> VaultSave(VaultEntry entry) => c.VaultSave(entry);
    public Task VaultDelete(string id) => c.VaultDelete(id);

    public bool CanWake(string deviceId) => c.CanWake(deviceId);
    public Task<string[]> WakeDevice(string deviceId) => c.WakeDevice(deviceId);

    public async Task<ICorePeer> Connect(string deviceId) => new RealPeer(await c.Connect(deviceId));
    public async Task<ICorePeer> ConnectAssist(string deviceCode, string verificationCode) => new RealPeer(await c.ConnectAssist(deviceCode, verificationCode));
    public async Task<ICoreSsh> ConnectSsh(string hostId, SecretPrompt prompt) => new RealSsh(await c.ConnectSsh(hostId, prompt));
    public async Task<ICorePortForward> StartPortForward(string entryId, SecretPrompt prompt) => new RealForward(await c.StartPortForward(entryId, prompt));
    public Task<RdpTarget> OpenRdp(string hostId) => c.OpenRdp(hostId);
    public void CloseRdp(string forwardId) => c.CloseRdp(forwardId);
    public Task<UpdateInfo?> CheckUpdate(string component, string platform, string arch, string current) => c.CheckUpdate(component, platform, arch, current);
    public void DownloadUpdate(UpdateInfo info, string dest, TransferListener listener) => c.DownloadUpdate(info, dest, listener);

    public Task<AiModels> AiModels() => c.AiModels();
    public async Task<ICoreConversation> NewConversation(ConversationConfig config, AgentListener listener) => new RealConversation(await c.NewConversation(config, listener));
    public Task<ActivityItem[]> AccountActivity(uint limit) => c.AccountActivity(limit);
    public ConversationSummary[] ListConversations() => c.ListConversations();
    public async Task<ICoreConversation> OpenConversation(string id, ConversationConfig? config, AgentListener listener) => new RealConversation(await c.OpenConversation(id, config, listener));
    public void DeleteConversation(string id) => c.DeleteConversation(id);

    /// 内核按 CoreConfig.host_enabled = false 不会起进程内被控；万一起了也关掉远程。
    public bool DisableBuiltInHost()
    {
        var h = c.Host();
        if (h == null) return true;
        if (h.Status().RemoteEnabled) h.SetRemoteEnabled(false);
        return true;
    }

    public ITermScreen CreateScreen(ushort rows, ushort cols, uint scrollback) => new RealScreen(new TerminalScreen(rows, cols, scrollback));
    public IFrameDecoder CreateDecoder() => new RealDecoder(new H264Decoder());
    public void SetLocale(string coreLocale) => GalaxyFfiMethods.CoreSetLocale(coreLocale);
}

internal sealed class RealPeer : ICorePeer
{
    private readonly PeerSession s;
    private ICoreFiles? files;
    public RealPeer(PeerSession session) { s = session; }

    public PeerInfo Info() => s.Info();
    public PathInfo Path() => s.Path();
    public bool IsClosed() => s.IsClosed();
    public void SetListener(PeerListener listener) => s.SetListener(listener);
    public void Disconnect() => s.Disconnect();
    public async Task<ICoreTerminal> OpenTerminal(TerminalOptions options, TerminalSink sink) => new RealTerminal(await s.OpenTerminal(options, sink));
    public Task<TerminalSessionInfo[]> ListTerminals() => s.ListTerminals();
    public Task KillTerminal(string sessionId) => s.KillTerminal(sessionId);
    public ICoreFiles Files() => files ??= new RealFiles(s.Files());
    public Task<ExecResult> Exec(ExecRequest request) => s.Exec(request);
    public Task<DisplayInfo[]> ListDisplays() => s.ListDisplays();
    public async Task<ICoreDisplay> OpenDisplay(DisplayOptions options, VideoSink sink) => new RealDisplay(await s.OpenDisplay(options, sink));
    public Task<ImageData> Screenshot(uint displayId, uint maxWidth) => s.Screenshot(displayId, maxWidth);
    public Task<ClipboardData?> ClipboardGet() => s.ClipboardGet();
    public Task ClipboardSet(ClipboardData data) => s.ClipboardSet(data);
    public async Task<ICorePortForward> ForwardPort(ushort localPort, string remoteHost, ushort remotePort) => new RealForward(await s.ForwardPort(localPort, remoteHost, remotePort));
    public Task WakeOnLan(string mac) => s.WakeOnLan(mac);
    public Task SetPrivacy(bool on) => s.SetPrivacy(on);
    public async Task<ICoreAudio> StartAudio(float volume) => new RealAudio(await s.StartAudio(volume));
    public ICoreClipboardTransfer ClipboardPushFiles(string[] localPaths, TransferListener listener) => new RealClipboardTransfer(s.ClipboardPushFiles(localPaths, listener));
    public ICoreClipboardTransfer ClipboardPullFiles(string[] remotePaths, string localDir, TransferListener listener) => new RealClipboardTransfer(s.ClipboardPullFiles(remotePaths, localDir, listener));
}

internal sealed class RealClipboardTransfer : ICoreClipboardTransfer
{
    private readonly ClipboardTransfer t;
    public RealClipboardTransfer(ClipboardTransfer transfer) { t = transfer; }
    public void Cancel() => t.Cancel();
    public string[] Paths() => t.Paths();
}

internal sealed class RealSsh : ICoreSsh
{
    private readonly SshSession s;
    private ICoreFiles? files;
    public RealSsh(SshSession session) { s = session; }

    public SshInfo Info() => s.Info();
    public bool IsClosed() => s.IsClosed();
    public async Task<ICoreTerminal> OpenTerminal(TerminalOptions options, TerminalSink sink) => new RealTerminal(await s.OpenTerminal(options, sink));
    public ICoreFiles Files() => files ??= new RealFiles(s.Files());
    public Task<ExecResult> Exec(ExecRequest request) => s.Exec(request);
    public async Task<ICorePortForward> ForwardPort(ushort localPort, string remoteHost, ushort remotePort) => new RealForward(await s.ForwardPort(localPort, remoteHost, remotePort));
    public void Disconnect() => s.Disconnect();
}

internal sealed class RealTerminal : ICoreTerminal
{
    private readonly TerminalChannel t;
    public RealTerminal(TerminalChannel channel) { t = channel; }
    public string SessionId() => t.SessionId();
    public string Title() => t.Title();
    public void Write(byte[] data) => t.Write(data);
    public void Resize(ushort cols, ushort rows) => t.Resize(cols, rows);
    public void Detach() => t.Detach();
    public void Kill() => t.Kill();
}

internal sealed class RealFiles : ICoreFiles
{
    private readonly FileClient f;
    public RealFiles(FileClient client) { f = client; }
    public Task<string> Home() => f.Home();
    public Task<FileEntry[]> List(string path) => f.List(path);
    public Task<FileEntry> Stat(string path) => f.Stat(path);
    public Task<string> Realpath(string path) => f.Realpath(path);
    public Task Mkdir(string path) => f.Mkdir(path);
    public Task Rename(string from, string to) => f.Rename(from, to);
    public Task Remove(string path, bool recursive) => f.Remove(path, recursive);
    public Task Chmod(string path, uint mode) => f.Chmod(path, mode);
    public Task<FileEntry[]> Search(string root, string pattern, bool content) => f.Search(root, pattern, content);
    public Task<string> ReadText(string path, ulong maxBytes) => f.ReadText(path, maxBytes);
    public Task WriteText(string path, string text) => f.WriteText(path, text);
    public ICoreTransfer Download(string remote, string local, TransferListener listener) => new RealTransfer(f.Download(remote, local, listener));
    public ICoreTransfer Upload(string local, string remote, TransferListener listener) => new RealTransfer(f.Upload(local, remote, listener));
}

internal sealed class RealTransfer : ICoreTransfer
{
    private readonly Transfer t;
    public RealTransfer(Transfer transfer) { t = transfer; }
    public void Cancel() => t.Cancel();
    public void Pause() => t.Pause();
    public void Resume() => t.Resume();
}

/// <summary>停止时同时释放内核对象（不等终结器）；之后的调用都是空操作</summary>
internal sealed class RealAudio : ICoreAudio
{
    private readonly AudioPlayback a;
    private bool stopped;
    public RealAudio(AudioPlayback playback) { a = playback; }

    public void SetVolume(float volume) { lock (a) { if (!stopped) a.SetVolume(volume); } }
    public void SetMuted(bool muted) { lock (a) { if (!stopped) a.SetMuted(muted); } }
    public bool IsActive() { lock (a) { return !stopped && a.IsActive(); } }

    public void Stop()
    {
        lock (a)
        {
            if (stopped) return;
            stopped = true;
            a.Stop();
            a.Dispose();
        }
    }
}

internal sealed class RealDisplay : ICoreDisplay
{
    private readonly DisplayChannel d;
    public RealDisplay(DisplayChannel channel) { d = channel; }
    public void SendInput(InputEvent e) => d.SendInput(e);
    public void RequestKeyframe() => d.RequestKeyframe();
    public void SetBitrate(uint kbps) => d.SetBitrate(kbps);
    public void Stop() => d.Stop();
    public Task StartRecording(string path, bool withAudio) => d.StartRecording(path, withAudio);
    public RecordingInfo StopRecording() => d.StopRecording();
    public bool IsRecording() => d.IsRecording();
}

internal sealed class RealForward : ICorePortForward
{
    private readonly PortForward p;
    public RealForward(PortForward forward) { p = forward; }
    public ushort LocalPort() => p.LocalPort();
    public void Stop() => p.Stop();
}

internal sealed class RealConversation : ICoreConversation
{
    private readonly AgentConversation c;
    public RealConversation(AgentConversation conversation) { c = conversation; }
    public string Id() => c.Id();
    public string Title() => c.Title();
    public void Send(string text) => c.Send(text);
    public void SendWithRefs(string text, ContextRef[] refs) => c.SendWithRefs(text, refs);
    public void SetModel(ulong providerId, string modelId) => c.SetModel(providerId, modelId);
    public ConversationModel Model() => c.Model();
    public void Decide(string callId, ApprovalDecision decision) => c.Decide(callId, decision);
    public void Answer(string questionId, string answer) => c.Answer(questionId, answer);
    public void ProvideSecret(string requestId, string secret) => c.ProvideSecret(requestId, secret);
    public void SetMode(AiMode mode, uint? trustMinutes) => c.SetMode(mode, trustMinutes);
    public void Stop() => c.Stop();
    public void SetForeground(bool foreground) => c.SetForeground(foreground);
    public AgentMessage[] Messages() => c.Messages();
}

internal sealed class RealScreen : ITermScreen
{
    private readonly TerminalScreen s;
    public RealScreen(TerminalScreen screen) { s = screen; }
    public void Feed(byte[] data) => s.Feed(data);
    public void Resize(ushort rows, ushort cols) => s.Resize(rows, cols);
    public TermScreenState State(uint scrollOffset) => s.State(scrollOffset);
    public string Text() => s.Text();
    public string TextBetween(ushort startRow, ushort startCol, ushort endRow, ushort endCol) => s.TextBetween(startRow, startCol, endRow, endCol);
}

internal sealed class RealDecoder : IFrameDecoder
{
    private readonly H264Decoder d;
    public RealDecoder(H264Decoder decoder) { d = decoder; }
    public DecodedFrame? Decode(byte[] data) => d.Decode(data);
}
