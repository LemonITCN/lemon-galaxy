package cn.lemonit.galaxy.app

import cn.lemonit.galaxy.core.bridge.DeviceInfo
import cn.lemonit.galaxy.core.bridge.DeviceKind
import cn.lemonit.galaxy.core.bridge.GalaxyClientInterface
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.UpgradeListener
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** 一台被控端的远程升级进度（设备详情里就地显示，列表里标「升级中」）。 */
sealed class DeviceUpgrade {
    /** 已点「升级被控端」，内核正在连对方发请求；[version] 是服务器报的新版本（可能为空）。 */
    data class Requesting(val version: String) : DeviceUpgrade()

    /** 对方开始升级到 [version]（下载、核对、安装）。 */
    data class Installing(val version: String) : DeviceUpgrade()

    /** 对方已断开，正在替换、重启。 */
    data class Restarting(val version: String) : DeviceUpgrade()

    /** 对方以 [version] 重新上线；[alreadyLatest] 为真表示对方本来就是最新，没有升级。 */
    data class Done(val version: String, val alreadyLatest: Boolean) : DeviceUpgrade()

    /** 失败，[reason] 是内核给的原因（已本地化）。 */
    data class Failed(val reason: String) : DeviceUpgrade()

    val running: Boolean get() = this is Requesting || this is Installing || this is Restarting

    /** 详情里显示的进度：升级完成后服务器又报了更新的版本时，按「有新版本」显示，不再停在上次的结果上。 */
    fun shownFor(d: DeviceInfo): DeviceUpgrade? =
        if (this is Done && !alreadyLatest && d.updateVersion.isNotEmpty() && d.updateVersion != version) null else this
}

/** 哪些设备能远程升级。 */
object RemoteUpgrade {
    /**
     * 能被控的设备：纯被控端（Windows / macOS / Linux 的 worker），或带「本机被控」的 macOS 客户端。
     * 本机、手机、Windows 客户端（不被控）都不算。
     */
    fun controllable(d: DeviceInfo): Boolean =
        !d.isSelf && (d.kind == DeviceKind.WORKER || (d.kind == DeviceKind.CONTROL && d.platform == Platform.MACOS))

    /** 列表里标「可升级」：能被控且服务器上有新版本。 */
    fun available(d: DeviceInfo): Boolean = controllable(d) && d.updateVersion.isNotEmpty()

    /** 详情里的「升级被控端」能不能点：有新版本或上次失败了（重试）、不在进行中、设备在线。 */
    fun canStart(d: DeviceInfo, state: DeviceUpgrade?): Boolean =
        controllable(d) && d.online && state?.running != true && (d.updateVersion.isNotEmpty() || state is DeviceUpgrade.Failed)
}

/**
 * 远程升级被控端（ffi.md 5.0 upgradeDevice）。点了直接交给内核，不再确认；进度按设备记在这里，
 * 关掉详情再打开仍能看到。内核回调在内核线程，这里切回 [scope]（主线程）再改状态；结束后 [afterFinish] 刷新设备列表。
 */
class DeviceUpgradeController(
    private val client: GalaxyClientInterface,
    private val scope: CoroutineScope,
    private val afterFinish: () -> Unit,
) {
    private val _states = MutableStateFlow<Map<String, DeviceUpgrade>>(emptyMap())
    val states: StateFlow<Map<String, DeviceUpgrade>> = _states.asStateFlow()

    /** 每台设备当前这一轮升级的编号：退出登录或重新开始后，上一轮迟到的回调不再改界面。 */
    private val rounds = HashMap<String, Long>()
    private var nextRound = 0L

    fun state(deviceId: String): DeviceUpgrade? = _states.value[deviceId]

    /** 点「升级被控端」。同一台正在升级时忽略。 */
    fun upgrade(device: DeviceInfo) {
        val id = device.id
        if (state(id)?.running == true) return
        val round = synchronized(rounds) { (++nextRound).also { rounds[id] = it } }
        set(id, DeviceUpgrade.Requesting(device.updateVersion))
        val listener = object : UpgradeListener {
            override fun onStarted(version: String) = post(id, round) { set(id, DeviceUpgrade.Installing(version)) }

            override fun onOffline() = post(id, round) {
                set(id, DeviceUpgrade.Restarting(targetOf(state(id)) ?: device.updateVersion))
            }

            override fun onFinished(version: String?, error: String?) = post(id, round) {
                val prev = state(id)
                val started = prev is DeviceUpgrade.Installing || prev is DeviceUpgrade.Restarting
                set(id, if (error != null) DeviceUpgrade.Failed(error) else DeviceUpgrade.Done(version ?: targetOf(prev).orEmpty(), alreadyLatest = !started))
                afterFinish()
            }
        }
        try {
            client.upgradeDevice(id, listener)
        } catch (e: Exception) {
            set(id, DeviceUpgrade.Failed(e.userMessage()))
        }
    }

    /** 退出登录：清掉所有进度，之后到达的回调忽略。 */
    fun clear() {
        synchronized(rounds) { rounds.clear() }
        _states.value = emptyMap()
    }

    private fun targetOf(s: DeviceUpgrade?): String? = when (s) {
        is DeviceUpgrade.Requesting -> s.version.ifEmpty { null }
        is DeviceUpgrade.Installing -> s.version
        is DeviceUpgrade.Restarting -> s.version
        else -> null
    }

    private fun set(id: String, s: DeviceUpgrade) = _states.update { it + (id to s) }

    private fun post(id: String, round: Long, block: () -> Unit) {
        scope.launch {
            if (synchronized(rounds) { rounds[id] } == round) block()
        }
    }
}
