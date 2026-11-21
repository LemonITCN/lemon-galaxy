@file:Suppress("unused")

package cn.lemonit.galaxy.core.bridge

/*
 * docs/ffi.md 的 Kotlin 对照（对象与回调部分）。
 *
 * 与 uniffi.galaxy_ffi 生成的 `XxxInterface` 同名同签名；界面只依赖这里的接口。
 * async fn → suspend fun（内核在自己的线程池执行）；错误统一抛 GalaxyException。
 * 回调（callback）由界面层实现，内核可能在任意线程调用，界面自己切回主线程。
 *
 * HostService（本机被控）安卓是 mobile 类型用不到，GalaxyClient.host() 在这里省略。
 */

/** 顶层函数与 GalaxyClient 构造器（ffi.md 第 1、2 节）。由 CoreProvider 选择实现。 */
interface GalaxyCore {
    fun coreVersion(): String
    fun coreInit(config: CoreConfig)

    /** 运行时切换界面语言（影响内核返回的错误文字）。 */
    fun coreSetLocale(locale: String)

    /** 对应 `GalaxyClient::new(store, listener)`。 */
    fun newGalaxyClient(store: SecureStore, listener: ClientListener): GalaxyClientInterface
}

// ---------- 1.1 平台安全存储（界面层实现） ----------

interface SecureStore {
    fun get(key: String): ByteArray?
    fun set(key: String, value: ByteArray)
    fun remove(key: String)
}

// ---------- 2. GalaxyClient ----------

interface GalaxyClientInterface {
    fun state(): ClientState
    fun currentUser(): UserInfo?
    fun server(): ServerInfo?

    /** server 为后台给出的完整地址 "host:port#sha256-指纹"。 */
    suspend fun login(server: String, username: String, password: String, totp: String?): LoginResult
    suspend fun completeFirstLogin(newPassword: String)
    suspend fun changePassword(oldPassword: String, newPassword: String, totp: String?)
    suspend fun logout()
    fun restore(): Boolean
    fun deviceId(): String

    suspend fun totpSetup(): TotpSetup
    suspend fun totpVerify(code: String)
    suspend fun totpDisable(code: String)

    /** 设备列表页打开（持有组播锁）时 true：内核每 10 秒做一次局域网查询。 */
    fun setLanDiscovery(active: Boolean)

    fun devices(): List<DeviceInfo>
    suspend fun refreshDevices()
    suspend fun removeDevice(deviceId: String)
    fun vault(): List<VaultEntry>
    suspend fun vaultSave(entry: VaultEntry): VaultEntry
    suspend fun vaultDelete(id: String)

    suspend fun connect(deviceId: String): PeerSessionInterface
    suspend fun connectAssist(deviceCode: String, verificationCode: String): PeerSessionInterface

    /** 连接加密库里的 ssh_host 条目（ffi.md 第 3.1 节）。需要密码、口令或确认主机密钥时经 [prompt] 问用户。 */
    suspend fun connectSsh(hostId: String, prompt: SecretPrompt): SshSessionInterface

    /** 加密库里的 rdp_host（ffi.md 3.1.1）：设了经由设备时在本机开端口转发穿过那台设备。 */
    suspend fun openRdp(hostId: String): RdpTarget

    /** 关掉 openRdp 开的端口转发。 */
    fun closeRdp(forwardId: String)

    /** 向服务器查询新版本（ffi.md 5.0），返回前已验发布签名；没有新版本返回 null。 */
    suspend fun checkUpdate(component: String, platform: String, arch: String, current: String): UpdateInfo?

    /** 下载新版本到本地文件并核对 SHA-256，进度与结果经 [listener]。 */
    fun downloadUpdate(info: UpdateInfo, dest: String, listener: TransferListener)

    suspend fun aiModels(): AiModels
    suspend fun newConversation(config: ConversationConfig, listener: AgentListener): AgentConversationInterface

    /** 本机保存的对话，按更新时间倒序。 */
    fun listConversations(): List<ConversationSummary>

    /** 重新打开历史对话（模型、目标、档位由界面给出）。 */
    suspend fun openConversation(id: String, config: ConversationConfig, listener: AgentListener): AgentConversationInterface
    fun deleteConversation(id: String)
}

interface ClientListener {
    fun onState(state: ClientState)
    fun onDevicesChanged()
    fun onVaultChanged()
    fun onSessionRevoked()
}

// ---------- 3. PeerSession ----------

interface PeerSessionInterface {
    fun info(): PeerInfo
    fun path(): PathInfo
    fun setListener(listener: PeerListener)
    fun disconnect()
    fun isClosed(): Boolean

    suspend fun openTerminal(options: TerminalOptions, sink: TerminalSink): TerminalChannelInterface
    suspend fun listTerminals(): List<TerminalSessionInfo>
    suspend fun killTerminal(sessionId: String)

    fun files(): FileClientInterface

    suspend fun exec(request: ExecRequest): ExecResult

    suspend fun listDisplays(): List<DisplayInfo>
    suspend fun openDisplay(options: DisplayOptions, sink: VideoSink): DisplayChannelInterface
    suspend fun screenshot(displayId: UInt, maxWidth: UInt): ImageData

    suspend fun clipboardGet(): ClipboardData?
    suspend fun clipboardSet(data: ClipboardData)

    suspend fun forwardPort(localPort: UShort, remoteHost: String, remotePort: UShort): PortForwardInterface
    suspend fun wakeOnLan(mac: String)

    /** 隐私屏：被控端本地屏幕黑掉、本地键鼠失效；本会话结束时自动关闭。能力列表里有 "privacy" 才支持。 */
    suspend fun setPrivacy(on: Boolean)
}

interface PeerListener {
    fun onPathChanged(path: PathInfo)
    fun onClosed(reason: String)
}

interface PortForwardInterface {
    fun localPort(): UShort
    fun stop()
}

// ---------- 3.1 终端 ----------

interface TerminalSink {
    fun onOutput(data: ByteArray)
    fun onExit(code: Int?)
}

interface TerminalChannelInterface {
    fun sessionId(): String
    fun title(): String
    fun write(data: ByteArray)
    fun resize(cols: UShort, rows: UShort)

    /** 断开但被控端保留会话。 */
    fun detach()
    fun kill()
}

// ---------- 3.1 SSH 直连 ----------

/** SSH 直连时向用户要东西。内核在自己的线程上同步调用，界面可以阻塞等用户回答（不能阻塞主线程）。 */
interface SecretPrompt {
    /** 密码、私钥口令、键盘交互的回答；返回 null 表示取消。[echo] 为 false 时输入框应遮挡。 */
    fun ask(prompt: String, echo: Boolean): String?

    /** 首次连接或主机密钥变化时确认。[changed] 为 true 表示与记录的不一致（可能被冒充），界面应醒目警告。 */
    fun confirmHostKey(host: String, port: UShort, keyType: String, fingerprint: String, changed: Boolean): Boolean
}

/** SSH 直连会话：终端、文件（SFTP）、命令、端口转发。终端与文件接口与设备会话相同。 */
interface SshSessionInterface {
    fun info(): SshInfo
    fun isClosed(): Boolean

    /** SSH 没有断线继承：detach 与 kill 都会结束这个终端。 */
    suspend fun openTerminal(options: TerminalOptions, sink: TerminalSink): TerminalChannelInterface
    fun files(): FileClientInterface
    suspend fun exec(request: ExecRequest): ExecResult

    /** 本地端口转发（本机 127.0.0.1 监听；localPort 为 0 时自动选）。 */
    suspend fun forwardPort(localPort: UShort, remoteHost: String, remotePort: UShort): PortForwardInterface
    fun disconnect()
}

// ---------- 3.2 文件 ----------

interface FileClientInterface {
    suspend fun home(): String
    suspend fun list(path: String): List<FileEntry>
    suspend fun stat(path: String): FileEntry
    suspend fun mkdir(path: String)
    suspend fun rename(from: String, to: String)
    suspend fun remove(path: String, recursive: Boolean)
    suspend fun chmod(path: String, mode: UInt)
    suspend fun readText(path: String, maxBytes: ULong): String
    suspend fun writeText(path: String, text: String)
    suspend fun realpath(path: String): String
    suspend fun search(root: String, pattern: String, content: Boolean): List<FileEntry>

    /** 支持断点续传。local 为本机文件路径。 */
    fun download(remote: String, local: String, listener: TransferListener): TransferInterface
    fun upload(local: String, remote: String, listener: TransferListener): TransferInterface
}

interface TransferListener {
    fun onProgress(done: ULong, total: ULong)
    fun onFinished(error: String?)
}

interface TransferInterface {
    fun cancel()
    fun pause()
    fun resume()
}

// ---------- 3.3 远程画面 ----------

interface VideoSink {
    fun onConfig(codec: VideoCodec, width: UInt, height: UInt)

    /** Annex B 码流。 */
    fun onFrame(data: ByteArray, ptsUs: ULong, keyframe: Boolean)
    fun onCursor(x: Int, y: Int, visible: Boolean)
    fun onClosed(reason: String)
}

interface DisplayChannelInterface {
    fun sendInput(event: InputEvent)
    fun requestKeyframe()
    fun setBitrate(kbps: UInt)
    fun stop()
}

// ---------- 4. AI 智能体 ----------

interface AgentConversationInterface {
    fun id(): String
    fun title(): String

    /** 发送一条消息；任务在内核后台运行，进度经监听器回调。 */
    fun send(text: String)

    /** 回应确认卡片。 */
    fun decide(callId: String, decision: ApprovalDecision)

    /** 回应 ask_user。 */
    fun answer(questionId: String, answer: String)

    /** 回应 request_secret，内容不进模型上下文。 */
    fun provideSecret(requestId: String, secret: String)
    fun setMode(mode: AiMode, trustMinutes: UInt?)
    fun stop()

    /** 切到后台时 false：内核停止正在进行的任务（ai-agent.md 第 1 节）。 */
    fun setForeground(foreground: Boolean)

    /** 历史（恢复界面用）。 */
    fun messages(): List<AgentMessage>
}

interface AgentListener {
    fun onEvent(event: AgentEvent)
}
