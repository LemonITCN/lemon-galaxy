package cn.lemonit.galaxy.ui.remote

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.PointerInputScope
import androidx.compose.ui.input.pointer.positionChange
import kotlin.math.abs
import kotlin.math.hypot

/** 手势识别结果（视图坐标）。 */
interface RemoteGestureSink {
    fun tap(at: Offset)
    fun twoFingerTap()
    fun longPress(at: Offset)
    /** 单指拖动：触控模式下按住左键拖；鼠标模式下 dragging=false 为移动光标，true 为长按后的拖拽。 */
    fun dragStart(at: Offset, afterLongPress: Boolean)
    fun dragMove(at: Offset, delta: Offset)
    fun dragEnd()
    fun scroll(dx: Float, dy: Float)
    fun pinch(factor: Float, centroid: Offset, pan: Offset)
    fun twoFingerPan(delta: Offset)
}

private fun centroid(ps: List<Offset>) = Offset(ps.sumOf { it.x.toDouble() }.toFloat() / ps.size, ps.sumOf { it.y.toDouble() }.toFloat() / ps.size)
private fun span(ps: List<Offset>, c: Offset) = ps.sumOf { hypot((it.x - c.x).toDouble(), (it.y - c.y).toDouble()) }.toFloat() / ps.size

/**
 * 远程桌面手势：单指点按 / 长按 / 拖动，双指轻点 / 拖动（滚轮或平移）/ 捏合缩放。
 * 双指手势在越过阈值时判定一次：间距变化为主 → 缩放（同时随质心平移），否则 → 滚动或平移。
 */
suspend fun PointerInputScope.detectRemoteGestures(sink: RemoteGestureSink, scrollPansWhenZoomed: () -> Boolean) {
    val slop = viewConfiguration.touchSlop
    val longPressMs = viewConfiguration.longPressTimeoutMillis
    awaitPointerEventScope {
        while (true) {
            // 等第一根手指
            var event = awaitPointerEvent()
            val first = event.changes.firstOrNull { it.pressed } ?: continue
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
            while (true) {
                val waitLong = !moved && !longPressed && maxPointers == 1
                val next = if (waitLong) {
                    val remaining = (longPressMs - (event.changes.firstOrNull()?.uptimeMillis ?: downTime) + downTime).coerceAtLeast(1)
                    withTimeoutOrNull(remaining) { awaitPointerEvent() }
                } else {
                    awaitPointerEvent()
                }
                if (next == null) {
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
                        when (twoMode) {
                            1 -> sink.pinch(if (prevSpan > 0f) s / prevSpan else 1f, c, c - prevC)
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
            when {
                dragging -> sink.dragEnd()
                maxPointers >= 2 && twoMode == 0 -> sink.twoFingerTap()
                maxPointers == 1 && !moved && !longPressed -> sink.tap(downAt)
            }
        }
    }
}
