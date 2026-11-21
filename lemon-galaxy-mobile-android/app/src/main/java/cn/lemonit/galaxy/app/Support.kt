package cn.lemonit.galaxy.app

import cn.lemonit.galaxy.core.bridge.GalaxyException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** 把内核异常转成界面直接显示的文字（GalaxyException.message 已按 locale 本地化）。 */
fun Throwable.userMessage(): String = when (this) {
    is GalaxyException.Cancelled -> ""
    is GalaxyException -> message ?: ""
    else -> message ?: javaClass.simpleName
}

/** 连接目标：被控设备，或加密库里的 SSH 主机。 */
sealed class TargetRef {
    abstract val key: String

    data class Device(val deviceId: String) : TargetRef() {
        override val key get() = "d:$deviceId"
    }

    data class Ssh(val hostId: String) : TargetRef() {
        override val key get() = "s:$hostId"
    }
}

/**
 * 内核同步回调 SecretPrompt（ask / confirmHostKey）与界面弹窗之间的桥：
 * 内核线程调用 [ask] 或 [confirmHostKey] 后阻塞，界面在 [request] 出现时弹框，用户回答后 [respond] / [respondHostKey] 解除阻塞。
 * 同一时间只处理一个问题；内核的多个询问自然排队（后来的等前一个回答完）。
 */
class SecretPromptBroker {
    sealed class Request {
        abstract val id: Long
        abstract val title: String?

        /** 密码、私钥口令、键盘交互的问题；[echo] 为 false 时遮挡输入。 */
        data class Ask(override val id: Long, val prompt: String, val echo: Boolean, override val title: String?) : Request()

        /** 首次连接或主机密钥变化时确认；[changed] 为 true 表示与记录的不一致。 */
        data class HostKey(
            override val id: Long,
            val host: String,
            val port: Int,
            val keyType: String,
            val fingerprint: String,
            val changed: Boolean,
            override val title: String?,
        ) : Request()
    }

    private val _request = MutableStateFlow<Request?>(null)
    val request: StateFlow<Request?> = _request.asStateFlow()
    private var seq = 0L
    private val gate = Any()
    private var latch: CountDownLatch? = null
    private var answer: Any? = null

    /** 在内核的后台线程调用，最长等 5 分钟；返回 null 表示取消。 */
    fun ask(prompt: String, echo: Boolean, title: String? = null): String? =
        await { id -> Request.Ask(id, prompt, echo, title) } as? String

    /** 在内核的后台线程调用，最长等 5 分钟；超时或关掉弹框都按不信任处理。 */
    fun confirmHostKey(host: String, port: Int, keyType: String, fingerprint: String, changed: Boolean, title: String? = null): Boolean =
        await { id -> Request.HostKey(id, host, port, keyType, fingerprint, changed, title) } == true

    private fun await(make: (Long) -> Request): Any? = synchronized(gate) {
        val l = CountDownLatch(1)
        synchronized(this) {
            latch = l
            answer = null
            _request.value = make(++seq)
        }
        l.await(5, TimeUnit.MINUTES)
        synchronized(this) {
            _request.value = null
            latch = null
            answer
        }
    }

    fun respond(value: String?) = resolve(value)

    fun respondHostKey(trust: Boolean) = resolve(trust)

    private fun resolve(value: Any?) {
        synchronized(this) {
            val l = latch ?: return
            // 只认第一次回答（弹框按钮与关闭可能先后触发）
            answer = value
            latch = null
            l.countDown()
        }
    }
}
