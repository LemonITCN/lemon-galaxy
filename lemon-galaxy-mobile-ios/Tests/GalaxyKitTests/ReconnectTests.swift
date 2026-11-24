import XCTest
@testable import GalaxyKit

/// 远程桌面自动重连的节奏：1、2、4、8、15 秒，之后每 15 秒，10 次（约 2 分钟）后放弃
final class ReconnectTests: XCTestCase {
    func testSchedule() {
        let waits = (0..<ReconnectSchedule.maxAttempts).map { ReconnectSchedule.delay(before: $0) }
        XCTAssertEqual(waits, [1, 2, 4, 8, 15, 15, 15, 15, 15, 15])
        XCTAssertNil(ReconnectSchedule.delay(before: ReconnectSchedule.maxAttempts))
        XCTAssertNil(ReconnectSchedule.delay(before: 99))
        XCTAssertNil(ReconnectSchedule.delay(before: -1))
    }

    func testGivesUpAfterAboutTwoMinutes() {
        // 等待共 105 秒，加上每次尝试本身的耗时约 2 分钟
        XCTAssertEqual(ReconnectSchedule.totalWait, 105)
        XCTAssertGreaterThan(ReconnectSchedule.firstFrameTimeout, 0)
    }
}
