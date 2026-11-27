import XCTest
import Combine
@testable import GalaxyKit

/// 远程升级被控端（Logic/DeviceUpgrade.swift + GalaxyAccount.upgradeDevice）：用模拟内核走成功、失败、本来就最新三条路。
/// 模拟内核里构建服务器能走完整个升级，前台电脑离线且版本太旧，Design-iMac 没有新版本
@MainActor
final class UpgradeTests: XCTestCase {
    let server = "127.0.0.1:17443#sha256-vS0YoCAj63KxITahXYerlvFetFcrpl-Yz0rZJ7ecgw8"

    func loggedIn() async throws -> GalaxyAccount {
        let config = CoreConfig(dataDir: NSTemporaryDirectory(), cacheDir: NSTemporaryDirectory(), logLevel: "info", platform: .ios,
                                deviceKind: .mobile, appVersion: "test", osVersion: "test", hostname: "test", locale: "zh-CN")
        let a = GalaxyAccount(config: config, store: MemorySecureStore(), forceMock: true, mockOptions: MockOptions(latencyScale: 0, liveSimulation: false))
        _ = try await a.login(server: server, username: "alice", password: "x", totp: nil)
        return a
    }

    func waitFor(_ what: String, timeout seconds: Double = 5, _ cond: @escaping () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(seconds)
        while !cond() {
            if Date() > deadline { XCTFail("超时：\(what)"); return }
            try await Task.sleep(nanoseconds: 20_000_000)
        }
    }

    func testWhichDevicesCanUpgrade() async throws {
        let a = try await loggedIn()
        let badged = Set(a.devices.filter { a.hasUpgrade($0) }.map(\.id))
        XCTAssertEqual(badged, ["dev-build", "dev-reception"])
        let supported = Set(a.devices.filter { DeviceUpgrade.supported($0) }.map(\.id))
        // 被控端与别的 Mac 客户端能升级；本机（手机）、安卓、Windows 客户端（被控交给系统服务那个设备）不能
        XCTAssertTrue(supported.isSuperset(of: ["dev-build", "dev-gpu", "dev-imac", "dev-nas", "dev-reception", "dev-mbp"]))
        XCTAssertTrue(supported.isDisjoint(with: ["dev-self", "dev-phone", "dev-office"]))
        // 离线时不能点
        XCTAssertFalse(a.canStartUpgrade(try XCTUnwrap(a.device("dev-reception"))))
        XCTAssertTrue(a.canStartUpgrade(try XCTUnwrap(a.device("dev-build"))))
    }

    func testUpgradeSucceeds() async throws {
        let a = try await loggedIn()
        var seen: [DeviceUpgrade] = []
        let sub = a.$upgrades.sink { all in
            if let s = all["dev-build"], seen.last != s { seen.append(s) }
        }
        var finished: (String, DeviceUpgrade)?
        a.onUpgradeFinished = { id, s in finished = (id, s) }
        a.upgradeDevice("dev-build")
        // 进行中再点不会重复发，也不能点
        XCTAssertFalse(a.canStartUpgrade(try XCTUnwrap(a.device("dev-build"))))
        a.upgradeDevice("dev-build")
        try await waitFor("升级完成") { finished != nil }
        sub.cancel()
        XCTAssertEqual(seen, [.requesting("0.2.0"), .installing("0.2.0"), .restarting("0.2.0"), .done("0.2.0")])
        XCTAssertEqual(finished?.0, "dev-build")
        XCTAssertEqual(finished?.1, .done("0.2.0"))
        try await waitFor("设备列表刷新") { a.device("dev-build")?.appVersion == "0.2.0" }
        let d = try XCTUnwrap(a.device("dev-build"))
        XCTAssertTrue(d.online)
        XCTAssertEqual(d.updateVersion, "")
        XCTAssertFalse(a.hasUpgrade(d))
    }

    func testUpgradeFailsWhenTooOld() async throws {
        let a = try await loggedIn()
        let mock = try XCTUnwrap(a.client as? MockGalaxyClient)
        mock.mockUpdateDevice("dev-reception") { $0.online = true }
        try await waitFor("前台电脑上线") { a.device("dev-reception")?.online == true }
        a.upgradeDevice("dev-reception")
        try await waitFor("升级失败") { if case .failed = a.upgrades["dev-reception"] { return true }; return false }
        guard case .failed(let reason) = a.upgrades["dev-reception"] else { return XCTFail() }
        XCTAssertTrue(reason.contains("太旧"), reason)
        XCTAssertEqual(a.device("dev-reception")?.appVersion, "0.1.0")
        // 失败后可以重试
        XCTAssertTrue(a.canStartUpgrade(try XCTUnwrap(a.device("dev-reception"))))
    }

    func testAlreadyUpToDate() async throws {
        let a = try await loggedIn()
        a.upgradeDevice("dev-imac")
        try await waitFor("已是最新") { a.upgrades["dev-imac"]?.inProgress == false }
        XCTAssertEqual(a.upgrades["dev-imac"], .upToDate("0.1.0"))
    }

    func testOfflineDeviceIsNotRequested() async throws {
        let a = try await loggedIn()
        a.upgradeDevice("dev-reception")
        XCTAssertNil(a.upgrades["dev-reception"])
        a.upgradeDevice("dev-self")
        XCTAssertNil(a.upgrades["dev-self"])
    }
}
