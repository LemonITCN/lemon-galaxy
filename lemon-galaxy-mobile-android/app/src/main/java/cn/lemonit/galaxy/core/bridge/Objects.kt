@file:Suppress("unused")

package cn.lemonit.galaxy.core.bridge

/*
 * docs/ffi.md 的 Kotlin 对照（对象与回调部分）。
 *
 * uniffi 给每个 object 生成 `XxxInterface` 接口和实现类 `Xxx`；界面只依赖 `XxxInterface`。
 * async fn → suspend fun；错误统一抛 GalaxyException。
 * 回调（callback）由界面层实现，内核可能在任意线程调用，界面自己切回主线程。
 *
 * 第 5 节 HostService（本机被控）安卓不需要，GalaxyClient.host() 在这里省略。
 */

/** 顶层函数与 GalaxyClient 构造器（ffi.md 第 1、2 节）。由 CoreProvider 选择实现。 */
interface GalaxyCore {
    fun coreVersion(): String
    fun coreInit(config: CoreConfig)

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
    suspend fun changePassword(oldPassword: String, newPassword: String)
    suspend fun logout()
    fun restore(): Boolean

    fun devices(): List<DeviceInfo>
    suspend fun refreshDevices()
    suspend fun removeDevice(deviceId: String)
    fun vault(): List<VaultEntry>
    suspend fun vaultSave(entry: VaultEntry): VaultEntry
    suspend fun vaultDelete(id: String)

    suspend fun connect(deviceId: String): PeerSessionInterface
    suspend fun connectAssist(deviceCode: String, verificationCode: String): PeerSessionInterface
    suspend fun connectSsh(hostId: String, secretPrompt: SecretPrompt): SshSessionInterface

    suspend fun aiModels(): AiModels
    fun newConversation(config: ConversationConfig, listener: AgentListener): AgentConversationInterface
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
    fun close()

    suspend fun openTerminal(options: TerminalOptions, sink: TerminalSink): TerminalChannelInterface
    suspend fun listTerminals(): List<TerminalSessionInfo>
    suspend fun killTerminal(sessionId: String)

    fun files(): FileClientInterface

    suspend fun exec(request: ExecRequest): ExecResult

    suspend fun listDisplays(): List<DisplayInfo>
    suspend fun openDisplay(options: DisplayOptions, sink: VideoSink): DisplayChannelInterface
    suspend fun screenshot(displayId: UInt, maxWidth: UInt): ImageData

    suspend fun clipboardGet(): ClipboardData
    suspend fun clipboardSet(data: ClipboardData)

    suspend fun forwardPort(localPort: UShort, remoteHost: String, remotePort: UShort): PortForwardInterface
}

interface PeerListener {
    fun onPathChanged(path: PathInfo)
    fun onClosed(reason: String)
}

/** ffi.md 未定义 PortForward 对象：这里按最小需要补充。 */
interface PortForwardInterface {
    fun localPort(): UShort
    fun close()
}

// ---------- 3.1 终端 ----------

interface TerminalSink {
    fun onOutput(data: ByteArray)
    fun onExit(code: Int?)
}

interface TerminalChannelInterface {
    fun sessionId(): String
    fun write(data: ByteArray)
    fun resize(cols: UShort, rows: UShort)

    /** 断开但被控端保留会话。 */
    fun detach()
    fun kill()
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
    suspend fun send(text: String)

    /** 回应确认卡片。 */
    fun decide(callId: String, decision: ApprovalDecision)

    /** 回应 ask_user。 */
    fun answer(questionId: String, answer: String)

    /** 回应 request_secret，内容不进模型上下文。 */
    fun provideSecret(requestId: String, secret: String)
    fun setMode(mode: AiMode, trustMinutes: UInt?)
    fun stop()

    /** 历史（恢复界面用）。 */
    fun messages(): List<AgentMessage>
}

interface AgentListener {
    fun onEvent(event: AgentEvent)
}

// ---------- 6. SSH 直连 ----------

/** 键盘交互认证、私钥口令、首次连接确认主机指纹。内核在后台线程同步等待返回。 */
interface SecretPrompt {
    fun ask(prompt: String, echo: Boolean): String?
}

interface SshSessionInterface {
    suspend fun openTerminal(cols: UShort, rows: UShort, sink: TerminalSink): TerminalChannelInterface
    fun files(): FileClientInterface
    suspend fun exec(request: ExecRequest): ExecResult
    suspend fun forwardLocal(localPort: UShort, remoteHost: String, remotePort: UShort): PortForwardInterface
    fun close()
}
