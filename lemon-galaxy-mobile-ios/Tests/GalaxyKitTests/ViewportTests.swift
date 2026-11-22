import XCTest
import CoreGraphics
@testable import GalaxyKit

/// 远程画面缩放 / 平移 / 坐标换算（与安卓端 ViewportTest.kt 逐条对应）
final class ViewportTests: XCTestCase {
    private let eps: CGFloat = 0.01

    /// 画面宽高比与被控屏幕一致
    private func assertAspect(_ v: Viewport, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertEqual(CGFloat(v.remoteW) / CGFloat(v.remoteH), v.contentW / v.contentH, accuracy: 1e-4, file: file, line: line)
    }

    /// 视图里点 (x, y) 下面的被控坐标（浮点）
    private func under(_ v: Viewport, _ x: CGFloat, _ y: CGFloat) -> CGPoint { v.toRemoteF(x, y) }

    private func assertSamePoint(_ a: CGPoint, _ b: CGPoint, _ tol: CGFloat = 0.01, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertEqual(a.x, b.x, accuracy: tol, file: file, line: line)
        XCTAssertEqual(a.y, b.y, accuracy: tol, file: file, line: line)
    }

    private func pinch(_ v: Viewport, _ factor: CGFloat, _ x: CGFloat, _ y: CGFloat) -> Viewport {
        v.pinched(factor, from: CGPoint(x: x, y: y), to: CGPoint(x: x, y: y))
    }

    func testFitIsLetterboxedAndCentered() {
        // 1920x1080 放进 1000x1000：宽度占满，上下留黑边
        let v = Viewport(containerW: 1000, containerH: 1000, remoteW: 1920, remoteH: 1080)
        XCTAssertEqual(v.contentW, 1000, accuracy: eps)
        XCTAssertEqual(v.contentH, 562.5, accuracy: eps)
        XCTAssertEqual(v.left, 0, accuracy: eps)
        XCTAssertEqual(v.top, 218.75, accuracy: eps)
        assertAspect(v)
        // 竖屏手机看 16:10 的 Mac：同样不变形
        let p = Viewport(containerW: 1440, containerH: 3200, remoteW: 2560, remoteH: 1600)
        XCTAssertEqual(p.contentW, 1440, accuracy: eps)
        XCTAssertEqual(p.contentH, 900, accuracy: eps)
        assertAspect(p)
        // 超宽视图：左右留黑边
        let l = Viewport(containerW: 3200, containerH: 1440, remoteW: 1920, remoteH: 1080)
        XCTAssertEqual(l.contentH, 1440, accuracy: eps)
        XCTAssertEqual(l.contentW, 2560, accuracy: eps)
        XCTAssertEqual(l.left, 320, accuracy: eps)
    }

    func testMappingClampsAndRoundTrips() {
        let v = Viewport(containerW: 1000, containerH: 1000, remoteW: 1920, remoteH: 1080)
        XCTAssertTrue(v.toRemote(500, 500) == (960, 540))
        XCTAssertTrue(v.toRemote(-50, 0) == (0, 0))
        XCTAssertTrue(v.toRemote(5000, 5000) == (1919, 1079))
        // 各种倍数、平移下，被控像素 → 视图（像素中心）→ 被控像素 都回到原处
        let views: [Viewport] = [
            v,
            v.zoomedTo(2, fx: 300, fy: 400),
            v.zoomedTo(3.7, fx: 900, fy: 100).panned(-123, 45),
            Viewport(containerW: 1440, containerH: 3200, remoteW: 2560, remoteH: 1600).zoomedTo(5, fx: 100, fy: 1600),
            Viewport(containerW: 3200, containerH: 1440, remoteW: 3840, remoteH: 2160).zoomedTo(1.3, fx: 3000, fy: 10),
            // iPhone 竖屏（点）看 4K：一个视图点小于一个被控像素
            Viewport(containerW: 393, containerH: 852, remoteW: 3840, remoteH: 2160).zoomedTo(4.2, fx: 200, fy: 400).panned(37, -12),
        ]
        for z in views {
            let points: [(Int32, Int32)] = [(0, 0), (1, 1), (17, 933), (Int32(z.remoteW / 2), Int32(z.remoteH / 3)), (Int32(z.remoteW - 1), Int32(z.remoteH - 1))]
            for (rx, ry) in points {
                let p = z.toView(rx, ry)
                let back = z.toRemote(p.x, p.y)
                XCTAssertTrue(back == (rx, ry), "zoom=\(z.zoom) (\(rx),\(ry)) -> \(back)")
            }
        }
    }

    func testPinchKeepsAspectAndFocalPoint() {
        // 视图与画面同比例：一放大两个方向都超出视图，双指中心下的画面点全程不动
        let v = Viewport(containerW: 1920, containerH: 1080, remoteW: 2560, remoteH: 1440)
        let before = under(v, 700, 300)
        var z = v
        // 一次捏合分多步：每步都绕同一质心
        for _ in 0..<10 {
            z = pinch(z, 1.12, 700, 300)
            assertAspect(z)
            assertSamePoint(before, under(z, 700, 300), 0.05)
        }
        XCTAssertGreaterThan(z.zoom, 3)
        // 缩回去也一样
        let back = pinch(z, 0.5, 700, 300)
        assertAspect(back)
        assertSamePoint(before, under(back, 700, 300), 0.05)
    }

    func testPinchOnLetterboxedAxisKeepsFocalAndRecenters() {
        // 宽屏手机（20:9）看 16:9：左右留黑边，从适应屏幕开始捏合，双指中心下的点也不动
        let v = Viewport(containerW: 2400, containerH: 1080, remoteW: 1920, remoteH: 1080)
        let before = under(v, 700, 300)
        var z = v
        for _ in 0..<3 {
            z = pinch(z, 1.05, 700, 300)
            assertAspect(z)
            assertSamePoint(before, under(z, 700, 300), 0.05)
            // 画面不出视图
            XCTAssertTrue(z.left >= -0.01 && z.left + z.contentW <= 2400.01)
        }
        // 缩回 1 倍：正好回到居中
        let fit = pinch(z, 0.01, 1500, 900)
        XCTAssertEqual(fit.zoom, 1, accuracy: 0.001)
        XCTAssertEqual(fit.panX, 0, accuracy: eps)
        XCTAssertEqual(fit.panY, 0, accuracy: eps)
    }

    func testPinchFollowsMovingCentroid() {
        // 捏合同时双指移动：原来在旧质心下的点跟到新质心下
        let v = Viewport(containerW: 2400, containerH: 1080, remoteW: 1920, remoteH: 1080).zoomedTo(3, fx: 1200, fy: 540)
        let from = CGPoint(x: 1000, y: 500), to = CGPoint(x: 1060, y: 470)
        let p = under(v, from.x, from.y)
        let z = v.pinched(1.2, from: from, to: to)
        assertSamePoint(p, under(z, to.x, to.y), 0.05)
    }

    func testZoomLimitsAndClamp() {
        let v = Viewport(containerW: 1000, containerH: 562.5, remoteW: 1920, remoteH: 1080)
        // 缩小不低于适应屏幕，放大不超过上限
        XCTAssertEqual(pinch(v, 0.2, 0, 0).zoom, 1, accuracy: 0.001)
        XCTAssertEqual(pinch(v, 100, 10, 10).zoom, v.maxZoom, accuracy: 0.001)
        // 平移被夹住：不露黑边
        let z = v.zoomedTo(2, fx: 250, fy: 140)
        let far = z.panned(10_000, 10_000)
        XCTAssertEqual(far.left, 0, accuracy: eps)
        XCTAssertEqual(far.top, 0, accuracy: eps)
        let far2 = z.panned(-10_000, -10_000)
        XCTAssertEqual(far2.left + far2.contentW, 1000, accuracy: eps)
        XCTAssertEqual(far2.top + far2.contentH, 562.5, accuracy: eps)
        // 某一方向比视图小：这一方向画面不出视图
        let tall = Viewport(containerW: 1000, containerH: 1000, remoteW: 1920, remoteH: 1080).zoomedTo(1.5, fx: 500, fy: 500).panned(0, 3000)
        XCTAssertEqual(tall.top + tall.contentH, 1000, accuracy: eps)
        // 1 倍时无论怎么拖都居中
        let still = Viewport(containerW: 1000, containerH: 1000, remoteW: 1920, remoteH: 1080).panned(0, 300)
        XCTAssertEqual(still.top, 218.75, accuracy: eps)
        // 被控分辨率高、视图小（点）时上限放宽到 1 个被控像素占 1.5 个点
        let phone = Viewport(containerW: 393, containerH: 852, remoteW: 3840, remoteH: 2160)
        XCTAssertEqual(phone.maxZoom, 1.5 / phone.base, accuracy: 0.001)
        XCTAssertGreaterThan(phone.maxZoom, Viewport.minMaxZoom)
    }

    func testToggleZoomAroundTapAndBack() {
        let v = Viewport(containerW: 2400, containerH: 1080, remoteW: 1920, remoteH: 1080)
        let p = under(v, 900, 400)
        let z = v.toggled(fx: 900, fy: 400)
        XCTAssertTrue(z.isZoomed)
        XCTAssertEqual(z.zoom, 2, accuracy: 0.001)
        assertSamePoint(p, under(z, 900, 400), 0.05)
        let back = z.toggled(fx: 10, fy: 10)
        XCTAssertFalse(back.isZoomed)
        XCTAssertEqual(back.panX, 0, accuracy: eps)
        XCTAssertEqual(back.panY, 0, accuracy: eps)
        // 被控分辨率高（4K 放进 1080 高的视图）：放到原始像素
        let k = Viewport(containerW: 2400, containerH: 1080, remoteW: 3840, remoteH: 2160)
        XCTAssertEqual(k.actualSizeZoom, 2, accuracy: 0.001)
        let big = Viewport(containerW: 1200, containerH: 540, remoteW: 3840, remoteH: 2160)
        XCTAssertEqual(big.toggled(fx: 600, fy: 270).zoom, 4, accuracy: 0.001)
    }

    func testRotationKeepsZoomAndCenter() {
        let v = Viewport(containerW: 2400, containerH: 1080, remoteW: 1920, remoteH: 1080).zoomedTo(3, fx: 1100, fy: 500)
        let center = under(v, 1200, 540)
        let r = v.resized(containerW: 1080, containerH: 2400, remoteW: 1920, remoteH: 1080)
        XCTAssertEqual(r.zoom, 3, accuracy: 0.001)
        assertAspect(r)
        // 竖过来后原来在视图中心的画面点仍在中心
        assertSamePoint(center, under(r, 540, 1200), 0.05)
        // 再转回来：倍数不变、中心不变
        let back = r.resized(containerW: 2400, containerH: 1080, remoteW: 1920, remoteH: 1080)
        XCTAssertEqual(back.zoom, 3, accuracy: 0.001)
        assertSamePoint(center, under(back, 1200, 540), 0.05)
        // 尺寸不变时原样返回
        XCTAssertEqual(r.resized(containerW: 1080, containerH: 2400, remoteW: 1920, remoteH: 1080), r)
    }

    func testRemoteResolutionChangeResetsToFit() {
        let v = Viewport(containerW: 2400, containerH: 1080, remoteW: 1920, remoteH: 1080).zoomedTo(3, fx: 600, fy: 300)
        let r = v.resized(containerW: 2400, containerH: 1080, remoteW: 2560, remoteH: 1600)
        XCTAssertEqual(r.zoom, 1, accuracy: 0.001)
        XCTAssertEqual(r.remoteW, 2560)
        assertAspect(r)
        XCTAssertEqual(r.left, (2400 - r.contentW) / 2, accuracy: eps)
    }

    func testFirstLayoutFromPlaceholderIsFit() {
        // 画面还没摆放（1x1 占位）时第一次拿到视图尺寸和被控分辨率：适应屏幕、居中
        let v = Viewport(containerW: 1, containerH: 1, remoteW: 1, remoteH: 1)
            .resized(containerW: 852, containerH: 393, remoteW: 1, remoteH: 1)
            .resized(containerW: 852, containerH: 393, remoteW: 2560, remoteH: 1440)
        XCTAssertEqual(v.zoom, 1)
        XCTAssertEqual(v.contentH, 393, accuracy: eps)
        XCTAssertEqual(v.left, (852 - v.contentW) / 2, accuracy: eps)
    }

    func testFollowKeepsCursorVisible() {
        let v = Viewport(containerW: 1000, containerH: 562.5, remoteW: 1920, remoteH: 1080).zoomedTo(3, fx: 0, fy: 0)
        let f = v.follow(1900, 1000)
        let p = f.toView(1900, 1000)
        XCTAssertTrue((0...1000).contains(p.x))
        XCTAssertTrue((0...562.5).contains(p.y))
        // 没放大时不动
        let fit = Viewport(containerW: 1000, containerH: 562.5, remoteW: 1920, remoteH: 1080)
        XCTAssertEqual(fit.follow(1900, 1000), fit)
    }

    func testFlags() {
        let v = Viewport(containerW: 1200, containerH: 540, remoteW: 3840, remoteH: 2160)
        XCTAssertEqual(ViewportFlags(v), ViewportFlags(zoomed: false, canActualSize: true, atActualSize: false))
        let actual = v.zoomedTo(v.actualSizeZoom, fx: 600, fy: 270)
        XCTAssertEqual(ViewportFlags(actual), ViewportFlags(zoomed: true, canActualSize: true, atActualSize: true))
        // 被控分辨率比视图还小：原始像素不可用
        let small = Viewport(containerW: 2400, containerH: 1080, remoteW: 1280, remoteH: 720)
        XCTAssertFalse(ViewportFlags(small).canActualSize)
        XCTAssertEqual(small.actualSizeZoom, 1, accuracy: 0.001)
    }
}
