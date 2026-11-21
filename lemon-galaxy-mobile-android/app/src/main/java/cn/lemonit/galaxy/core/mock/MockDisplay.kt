package cn.lemonit.galaxy.core.mock

import cn.lemonit.galaxy.core.bridge.DisplayChannelInterface
import cn.lemonit.galaxy.core.bridge.DisplayInfo
import cn.lemonit.galaxy.core.bridge.InputEvent
import cn.lemonit.galaxy.core.bridge.VideoCodec
import cn.lemonit.galaxy.core.bridge.VideoSink
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/** H.264 Annex B 码流按访问单元（以 AUD 分隔）切成帧。 */
internal object AnnexB {
    data class AccessUnit(val data: ByteArray, val keyframe: Boolean)

    fun split(stream: ByteArray): List<AccessUnit> {
        val starts = mutableListOf<Int>()
        var i = 0
        while (i + 3 < stream.size) {
            if (stream[i] == 0.toByte() && stream[i + 1] == 0.toByte() &&
                (stream[i + 2] == 1.toByte() || (stream[i + 2] == 0.toByte() && stream[i + 3] == 1.toByte()))
            ) {
                val hdr = if (stream[i + 2] == 1.toByte()) i + 3 else i + 4
                val type = stream[hdr].toInt() and 0x1F
                if (type == 9) starts.add(i)
                i = hdr
            } else {
                i++
            }
        }
        if (starts.isEmpty()) return listOf(AccessUnit(stream, true))
        return starts.mapIndexed { idx, s ->
            val end = if (idx + 1 < starts.size) starts[idx + 1] else stream.size
            val au = stream.copyOfRange(s, end)
            AccessUnit(au, containsIdr(au))
        }
    }

    private fun containsIdr(au: ByteArray): Boolean {
        var i = 0
        while (i + 3 < au.size) {
            if (au[i] == 0.toByte() && au[i + 1] == 0.toByte() && au[i + 2] == 1.toByte()) {
                if (au[i + 3].toInt() and 0x1F == 5) return true
                i += 3
            } else {
                i++
            }
        }
        return false
    }
}

/**
 * 模拟远程画面：循环推送 assets 里预先编码好的 1280x720 H.264 片段（假桌面），
 * 鼠标移动时回报光标位置（被控屏幕像素）。
 */
internal class MockDisplayChannel(
    private val display: DisplayInfo,
    private val frames: List<AnnexB.AccessUnit>,
    private val sink: VideoSink,
    scope: CoroutineScope,
    fps: Int,
) : DisplayChannelInterface {
    @Volatile private var next = 0
    @Volatile private var stopped = false
    private val job: Job

    init {
        val interval = (1000 / fps.coerceIn(1, 60)).toLong()
        job = scope.launch {
            sink.onConfig(VideoCodec.H264, 1280u, 720u)
            sink.onCursor((display.width / 2u).toInt(), (display.height / 2u).toInt(), true)
            var pts = 0L
            while (isActive && !stopped) {
                val au = frames[next]
                sink.onFrame(au.data, pts.toULong(), au.keyframe)
                next = (next + 1) % frames.size
                pts += interval * 1000
                delay(interval)
            }
        }
    }

    override fun sendInput(event: InputEvent) {
        when (event) {
            is InputEvent.MouseMove -> sink.onCursor(event.x, event.y, true)
            is InputEvent.MouseButton -> sink.onCursor(event.x, event.y, true)
            else -> Unit
        }
    }

    override fun requestKeyframe() {
        // 片段第一帧是 IDR（带 SPS/PPS）
        next = 0
    }

    override fun setBitrate(kbps: UInt) = Unit

    override fun stop() {
        stopped = true
        job.cancel()
        sink.onClosed("stopped")
    }
}
