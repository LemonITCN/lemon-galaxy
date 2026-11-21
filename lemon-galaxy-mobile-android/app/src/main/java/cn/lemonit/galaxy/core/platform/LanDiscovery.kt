package cn.lemonit.galaxy.core.platform

import android.content.Context
import android.net.wifi.WifiManager
import android.util.Log
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.platform.LocalContext

/**
 * 局域网发现钩子（protocol.md 第 5 节）。
 *
 * 安卓默认丢弃发给本机的组播包，内核收组播 / 广播回应前必须持有 WifiManager.MulticastLock。
 * 连接中与设备列表页打开期间持有，离开时释放；引用计数，多处同时持有时最后一个释放才真正释放。
 * 设备列表页（discovery = true）持有期间还通知内核 GalaxyClient.setLanDiscovery(true)：内核每 10 秒查询一次局域网。
 */
object LanDiscovery {
    private const val TAG = "LanDiscovery"
    private var lock: WifiManager.MulticastLock? = null
    private var holders = 0
    private var discoveryHolders = 0

    /** 由应用容器接到 GalaxyClient.setLanDiscovery。 */
    @Volatile var onDiscoveryChanged: ((Boolean) -> Unit)? = null

    @Synchronized
    fun acquire(context: Context, discovery: Boolean = false) {
        holders++
        if (discovery && discoveryHolders++ == 0) runCatching { onDiscoveryChanged?.invoke(true) }
        if (lock == null) {
            val wifi = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager ?: return
            lock = wifi.createMulticastLock("lemongalaxy-lan").apply { setReferenceCounted(false) }
        }
        val l = lock ?: return
        if (!l.isHeld) {
            runCatching { l.acquire() }.onFailure { Log.w(TAG, "acquire failed: ${it.message}") }
        }
    }

    @Synchronized
    fun release(discovery: Boolean = false) {
        if (discovery && discoveryHolders > 0 && --discoveryHolders == 0) runCatching { onDiscoveryChanged?.invoke(false) }
        if (holders > 0) holders--
        if (holders == 0) {
            lock?.takeIf { it.isHeld }?.let { runCatching { it.release() } }
        }
    }

    @Synchronized
    fun isHeld(): Boolean = lock?.isHeld == true
}

/** 在当前页面可见期间持有组播锁；discovery = true 时同时让内核定期查询局域网（设备列表页）。 */
@Composable
fun HoldMulticastLock(discovery: Boolean = false) {
    val context = LocalContext.current
    DisposableEffect(discovery) {
        LanDiscovery.acquire(context, discovery)
        onDispose { LanDiscovery.release(discovery) }
    }
}
