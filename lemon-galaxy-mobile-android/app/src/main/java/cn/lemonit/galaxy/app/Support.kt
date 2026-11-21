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
 * 内核同步回调 SecretPrompt.ask() 与界面弹窗之间的桥：
 * 内核线程调用 [ask] 后阻塞，界面在 [request] 出现时弹框，用户回答后 [respond] 解除阻塞。
 */
class SecretPromptBroker {
    data class Request(val id: Long, val prompt: String, val echo: Boolean, val title: String?)

    private val _request = MutableStateFlow<Request?>(null)
    val request: StateFlow<Request?> = _request.asStateFlow()
    private var seq = 0L
    private var latch: CountDownLatch? = null
    private var answer: String? = null

    /** 在内核的后台线程调用，最长等 5 分钟。 */
    fun ask(prompt: String, echo: Boolean, title: String? = null): String? {
        val l = CountDownLatch(1)
        synchronized(this) {
            latch = l
            answer = null
            _request.value = Request(++seq, prompt, echo, title)
        }
        l.await(5, TimeUnit.MINUTES)
        synchronized(this) {
            _request.value = null
            latch = null
            return answer
        }
    }

    fun respond(value: String?) {
        synchronized(this) {
            answer = value
            latch?.countDown()
        }
    }
}
