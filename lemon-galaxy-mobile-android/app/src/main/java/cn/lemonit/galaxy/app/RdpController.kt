package cn.lemonit.galaxy.app

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import cn.lemonit.galaxy.core.bridge.GalaxyClientInterface
import cn.lemonit.galaxy.core.bridge.RdpTarget
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.net.URLEncoder

/** 一条经设备转发的 RDP：本机 127.0.0.1:[port] → 那台设备 → 目标的 RDP 端口。 */
data class RdpForward(val id: String, val hostId: String, val label: String, val port: Int)

/** 交给微软 Windows App 的 rdp:// 链接。密码不放进链接，由用户在 Windows App 里输入。 */
object RdpLinks {
    /** Play 商店里的 Windows App（原 Microsoft Remote Desktop）。 */
    const val PLAY_STORE = "https://play.google.com/store/apps/details?id=com.microsoft.rdc.androidx"

    /** rdp://full%20address=s:<host>:<port>&username=s:<user>，键和值各自 URL 编码（域账号写成 域\用户），
     *  值里的冒号与方括号按微软文档原样保留（与 Mac、iOS 一致）。 */
    fun uri(t: RdpTarget): String {
        val host = if (t.host.contains(':') && !t.host.startsWith("[")) "[${t.host}]" else t.host
        val params = mutableListOf("full address" to "s:$host:${t.port}")
        var user = t.username
        val domain = t.domain
        if (!domain.isNullOrEmpty() && user.isNotEmpty() && '\\' !in user && '@' !in user) user = "$domain\\$user"
        if (user.isNotEmpty()) params += "username" to "s:$user"
        return "rdp://" + params.joinToString("&") { (k, v) -> enc(k) + "=" + enc(v).replace("%3A", ":").replace("%5B", "[").replace("%5D", "]") }
    }

    /** URLEncoder 把空格编码成 +，Windows App 要的是 %20 */
    private fun enc(s: String) = URLEncoder.encode(s, "UTF-8").replace("+", "%20")
}

/**
 * RDP 主机：内核准备地址（经设备时在本机开端口转发），再用 ACTION_VIEW 打开 rdp:// 链接交给 Windows App。
 * 经设备的转发列在 [forwards] 里，可以手动停止；退出登录、应用退出时全部关掉。
 */
class RdpController(
    private val client: GalaxyClientInterface,
    private val scope: CoroutineScope,
    private val appContext: Context?,
    private val toast: (String) -> Unit,
) {
    private val _forwards = MutableStateFlow<List<RdpForward>>(emptyList())
    val forwards: StateFlow<List<RdpForward>> = _forwards.asStateFlow()

    private val _missingClient = MutableStateFlow(false)
    /** 没有能打开 rdp:// 的应用：界面提示安装 Windows App。 */
    val missingClient: StateFlow<Boolean> = _missingClient.asStateFlow()

    /** 测试、截图用：不真的启动别的应用。 */
    var launcher: (String) -> Boolean = ::launch

    fun open(hostId: String) {
        scope.launch {
            try {
                val t = client.openRdp(hostId)
                if (t.forwarded) {
                    _forwards.update { list -> list.filter { it.id != t.forwardId } + RdpForward(t.forwardId, hostId, t.label, t.port.toInt()) }
                }
                if (!launcher(RdpLinks.uri(t))) {
                    if (t.forwarded) close(t.forwardId)
                    _missingClient.value = true
                } else if (t.forwarded) {
                    appContext?.getString(cn.lemonit.galaxy.R.string.rdp_forwarding, t.label, t.port.toInt())?.let(toast)
                }
            } catch (e: Exception) {
                toast(e.userMessage())
            }
        }
    }

    private fun launch(uri: String): Boolean {
        val ctx = appContext ?: return false
        return try {
            ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(uri)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            true
        } catch (_: ActivityNotFoundException) {
            false
        }
    }

    fun dismissMissingClient() {
        _missingClient.value = false
    }

    fun close(forwardId: String) {
        runCatching { client.closeRdp(forwardId) }
        _forwards.update { list -> list.filter { it.id != forwardId } }
    }

    fun closeAll() {
        _forwards.value.forEach { runCatching { client.closeRdp(it.id) } }
        _forwards.value = emptyList()
    }
}
