package cn.lemonit.galaxy.core.rust

import android.content.Context
import cn.lemonit.galaxy.core.bridge.ActivityItem
import cn.lemonit.galaxy.core.bridge.AgentConversationInterface
import cn.lemonit.galaxy.core.bridge.ContextRef
import cn.lemonit.galaxy.core.bridge.ConversationModel
import cn.lemonit.galaxy.core.bridge.AgentListener
import cn.lemonit.galaxy.core.bridge.AgentMessage
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.AiModels
import cn.lemonit.galaxy.core.bridge.AudioPlaybackInterface
import cn.lemonit.galaxy.core.bridge.ApprovalDecision
import cn.lemonit.galaxy.core.bridge.ClientListener
import cn.lemonit.galaxy.core.bridge.ClientState
import cn.lemonit.galaxy.core.bridge.ClipboardData
import cn.lemonit.galaxy.core.bridge.ConversationConfig
import cn.lemonit.galaxy.core.bridge.ConversationSummary
import cn.lemonit.galaxy.core.bridge.CoreConfig
import cn.lemonit.galaxy.core.bridge.DeviceInfo
import cn.lemonit.galaxy.core.bridge.DisplayChannelInterface
import cn.lemonit.galaxy.core.bridge.DisplayInfo
import cn.lemonit.galaxy.core.bridge.DisplayOptions
import cn.lemonit.galaxy.core.bridge.ExecRequest
import cn.lemonit.galaxy.core.bridge.ExecResult
import cn.lemonit.galaxy.core.bridge.FileClientInterface
import cn.lemonit.galaxy.core.bridge.FileEntry
import cn.lemonit.galaxy.core.bridge.GalaxyClientInterface
import cn.lemonit.galaxy.core.bridge.GalaxyCore
import cn.lemonit.galaxy.core.bridge.ImageData
import cn.lemonit.galaxy.core.bridge.InputEvent
import cn.lemonit.galaxy.core.bridge.LoginResult
import cn.lemonit.galaxy.core.bridge.PathInfo
import cn.lemonit.galaxy.core.bridge.PeerInfo
import cn.lemonit.galaxy.core.bridge.PeerListener
import cn.lemonit.galaxy.core.bridge.PeerSessionInterface
import cn.lemonit.galaxy.core.bridge.PortForwardInterface
import cn.lemonit.galaxy.core.bridge.RdpTarget
import cn.lemonit.galaxy.core.bridge.RecordingInfo
import cn.lemonit.galaxy.core.bridge.SecretPrompt
import cn.lemonit.galaxy.core.bridge.SecureStore
import cn.lemonit.galaxy.core.bridge.ServerInfo
import cn.lemonit.galaxy.core.bridge.SshInfo
import cn.lemonit.galaxy.core.bridge.SshSessionInterface
import cn.lemonit.galaxy.core.bridge.TerminalChannelInterface
import cn.lemonit.galaxy.core.bridge.TerminalOptions
import cn.lemonit.galaxy.core.bridge.TerminalSessionInfo
import cn.lemonit.galaxy.core.bridge.TerminalSink
import cn.lemonit.galaxy.core.bridge.TotpSetup
import cn.lemonit.galaxy.core.bridge.TransferInterface
import cn.lemonit.galaxy.core.bridge.TransferListener
import cn.lemonit.galaxy.core.bridge.UpdateInfo
import cn.lemonit.galaxy.core.bridge.UserInfo
import cn.lemonit.galaxy.core.bridge.VaultEntry
import cn.lemonit.galaxy.core.bridge.VideoSink
import uniffi.galaxy_ffi.AgentConversation as FAgentConversation
import uniffi.galaxy_ffi.AgentEvent as FAgentEvent
import uniffi.galaxy_ffi.AgentListener as FAgentListener
import uniffi.galaxy_ffi.AudioPlayback as FAudioPlayback
import uniffi.galaxy_ffi.ClientListener as FClientListener
import uniffi.galaxy_ffi.ClientState as FClientState
import uniffi.galaxy_ffi.DisplayChannel as FDisplayChannel
import uniffi.galaxy_ffi.FileClient as FFileClient
import uniffi.galaxy_ffi.GalaxyClient as FGalaxyClient
import uniffi.galaxy_ffi.PathInfo as FPathInfo
import uniffi.galaxy_ffi.PeerListener as FPeerListener
import uniffi.galaxy_ffi.PeerSession as FPeerSession
import uniffi.galaxy_ffi.PortForward as FPortForward
import uniffi.galaxy_ffi.SecretPrompt as FSecretPrompt
import uniffi.galaxy_ffi.SecureStore as FSecureStore
import uniffi.galaxy_ffi.SshSession as FSshSession
import uniffi.galaxy_ffi.TerminalChannel as FTerminalChannel
import uniffi.galaxy_ffi.TerminalSink as FTerminalSink
import uniffi.galaxy_ffi.Transfer as FTransfer
import uniffi.galaxy_ffi.TransferListener as FTransferListener
import uniffi.galaxy_ffi.VideoCodec as FVideoCodec
import uniffi.galaxy_ffi.VideoSink as FVideoSink

/**
 * 真内核（galaxy-ffi，uniffi 0.32 生成的 uniffi.galaxy_ffi.*）到界面接口（core.bridge）的适配层。
 * CoreProvider 在 BuildConfig.USE_MOCK_CORE = false 时通过反射创建它：安卓上用带 Context 的构造（先登记 JavaVM 与 Context，
 * 内核的声音输出要用），主机 JVM 上的冒烟测试用无参构造。
 *
 * 约定：
 *  - 内核的 suspend 函数在内核自己的线程池执行，这里直接 await；
 *  - 内核回调可能在任意内核线程触发，这里原样转给界面层的回调（界面层自己切回主线程）；
 *  - 内核抛出的 uniffi GalaxyException 统一换成 core.bridge.GalaxyException（message 已本地化）。
 */
class UniffiCore() : GalaxyCore {
    /** 安卓上用：在调用方 coreInit 之前给内核登记 JavaVM 与 Application Context（见 [NativeInit]）。 */
    constructor(context: Context) : this() {
        NativeInit.register(context)
    }

    override fun coreVersion(): String = uniffi.galaxy_ffi.coreVersion()

    override fun coreInit(config: CoreConfig) = uniffi.galaxy_ffi.coreInit(config.toFfi())

    override fun coreSetLocale(locale: String) = uniffi.galaxy_ffi.coreSetLocale(locale)

    override fun newGalaxyClient(store: SecureStore, listener: ClientListener): GalaxyClientInterface {
        val s = object : FSecureStore {
            override fun get(key: String): ByteArray? = store.get(key)
            override fun set(key: String, value: ByteArray) = store.set(key, value)
            override fun remove(key: String) = store.remove(key)
        }
        val l = object : FClientListener {
            override fun onState(state: FClientState) = listener.onState(state.toBridge())
            override fun onDevicesChanged() = listener.onDevicesChanged()
            override fun onVaultChanged() = listener.onVaultChanged()
            override fun onSessionRevoked() = listener.onSessionRevoked()
        }
        return UniffiClient(FGalaxyClient(s, l))
    }
}

internal class UniffiClient(private val c: FGalaxyClient) : GalaxyClientInterface {
    override fun state(): ClientState = c.state().toBridge()
    override fun currentUser(): UserInfo? = c.currentUser()?.toBridge()
    override fun server(): ServerInfo? = c.server()?.toBridge()

    override suspend fun login(server: String, username: String, password: String, totp: String?): LoginResult =
        ffi { c.login(server, username, password, totp).toBridge() }

    override suspend fun completeFirstLogin(newPassword: String) = ffi { c.completeFirstLogin(newPassword) }
    override suspend fun changePassword(oldPassword: String, newPassword: String, totp: String?) = ffi { c.changePassword(oldPassword, newPassword, totp) }
    override suspend fun logout() = ffi { c.logout() }
    override fun restore(): Boolean = ffi { c.restore() }
    override fun deviceId(): String = c.deviceId()

    override suspend fun totpSetup(): TotpSetup = ffi { c.totpSetup().toBridge() }
    override suspend fun totpVerify(code: String) = ffi { c.totpVerify(code) }
    override suspend fun totpDisable(code: String) = ffi { c.totpDisable(code) }
    override fun setLanDiscovery(active: Boolean) = c.setLanDiscovery(active)

    override fun devices(): List<DeviceInfo> = c.devices().map { it.toBridge() }
    override suspend fun refreshDevices() = ffi { c.refreshDevices() }
    override suspend fun removeDevice(deviceId: String) = ffi { c.removeDevice(deviceId) }
    override fun vault(): List<VaultEntry> = c.vault().map { it.toBridge() }
    override suspend fun vaultSave(entry: VaultEntry): VaultEntry = ffi { c.vaultSave(entry.toFfi()).toBridge() }
    override suspend fun vaultDelete(id: String) = ffi { c.vaultDelete(id) }

    override fun canWake(deviceId: String): Boolean = c.canWake(deviceId)
    override suspend fun wakeDevice(deviceId: String): List<String> = ffi { c.wakeDevice(deviceId) }

    override suspend fun connect(deviceId: String): PeerSessionInterface = ffi { UniffiPeer(c.connect(deviceId)) }
    override suspend fun connectAssist(deviceCode: String, verificationCode: String): PeerSessionInterface =
        ffi { UniffiPeer(c.connectAssist(deviceCode, verificationCode)) }

    override suspend fun connectSsh(hostId: String, prompt: SecretPrompt): SshSessionInterface {
        // 内核在自己的线程上同步调用，原样转给界面层（界面层阻塞等用户回答）
        val ui = prompt
        val p = object : FSecretPrompt {
            override fun ask(prompt: String, echo: Boolean): String? = ui.ask(prompt, echo)
            override fun confirmHostKey(host: String, port: UShort, keyType: String, fingerprint: String, changed: Boolean): Boolean =
                ui.confirmHostKey(host, port, keyType, fingerprint, changed)
        }
        return ffi { UniffiSsh(c.connectSsh(hostId, p)) }
    }

    override suspend fun openRdp(hostId: String): RdpTarget = ffi { c.openRdp(hostId).toBridge() }
    override fun closeRdp(forwardId: String) = c.closeRdp(forwardId)

    override suspend fun checkUpdate(component: String, platform: String, arch: String, current: String): UpdateInfo? =
        ffi { c.checkUpdate(component, platform, arch, current)?.toBridge() }

    override fun downloadUpdate(info: UpdateInfo, dest: String, listener: TransferListener) =
        c.downloadUpdate(
            info.toFfi(), dest,
            object : FTransferListener {
                override fun onProgress(done: ULong, total: ULong) = listener.onProgress(done, total)
                override fun onFinished(error: String?) = listener.onFinished(error)
            },
        )

    override suspend fun aiModels(): AiModels = ffi { c.aiModels().toBridge() }

    override suspend fun newConversation(config: ConversationConfig, listener: AgentListener): AgentConversationInterface =
        ffi { UniffiConversation(c.newConversation(config.toFfi(), agentListener(listener))) }

    override suspend fun accountActivity(limit: UInt): List<ActivityItem> = ffi { c.accountActivity(limit).map { it.toBridge() } }

    override fun listConversations(): List<ConversationSummary> = c.listConversations().map { it.toBridge() }

    override suspend fun openConversation(id: String, config: ConversationConfig?, listener: AgentListener): AgentConversationInterface =
        ffi { UniffiConversation(c.openConversation(id, config?.toFfi(), agentListener(listener))) }

    override fun deleteConversation(id: String) = c.deleteConversation(id)

    private fun agentListener(l: AgentListener) = object : FAgentListener {
        override fun onEvent(event: FAgentEvent) = l.onEvent(event.toBridge())
    }
}

internal class UniffiPeer(private val p: FPeerSession) : PeerSessionInterface {
    override fun info(): PeerInfo = p.info().toBridge()
    override fun path(): PathInfo = p.path().toBridge()

    override fun setListener(listener: PeerListener) = p.setListener(object : FPeerListener {
        override fun onPathChanged(path: FPathInfo) = listener.onPathChanged(path.toBridge())
        override fun onClosed(reason: String) = listener.onClosed(reason)
    })

    override fun disconnect() = p.disconnect()
    override fun isClosed(): Boolean = p.isClosed()

    override suspend fun openTerminal(options: TerminalOptions, sink: TerminalSink): TerminalChannelInterface = ffi {
        UniffiTerminal(
            p.openTerminal(
                options.toFfi(),
                object : FTerminalSink {
                    override fun onOutput(data: ByteArray) = sink.onOutput(data)
                    override fun onExit(code: Int?) = sink.onExit(code)
                },
            ),
        )
    }

    override suspend fun listTerminals(): List<TerminalSessionInfo> = ffi { p.listTerminals().map { it.toBridge() } }
    override suspend fun killTerminal(sessionId: String) = ffi { p.killTerminal(sessionId) }
    override fun files(): FileClientInterface = UniffiFiles(p.files())
    override suspend fun exec(request: ExecRequest): ExecResult = ffi { p.exec(request.toFfi()).toBridge() }
    override suspend fun listDisplays(): List<DisplayInfo> = ffi { p.listDisplays().map { it.toBridge() } }

    override suspend fun openDisplay(options: DisplayOptions, sink: VideoSink): DisplayChannelInterface = ffi {
        UniffiDisplay(
            p.openDisplay(
                options.toFfi(),
                object : FVideoSink {
                    override fun onConfig(codec: FVideoCodec, width: UInt, height: UInt) = sink.onConfig(codec.toBridge(), width, height)
                    override fun onFrame(data: ByteArray, ptsUs: ULong, keyframe: Boolean) = sink.onFrame(data, ptsUs, keyframe)
                    override fun onCursor(x: Int, y: Int, visible: Boolean) = sink.onCursor(x, y, visible)
                    override fun onClosed(reason: String) = sink.onClosed(reason)
                },
            ),
        )
    }

    override suspend fun screenshot(displayId: UInt, maxWidth: UInt): ImageData = ffi { p.screenshot(displayId, maxWidth).toBridge() }
    override fun virtualDisplayMax(): UInt = p.virtualDisplayMax()

    override suspend fun addVirtualDisplay(width: UInt, height: UInt, refresh: UInt): DisplayInfo =
        ffi { p.addVirtualDisplay(width, height, refresh).toBridge() }

    override suspend fun removeVirtualDisplay(displayId: UInt) = ffi { p.removeVirtualDisplay(displayId) }
    override suspend fun clipboardGet(): ClipboardData? = ffi { p.clipboardGet()?.toBridge() }
    override suspend fun clipboardSet(data: ClipboardData) = ffi { p.clipboardSet(data.toFfi()) }

    override suspend fun forwardPort(localPort: UShort, remoteHost: String, remotePort: UShort): PortForwardInterface =
        ffi { UniffiPortForward(p.forwardPort(localPort, remoteHost, remotePort)) }

    override suspend fun wakeOnLan(mac: String) = ffi { p.wakeOnLan(mac) }
    override suspend fun setPrivacy(on: Boolean) = ffi { p.setPrivacy(on) }
    override suspend fun startAudio(volume: Float): AudioPlaybackInterface = ffi { UniffiAudio(p.startAudio(volume)) }
}

/** 停止后立即释放内核对象（不等 GC），之后的调用都是空操作。 */
internal class UniffiAudio(private val a: FAudioPlayback) : AudioPlaybackInterface {
    private var closed = false

    @Synchronized override fun setVolume(volume: Float) {
        if (!closed) a.setVolume(volume)
    }

    @Synchronized override fun setMuted(muted: Boolean) {
        if (!closed) a.setMuted(muted)
    }

    @Synchronized override fun isActive(): Boolean = !closed && a.isActive()

    @Synchronized override fun stop() {
        if (closed) return
        closed = true
        a.stop()
        a.close()
    }
}

internal class UniffiSsh(private val s: FSshSession) : SshSessionInterface {
    override fun info(): SshInfo = s.info().toBridge()
    override fun isClosed(): Boolean = s.isClosed()

    override suspend fun openTerminal(options: TerminalOptions, sink: TerminalSink): TerminalChannelInterface = ffi {
        UniffiTerminal(
            s.openTerminal(
                options.toFfi(),
                object : FTerminalSink {
                    override fun onOutput(data: ByteArray) = sink.onOutput(data)
                    override fun onExit(code: Int?) = sink.onExit(code)
                },
            ),
        )
    }

    override fun files(): FileClientInterface = UniffiFiles(s.files())
    override suspend fun exec(request: ExecRequest): ExecResult = ffi { s.exec(request.toFfi()).toBridge() }

    override suspend fun forwardPort(localPort: UShort, remoteHost: String, remotePort: UShort): PortForwardInterface =
        ffi { UniffiPortForward(s.forwardPort(localPort, remoteHost, remotePort)) }

    override fun disconnect() = s.disconnect()
}

internal class UniffiTerminal(private val t: FTerminalChannel) : TerminalChannelInterface {
    override fun sessionId(): String = t.sessionId()
    override fun title(): String = t.title()
    override fun write(data: ByteArray) = t.write(data)
    override fun resize(cols: UShort, rows: UShort) = t.resize(cols, rows)
    override fun detach() = t.detach()
    override fun kill() = t.kill()
}

internal class UniffiFiles(private val f: FFileClient) : FileClientInterface {
    override suspend fun home(): String = ffi { f.home() }
    override suspend fun list(path: String): List<FileEntry> = ffi { f.list(path).map { it.toBridge() } }
    override suspend fun stat(path: String): FileEntry = ffi { f.stat(path).toBridge() }
    override suspend fun mkdir(path: String) = ffi { f.mkdir(path) }
    override suspend fun rename(from: String, to: String) = ffi { f.rename(from, to) }
    override suspend fun remove(path: String, recursive: Boolean) = ffi { f.remove(path, recursive) }
    override suspend fun chmod(path: String, mode: UInt) = ffi { f.chmod(path, mode) }
    override suspend fun readText(path: String, maxBytes: ULong): String = ffi { f.readText(path, maxBytes) }
    override suspend fun writeText(path: String, text: String) = ffi { f.writeText(path, text) }
    override suspend fun realpath(path: String): String = ffi { f.realpath(path) }
    override suspend fun search(root: String, pattern: String, content: Boolean): List<FileEntry> = ffi { f.search(root, pattern, content).map { it.toBridge() } }

    override fun download(remote: String, local: String, listener: TransferListener): TransferInterface =
        UniffiTransfer(ffi { f.download(remote, local, transferListener(listener)) })

    override fun upload(local: String, remote: String, listener: TransferListener): TransferInterface =
        UniffiTransfer(ffi { f.upload(local, remote, transferListener(listener)) })

    private fun transferListener(l: TransferListener) = object : FTransferListener {
        override fun onProgress(done: ULong, total: ULong) = l.onProgress(done, total)
        override fun onFinished(error: String?) = l.onFinished(error)
    }
}

internal class UniffiTransfer(private val t: FTransfer) : TransferInterface {
    override fun cancel() = t.cancel()
    override fun pause() = t.pause()
    override fun resume() = t.resume()
}

internal class UniffiDisplay(private val d: FDisplayChannel) : DisplayChannelInterface {
    override fun sendInput(event: InputEvent) = d.sendInput(event.toFfi())
    override fun requestKeyframe() = d.requestKeyframe()
    override fun setBitrate(kbps: UInt) = d.setBitrate(kbps)
    override fun setClientCursor(on: Boolean) = d.setClientCursor(on)
    override fun stop() = d.stop()
    override suspend fun startRecording(path: String, withAudio: Boolean) = ffi { d.startRecording(path, withAudio) }
    override fun stopRecording(): RecordingInfo = ffi { d.stopRecording().toBridge() }
    override fun isRecording(): Boolean = d.isRecording()
}

internal class UniffiPortForward(private val p: FPortForward) : PortForwardInterface {
    override fun localPort(): UShort = p.localPort()
    override fun stop() = p.stop()
}

internal class UniffiConversation(private val a: FAgentConversation) : AgentConversationInterface {
    override fun id(): String = a.id()
    override fun title(): String = a.title()
    override fun send(text: String) = a.send(text)
    override fun sendWithRefs(text: String, refs: List<ContextRef>) = a.sendWithRefs(text, refs.map { it.toFfi() })
    override fun setModel(providerId: ULong, modelId: String) = ffi { a.setModel(providerId, modelId) }
    override fun model(): ConversationModel = a.model().toBridge()
    override fun decide(callId: String, decision: ApprovalDecision) = a.decide(callId, decision.toFfi())
    override fun answer(questionId: String, answer: String) = a.answer(questionId, answer)
    override fun provideSecret(requestId: String, secret: String) = a.provideSecret(requestId, secret)
    override fun setMode(mode: AiMode, trustMinutes: UInt?) = a.setMode(mode.toFfi(), trustMinutes)
    override fun stop() = a.stop()
    override fun setForeground(foreground: Boolean) = a.setForeground(foreground)
    override fun messages(): List<AgentMessage> = a.messages().map { it.toBridge() }
}
