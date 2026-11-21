namespace LemonGalaxy.Core;

// 界面使用的内核接口。
// uniffi 生成的 IGalaxyClient / IPeerSession 等接口的方法返回的是具体类（PeerSession、FileClient …），模拟内核无法实现，
// 所以这里按生成接口逐个方法镜像一份，只把返回的对象换成本文件里的接口；记录、枚举、回调接口（ClientListener、TerminalSink …）
// 直接用生成的类型。真内核由 RealCore.cs 包装生成的类，模拟内核在 Core/Mock 里。

internal interface ICoreClient
{
    bool IsMock { get; }
    string CoreVersion { get; }

    string DeviceId();
    ClientState State();
    UserInfo? CurrentUser();
    ServerInfo? Server();
    bool Restore();
    Task<LoginResult> Login(string server, string username, string password, string? totp);
    Task CompleteFirstLogin(string newPassword);
    Task ChangePassword(string oldPassword, string newPassword, string? totp);
    Task Logout();
    Task<TotpSetup> TotpSetup();
    Task TotpVerify(string code);
    Task TotpDisable(string code);

    DeviceInfo[] Devices();
    Task RefreshDevices();
    Task RemoveDevice(string deviceId);
    void SetLanDiscovery(bool active);

    VaultEntry[] Vault();
    Task<VaultEntry> VaultSave(VaultEntry entry);
    Task VaultDelete(string id);

    /// <summary>能不能远程开机：设备离线、能被控、上报过网卡地址</summary>
    bool CanWake(string deviceId);
    /// <summary>远程开机：本机或同一网络里在线的设备代发唤醒包，返回代发设备名（空字符串表示本机）</summary>
    Task<string[]> WakeDevice(string deviceId);

    Task<ICorePeer> Connect(string deviceId);
    Task<ICorePeer> ConnectAssist(string deviceCode, string verificationCode);
    Task<ICoreSsh> ConnectSsh(string hostId, SecretPrompt prompt);
    /// <summary>按加密库里的端口映射规则开启映射（经由设备或 SSH 主机），stop() 之前一直有效</summary>
    Task<ICorePortForward> StartPortForward(string entryId, SecretPrompt prompt);
    /// <summary>准备连接加密库里的 rdp_host：经由设备时在本机开端口转发</summary>
    Task<RdpTarget> OpenRdp(string hostId);
    void CloseRdp(string forwardId);

    /// <summary>向自己的服务器查询新版本（返回前已验发布签名）</summary>
    Task<UpdateInfo?> CheckUpdate(string component, string platform, string arch, string current);
    void DownloadUpdate(UpdateInfo info, string dest, TransferListener listener);

    Task<AiModels> AiModels();
    Task<ICoreConversation> NewConversation(ConversationConfig config, AgentListener listener);
    ConversationSummary[] ListConversations();
    Task<ICoreConversation> OpenConversation(string id, ConversationConfig? config, AgentListener listener);
    void DeleteConversation(string id);

    /// <summary>control 客户端内核自带的本机被控服务。Windows 上被控交给 lemon-galaxy-worker 系统服务，这里只用来关掉它。</summary>
    /// <returns>已关闭（或本来就关着）时为 true；内核还没给出被控服务时为 false，稍后再试</returns>
    bool DisableBuiltInHost();

    /// <summary>终端屏幕模型（真内核：vt100；模拟内核：界面自带的简易实现）</summary>
    ITermScreen CreateScreen(ushort rows, ushort cols, uint scrollback);
    /// <summary>H.264 软解（模拟内核：解析模拟画面的原始帧）</summary>
    IFrameDecoder CreateDecoder();
    void SetLocale(string coreLocale);
}

internal interface ICorePeer
{
    PeerInfo Info();
    PathInfo Path();
    bool IsClosed();
    void SetListener(PeerListener listener);
    void Disconnect();
    Task<ICoreTerminal> OpenTerminal(TerminalOptions options, TerminalSink sink);
    Task<TerminalSessionInfo[]> ListTerminals();
    Task KillTerminal(string sessionId);
    ICoreFiles Files();
    Task<ExecResult> Exec(ExecRequest request);
    Task<DisplayInfo[]> ListDisplays();
    Task<ICoreDisplay> OpenDisplay(DisplayOptions options, VideoSink sink);
    Task<ImageData> Screenshot(uint displayId, uint maxWidth);
    Task<ClipboardData?> ClipboardGet();
    Task ClipboardSet(ClipboardData data);
    Task<ICorePortForward> ForwardPort(ushort localPort, string remoteHost, ushort remotePort);
    Task WakeOnLan(string mac);
    /// <summary>隐私屏：被控端本地屏幕黑掉、本地键鼠失效；能力列表里有 "privacy" 才支持</summary>
    Task SetPrivacy(bool on);
    /// <summary>收听被控端的系统声音，内核用本机默认输出设备（WASAPI）播放；能力列表里有 "audio" 才支持。volume 0..1</summary>
    Task<ICoreAudio> StartAudio(float volume);
}

/// <summary>正在播放的远端声音</summary>
internal interface ICoreAudio
{
    void SetVolume(float volume);
    void SetMuted(bool muted);
    bool IsActive();
    /// <summary>停止收听并关闭输出设备（可重复调用）</summary>
    void Stop();
}

internal interface ICoreSsh
{
    SshInfo Info();
    bool IsClosed();
    Task<ICoreTerminal> OpenTerminal(TerminalOptions options, TerminalSink sink);
    ICoreFiles Files();
    Task<ExecResult> Exec(ExecRequest request);
    Task<ICorePortForward> ForwardPort(ushort localPort, string remoteHost, ushort remotePort);
    void Disconnect();
}

internal interface ICoreTerminal
{
    string SessionId();
    string Title();
    void Write(byte[] data);
    void Resize(ushort cols, ushort rows);
    void Detach();
    void Kill();
}

internal interface ICoreFiles
{
    Task<string> Home();
    Task<FileEntry[]> List(string path);
    Task<FileEntry> Stat(string path);
    Task<string> Realpath(string path);
    Task Mkdir(string path);
    Task Rename(string from, string to);
    Task Remove(string path, bool recursive);
    Task Chmod(string path, uint mode);
    Task<FileEntry[]> Search(string root, string pattern, bool content);
    Task<string> ReadText(string path, ulong maxBytes);
    Task WriteText(string path, string text);
    ICoreTransfer Download(string remote, string local, TransferListener listener);
    ICoreTransfer Upload(string local, string remote, TransferListener listener);
}

internal interface ICoreTransfer
{
    void Cancel();
    void Pause();
    void Resume();
}

internal interface ICoreDisplay
{
    void SendInput(InputEvent e);
    void RequestKeyframe();
    void SetBitrate(uint kbps);
    /// <summary>关画面；正在录像时内核会自动结束录像</summary>
    void Stop();
    /// <summary>开始录像到 path（分片 MP4，不重新编码）；withAudio 时把正在收听的声音一起录</summary>
    Task StartRecording(string path, bool withAudio);
    /// <summary>停止录像（没在录时报错）；被控端分辨率中途变化时会分成几个文件</summary>
    RecordingInfo StopRecording();
    bool IsRecording();
}

internal interface ICorePortForward
{
    ushort LocalPort();
    void Stop();
}

internal interface ICoreConversation
{
    string Id();
    string Title();
    void Send(string text);
    void Decide(string callId, ApprovalDecision decision);
    void Answer(string questionId, string answer);
    void ProvideSecret(string requestId, string secret);
    void SetMode(AiMode mode, uint? trustMinutes);
    void Stop();
    void SetForeground(bool foreground);
    AgentMessage[] Messages();
}

/// <summary>终端屏幕模型（线程安全：feed 可在内核线程调用，state 在界面线程调用）</summary>
internal interface ITermScreen
{
    void Feed(byte[] data);
    void Resize(ushort rows, ushort cols);
    TermScreenState State(uint scrollOffset);
    string Text();
    string TextBetween(ushort startRow, ushort startCol, ushort endRow, ushort endCol);
}

internal interface IFrameDecoder
{
    /// <summary>解码一帧 Annex-B；还没有可显示的画面时返回 null</summary>
    DecodedFrame? Decode(byte[] data);
}
