package cn.lemonit.galaxy.app

import cn.lemonit.galaxy.core.bridge.FileClientInterface
import cn.lemonit.galaxy.core.bridge.GalaxyClientInterface
import cn.lemonit.galaxy.core.bridge.PathInfo
import cn.lemonit.galaxy.core.bridge.PeerInfo
import cn.lemonit.galaxy.core.bridge.PeerListener
import cn.lemonit.galaxy.core.bridge.PeerSessionInterface
import cn.lemonit.galaxy.core.bridge.SecretPrompt
import cn.lemonit.galaxy.core.bridge.SshInfo
import cn.lemonit.galaxy.core.bridge.SshSessionInterface
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
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

    private val _myVirtualDisplays = MutableStateFlow<Set<UInt>>(emptySet())
    /**
     * 本会话里自己加的虚拟屏 id（只有这些能删；DisplayInfo 不带归属）。跟着会话走：离开远程桌面再回来（如去文件管理）还在，
     * 会话结束时被控端自动删掉、这里清空；重连是新的 PeerHandle，从空开始。
     */
    val myVirtualDisplays: StateFlow<Set<UInt>> = _myVirtualDisplays.asStateFlow()

    fun virtualDisplayAdded(id: UInt) = _myVirtualDisplays.update { it + id }

    fun virtualDisplayRemoved(id: UInt) = _myVirtualDisplays.update { it - id }

    /** 只留下满足 [keep] 的（重新取屏幕列表后，被控端那边已经没了的去掉）。 */
    fun retainVirtualDisplays(keep: (UInt) -> Boolean) = _myVirtualDisplays.update { ids -> ids.filterTo(LinkedHashSet(), keep) }

    init {
        session.setListener(object : PeerListener {
            override fun onPathChanged(path: PathInfo) {
                _path.value = path
                onPath(deviceId, path)
            }

            override fun onClosed(reason: String) {
                _myVirtualDisplays.value = emptySet()
                _closedReason.value = reason
                onPath(deviceId, null)
            }
        })
    }

    fun has(capability: String) = capability in info.capabilities
}

/** 与一台 SSH 主机的直连会话（终端、文件共用）。 */
class SshHandle(val hostId: String, val session: SshSessionInterface) {
    /** 连接时内核给的信息，via 表示怎么连过去的（直连 / 经设备 / 经 SSH 主机）。 */
    val info: SshInfo = session.info()
    val closed get() = session.isClosed()
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

    private val _sshVia = MutableStateFlow<Map<String, String>>(emptyMap())
    /** 已连上的 SSH 主机 → 内核报告的路径（direct / device:<名> / ssh:<名>），主机详情显示用。 */
    val sshVia: StateFlow<Map<String, String>> = _sshVia.asStateFlow()

    suspend fun ssh(hostId: String, title: String?): SshHandle = mutex.withLock {
        ssh[hostId]?.takeIf { !it.closed }?.let { return it }
        val prompt = object : SecretPrompt {
            override fun ask(prompt: String, echo: Boolean): String? = prompts.ask(prompt, echo, title)
            override fun confirmHostKey(host: String, port: UShort, keyType: String, fingerprint: String, changed: Boolean): Boolean =
                prompts.confirmHostKey(host, port.toInt(), keyType, fingerprint, changed, title)
        }
        val h = SshHandle(hostId, client.connectSsh(hostId, prompt))
        ssh[hostId] = h
        _sshVia.value = _sshVia.value + (hostId to h.info.via)
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
        ssh.values.forEach { runCatching { it.session.disconnect() } }
        ssh.clear()
        _sshVia.value = emptyMap()
    }
}
