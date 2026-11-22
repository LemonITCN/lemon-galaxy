import XCTest
@testable import GalaxyKit

/// 悬浮鼠标按键：点击 / 按住 / 双击锁定 / 解锁。与安卓端 MouseKeysTest.kt 逐条对应
final class MouseKeysTests: XCTestCase {
    typealias P = MouseKeysLogic.Press
    let click = [P(.left, true), P(.left, false)]
    let hold = MouseKeysLogic.holdInterval
    let dbl = MouseKeysLogic.doubleInterval

    func testTapClicksAfterDoubleWindow() {
        var k = MouseKeysLogic()
        XCTAssertEqual(k.down(.left, now: 0), [])
        XCTAssertEqual(k.up(.left, now: 0.08), [])
        XCTAssertEqual(k.tick(now: 0.08 + dbl), [])
        XCTAssertEqual(k.tick(now: k.deadline!), click)
        XCTAssertEqual(k.phase, .idle)
        XCTAssertNil(k.deadline)
    }

    func testHoldPressesAndReleases() {
        var k = MouseKeysLogic()
        _ = k.down(.left, now: 0)
        XCTAssertEqual(k.tick(now: hold - 0.001), [])
        XCTAssertEqual(k.tick(now: hold), [P(.left, true)])
        XCTAssertTrue(k.isDown)
        XCTAssertFalse(k.isLocked)
        XCTAssertEqual(k.up(.left, now: 1), [P(.left, false)])
        XCTAssertFalse(k.isDown)
    }

    func testMovingWhilePendingPressesImmediately() {
        var k = MouseKeysLogic()
        _ = k.down(.right, now: 0)
        XCTAssertEqual(k.beforeMove(), [P(.right, true)])
        XCTAssertEqual(k.beforeMove(), [])
        XCTAssertEqual(k.up(.right, now: 0.05), [P(.right, false)])
    }

    func testDoubleTapLocksAndTapUnlocks() {
        var k = MouseKeysLogic()
        _ = k.down(.left, now: 0)
        _ = k.up(.left, now: 0.06)
        XCTAssertEqual(k.down(.left, now: 0.15), [P(.left, true)])
        XCTAssertTrue(k.isLocked)
        XCTAssertEqual(k.up(.left, now: 0.2), [])
        XCTAssertTrue(k.isLocked)
        XCTAssertEqual(k.tick(now: 10), [])
        XCTAssertTrue(k.busy)
        XCTAssertEqual(k.down(.left, now: 20), [P(.left, false)])
        XCTAssertEqual(k.up(.left, now: 20.05), [])
        XCTAssertEqual(k.phase, .idle)
        XCTAssertNil(k.deadline)
    }

    func testSlowSecondTapIsTwoClicks() {
        var k = MouseKeysLogic()
        _ = k.down(.left, now: 0)
        _ = k.up(.left, now: 0.05)
        XCTAssertEqual(k.tick(now: 0.05 + dbl + 0.001), click)
        _ = k.down(.left, now: 1)
        XCTAssertFalse(k.isLocked)
    }

    func testOtherKeyFinishesPendingClickFirst() {
        var k = MouseKeysLogic()
        _ = k.down(.left, now: 0)
        _ = k.up(.left, now: 0.05)
        XCTAssertEqual(k.down(.right, now: 0.1), click)
        XCTAssertEqual(k.button, .right)
    }

    func testOtherKeyReleasesLockedKey() {
        var k = MouseKeysLogic()
        _ = k.down(.left, now: 0); _ = k.up(.left, now: 0.05); _ = k.down(.left, now: 0.1); _ = k.up(.left, now: 0.15)
        XCTAssertEqual(k.down(.right, now: 0.5), [P(.left, false)])
        XCTAssertEqual(k.button, .right)
        XCTAssertEqual(k.phase, .pending)
    }

    func testCancelReleasesHeldOrLocked() {
        var k = MouseKeysLogic()
        _ = k.down(.left, now: 0); _ = k.up(.left, now: 0.05); _ = k.down(.left, now: 0.1)
        XCTAssertEqual(k.cancel(), [P(.left, false)])
        XCTAssertEqual(k.up(.left, now: 0.2), [])
        XCTAssertEqual(k.cancel(), [])
        _ = k.down(.left, now: 1); _ = k.up(.left, now: 1.05)
        XCTAssertEqual(k.cancel(), [])
    }
}
