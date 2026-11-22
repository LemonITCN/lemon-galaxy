package cn.lemonit.galaxy

import cn.lemonit.galaxy.core.bridge.PointerButton.LEFT
import cn.lemonit.galaxy.core.bridge.PointerButton.RIGHT
import cn.lemonit.galaxy.ui.remote.MouseKeysLogic
import cn.lemonit.galaxy.ui.remote.MouseKeysLogic.Companion.DOUBLE_MS
import cn.lemonit.galaxy.ui.remote.MouseKeysLogic.Companion.HOLD_MS
import cn.lemonit.galaxy.ui.remote.MouseKeysLogic.Press
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class MouseKeysTest {
    private val click = listOf(Press(LEFT, true), Press(LEFT, false))

    @Test
    fun tapClicksAfterDoubleWindow() {
        val k = MouseKeysLogic()
        assertEquals(emptyList<Press>(), k.down(LEFT, 0))
        assertEquals(emptyList<Press>(), k.up(LEFT, 80))
        // 双击窗口内不发
        assertEquals(emptyList<Press>(), k.tick(80 + DOUBLE_MS))
        assertEquals(click, k.tick(k.deadline()!!))
        assertEquals(MouseKeysLogic.Phase.IDLE, k.phase)
        assertNull(k.deadline())
    }

    @Test
    fun holdPressesAndReleases() {
        val k = MouseKeysLogic()
        k.down(LEFT, 0)
        assertEquals(emptyList<Press>(), k.tick(HOLD_MS - 1))
        assertEquals(listOf(Press(LEFT, true)), k.tick(HOLD_MS))
        assertTrue(k.isDown)
        assertFalse(k.isLocked)
        assertEquals(listOf(Press(LEFT, false)), k.up(LEFT, 1000))
        assertFalse(k.isDown)
    }

    @Test
    fun movingWhilePendingPressesImmediately() {
        val k = MouseKeysLogic()
        k.down(RIGHT, 0)
        assertEquals(listOf(Press(RIGHT, true)), k.beforeMove())
        assertEquals(emptyList<Press>(), k.beforeMove())
        assertEquals(listOf(Press(RIGHT, false)), k.up(RIGHT, 50))
    }

    @Test
    fun doubleTapLocksAndTapUnlocks() {
        val k = MouseKeysLogic()
        k.down(LEFT, 0)
        k.up(LEFT, 60)
        // 第二下按下立即锁定，只发一次按下（不会先发单击）
        assertEquals(listOf(Press(LEFT, true)), k.down(LEFT, 150))
        assertTrue(k.isLocked)
        assertEquals(emptyList<Press>(), k.up(LEFT, 200))
        assertTrue(k.isLocked)
        // 锁定期间不会自己抬起
        assertEquals(emptyList<Press>(), k.tick(10_000))
        assertTrue(k.busy)
        // 再点一下解锁
        assertEquals(listOf(Press(LEFT, false)), k.down(LEFT, 20_000))
        assertEquals(emptyList<Press>(), k.up(LEFT, 20_050))
        assertEquals(MouseKeysLogic.Phase.IDLE, k.phase)
        assertNull(k.deadline())
    }

    @Test
    fun slowSecondTapIsTwoClicks() {
        val k = MouseKeysLogic()
        k.down(LEFT, 0)
        k.up(LEFT, 50)
        assertEquals(click, k.tick(50 + DOUBLE_MS + 1))
        k.down(LEFT, 1000)
        assertFalse(k.isLocked)
    }

    @Test
    fun otherKeyFinishesPendingClickFirst() {
        val k = MouseKeysLogic()
        k.down(LEFT, 0)
        k.up(LEFT, 50)
        assertEquals(click, k.down(RIGHT, 100))
        assertEquals(RIGHT, k.button)
    }

    @Test
    fun otherKeyReleasesLockedKey() {
        val k = MouseKeysLogic()
        k.down(LEFT, 0); k.up(LEFT, 50); k.down(LEFT, 100); k.up(LEFT, 150)
        assertEquals(listOf(Press(LEFT, false)), k.down(RIGHT, 500))
        assertEquals(RIGHT, k.button)
        assertEquals(MouseKeysLogic.Phase.PENDING, k.phase)
    }

    @Test
    fun cancelReleasesHeldOrLocked() {
        val k = MouseKeysLogic()
        k.down(LEFT, 0); k.up(LEFT, 50); k.down(LEFT, 100)
        assertEquals(listOf(Press(LEFT, false)), k.cancel())
        // 被取消的那次按下，松手不再产生事件
        assertEquals(emptyList<Press>(), k.up(LEFT, 200))
        assertEquals(emptyList<Press>(), k.cancel())
        // 没发出的单击直接丢掉
        k.down(LEFT, 1000); k.up(LEFT, 1050)
        assertEquals(emptyList<Press>(), k.cancel())
    }
}
