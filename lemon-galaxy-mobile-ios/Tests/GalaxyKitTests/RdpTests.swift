import XCTest
@testable import GalaxyKit

final class RdpLinkTests: XCTestCase {
    func target(host: String, port: UInt16 = 3389, user: String = "Administrator", domain: String? = nil, password: String? = "p@ss&w=rd") -> RdpTarget {
        RdpTarget(label: "srv", host: host, port: port, username: user, domain: domain, password: password, forwarded: false, forwardId: "")
    }

    func testBasicLink() {
        let u = RdpLink.url(for: target(host: "10.0.3.5"))
        XCTAssertEqual(u?.absoluteString, "rdp://full%20address=s:10.0.3.5:3389&username=s:Administrator")
        XCTAssertEqual(u?.scheme, "rdp")
    }

    func testDomainUserIsEncodedAndPasswordNeverIncluded() {
        let u = RdpLink.url(for: target(host: "127.0.0.1", port: 50389, user: "alice smith", domain: "CORP"))!.absoluteString
        XCTAssertEqual(u, "rdp://full%20address=s:127.0.0.1:50389&username=s:CORP%5Calice%20smith")
        XCTAssertFalse(u.contains("p%40ss"))
        XCTAssertFalse(u.lowercased().contains("password"))
    }

    func testSpecialCharactersAndIPv6() {
        // Foundation 的 URL(string:) 不接受这个格式；确认构造出来的 URL 不为空且文字原样保留
        XCTAssertNil(URL(string: RdpLink.string(for: target(host: "10.0.3.5"))))
        let u = RdpLink.url(for: target(host: "fe80::1", user: "a&b=c"))!.absoluteString
        // 不同系统版本的 Foundation 对方括号处理不同（macOS 26 转成 %5B %5D，27 原样保留），Windows App 两种都会解码
        let normalized = u.replacingOccurrences(of: "%5B", with: "[").replacingOccurrences(of: "%5D", with: "]")
        XCTAssertEqual(normalized, "rdp://full%20address=s:[fe80::1]:3389&username=s:a%26b%3Dc")
        // 没有用户名时只带地址
        XCTAssertEqual(RdpLink.url(for: target(host: "pc.example.com", user: ""))?.absoluteString, "rdp://full%20address=s:pc.example.com:3389")
    }
}

@MainActor
final class MockRdpAndPrivacyTests: XCTestCase {
    func makeAccount() -> GalaxyAccount {
        let config = CoreConfig(dataDir: NSTemporaryDirectory(), cacheDir: NSTemporaryDirectory(), logLevel: "info", platform: .ios,
                                deviceKind: .mobile, appVersion: "test", osVersion: "test", hostname: "test", locale: "en")
        return GalaxyAccount(config: config, store: MemorySecureStore(), forceMock: true, mockOptions: MockOptions(latencyScale: 0, liveSimulation: false))
    }

    func testForwardedRdpIsTrackedAndClosedOnLogout() async throws {
        let a = makeAccount()
        _ = try await a.login(server: "127.0.0.1:17443#sha256-vS0YoCAj63KxITahXYerlvFetFcrpl-Yz0rZJ7ecgw8", username: "alice", password: "x", totp: nil)
        // r-win 设了经由 dev-build：经设备转发到本机端口
        let t = try await a.openRdp("r-win")
        XCTAssertTrue(t.forwarded)
        XCTAssertEqual(t.host, "127.0.0.1")
        XCTAssertFalse(t.forwardId.isEmpty)
        XCTAssertEqual(a.rdpForwards.map(\.id), [t.forwardId])
        XCTAssertEqual(a.rdpForwards.first?.localPort, t.port)
        let mock = try XCTUnwrap(a.client as? MockGalaxyClient)
        XCTAssertEqual(mock.openRdpForwardCount, 1)
        a.closeRdp(t.forwardId)
        XCTAssertTrue(a.rdpForwards.isEmpty)
        XCTAssertEqual(mock.openRdpForwardCount, 0)
        // 退出登录时全部关闭
        _ = try await a.openRdp("r-win")
        _ = try await a.openRdp("r-win")
        XCTAssertEqual(mock.openRdpForwardCount, 2)
        await a.logout()
        XCTAssertTrue(a.rdpForwards.isEmpty)
        XCTAssertEqual(mock.openRdpForwardCount, 0)
    }

    func testDirectRdpIsNotTracked() async throws {
        let a = makeAccount()
        _ = try await a.login(server: "127.0.0.1:17443#sha256-vS0YoCAj63KxITahXYerlvFetFcrpl-Yz0rZJ7ecgw8", username: "alice", password: "x", totp: nil)
        let e = try XCTUnwrap(a.entry("r-win"))
        guard case let .rdpHost(l, h, p, u, pw, d, _, tg, n, f) = e.data else { return XCTFail() }
        try await a.saveData(id: e.id, folderId: e.folderId, data: .rdpHost(label: l, host: h, port: p, username: u, password: pw, domain: d,
                                                                             jumpDeviceId: nil, tags: tg, note: n, favorite: f))
        let t = try await a.openRdp("r-win")
        XCTAssertFalse(t.forwarded)
        XCTAssertEqual(t.host, "10.0.3.5")
        XCTAssertTrue(a.rdpForwards.isEmpty)
        XCTAssertEqual(RdpLink.url(for: t)?.absoluteString, "rdp://full%20address=s:10.0.3.5:3389&username=s:CORP%5CAdministrator")
    }

    func testPrivacyCapability() async throws {
        let a = makeAccount()
        _ = try await a.login(server: "127.0.0.1:17443#sha256-vS0YoCAj63KxITahXYerlvFetFcrpl-Yz0rZJ7ecgw8", username: "alice", password: "x", totp: nil)
        let win = try await a.peer("dev-office")
        XCTAssertTrue(win.info.capabilities.contains("privacy"))
        try await win.session.setPrivacy(on: true)
        let linux = try await a.peer("dev-build")
        XCTAssertFalse(linux.info.capabilities.contains("privacy"))
        do { try await linux.session.setPrivacy(on: true); XCTFail("不支持时应当报错") } catch {
            XCTAssertFalse(describeError(error).isEmpty)
        }
    }
}
