import XCTest
import CoreGraphics
@testable import GalaxyKit

/// 本机光标：本机动过 0.5 秒内不采用被控端回报的位置（回声），停手后采用（对面有人动了鼠标）。与安卓端 LocalPointerTest.kt 对应
final class LocalPointerTests: XCTestCase {
    func testEchoWhileMovingIsIgnored() {
        var p = LocalPointer(x: 100, y: 100)
        p.moved(120, 100, now: 1.000)
        p.moved(140, 100, now: 1.016)
        // 自己前面那次移动绕回来的旧位置：不往回跳
        XCTAssertFalse(p.hostReport(120, 100, visible: true, now: 1.080))
        XCTAssertEqual(p.x, 140)
        XCTAssertEqual(p.y, 100)
        XCTAssertFalse(p.hostReport(140, 100, visible: true, now: 1.515))
    }

    func testHostMoveAfterQuietIsAdopted() {
        var p = LocalPointer(x: 100, y: 100)
        p.moved(140, 100, now: 1)
        XCTAssertTrue(p.hostReport(800, 600, visible: true, now: 1 + LocalPointer.quietInterval))
        XCTAssertEqual(p.x, 800)
        XCTAssertEqual(p.y, 600)
        // 从没动过时直接采用
        var q = LocalPointer()
        XCTAssertTrue(q.hostReport(5, 6, visible: true, now: 0))
        XCTAssertEqual(q.x, 5)
        XCTAssertEqual(q.y, 6)
    }

    func testVisibilityAlwaysFollowsHost() {
        var p = LocalPointer(x: 10, y: 10)
        p.moved(20, 20, now: 1)
        p.hostReport(20, 20, visible: false, now: 1.05)
        XCTAssertFalse(p.visible)
        p.hostReport(20, 20, visible: true, now: 1.1)
        XCTAssertTrue(p.visible)
        p.hostReport(20, 20, visible: false, now: 1.2)
        p.reset(50, 60)
        XCTAssertTrue(p.visible)
        XCTAssertEqual(p.x, 50)
        // reset 后忘掉本机移动时刻：回报立即采用
        XCTAssertTrue(p.hostReport(1, 2, visible: true, now: 1.201))
    }

    func testVideoToRemoteScales() {
        let r = LocalPointer.videoToRemote(640, 360, video: CGSize(width: 1280, height: 720), remoteW: 2560, remoteH: 1440)
        XCTAssertEqual(r.0, 1280)
        XCTAssertEqual(r.1, 720)
        let same = LocalPointer.videoToRemote(7, 9, video: .zero, remoteW: 1920, remoteH: 1080)
        XCTAssertEqual(same.0, 7)
        XCTAssertEqual(same.1, 9)
    }

    func testArrowTipAtOrigin() {
        let b = PointerArrow.path().boundingBox
        XCTAssertEqual(b.minX, 0, accuracy: 0.001)
        XCTAssertEqual(b.minY, 0, accuracy: 0.001)
        XCTAssertGreaterThan(b.height, 20)
    }
}
