package cn.lemonit.galaxy.ui.remote

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.PointerEvent
import androidx.compose.ui.input.pointer.PointerInputScope
import androidx.compose.ui.input.pointer.positionChange
import kotlin.math.abs
import kotlin.math.hypot

/** 手势识别结果（视图坐标）。 */
interface RemoteGestureSink {
    fun tap(at: Offset)
    /** 双指轻点（右键）。为了识别双指双击，会在双击间隔过后才回调。 */
    fun twoFingerTap()
    /** 双指双击：适应屏幕 / 放大切换。 */
    fun twoFingerDoubleTap(at: Offset)
    fun longPress(at: Offset)
    /** 单指拖动：触控模式下按住左键拖；鼠标模式下 dragging=false 为移动光标，true 为长按后的拖拽。 */
    fun dragStart(at: Offset, afterLongPress: Boolean)
    fun dragMove(at: Offset, delta: Offset)
    fun dragEnd()
    fun scroll(dx: Float, dy: Float)
    /** 捏合一步：缩放 [factor] 倍，原来在 [from] 下的画面点移到 [to]。 */
    fun pinch(factor: Float, from: Offset, to: Offset)
    /** 捏合结束。 */
    fun pinchEnd() = Unit
    fun twoFingerPan(delta: Offset)
}

private fun centroid(ps: List<Offset>) = Offset(ps.sumOf { it.x.toDouble() }.toFloat() / ps.size, ps.sumOf { it.y.toDouble() }.toFloat() / ps.size)
private fun span(ps: List<Offset>, c: Offset) = ps.sumOf { hypot((it.x - c.x).toDouble(), (it.y - c.y).toDouble()) }.toFloat() / ps.size

/**
 * 远程桌面手势：单指点按 / 长按 / 拖动，双指轻点 / 拖动（滚轮或平移）/ 捏合缩放。
 * 双指手势在越过阈值时判定一次：间距变化为主 → 缩放（同时随质心平移），否则 → 滚动或平移。
 * 双指轻点先不回调，等双击间隔：期间再来一次双指轻点 → 双指双击（切换缩放），否则 → 右键。
 * 单指双击不拦截，照常发两次单击（被控端收到的就是双击）。
 */
suspend fun PointerInputScope.detectRemoteGestures(sink: RemoteGestureSink, scrollPansWhenZoomed: () -> Boolean) {
    val slop = viewConfiguration.touchSlop
    val longPressMs = viewConfiguration.longPressTimeoutMillis
    val doubleTapMs = viewConfiguration.doubleTapTimeoutMillis
    awaitPointerEventScope {
        // 等待第二次的双指轻点：抬起时刻与位置
        var pendingTwoTapAt = 0L
        var pendingTwoTapPos: Offset? = null
        fun flushTwoTap() {
            if (pendingTwoTapPos != null) {
                pendingTwoTapPos = null
                sink.twoFingerTap()
            }
        }
        while (true) {
            // 等第一根手指（有待定的双指轻点时最多等到双击间隔结束）
            val waited = if (pendingTwoTapPos != null) {
                val elapsed = (currentEvent.changes.firstOrNull()?.uptimeMillis ?: pendingTwoTapAt) - pendingTwoTapAt
                withTimeoutOrNull((doubleTapMs - elapsed).coerceAtLeast(1)) { awaitPointerEvent() }
            } else {
                awaitPointerEvent()
            }
            if (waited == null) {
                flushTwoTap()
                continue
            }
            var event: PointerEvent = waited
            val first = event.changes.firstOrNull { it.pressed } ?: continue
            if (pendingTwoTapPos != null && first.uptimeMillis - pendingTwoTapAt > doubleTapMs) flushTwoTap()
            val downAt = first.position
            val downTime = first.uptimeMillis
            var maxPointers = 1
            var moved = false
            var longPressed = false
            var dragging = false
            var twoMode = 0 // 0 未定 1 缩放 2 滚动/平移
            var twoStartC = Offset.Zero
            var twoStartSpan = 0f
            var prevC = Offset.Zero
            var prevSpan = 0f
            var lastPos = downAt
            var downCentroid: Offset? = null
            while (true) {
                val waitLong = !moved && !longPressed && maxPointers == 1
                val next = if (waitLong) {
                    val remaining = (longPressMs - (event.changes.firstOrNull()?.uptimeMillis ?: downTime) + downTime).coerceAtLeast(1)
                    withTimeoutOrNull(remaining) { awaitPointerEvent() }
                } else {
                    awaitPointerEvent()
                }
                if (next == null) {
                    flushTwoTap()
                    longPressed = true
                    sink.longPress(downAt)
                    continue
                }
                event = next
                val pressed = event.changes.filter { it.pressed }
                if (pressed.isEmpty()) break
                maxPointers = maxOf(maxPointers, pressed.size)
                if (pressed.size >= 2) {
                    if (dragging) {
                        sink.dragEnd()
                        dragging = false
                    }
                    if (maxPointers == 2 && pressed.size == 2 && downCentroid == null) downCentroid = centroid(pressed.map { it.position })
                    val pts = pressed.map { it.position }
                    val c = centroid(pts)
                    val s = span(pts, c)
                    if (twoStartSpan == 0f) {
                        twoStartC = c
                        twoStartSpan = s
                        prevC = c
                        prevSpan = s
                    } else {
                        if (twoMode == 0) {
                            val ds = abs(s - twoStartSpan)
                            val dc = (c - twoStartC).getDistance()
                            if (ds > slop * 1.5f && ds > dc * 0.8f) twoMode = 1 else if (dc > slop) twoMode = 2
                        }
                        if (twoMode != 0) flushTwoTap()
                        when (twoMode) {
                            1 -> sink.pinch(if (prevSpan > 0f) s / prevSpan else 1f, prevC, c)
                            2 -> if (scrollPansWhenZoomed()) sink.twoFingerPan(c - prevC) else sink.scroll((c - prevC).x, (c - prevC).y)
                        }
                        prevC = c
                        prevSpan = s
                        if (twoMode != 0) moved = true
                    }
                    event.changes.forEach { it.consume() }
                    continue
                }
                if (maxPointers >= 2) {
                    // 双指手势结束后剩一根手指：忽略直到全部抬起
                    event.changes.forEach { it.consume() }
                    continue
                }
                val p = pressed.first()
                val pos = p.position
                if (!moved && (pos - downAt).getDistance() > slop) {
                    flushTwoTap()
                    moved = true
                    dragging = true
                    sink.dragStart(downAt, longPressed)
                    lastPos = downAt
                }
                if (dragging) {
                    sink.dragMove(pos, pos - lastPos)
                    lastPos = pos
                }
                if (p.positionChange() != Offset.Zero) p.consume()
            }
            val upAt = event.changes.firstOrNull()?.uptimeMillis ?: downTime
            when {
                dragging -> sink.dragEnd()
                maxPointers >= 2 && twoMode == 0 -> {
                    val at = downCentroid ?: downAt
                    val prev = pendingTwoTapPos
                    if (prev != null && (prev - at).getDistance() < slop * 6) {
                        pendingTwoTapPos = null
                        sink.twoFingerDoubleTap(at)
                    } else {
                        flushTwoTap()
                        pendingTwoTapPos = at
                        pendingTwoTapAt = upAt
                    }
                }
                maxPointers >= 2 && twoMode == 1 -> sink.pinchEnd()
                maxPointers == 1 && !moved && !longPressed -> {
                    flushTwoTap()
                    sink.tap(downAt)
                }
                else -> flushTwoTap()
            }
        }
    }
}
