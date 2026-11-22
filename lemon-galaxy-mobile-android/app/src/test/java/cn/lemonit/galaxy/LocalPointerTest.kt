package cn.lemonit.galaxy

import cn.lemonit.galaxy.ui.remote.LocalPointer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** 本机光标：本机动过 500 ms 内不采用被控端回报的位置（回声），停手后采用（对面有人动了鼠标）。 */
class LocalPointerTest {
    @Test
    fun echoWhileMovingIsIgnored() {
        val p = LocalPointer(100, 100)
        p.moved(120, 100, now = 1_000)
        p.moved(140, 100, now = 1_016)
        // 自己前面那次移动绕回来的旧位置：不往回跳
        assertFalse(p.hostReport(120, 100, true, now = 1_080))
        assertEquals(140 to 100, p.x to p.y)
        assertFalse(p.hostReport(140, 100, true, now = 1_515))
    }

    @Test
    fun hostMoveAfterQuietIsAdopted() {
        val p = LocalPointer(100, 100)
        p.moved(140, 100, now = 1_000)
        assertTrue(p.hostReport(800, 600, true, now = 1_000 + LocalPointer.QUIET_MS))
        assertEquals(800 to 600, p.x to p.y)
        // 从没动过时直接采用
        val q = LocalPointer(0, 0)
        assertTrue(q.hostReport(5, 6, true, now = 0))
        assertEquals(5 to 6, q.x to q.y)
    }

    @Test
    fun visibilityAlwaysFollowsHost() {
        val p = LocalPointer(10, 10)
        p.moved(20, 20, now = 1_000)
        p.hostReport(20, 20, false, now = 1_050)
        assertFalse(p.visible)
        p.hostReport(20, 20, true, now = 1_100)
        assertTrue(p.visible)
        p.hostReport(20, 20, false, now = 1_200)
        p.reset(50, 60)
        assertTrue(p.visible)
        assertEquals(50 to 60, p.x to p.y)
        // reset 后忘掉本机移动时刻：回报立即采用
        assertTrue(p.hostReport(1, 2, true, now = 1_201))
    }
}
