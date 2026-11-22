package cn.lemonit.galaxy.ui.remote

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.DragIndicator
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.core.bridge.PointerButton
import cn.lemonit.galaxy.ui.components.Clock
import kotlinx.coroutines.delay
import kotlin.math.roundToInt

/**
 * 悬浮鼠标按键（左键、右键）的状态机，与 iOS 的 MouseKeys 行为一致：
 * - 点一下 = 单击。为了和双击区分，松手后等 [DOUBLE_MS] 没有第二下才发出单击；
 * - 按住超过 [HOLD_MS]（或按着时另一根手指开始移动光标）= 按下不放，松手抬起，期间移动光标就是拖拽；
 * - 双击 = 锁定按下（第二下按下时立即锁定，不再发单击），之后单指移动光标即拖拽；再点一下解锁抬起。
 * 同一时刻只有一个键在用：按另一个键会先结束当前的键。
 *
 * 只产生「哪个键按下 / 抬起」，坐标由界面按当前光标位置补上。时间由调用方传入，便于单元测试。
 */
class MouseKeysLogic {
    enum class Phase { IDLE, PENDING, HELD, TAPPED, LOCKED }

    data class Press(val button: PointerButton, val down: Boolean)

    var phase = Phase.IDLE
        private set
    var button: PointerButton? = null
        private set
    private var at = 0L
    /** 这次按下已经处理完（解锁 / 锁定），对应的松手不再处理。 */
    private var swallowUp = false

    /** 键处于按下状态（按住或锁定）：界面高亮。 */
    val isDown: Boolean get() = phase == Phase.HELD || phase == Phase.LOCKED
    val isLocked: Boolean get() = phase == Phase.LOCKED

    /** 画面上的单击要不要让开：键按着或锁定时，画面轻点不再发单击，免得打断拖拽。 */
    val busy: Boolean get() = phase == Phase.HELD || phase == Phase.LOCKED || phase == Phase.PENDING

    fun down(b: PointerButton, now: Long): List<Press> {
        val out = mutableListOf<Press>()
        when (phase) {
            Phase.LOCKED -> {
                out += Press(button!!, false)
                val same = button == b
                reset()
                if (same) {
                    // 再点一下 = 解锁
                    swallowUp = true
                    return out
                }
            }
            Phase.TAPPED -> {
                if (button == b && now - at <= DOUBLE_MS) {
                    // 双击的第二下：锁定按下
                    phase = Phase.LOCKED
                    swallowUp = true
                    out += Press(b, true)
                    return out
                }
                out += click(button!!)
                reset()
            }
            Phase.PENDING, Phase.HELD -> out += finish()
            Phase.IDLE -> Unit
        }
        phase = Phase.PENDING
        button = b
        at = now
        swallowUp = false
        return out
    }

    fun up(b: PointerButton, now: Long): List<Press> {
        if (swallowUp) {
            swallowUp = false
            return emptyList()
        }
        if (button != b) return emptyList()
        return when (phase) {
            Phase.PENDING -> {
                phase = Phase.TAPPED
                at = now
                emptyList()
            }
            Phase.HELD -> {
                reset()
                listOf(Press(b, false))
            }
            else -> emptyList()
        }
    }

    /** 到时间点检查：按住够久变成按下，单击等不到第二下就发出。 */
    fun tick(now: Long): List<Press> = when (phase) {
        Phase.PENDING -> if (now - at >= HOLD_MS) hold() else emptyList()
        Phase.TAPPED -> if (now - at > DOUBLE_MS) click(button!!).also { reset() } else emptyList()
        else -> emptyList()
    }

    /** 光标要移动了：键还按着没定性时先按下，保证拖拽从起点开始。 */
    fun beforeMove(): List<Press> = if (phase == Phase.PENDING) hold() else emptyList()

    /** 下一个需要 [tick] 的时刻；没有返回 null。 */
    fun deadline(): Long? = when (phase) {
        Phase.PENDING -> at + HOLD_MS
        Phase.TAPPED -> at + DOUBLE_MS + 1
        else -> null
    }

    /** 离开画面、切到触控模式、隐藏按键时：按着的键抬起，没发出的单击丢掉。 */
    fun cancel(): List<Press> {
        val out = if (phase == Phase.HELD || phase == Phase.LOCKED) listOf(Press(button!!, false)) else emptyList()
        reset()
        swallowUp = false
        return out
    }

    private fun hold(): List<Press> {
        phase = Phase.HELD
        return listOf(Press(button!!, true))
    }

    private fun finish(): List<Press> {
        val out = when (phase) {
            Phase.PENDING -> click(button!!)
            Phase.HELD, Phase.LOCKED -> listOf(Press(button!!, false))
            else -> emptyList()
        }
        reset()
        return out
    }

    private fun click(b: PointerButton) = listOf(Press(b, true), Press(b, false))

    private fun reset() {
        phase = Phase.IDLE
        button = null
    }

    companion object {
        const val HOLD_MS = 200L
        const val DOUBLE_MS = 260L
    }
}

/** 界面层包一层：状态变化触发重组。 */
@Stable
class MouseKeysState {
    val logic = MouseKeysLogic()
    var version by mutableIntStateOf(0)
        private set

    fun run(block: MouseKeysLogic.() -> List<MouseKeysLogic.Press>): List<MouseKeysLogic.Press> =
        logic.block().also { version++ }
}

/**
 * 画面左侧的悬浮鼠标按键条：顶部把手（只有拖把手才挪位置）、左键、右键。
 * [fx]/[fy] 是条左上角在可用区域里的比例位置，拖完把手调 [onMoved] 记到本机设置。
 */
@Composable
fun MouseKeysBar(
    keys: MouseKeysState,
    fx: Float,
    fy: Float,
    areaW: Float,
    areaH: Float,
    onPress: (List<MouseKeysLogic.Press>) -> Unit,
    onMoved: (Float, Float) -> Unit,
    modifier: Modifier = Modifier,
) {
    val density = LocalDensity.current
    val barW = with(density) { BAR_W.toPx() }
    val barH = with(density) { BAR_H.toPx() }
    val maxX = (areaW - barW).coerceAtLeast(0f)
    val maxY = (areaH - barH).coerceAtLeast(0f)
    var pos by remember { mutableStateOf(fx to fy) }
    val currentOnPress by rememberUpdatedState(onPress)
    val currentOnMoved by rememberUpdatedState(onMoved)

    // 到时间点推进状态机（按住定性、单击发出）
    LaunchedEffect(keys.version) {
        val d = keys.logic.deadline() ?: return@LaunchedEffect
        delay((d - Clock.now()).coerceAtLeast(0))
        currentOnPress(keys.run { tick(Clock.now()) })
    }

    val v = keys.version
    val x = (pos.first * maxX).roundToInt()
    val y = (pos.second * maxY).roundToInt()
    Column(
        modifier
            .offset { IntOffset(x, y) }
            .width(BAR_W)
            .height(BAR_H)
            .clip(RoundedCornerShape(14.dp))
            .background(Color(0xCC1B1E2B))
            .border(1.dp, DockColors.Line, RoundedCornerShape(14.dp))
            .testTag("mouse-keys"),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        val moveDesc = stringResource(R.string.rd_mouse_keys_move)
        Box(
            Modifier
                .fillMaxWidth()
                .height(26.dp)
                .semantics { contentDescription = moveDesc }
                .testTag("mouse-keys-handle")
                .pointerInput(maxX, maxY) {
                    detectDragGestures(
                        onDragEnd = { currentOnMoved(pos.first, pos.second) },
                        onDragCancel = { currentOnMoved(pos.first, pos.second) },
                    ) { change, drag ->
                        change.consume()
                        val nx = if (maxX > 0) (pos.first * maxX + drag.x).coerceIn(0f, maxX) / maxX else 0f
                        val ny = if (maxY > 0) (pos.second * maxY + drag.y).coerceIn(0f, maxY) / maxY else 0f
                        pos = nx to ny
                    }
                },
            contentAlignment = Alignment.Center,
        ) {
            Icon(Icons.Rounded.DragIndicator, null, tint = DockColors.Dim, modifier = Modifier.size(18.dp))
        }
        KeyButton(PointerButton.LEFT, stringResource(R.string.rd_mouse_left), keys, v, currentOnPress)
        Box(Modifier.padding(horizontal = 8.dp).fillMaxWidth().height(1.dp).background(DockColors.Line))
        KeyButton(PointerButton.RIGHT, stringResource(R.string.rd_mouse_right), keys, v, currentOnPress)
    }
}

@Composable
private fun KeyButton(
    button: PointerButton,
    label: String,
    keys: MouseKeysState,
    @Suppress("UNUSED_PARAMETER") version: Int,
    onPress: (List<MouseKeysLogic.Press>) -> Unit,
) {
    val l = keys.logic
    val active = l.button == button && l.isDown
    val locked = active && l.isLocked
    Box(
        Modifier
            .padding(4.dp)
            .fillMaxWidth()
            .height(KEY_H)
            .clip(RoundedCornerShape(10.dp))
            .background(if (active) DockColors.Accent else Color.Transparent)
            .semantics { contentDescription = label }
            .testTag(if (button == PointerButton.LEFT) "mouse-key-left" else "mouse-key-right")
            .pointerInput(button) {
                awaitEachGesture {
                    val d = awaitFirstDown()
                    d.consume()
                    onPress(keys.run { down(button, Clock.now()) })
                    // 等这根手指抬起（期间的移动不挪位置，也不当成别的手势）
                    while (true) {
                        val e = awaitPointerEvent()
                        val c = e.changes.firstOrNull { it.id == d.id } ?: break
                        c.consume()
                        if (!c.pressed) break
                    }
                    onPress(keys.run { up(button, Clock.now()) })
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        val fg = if (active) DockColors.OnAccent else Color.White
        Text(label, color = fg, fontSize = 13.sp)
        if (locked) {
            Icon(
                Icons.Rounded.Lock, null, tint = fg,
                modifier = Modifier.align(Alignment.TopEnd).padding(3.dp).size(11.dp),
            )
        }
    }
}

private val BAR_W = 52.dp
private val KEY_H = 56.dp
private val BAR_H = 26.dp + (KEY_H + 8.dp) * 2 + 1.dp
