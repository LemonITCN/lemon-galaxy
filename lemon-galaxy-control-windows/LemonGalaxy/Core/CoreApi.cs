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

    /// <summary>
    /// 远程升级被控端（设备详情的「升级被控端」，不用先连远程）：立即返回；内核连上对方发请求，对方自己走完零交互升级，
    /// 内核接着等它以新版本重新上线（最多 10 分钟）。listener 在内核线程回调。
    /// </summary>
    void UpgradeDevice(string deviceId, UpgradeListener listener);

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
    /// <summary>新建对话：targets 可以为空（聊天式），引用过的设备用到时才连接</summary>
    Task<ICoreConversation> NewConversation(ConversationConfig config, AgentListener listener);
    /// <summary>最近的登录与连接记录（新的在前）</summary>
    Task<ActivityItem[]> AccountActivity(uint limit);
    ConversationSummary[] ListConversations();
    Task<ICoreConversation> OpenConversation(string id, ConversationConfig? config, AgentListener listener);
    void DeleteConversation(string id);

    // ---------- 本机 MCP 服务（docs/mcp.md 第 4 节）----------

    /// <summary>默认端口（内核常量，外部客户端的配置不用跟着变）</summary>
    ushort McpDefaultPort();
    /// <summary>开启（或按新设置重启）；listener 在任意内核线程回调。端口被占用等失败时抛错（同时 on_status 带 error）</summary>
    Task<McpStatus> McpStart(McpConfig config, McpListener listener);
    /// <summary>关闭服务：等批准的步骤一律拒绝，正在跑的命令停止</summary>
    void McpStop();
    /// <summary>没开时也能拿到令牌（从端点文件读，还没生成过时为空）</summary>
    McpStatus McpStatus();
    /// <summary>换新令牌；服务开着时随即用新令牌重启（外部客户端的智能体随之作废）</summary>
    Task<McpStatus> McpRegenerateToken();
    /// <summary>档位立即作用于所有外部客户端；服务没开时不生效（下次开启按 McpConfig）</summary>
    void McpSetMode(AiMode mode, uint? trustMinutes);
    void McpDecide(string clientKey, string callId, ApprovalDecision decision);
    /// <summary>回应 request_secret：内容直接送到设备，不经过外部智能体</summary>
    void McpProvideSecret(string clientKey, string requestId, string secret);
    /// <summary>拒绝这个客户端等批准的步骤、停止它正在跑的命令（之后的新调用照常）</summary>
    void McpStopClient(string clientKey);
    /// <summary>某个客户端最近的操作记录（只有工具卡片，没有时间）</summary>
    AgentMessage[] McpHistory(string clientKey);

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
    /// <summary>最多能在被控端加几块虚拟屏（能力列表 "virtual_display" + "virtual_display_max=N"）；不支持或没有控制权限时为 0</summary>
    uint VirtualDisplayMax();
    /// <summary>在被控端加一块虚拟屏（扩展桌面），返回它的屏幕信息（IsVirtual 为真），id 直接用于 OpenDisplay。
    /// 宽 640–7680、高 480–4320、刷新率 24–240（0 表示 60）。属于本会话，断开时被控端自动删掉；要控制权限</summary>
    Task<DisplayInfo> AddVirtualDisplay(uint width, uint height, uint refresh);
    /// <summary>删掉本会话加的虚拟屏（别的会话加的、被控端自动加的无头兜底屏都删不了）</summary>
    Task RemoveVirtualDisplay(uint displayId);
    /// <summary>复制粘贴文件，本机复制 → 远端粘贴（protocol.md 8.4.1）：只把本机文件 / 文件夹的清单放到被控端剪贴板，
    /// 对方粘贴时才经反向流来读（内核自己应答）。能力列表要有 "clipboard_offer"；需要剪贴板与文件权限</summary>
    Task<ClipOfferInfo> ClipboardOfferFiles(string[] localPaths);
    /// <summary>复制粘贴文件，远端复制 → 本机粘贴：把远端剪贴板里的文件（ClipboardGet 的 Files）展开成清单，不下载内容；
    /// 本机粘贴时按项 Open 读。老被控端也行；需要文件权限</summary>
    Task<ICoreRemoteClipOffer> ClipboardRemoteOffer(string[] remotePaths);
    /// <summary>复制粘贴文件（本机 → 没有 "clipboard_offer" 的老被控端）：本机文件 / 文件夹整批传到被控端暂存区，再把被控端剪贴板设成它们。
    /// 超过 4 GB 或 1 万个文件时报错；需要剪贴板与文件权限</summary>
    ICoreClipboardTransfer ClipboardPushFiles(string[] localPaths, TransferListener listener);
}

/// <summary>一次复制粘贴文件的传输（老被控端的整批上传）</summary>
internal interface ICoreClipboardTransfer
{
    void Cancel();
    /// <summary>完成后被控端暂存区里的顶层路径</summary>
    string[] Paths();
}

/// <summary>远端剪贴板里文件的清单（粘贴时才读内容）。Dispose 后不能再 Open，已打开的读取器不受影响</summary>
internal interface ICoreRemoteClipOffer : IDisposable
{
    ClipOfferInfo Info();
    /// <summary>打开第 index 项（文件）从 offset 读到末尾。会阻塞到对方应答（最多约 20 秒）：只在后台线程调用</summary>
    ICoreClipReader Open(uint index, ulong offset);
}

/// <summary>剪贴板文件的读取器：Read 会阻塞，只在后台线程调用</summary>
internal interface ICoreClipReader
{
    /// <summary>文件现在的大小（复制后文件又变了时与清单里的不同）</summary>
    ulong Size();
    /// <summary>读最多 max 字节；返回空表示读完了。断线、对方取消时抛错</summary>
    byte[] Read(uint max);
    /// <summary>不读了：取消并释放（可重复调用）</summary>
    void Close();
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
    /// <summary>带引用发送：设备与主机本对话内可直接使用；文件、目录、片段、端口映射、连接记录展开后随消息给模型</summary>
    void SendWithRefs(string text, ContextRef[] refs);
    /// <summary>中途换模型（上下文保留）；模型列表还没加载或未开通该模型时抛错</summary>
    void SetModel(ulong providerId, string modelId);
    ConversationModel Model();
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
