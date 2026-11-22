package cn.lemonit.galaxy.ui.remote

import androidx.compose.foundation.Canvas
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Fill
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.platform.testTag

/**
 * 本机记的光标位置（被控屏幕像素）：鼠标模式相对移动、画面跟随、本地画箭头都用它。
 *
 * 位置以本机为准，手指一动立即更新（跟手）。被控端回报的位置只在本机 [QUIET_MS] 内没发过移动时才采用：
 * 那是被控端那边有人动了鼠标；本机刚动过时收到的回报多半是自己发出去的移动隔着网络绕回来的旧位置，
 * 采用了光标会往回跳。是否显示一律听被控端的（对面隐藏了光标，这里也不画）。
 */
@Stable
class LocalPointer(x: Int = 0, y: Int = 0) {
    var x by mutableIntStateOf(x)
        private set
    var y by mutableIntStateOf(y)
        private set
    var visible by mutableStateOf(true)
        private set

    /** 本机最后一次发出移动 / 点击的时刻（毫秒）。 */
    private var lastLocalAt = Long.MIN_VALUE / 2

    /** 本机发出了移动或点击（坐标就是发出去的坐标）。 */
    fun moved(nx: Int, ny: Int, now: Long) {
        x = nx
        y = ny
        lastLocalAt = now
    }

    /** 被控端回报光标；返回是否采用了位置。 */
    fun hostReport(hx: Int, hy: Int, shown: Boolean, now: Long): Boolean {
        visible = shown
        if (now - lastLocalAt < QUIET_MS) return false
        x = hx
        y = hy
        return true
    }

    /** 换屏幕 / 重开画面：放到给定位置，忘掉本机移动的时刻。 */
    fun reset(nx: Int, ny: Int) {
        x = nx
        y = ny
        visible = true
        lastLocalAt = Long.MIN_VALUE / 2
    }

    companion object {
        /** 本机停手多久之后才采用被控端回报的位置。 */
        const val QUIET_MS = 500L
    }
}

/**
 * 标准箭头光标的外形（尖端在原点，单位约等于 1dp 的 1/1.3）：白色填充、黑色描边。
 * 大小固定、不随画面缩放，和电脑上的光标一样始终清晰好找。
 */
private val ArrowPoints = listOf(
    0f to 0f, 0f to 16.5f, 4f to 12.8f, 6.6f to 18.8f, 9.4f to 17.6f, 6.8f to 11.8f, 12f to 11.8f,
)

/**
 * 在画面上本地画光标（尖端 = 热点，对准 [pointer] 所在的被控像素）。
 * 只在绘制阶段读位置和画面摆放：手指移动、缩放平移只重画这一层，不重组。
 */
@Composable
internal fun LocalPointerArrow(pointer: LocalPointer, viewport: () -> Viewport, modifier: Modifier = Modifier) {
    val path = remember {
        Path().apply {
            ArrowPoints.forEachIndexed { i, (px, py) -> if (i == 0) moveTo(px, py) else lineTo(px, py) }
            close()
        }
    }
    Canvas(modifier.testTag("remote-local-pointer")) {
        if (!pointer.visible) return@Canvas
        val v = viewport()
        val tip = Offset(v.left + (pointer.x + 0.5f) * v.scale, v.top + (pointer.y + 0.5f) * v.scale)
        val k = 1.3f * density
        translate(tip.x, tip.y) {
            scale(k, k, pivot = Offset.Zero) {
                drawPath(path, Color.White, style = Fill)
                drawPath(path, Color.Black, style = Stroke(width = 1.1f, join = StrokeJoin.Round))
            }
        }
    }
}
