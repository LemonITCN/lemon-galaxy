import XCTest
import CoreGraphics
@testable import GalaxyKit

/// 右侧贴边菜单与双指手势的状态逻辑
final class RemoteDockTests: XCTestCase {
    func testDisplayCycleWrapsBothWays() {
        let ids: [UInt32] = [1, 2, 3]
        XCTAssertEqual(DisplayCycle.next(after: 1, in: ids, step: 1), 2)
        XCTAssertEqual(DisplayCycle.next(after: 3, in: ids, step: 1), 1)
        XCTAssertEqual(DisplayCycle.next(after: 1, in: ids, step: -1), 3)
        // 两块屏：上滑下滑都换到另一块
        XCTAssertEqual(DisplayCycle.next(after: 7, in: [7, 9], step: 1), 9)
        XCTAssertEqual(DisplayCycle.next(after: 7, in: [7, 9], step: -1), 9)
        // 只有一块不切换；当前不在列表里按第 1 块算
        XCTAssertNil(DisplayCycle.next(after: 1, in: [1], step: 1))
        XCTAssertNil(DisplayCycle.next(after: nil, in: [], step: 1))
        XCTAssertEqual(DisplayCycle.next(after: 42, in: ids, step: 1), 2)
        XCTAssertEqual(DisplayCycle.index(of: 3, in: ids), 2)
    }

    func testHandleTapSwipeAndMove() {
        // 不动就抬起：轻点展开
        var t = DockHandleTracker(canSwitch: true)
        XCTAssertEqual(t.moved(to: 4), 0)
        XCTAssertEqual(t.ended(), .tap)
        // 上滑够远：下一块
        t = DockHandleTracker(canSwitch: true)
        for y in stride(from: -6, through: -48, by: -6) { XCTAssertEqual(t.moved(to: CGFloat(y)), 0) }
        XCTAssertEqual(t.mode, .swipe)
        XCTAssertEqual(t.ended(), .switchDisplay(1))
        // 下滑：上一块
        t = DockHandleTracker(canSwitch: true)
        _ = t.moved(to: 40)
        XCTAssertEqual(t.ended(), .switchDisplay(-1))
        // 越过阈值但不够远：什么也不做
        t = DockHandleTracker(canSwitch: true)
        _ = t.moved(to: -20)
        XCTAssertEqual(t.ended(), .none)
        // 长按后拖动：移动把手，抬起不切换
        t = DockHandleTracker(canSwitch: true)
        XCTAssertTrue(t.longPressElapsed())
        XCTAssertFalse(t.longPressElapsed())
        XCTAssertEqual(t.moved(to: 30), 30)
        XCTAssertEqual(t.moved(to: 45), 15)
        XCTAssertEqual(t.ended(), .none)
        // 已经在滑动时长按计时到了也不改成拖动
        t = DockHandleTracker(canSwitch: true)
        _ = t.moved(to: -15)
        XCTAssertFalse(t.longPressElapsed())
        XCTAssertEqual(t.mode, .swipe)
        // 只有一块屏：一动就是拖动改位置（从按下点算起）
        t = DockHandleTracker(canSwitch: false)
        XCTAssertEqual(t.moved(to: 5), 0)
        XCTAssertEqual(t.moved(to: 12), 12)
        XCTAssertEqual(t.moved(to: 20), 8)
        XCTAssertEqual(t.ended(), .none)
    }

    func testRailSwipe() {
        var r = RailSwipeTracker()
        r.moved(to: CGSize(width: 2, height: -5))
        XCTAssertFalse(r.dragging)
        r.moved(to: CGSize(width: 3, height: -40))
        XCTAssertTrue(r.dragging)
        XCTAssertEqual(r.ended(), 1)
        // 横向为主不算
        r = RailSwipeTracker()
        r.moved(to: CGSize(width: 50, height: 30))
        XCTAssertFalse(r.dragging)
        XCTAssertNil(r.ended())
        // 往下滑：上一块
        r = RailSwipeTracker()
        r.moved(to: CGSize(width: 0, height: 60))
        XCTAssertEqual(r.ended(), -1)
    }

    func testHandlePlacementRespectsSafeArea() {
        // iPhone 竖屏：顶部 59（灵动岛）、底部 34（主屏指示条）
        let h: CGFloat = 852
        let r = DockLayout.handleRange(containerH: h, topInset: 59, bottomInset: 34)
        XCTAssertEqual(r.lowerBound, 67)
        XCTAssertEqual(r.upperBound, h - 34 - 8 - DockLayout.handleTouchHeight)
        XCTAssertEqual(DockLayout.handleY(frac: 0, containerH: h, topInset: 59, bottomInset: 34), r.lowerBound)
        XCTAssertEqual(DockLayout.handleY(frac: 1, containerH: h, topInset: 59, bottomInset: 34), r.upperBound)
        XCTAssertEqual(DockLayout.handleY(frac: 3, containerH: h, topInset: 59, bottomInset: 34), r.upperBound)
        // 拖动：按可用范围换算成比例，夹在 0…1
        let span = r.upperBound - r.lowerBound
        XCTAssertEqual(DockLayout.movedFrac(0.4, by: span * 0.1, containerH: h, topInset: 59, bottomInset: 34), 0.5, accuracy: 1e-6)
        XCTAssertEqual(DockLayout.movedFrac(0.4, by: -10_000, containerH: h, topInset: 59, bottomInset: 34), 0)
        // 极矮的视图：范围退化成一个点，不出界
        let tiny = DockLayout.handleRange(containerH: 60, topInset: 0, bottomInset: 0)
        XCTAssertEqual(tiny.lowerBound, tiny.upperBound)
    }

    func testRailFitsWithoutScrolling() {
        // iPhone 横屏可用高度约 393 - 21（主屏指示条）
        let avail: CGFloat = 393 - 21 - 12
        let item = DockLayout.railItemHeight(available: avail, multiDisplay: true)
        XCTAssertGreaterThanOrEqual(item, 36)
        let total = 24 + 30 + 54 + 9 + 12 + item * CGFloat(DockLayout.railItems)
        XCTAssertLessThanOrEqual(total, avail + 0.5)
        // 高的屏幕：每项不超过 54
        XCTAssertEqual(DockLayout.railItemHeight(available: 2000, multiDisplay: false), 54)
        XCTAssertEqual(DockLayout.panelMaxWidth(containerW: 2000), 384)
        XCTAssertEqual(DockLayout.panelMaxWidth(containerW: 200), 200)
    }

    func testDotColor() {
        XCTAssertEqual(DockDot.of(path: .lan, recording: false), .online)
        XCTAssertEqual(DockDot.of(path: .direct, recording: false), .online)
        XCTAssertEqual(DockDot.of(path: .relayUdp, recording: false), .relay)
        XCTAssertEqual(DockDot.of(path: .relayTcp, recording: false), .relay)
        XCTAssertEqual(DockDot.of(path: .connecting, recording: false), .none)
        XCTAssertEqual(DockDot.of(path: nil, recording: false), .none)
        XCTAssertEqual(DockDot.of(path: .lan, recording: true), .recording)
    }

    func testTwoFingerTapVsDoubleTap() {
        var r = TwoFingerTapResolver(interval: 0.3, maxDistance: 60)
        // 第一次：先挂起
        XCTAssertEqual(r.tapped(at: CGPoint(x: 100, y: 100), time: 0), .pending(flushed: false))
        XCTAssertFalse(r.expired(now: 0.2))
        // 间隔内附近再来一次：双击，不发右键
        XCTAssertEqual(r.tapped(at: CGPoint(x: 110, y: 95), time: 0.25), .doubleTap(CGPoint(x: 110, y: 95)))
        XCTAssertFalse(r.hasPending)
        XCTAssertFalse(r.flush())
        // 单独一次：过了间隔交出去当右键
        _ = r.tapped(at: CGPoint(x: 100, y: 100), time: 1)
        XCTAssertTrue(r.expired(now: 1.31))
        XCTAssertTrue(r.flush())
        XCTAssertFalse(r.flush())
        // 离得太远：前一次先当右键，这一次重新挂起
        _ = r.tapped(at: CGPoint(x: 100, y: 100), time: 2)
        XCTAssertEqual(r.tapped(at: CGPoint(x: 400, y: 100), time: 2.1), .pending(flushed: true))
        // 超过间隔：同样不算双击
        XCTAssertEqual(r.tapped(at: CGPoint(x: 400, y: 100), time: 2.5), .pending(flushed: true))
    }
}
