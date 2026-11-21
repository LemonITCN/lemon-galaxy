package cn.lemonit.galaxy.ui.remote

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlin.math.abs
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min

/**
 * 画面在视图里的摆放：先等比适配（留黑边），再乘缩放、加平移。
 * 宽高始终按同一个比例缩放，画面比例永远等于被控屏幕比例。
 * 坐标换算全部在这里，输入事件统一换成被控屏幕像素。
 *
 * - [zoom] 相对“适应屏幕”的倍数，1 = 适应屏幕。
 * - [panX]/[panY] 画面中心相对视图中心的偏移（视图像素）。
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
    /** 适应屏幕时 1 个被控像素占多少视图像素。 */
    val base: Float get() = if (remoteW <= 0 || remoteH <= 0) 1f else min(containerW / remoteW, containerH / remoteH)
    val scale: Float get() = base * zoom
    val contentW: Float get() = remoteW * scale
    val contentH: Float get() = remoteH * scale
    val left: Float get() = (containerW - contentW) / 2f + panX
    val top: Float get() = (containerH - contentH) / 2f + panY

    /** 最大倍数：至少 5 倍，且能放大到 1 个被控像素占 1.5 个视图像素。 */
    val maxZoom: Float get() = max(MAX_ZOOM, 1.5f / base)

    /** “原始像素”对应的倍数（1 个被控像素 = 1 个视图像素），不小于适应屏幕。 */
    val actualSizeZoom: Float get() = (1f / base).coerceIn(1f, maxZoom)

    val isZoomed: Boolean get() = zoom > 1.01f

    /** 视图坐标 → 被控坐标（浮点、不夹）。 */
    fun toRemoteF(x: Float, y: Float): Pair<Float, Float> = (x - left) / scale to (y - top) / scale

    /** 视图坐标 → 被控屏幕像素（取所在像素，夹在屏幕范围内）。 */
    fun toRemote(x: Float, y: Float): Pair<Int, Int> {
        val (fx, fy) = toRemoteF(x, y)
        val rx = floor(fx).toInt().coerceIn(0, max(0, remoteW - 1))
        val ry = floor(fy).toInt().coerceIn(0, max(0, remoteH - 1))
        return rx to ry
    }

    /** 被控像素 → 视图坐标（像素中心）。 */
    fun toView(rx: Int, ry: Int): Pair<Float, Float> = (left + (rx + 0.5f) * scale) to (top + (ry + 0.5f) * scale)

    /** 视图上的位移 → 被控像素位移（鼠标模式相对移动用）。 */
    fun remotePerViewPx(): Float = if (scale <= 0f) 1f else 1f / scale

    /** 按左上角位置与倍数重建（换算回中心偏移）。 */
    private fun placed(nl: Float, nt: Float, nz: Float): Viewport {
        val cw = remoteW * base * nz
        val ch = remoteH * base * nz
        return copy(zoom = nz, panX = nl - (containerW - cw) / 2f, panY = nt - (containerH - ch) / 2f)
    }

    /**
     * 捏合一步：缩放 [factor] 倍，同时让原来在 (fromX, fromY) 下的画面点移到 (toX, toY)（双指质心随手指移动）。
     * 最后夹住：放大后不露出多余黑边，缩小不低于适应屏幕。
     */
    fun pinched(factor: Float, fromX: Float, fromY: Float, toX: Float, toY: Float): Viewport {
        val nz = (zoom * factor).coerceIn(1f, maxZoom)
        val f = nz / zoom
        return placed(toX - (fromX - left) * f, toY - (fromY - top) * f, nz).clamped()
    }

    /** 以 (fx, fy) 为中心缩放到 [z] 倍。 */
    fun zoomedTo(z: Float, fx: Float, fy: Float): Viewport = pinched(z / zoom, fx, fy, fx, fy)

    fun panned(dx: Float, dy: Float) = copy(panX = panX + dx, panY = panY + dy).clamped()

    /** 回到适应屏幕。 */
    fun fit() = copy(zoom = 1f, panX = 0f, panY = 0f)

    /** 双指双击：已放大 → 适应屏幕；否则以点按处为中心放大（至少 2 倍，被控分辨率高时放到原始像素）。 */
    fun toggled(fx: Float, fy: Float): Viewport =
        if (isZoomed) fit() else zoomedTo(max(2f, actualSizeZoom).coerceAtMost(maxZoom), fx, fy)

    /**
     * 夹住平移：
     * - 画面比视图大的方向：不许露出黑边；
     * - 画面比视图小的方向（留黑边那一维）：画面不出视图，且偏离居中的量不超过 (倍数-1)×适应尺寸/2。
     *   这样从适应屏幕开始捏合时双指中心下的点不会被强行拉回中间，缩回 1 倍时又正好回到居中。
     */
    fun clamped(): Viewport {
        fun limit(content: Float, container: Float, fitSize: Float): Float =
            if (content >= container) (content - container) / 2f else min((container - content) / 2f, (zoom - 1f) * fitSize / 2f).coerceAtLeast(0f)
        val maxX = limit(contentW, containerW, remoteW * base)
        val maxY = limit(contentH, containerH, remoteH * base)
        return copy(panX = panX.coerceIn(-maxX, maxX), panY = panY.coerceIn(-maxY, maxY))
    }

    /** 让被控坐标 (rx, ry) 保持在可见区域内（鼠标模式放大后画面跟随光标）。 */
    fun follow(rx: Int, ry: Int, margin: Float = 48f): Viewport {
        if (!isZoomed) return this
        val (vx, vy) = toView(rx, ry)
        var dx = 0f
        var dy = 0f
        if (vx < margin) dx = margin - vx else if (vx > containerW - margin) dx = containerW - margin - vx
        if (vy < margin) dy = margin - vy else if (vy > containerH - margin) dy = containerH - margin - vy
        return if (dx == 0f && dy == 0f) this else panned(dx, dy)
    }

    /**
     * 视图或被控屏幕尺寸变化：
     * - 被控分辨率变了（换屏、对方改分辨率）→ 回到适应屏幕；
     * - 只是视图变了（旋转、弹出键盘）→ 保持倍数，原来在视图中心的画面点仍放在中心。
     */
    fun resized(cw: Float, ch: Float, rw: Int, rh: Int): Viewport {
        val w = max(cw, 1f)
        val h = max(ch, 1f)
        val nrw = max(rw, 1)
        val nrh = max(rh, 1)
        if (nrw != remoteW || nrh != remoteH) return Viewport(w, h, nrw, nrh)
        if (abs(w - containerW) < 0.5f && abs(h - containerH) < 0.5f) return this
        val (cx, cy) = toRemoteF(containerW / 2f, containerH / 2f)
        val next = Viewport(w, h, nrw, nrh, zoom)
        // 新视图里把 (cx, cy) 放到中心
        return next.placed(w / 2f - cx * next.scale, h / 2f - cy * next.scale, zoom).clamped()
    }

    companion object {
        const val MAX_ZOOM = 5f
    }
}

/** 远程画面的视图状态（可由调用方持有，测试里用来读取和设置当前缩放）。 */
@Stable
class RemoteViewState {
    var viewport by mutableStateOf(Viewport(1f, 1f, 1, 1))
}
