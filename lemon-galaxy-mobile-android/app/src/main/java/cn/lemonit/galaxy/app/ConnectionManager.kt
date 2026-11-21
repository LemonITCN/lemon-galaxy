package cn.lemonit.galaxy.app

import cn.lemonit.galaxy.core.bridge.FileClientInterface
import cn.lemonit.galaxy.core.bridge.GalaxyClientInterface
import cn.lemonit.galaxy.core.bridge.PathInfo
import cn.lemonit.galaxy.core.bridge.PeerInfo
import cn.lemonit.galaxy.core.bridge.PeerListener
import cn.lemonit.galaxy.core.bridge.PeerSessionInterface
import cn.lemonit.galaxy.core.bridge.GalaxyException
import cn.lemonit.galaxy.core.bridge.SecretPrompt
import cn.lemonit.galaxy.core.bridge.SshClientInterface
import cn.lemonit.galaxy.core.bridge.SshSessionInterface
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** 与一台设备的会话（多个功能共用一条 PeerSession）。 */
class PeerHandle(val deviceId: String, val session: PeerSessionInterface, private val onPath: (String, PathInfo?) -> Unit = { _, _ -> }) {
    val info: PeerInfo = session.info()
    private val _path = MutableStateFlow<PathInfo?>(session.path())
    val path: StateFlow<PathInfo?> = _path.asStateFlow()
    private val _closedReason = MutableStateFlow<String?>(null)
    val closedReason: StateFlow<String?> = _closedReason.asStateFlow()
    val closed get() = _closedReason.value != null

    init {
        session.setListener(object : PeerListener {
            override fun onPathChanged(path: PathInfo) {
                _path.value = path
                onPath(deviceId, path)
            }

            override fun onClosed(reason: String) {
                _closedReason.value = reason
                onPath(deviceId, null)
            }
        })
    }

    fun has(capability: String) = capability in info.capabilities
}

class SshHandle(val hostId: String, val session: SshSessionInterface) {
    @Volatile var closed = false
}

/**
 * 连接复用：同一台设备的终端、文件、远程桌面、AI 共用一条 PeerSession；SSH 主机同理。
 * 连接失败的异常原样抛给调用方显示。
 */
class ConnectionManager(
    private val client: GalaxyClientInterface,
    private val prompts: SecretPromptBroker,
) {
    private val peers = mutableMapOf<String, PeerHandle>()
    private val ssh = mutableMapOf<String, SshHandle>()
    private val mutex = Mutex()

    private val _paths = MutableStateFlow<Map<String, PathInfo>>(emptyMap())
    /** 每台已连接设备的当前路径与延迟（终端、远程桌面顶部显示）。 */
    val paths: StateFlow<Map<String, PathInfo>> = _paths.asStateFlow()

    private fun updatePath(deviceId: String, path: PathInfo?) {
        _paths.value = if (path == null) _paths.value - deviceId else _paths.value + (deviceId to path)
    }

    suspend fun peer(deviceId: String): PeerHandle = mutex.withLock {
        peers[deviceId]?.takeIf { !it.closed && !it.session.isClosed() }?.let { return it }
        val h = PeerHandle(deviceId, client.connect(deviceId), ::updatePath)
        peers[deviceId] = h
        updatePath(deviceId, h.path.value)
        h
    }

    fun existingPeer(deviceId: String): PeerHandle? = peers[deviceId]?.takeIf { !it.closed }

    /** 界面语言的「暂不支持 SSH 直连」文字，由容器设置。 */
    var sshUnsupportedMessage: String = "SSH"

    /** 当前内核是否支持 SSH 直连（galaxy-ffi 还没导出 connect_ssh 时为 false）。 */
    val sshSupported: Boolean get() = client is SshClientInterface

    suspend fun ssh(hostId: String, title: String?): SshHandle = mutex.withLock {
        ssh[hostId]?.takeIf { !it.closed }?.let { return it }
        val sshClient = client as? SshClientInterface
            ?: throw GalaxyException.Other(sshUnsupportedMessage)
        val prompt = object : SecretPrompt {
            override fun ask(prompt: String, echo: Boolean): String? = prompts.ask(prompt, echo, title)
        }
        val h = SshHandle(hostId, sshClient.connectSsh(hostId, prompt))
        ssh[hostId] = h
        h
    }

    suspend fun files(target: TargetRef): FileClientInterface = when (target) {
        is TargetRef.Device -> peer(target.deviceId).session.files()
        is TargetRef.Ssh -> ssh(target.hostId, null).session.files()
    }

    fun disconnect(deviceId: String) {
        peers.remove(deviceId)?.session?.disconnect()
        updatePath(deviceId, null)
    }

    fun disconnectAll() {
        peers.values.forEach { runCatching { it.session.disconnect() } }
        peers.clear()
        _paths.value = emptyMap()
        ssh.values.forEach { runCatching { it.session.close() } }
        ssh.clear()
    }
}
