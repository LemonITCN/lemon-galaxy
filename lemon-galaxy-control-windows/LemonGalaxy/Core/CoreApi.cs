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

    Task<ICorePeer> Connect(string deviceId);
    Task<ICorePeer> ConnectAssist(string deviceCode, string verificationCode);
    Task<ICoreSsh> ConnectSsh(string hostId, SecretPrompt prompt);
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
    void Stop();
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
