package cn.lemonit.galaxy

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.click
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.pinch
import androidx.compose.ui.test.swipe
import androidx.test.ext.junit.runners.AndroidJUnit4
import cn.lemonit.galaxy.core.bridge.DisplayInfo
import cn.lemonit.galaxy.core.bridge.InputEvent
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.PointerButton
import cn.lemonit.galaxy.data.VideoQuality
import cn.lemonit.galaxy.ui.remote.RemoteActions
import cn.lemonit.galaxy.ui.remote.RemoteDesktopScreen
import cn.lemonit.galaxy.ui.remote.RemoteStatus
import cn.lemonit.galaxy.ui.remote.RemoteUi
import cn.lemonit.galaxy.ui.remote.RemoteViewState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** 远程画面手势在真实 Compose 布局里的效果：画面比例、捏合中心、点按坐标。 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w860dp-h400dp-land-xhdpi")
class RemoteGestureTest {
    @get:Rule
    val compose = createComposeRule()

    private val events = mutableListOf<InputEvent>()
    private val switched = mutableListOf<UInt>()

    private fun ui(mouse: Boolean, clientCursor: Boolean = false) = RemoteUi(
        title = "PC",
        platform = Platform.WINDOWS,
        status = RemoteStatus.STREAMING,
        displays = listOf(
            DisplayInfo(1u, "D1", 1920u, 1080u, 1f, true, false),
            DisplayInfo(2u, "D2", 2560u, 1440u, 1f, false, false),
            DisplayInfo(3u, "D3", 1280u, 1024u, 1f, false, false),
        ),
        displayId = 1u,
        quality = VideoQuality.AUTO,
        mouseMode = mouse,
        clientCursor = clientCursor,
        cursorX = 100,
        cursorY = 200,
        videoW = 1920,
        videoH = 1080,
    )

    private fun show(mouse: Boolean = false, clientCursor: Boolean = false): RemoteViewState {
        val state = RemoteViewState()
        val actions = object : RemoteActions {
            override fun onInput(events: List<InputEvent>) {
                this@RemoteGestureTest.events += events
            }
            override fun onDisplay(id: UInt) {
                switched += id
            }
            override fun onQuality(q: VideoQuality) = Unit
            override fun onMouseMode(on: Boolean) = Unit
            override fun onClipboardSend() = Unit
            override fun onClipboardGet() = Unit
            override fun onDisconnect() = Unit
        }
        compose.setContent {
            RemoteDesktopScreen(ui(mouse, clientCursor), actions, video = { m -> Box(m.background(Color.Gray)) }, state = state)
        }
        compose.waitForIdle()
        return state
    }

    private fun videoAspect(): Float {
        val size = compose.onNodeWithTag("remote-video").fetchSemanticsNode().size
        return size.width.toFloat() / size.height
    }

    @Test
    fun pinchKeepsAspectAndFocalPoint() {
        val state = show()
        assertEquals(1920f / 1080f, videoAspect(), 0.01f)
        // 视图 1720x800、画面适应后 1422x800：质心取水平中线上，放大全程不触发“不露黑边”的夹持
        val c = Offset(860f, 260f)
        val before = state.viewport.toRemoteF(c.x, c.y)
        compose.onNodeWithTag("remote-surface").performTouchInput {
            pinch(c - Offset(30f, 0f), c - Offset(300f, 0f), c + Offset(30f, 0f), c + Offset(300f, 0f), 500)
        }
        compose.waitForIdle()
        val v = state.viewport
        assertTrue("zoom=${v.zoom}", v.zoom > 3f)
        // 放大后画面比整个视图还大：宽高仍按同一比例（旧实现会被父布局夹成屏幕大小而变形）
        assertTrue(v.contentW > v.containerW && v.contentH > v.containerH)
        assertEquals(1920f / 1080f, videoAspect(), 0.01f)
        // 双指中心下的画面点不动（误差 1 个被控像素内）
        val after = v.toRemoteF(c.x, c.y)
        assertEquals(before.first, after.first, 1f)
        assertEquals(before.second, after.second, 1f)
        // 捏合不发任何输入
        assertTrue(events.isEmpty())
    }

    @Test
    fun tapLandsUnderFingerWhenZoomed() {
        val state = show()
        compose.runOnIdle { state.viewport = state.viewport.zoomedTo(2.7f, 900f, 200f).panned(-150f, 40f) }
        compose.waitForIdle()
        val p = Offset(1234f, 567f)
        compose.onNodeWithTag("remote-surface").performTouchInput { click(p) }
        compose.waitForIdle()
        val expected = state.viewport.toRemote(p.x, p.y)
        val move = events.filterIsInstance<InputEvent.MouseMove>().single()
        assertEquals(expected, move.x to move.y)
        val buttons = events.filterIsInstance<InputEvent.MouseButton>()
        assertEquals(listOf(true, false), buttons.map { it.down })
        assertTrue(buttons.all { it.button == PointerButton.LEFT && it.x to it.y == expected })
    }

    @Test
    fun twoFingerDoubleTapTogglesZoomWithoutRightClick() {
        val state = show()
        fun twoTap() = compose.onNodeWithTag("remote-surface").performTouchInput {
            down(0, Offset(800f, 300f))
            down(1, Offset(900f, 300f))
            advanceEventTime(50)
            up(0)
            up(1)
        }
        twoTap()
        compose.onNodeWithTag("remote-surface").performTouchInput { advanceEventTime(120) }
        twoTap()
        compose.waitForIdle()
        assertTrue(state.viewport.isZoomed)
        compose.mainClock.advanceTimeBy(1000)
        assertTrue("双击不该发右键：$events", events.isEmpty())
        // 单独一次双指轻点：等过双击间隔后才发右键
        twoTap()
        compose.mainClock.advanceTimeBy(1000)
        compose.waitForIdle()
        val right = events.filterIsInstance<InputEvent.MouseButton>()
        assertEquals(2, right.size)
        assertTrue(right.all { it.button == PointerButton.RIGHT })
    }

    @Test
    fun swipeOnHandleSwitchesDisplay() {
        show()
        compose.onNodeWithTag("remote-dock-handle").performTouchInput {
            down(center)
            repeat(8) { moveBy(Offset(0f, -12f)) }
            up()
        }
        compose.waitForIdle()
        assertEquals(listOf(2u), switched)
        compose.onNodeWithTag("remote-notice").assertExists()
        // 往下滑：循环回到上一块（这里界面没跟着换屏，仍从第 1 块算 → 第 3 块）
        compose.onNodeWithTag("remote-dock-handle").performTouchInput {
            down(center)
            repeat(8) { moveBy(Offset(0f, 12f)) }
            up()
        }
        compose.waitForIdle()
        assertEquals(listOf(2u, 3u), switched)
        // 轻点展开菜单，菜单上滑动同样切换
        compose.onNodeWithTag("remote-dock-handle").performTouchInput { click(center) }
        compose.waitForIdle()
        compose.onNodeWithTag("remote-dock-rail").performTouchInput { swipe(center + Offset(0f, 150f), center - Offset(0f, 150f), 200) }
        compose.waitForIdle()
        assertEquals(listOf(2u, 3u, 2u), switched)
        assertTrue(events.isEmpty())
    }

    @Test
    fun pinchShowsZoomNotice() {
        show()
        compose.onNodeWithTag("remote-surface").performTouchInput {
            pinch(Offset(820f, 300f), Offset(600f, 300f), Offset(900f, 300f), Offset(1120f, 300f), 400)
        }
        compose.waitForIdle()
        compose.onNodeWithTag("remote-notice").assertExists()
    }

    @Test
    fun mouseModeTracksPointerLocallyAndDrawsArrow() {
        val state = show(mouse = true, clientCursor = true)
        compose.onNodeWithTag("remote-local-pointer").assertExists()
        // 连着两次滑动：第二次从第一次结束处接着走（本机记位置，不等被控端回报）
        repeat(2) {
            compose.onNodeWithTag("remote-surface").performTouchInput { swipe(Offset(600f, 400f), Offset(700f, 400f), 200) }
            compose.waitForIdle()
        }
        val moves = events.filterIsInstance<InputEvent.MouseMove>()
        assertTrue(moves.isNotEmpty())
        val last = moves.last()
        assertEquals(state.pointer.x to state.pointer.y, last.x to last.y)
        // 两次各移动约 100 视图像素 × 1.6 加速 × 每视图像素的被控像素
        val expected = 100 + 2 * 100f * 1.6f * state.viewport.remotePerViewPx()
        assertEquals(expected, last.x.toFloat(), 40f)
        assertEquals(200, last.y)
    }

    @Test
    fun noLocalArrowWhenHostDrawsCursor() {
        show(mouse = true, clientCursor = false)
        compose.onNodeWithTag("remote-local-pointer").assertDoesNotExist()
    }

    @Test
    fun noLocalArrowInTouchMode() {
        show(mouse = false, clientCursor = true)
        compose.onNodeWithTag("remote-local-pointer").assertDoesNotExist()
    }
}
