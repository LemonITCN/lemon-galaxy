package cn.lemonit.galaxy.ui.remote

import android.view.Surface
import cn.lemonit.galaxy.app.ConnectionManager
import cn.lemonit.galaxy.app.userMessage
import cn.lemonit.galaxy.core.bridge.ClipboardData
import cn.lemonit.galaxy.core.bridge.DisplayChannelInterface
import cn.lemonit.galaxy.core.bridge.DisplayInfo
import cn.lemonit.galaxy.core.bridge.DisplayOptions
import cn.lemonit.galaxy.core.bridge.InputEvent
import cn.lemonit.galaxy.core.bridge.PathInfo
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.VideoCodec
import cn.lemonit.galaxy.data.VideoQuality
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/**
 * 画面在视图里的摆放：等比适配（letterbox）后再乘缩放、加平移。
 * 坐标换算全部在这里，输入事件统一换成被控屏幕像素。
 */
data class Viewport(
    val containerW: Float,
    val containerH: Float,
    val remoteW: Int,
    val remoteH: Int,
    val zoom: Float = 1f,
    val panX: Float = 0f,
    val panY: Float = 0f,
) {
    private val base: Float get() = if (remoteW <= 0 || remoteH <= 0) 1f else min(containerW / remoteW, containerH / remoteH)
    val contentW: Float get() = remoteW * base * zoom
    val contentH: Float get() = remoteH * base * zoom
    val left: Float get() = (containerW - contentW) / 2f + panX
    val top: Float get() = (containerH - contentH) / 2f + panY

    /** 视图坐标 → 被控屏幕像素（夹在屏幕范围内）。 */
    fun toRemote(x: Float, y: Float): Pair<Int, Int> {
        val rx = ((x - left) / contentW * remoteW).roundToInt().coerceIn(0, max(0, remoteW - 1))
        val ry = ((y - top) / contentH * remoteH).roundToInt().coerceIn(0, max(0, remoteH - 1))
        return rx to ry
    }

    fun toView(rx: Int, ry: Int): Pair<Float, Float> = (left + rx.toFloat() / remoteW * contentW) to (top + ry.toFloat() / remoteH * contentH)

    /** 视图上的位移 → 被控像素位移（鼠标模式相对移动用）。 */
    fun remotePerViewPx(): Float = if (contentW <= 0f) 1f else remoteW / contentW

    /** 以 (fx, fy) 为中心缩放，再加平移，最后夹住：放大后不露出黑边。 */
    fun zoomed(factor: Float, fx: Float, fy: Float, dx: Float, dy: Float): Viewport {
        val nz = (zoom * factor).coerceIn(1f, 5f)
        val f = nz / zoom
        val nl = fx - (fx - left) * f + dx
        val nt = fy - (fy - top) * f + dy
        val ncw = remoteW * base * nz
        val nch = remoteH * base * nz
        return copy(zoom = nz, panX = nl - (containerW - ncw) / 2f, panY = nt - (containerH - nch) / 2f).clamped()
    }

    fun panned(dx: Float, dy: Float) = copy(panX = panX + dx, panY = panY + dy).clamped()

    fun clamped(): Viewport {
        val maxX = max(0f, (contentW - containerW) / 2f)
        val maxY = max(0f, (contentH - containerH) / 2f)
        return copy(panX = panX.coerceIn(-maxX, maxX), panY = panY.coerceIn(-maxY, maxY))
    }

    /** 让被控坐标 (rx, ry) 保持在可见区域内（鼠标模式放大后画面跟随光标）。 */
    fun follow(rx: Int, ry: Int, margin: Float = 48f): Viewport {
        if (zoom <= 1f) return this
        val (vx, vy) = toView(rx, ry)
        var dx = 0f
        var dy = 0f
        if (vx < margin) dx = margin - vx else if (vx > containerW - margin) dx = containerW - margin - vx
        if (vy < margin) dy = margin - vy else if (vy > containerH - margin) dy = containerH - margin - vy
        return if (dx == 0f && dy == 0f) this else panned(dx, dy)
    }
}

enum class RemoteStatus { CONNECTING, STREAMING, FAILED }

data class RemoteUi(
    val title: String,
    val platform: Platform,
    val status: RemoteStatus = RemoteStatus.CONNECTING,
    val error: String? = null,
    val displays: List<DisplayInfo> = emptyList(),
    val displayId: UInt? = null,
    val quality: VideoQuality = VideoQuality.AUTO,
    val mouseMode: Boolean = false,
    val cursorX: Int = 0,
    val cursorY: Int = 0,
    val cursorVisible: Boolean = false,
    val videoW: Int = 0,
    val videoH: Int = 0,
    val path: PathInfo? = null,
) {
    val display: DisplayInfo? get() = displays.firstOrNull { it.id == displayId }
    val remoteW: Int get() = display?.width?.toInt() ?: videoW
    val remoteH: Int get() = display?.height?.toInt() ?: videoH
}

class RemoteDesktopController(
    private val deviceId: String,
    title: String,
    platform: Platform,
    private val conn: ConnectionManager,
    private val scope: CoroutineScope,
    quality: VideoQuality,
    mouseMode: Boolean,
) {
    private val _ui = MutableStateFlow(RemoteUi(title, platform, quality = quality, mouseMode = mouseMode))
    val ui: StateFlow<RemoteUi> = _ui.asStateFlow()
    @Volatile private var channel: DisplayChannelInterface? = null

    val decoder = VideoDecoder(
        requestKeyframe = { channel?.requestKeyframe() },
        onVideoSize = { w, h -> _ui.update { it.copy(videoW = w, videoH = h, status = RemoteStatus.STREAMING) } },
        onCursorChanged = { x, y, v -> _ui.update { it.copy(cursorX = x, cursorY = y, cursorVisible = v) } },
    )

    fun start() {
        scope.launch {
            try {
                val peer = conn.peer(deviceId)
                _ui.update { it.copy(path = peer.path.value) }
                scope.launch { peer.path.collect { p -> _ui.update { it.copy(path = p) } } }
                val displays = peer.session.listDisplays()
                val first = displays.firstOrNull { it.primary } ?: displays.firstOrNull()
                    ?: throw IllegalStateException("no display")
                _ui.update { it.copy(displays = displays, cursorX = (first.width / 2u).toInt(), cursorY = (first.height / 2u).toInt()) }
                open(first.id)
            } catch (e: Exception) {
                _ui.update { it.copy(status = RemoteStatus.FAILED, error = e.userMessage()) }
            }
        }
    }

    private suspend fun open(displayId: UInt) {
        channel?.stop()
        channel = null
        _ui.update { it.copy(displayId = displayId, status = RemoteStatus.CONNECTING, error = null) }
        val q = _ui.value.quality
        val peer = conn.peer(deviceId)
        channel = peer.session.openDisplay(
            DisplayOptions(displayId, q.maxFps, q.bitrateKbps, listOf(VideoCodec.H265, VideoCodec.H264)),
            decoder,
        )
    }

    fun switchDisplay(id: UInt) {
        if (id == _ui.value.displayId) return
        scope.launch {
            try {
                open(id)
            } catch (e: Exception) {
                _ui.update { it.copy(status = RemoteStatus.FAILED, error = e.userMessage()) }
            }
        }
    }

    fun setQuality(q: VideoQuality) {
        _ui.update { it.copy(quality = q) }
        channel?.setBitrate(q.bitrateKbps)
    }

    fun setMouseMode(on: Boolean) = _ui.update { it.copy(mouseMode = on) }

    fun setSurface(s: Surface?) = decoder.setSurface(s)

    fun send(e: InputEvent) {
        channel?.sendInput(e)
        // 本地先更新光标，减少等待回报的拖影
        when (e) {
            is InputEvent.MouseMove -> _ui.update { it.copy(cursorX = e.x, cursorY = e.y) }
            is InputEvent.MouseButton -> _ui.update { it.copy(cursorX = e.x, cursorY = e.y) }
            else -> Unit
        }
    }

    fun sendAll(events: List<InputEvent>) = events.forEach(::send)

    suspend fun clipboardSend(text: String) = withContext(Dispatchers.Default) {
        conn.peer(deviceId).session.clipboardSet(ClipboardData(text, null))
    }

    suspend fun clipboardGet(): String? = withContext(Dispatchers.Default) {
        conn.peer(deviceId).session.clipboardGet().text
    }

    fun stop() {
        channel?.stop()
        channel = null
        decoder.release()
    }
}
