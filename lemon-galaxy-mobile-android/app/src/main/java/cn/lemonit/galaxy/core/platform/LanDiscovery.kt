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
 * 真内核接入后，内核的局域网查询只在锁持有期间进行（设备列表页每 10 秒一次）。
 */
object LanDiscovery {
    private const val TAG = "LanDiscovery"
    private var lock: WifiManager.MulticastLock? = null
    private var holders = 0

    @Synchronized
    fun acquire(context: Context) {
        holders++
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
    fun release() {
        if (holders > 0) holders--
        if (holders == 0) {
            lock?.takeIf { it.isHeld }?.let { runCatching { it.release() } }
        }
    }

    @Synchronized
    fun isHeld(): Boolean = lock?.isHeld == true
}

/** 在当前页面可见期间持有组播锁。 */
@Composable
fun HoldMulticastLock() {
    val context = LocalContext.current
    DisposableEffect(Unit) {
        LanDiscovery.acquire(context)
        onDispose { LanDiscovery.release() }
    }
}
