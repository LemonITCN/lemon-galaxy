package cn.lemonit.galaxy

import cn.lemonit.galaxy.ui.remote.Viewport
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** 远程画面缩放 / 平移 / 坐标换算（纯逻辑）。 */
class ViewportTest {
    private val eps = 0.01f

    /** 画面宽高比与被控屏幕一致。 */
    private fun assertAspect(v: Viewport) =
        assertEquals(v.remoteW.toFloat() / v.remoteH, v.contentW / v.contentH, 1e-4f)

    /** 视图里点 (x, y) 下面的被控坐标（浮点）。 */
    private fun under(v: Viewport, x: Float, y: Float) = v.toRemoteF(x, y)

    private fun assertSamePoint(a: Pair<Float, Float>, b: Pair<Float, Float>, tol: Float = 0.01f) {
        assertEquals(a.first, b.first, tol)
        assertEquals(a.second, b.second, tol)
    }

    @Test
    fun fitIsLetterboxedAndCentered() {
        // 1920x1080 放进 1000x1000：宽度占满，上下留黑边
        val v = Viewport(1000f, 1000f, 1920, 1080)
        assertEquals(1000f, v.contentW, eps)
        assertEquals(562.5f, v.contentH, eps)
        assertEquals(0f, v.left, eps)
        assertEquals(218.75f, v.top, eps)
        assertAspect(v)
        // 竖屏手机看 16:10 的 Mac：同样不变形
        val p = Viewport(1440f, 3200f, 2560, 1600)
        assertEquals(1440f, p.contentW, eps)
        assertEquals(900f, p.contentH, eps)
        assertAspect(p)
        // 超宽视图：左右留黑边
        val l = Viewport(3200f, 1440f, 1920, 1080)
        assertEquals(1440f, l.contentH, eps)
        assertEquals(2560f, l.contentW, eps)
        assertEquals(320f, l.left, eps)
    }

    @Test
    fun mappingClampsAndRoundTrips() {
        val v = Viewport(1000f, 1000f, 1920, 1080)
        assertEquals(960 to 540, v.toRemote(500f, 500f))
        assertEquals(0 to 0, v.toRemote(-50f, 0f))
        assertEquals(1919 to 1079, v.toRemote(5000f, 5000f))
        // 各种倍数、平移下，被控像素 → 视图（像素中心）→ 被控像素 都回到原处
        val views = listOf(
            v,
            v.zoomedTo(2f, 300f, 400f),
            v.zoomedTo(3.7f, 900f, 100f).panned(-123f, 45f),
            Viewport(1440f, 3200f, 2560, 1600).zoomedTo(5f, 100f, 1600f),
            Viewport(3200f, 1440f, 3840, 2160).zoomedTo(1.3f, 3000f, 10f),
        )
        for (z in views) {
            for ((rx, ry) in listOf(0 to 0, 1 to 1, 17 to 933, z.remoteW / 2 to z.remoteH / 3, z.remoteW - 1 to z.remoteH - 1)) {
                val (x, y) = z.toView(rx, ry)
                assertEquals("zoom=${z.zoom} ($rx,$ry)", rx to ry, z.toRemote(x, y))
            }
        }
    }

    @Test
    fun pinchKeepsAspectAndFocalPoint() {
        // 视图与画面同比例：一放大两个方向都超出视图，双指中心下的画面点全程不动
        val v = Viewport(1920f, 1080f, 2560, 1440)
        val focal = 700f to 300f
        val before = under(v, focal.first, focal.second)
        var z = v
        // 一次捏合分多步：每步都绕同一质心
        repeat(10) {
            z = z.pinched(1.12f, focal.first, focal.second, focal.first, focal.second)
            assertAspect(z)
            assertSamePoint(before, under(z, focal.first, focal.second), 0.05f)
        }
        assertTrue(z.zoom > 3f)
        // 缩回去也一样
        val back = z.pinched(0.5f, focal.first, focal.second, focal.first, focal.second)
        assertAspect(back)
        assertSamePoint(before, under(back, focal.first, focal.second), 0.05f)
    }

    @Test
    fun pinchOnLetterboxedAxisKeepsFocalAndRecenters() {
        // 宽屏手机（20:9）看 16:9：左右留黑边，从适应屏幕开始捏合，双指中心下的点也不动
        val v = Viewport(2400f, 1080f, 1920, 1080)
        val before = under(v, 700f, 300f)
        var z = v
        // 画面在视图里还放得下（且不触边）的范围内全程不动；再放大触到边后按“不露黑边”夹住
        repeat(3) {
            z = z.pinched(1.05f, 700f, 300f, 700f, 300f)
            assertAspect(z)
            assertSamePoint(before, under(z, 700f, 300f), 0.05f)
            // 画面不出视图
            assertTrue(z.left >= -0.01f && z.left + z.contentW <= 2400.01f)
        }
        // 缩回 1 倍：正好回到居中
        val fit = z.pinched(0.01f, 1500f, 900f, 1500f, 900f)
        assertEquals(1f, fit.zoom, 0.001f)
        assertEquals(0f, fit.panX, eps)
        assertEquals(0f, fit.panY, eps)
    }

    @Test
    fun pinchFollowsMovingCentroid() {
        // 捏合同时双指移动：原来在旧质心下的点跟到新质心下
        val v = Viewport(2400f, 1080f, 1920, 1080).zoomedTo(3f, 1200f, 540f)
        val from = 1000f to 500f
        val to = 1060f to 470f
        val p = under(v, from.first, from.second)
        val z = v.pinched(1.2f, from.first, from.second, to.first, to.second)
        assertSamePoint(p, under(z, to.first, to.second), 0.05f)
    }

    @Test
    fun zoomLimitsAndClamp() {
        val v = Viewport(1000f, 562.5f, 1920, 1080)
        // 缩小不低于适应屏幕，放大不超过上限
        assertEquals(1f, v.pinched(0.2f, 0f, 0f, 0f, 0f).zoom, 0.001f)
        assertEquals(v.maxZoom, v.pinched(100f, 10f, 10f, 10f, 10f).zoom, 0.001f)
        // 平移被夹住：不露黑边
        val z = v.zoomedTo(2f, 250f, 140f)
        val far = z.panned(10_000f, 10_000f)
        assertEquals(0f, far.left, eps)
        assertEquals(0f, far.top, eps)
        val far2 = z.panned(-10_000f, -10_000f)
        assertEquals(1000f, far2.left + far2.contentW, eps)
        assertEquals(562.5f, far2.top + far2.contentH, eps)
        // 某一方向比视图小：这一方向画面不出视图
        val tall = Viewport(1000f, 1000f, 1920, 1080).zoomedTo(1.5f, 500f, 500f).panned(0f, 3000f)
        assertEquals(1000f, tall.top + tall.contentH, eps)
        // 1 倍时无论怎么拖都居中
        val still = Viewport(1000f, 1000f, 1920, 1080).panned(0f, 300f)
        assertEquals(218.75f, still.top, eps)
    }

    @Test
    fun toggleZoomAroundTapAndBack() {
        val v = Viewport(2400f, 1080f, 1920, 1080)
        val p = under(v, 900f, 400f)
        val z = v.toggled(900f, 400f)
        assertTrue(z.isZoomed)
        assertEquals(2f, z.zoom, 0.001f)
        assertSamePoint(p, under(z, 900f, 400f), 0.05f)
        val back = z.toggled(10f, 10f)
        assertFalse(back.isZoomed)
        assertEquals(0f, back.panX, eps)
        assertEquals(0f, back.panY, eps)
        // 被控分辨率高（4K 放进 1080 高的视图）：放到原始像素
        val k = Viewport(2400f, 1080f, 3840, 2160)
        assertEquals(2f, k.actualSizeZoom, 0.001f)
        val big = Viewport(1200f, 540f, 3840, 2160)
        assertEquals(4f, big.toggled(600f, 270f).zoom, 0.001f)
    }

    @Test
    fun rotationKeepsZoomAndCenter() {
        val v = Viewport(2400f, 1080f, 1920, 1080).zoomedTo(3f, 1100f, 500f)
        val center = under(v, 1200f, 540f)
        val r = v.resized(1080f, 2400f, 1920, 1080)
        assertEquals(3f, r.zoom, 0.001f)
        assertAspect(r)
        // 竖过来后原来在视图中心的画面点仍在中心
        assertSamePoint(center, under(r, 540f, 1200f), 0.05f)
        // 再转回来：倍数不变、中心不变
        val back = r.resized(2400f, 1080f, 1920, 1080)
        assertEquals(3f, back.zoom, 0.001f)
        assertSamePoint(center, under(back, 1200f, 540f), 0.05f)
        // 尺寸不变时原样返回
        assertTrue(r === r.resized(1080f, 2400f, 1920, 1080))
    }

    @Test
    fun remoteResolutionChangeResetsToFit() {
        val v = Viewport(2400f, 1080f, 1920, 1080).zoomedTo(3f, 600f, 300f)
        val r = v.resized(2400f, 1080f, 2560, 1600)
        assertEquals(1f, r.zoom, 0.001f)
        assertEquals(2560, r.remoteW)
        assertAspect(r)
        assertEquals((2400f - r.contentW) / 2f, r.left, eps)
    }

    @Test
    fun followKeepsCursorVisible() {
        val v = Viewport(1000f, 562.5f, 1920, 1080).zoomedTo(3f, 0f, 0f)
        val f = v.follow(1900, 1000)
        val (x, y) = f.toView(1900, 1000)
        assertTrue(x in 0f..1000f)
        assertTrue(y in 0f..562.5f)
        // 没放大时不动
        val fit = Viewport(1000f, 562.5f, 1920, 1080)
        assertTrue(fit === fit.follow(1900, 1000))
    }
}
