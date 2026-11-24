import XCTest
@testable import GalaxyKit

/// 加虚拟屏：表单档位与校验、「我加的」集合、删屏后切去哪块；模拟内核的三个方法
final class VirtualDisplayTests: XCTestCase {
    func testDefaultsAndChoices() {
        let f = VirtualDisplayForm()
        XCTAssertEqual(f.request, VirtualDisplayRequest(width: 1920, height: 1080, refresh: 60))
        // 没有本机屏幕档位：6 个常用档位 + 自定义
        XCTAssertEqual(f.resolutionChoices.count, 7)
        XCTAssertEqual(f.resolutionChoices.first, .preset(VirtualDisplaySize(1280, 720)))
        XCTAssertEqual(f.resolutionChoices.last, .custom)
        XCTAssertFalse(f.resolutionChoices.contains(.local))
        XCTAssertEqual(f.refreshChoices, [.preset(30), .preset(60), .preset(120), .preset(144), .custom])
        // 有本机屏幕：放在自定义前面
        let g = VirtualDisplayForm(local: VirtualDisplaySize(2556, 1179))
        XCTAssertEqual(g.resolutionChoices.suffix(2), [.local, .custom])
        XCTAssertEqual(VirtualDisplaySize(2560, 1440).label, "2560×1440")
    }

    func testLocalScreenIsLandscapeAndInRange() {
        // 手机竖屏的物理像素：长边为宽
        XCTAssertEqual(VirtualDisplayForm.localSize(pixelWidth: 1179, pixelHeight: 2556), VirtualDisplaySize(2556, 1179))
        XCTAssertEqual(VirtualDisplayForm.localSize(pixelWidth: 2732, pixelHeight: 2048), VirtualDisplaySize(2732, 2048))
        // 边界
        XCTAssertEqual(VirtualDisplayForm.localSize(pixelWidth: 480, pixelHeight: 640), VirtualDisplaySize(640, 480))
        XCTAssertEqual(VirtualDisplayForm.localSize(pixelWidth: 7680, pixelHeight: 4320), VirtualDisplaySize(7680, 4320))
        // 超出范围不显示这一项
        XCTAssertNil(VirtualDisplayForm.localSize(pixelWidth: 320, pixelHeight: 640))
        XCTAssertNil(VirtualDisplayForm.localSize(pixelWidth: 8000, pixelHeight: 4000))
        XCTAssertNil(VirtualDisplayForm.localSize(pixelWidth: 7680, pixelHeight: 4400))
        XCTAssertNil(VirtualDisplayForm.localSize(pixelWidth: 0, pixelHeight: 0))
        XCTAssertNil(VirtualDisplayForm.localSize(pixelWidth: .nan, pixelHeight: 1080))
        // 选本机屏幕
        var f = VirtualDisplayForm(local: VirtualDisplaySize(2556, 1179))
        f.selectResolution(.local)
        XCTAssertEqual(f.request, VirtualDisplayRequest(width: 2556, height: 1179, refresh: 60))
    }

    func testCustomSizeValidation() {
        var f = VirtualDisplayForm()
        f.selectResolution(.preset(VirtualDisplaySize(2560, 1440)))
        // 选自定义：空输入框先填上当前分辨率，仍然合法
        f.selectResolution(.custom)
        XCTAssertEqual(f.customWidth, "2560")
        XCTAssertEqual(f.customHeight, "1440")
        XCTAssertFalse(f.sizeInvalid)
        XCTAssertEqual(f.size, VirtualDisplaySize(2560, 1440))
        // 改成不合法：按钮置灰，范围说明放在分辨率下面，已经输入了内容所以醒目
        f.setCustomWidth("639")
        XCTAssertTrue(f.sizeInvalid)
        XCTAssertNil(f.request)
        XCTAssertTrue(f.rangeHintAfterSize)
        XCTAssertFalse(f.rangeHintAfterRefresh)
        XCTAssertTrue(f.rangeHintIsError)
        f.setCustomWidth("7680")
        f.setCustomHeight("4320")
        XCTAssertEqual(f.request, VirtualDisplayRequest(width: 7680, height: 4320, refresh: 60))
        f.setCustomHeight("4321")
        XCTAssertTrue(f.sizeInvalid)
        f.setCustomHeight("479")
        XCTAssertTrue(f.sizeInvalid)
        // 清空：仍不合法，但只是提示
        f.setCustomWidth("")
        f.setCustomHeight("")
        XCTAssertTrue(f.sizeInvalid)
        XCTAssertFalse(f.rangeHintIsError)
        // 已经有内容时再选自定义不覆盖
        f.setCustomWidth("1000")
        f.selectResolution(.preset(VirtualDisplaySize(1280, 720)))
        f.selectResolution(.custom)
        XCTAssertEqual(f.customWidth, "1000")
        XCTAssertEqual(f.customHeight, "")
        // 选回档位：自定义里的不合法值不影响
        f.selectResolution(.preset(VirtualDisplaySize(1920, 1200)))
        XCTAssertFalse(f.sizeInvalid)
        XCTAssertEqual(f.request?.height, 1200)
    }

    func testCustomRefreshValidation() {
        var f = VirtualDisplayForm()
        f.selectRefresh(.preset(144))
        f.selectRefresh(.custom)
        XCTAssertEqual(f.customRefresh, "144")
        XCTAssertEqual(f.refreshRate, 144)
        f.setCustomRefresh("23")
        XCTAssertTrue(f.refreshInvalid)
        XCTAssertNil(f.request)
        XCTAssertTrue(f.rangeHintAfterRefresh)
        f.setCustomRefresh("24")
        XCTAssertEqual(f.request?.refresh, 24)
        f.setCustomRefresh("240")
        XCTAssertEqual(f.request?.refresh, 240)
        f.setCustomRefresh("241")
        XCTAssertTrue(f.refreshInvalid)
        // 分辨率和刷新率都不合法：同一句范围说明只出现一次（在分辨率下面）
        f.selectResolution(.custom)
        f.setCustomWidth("1")
        XCTAssertTrue(f.rangeHintAfterSize)
        XCTAssertFalse(f.rangeHintAfterRefresh)
    }

    func testInputFiltering() {
        XCTAssertEqual(VirtualDisplayForm.digits(" 19a20x", max: 4), "1920")
        XCTAssertEqual(VirtualDisplayForm.digits("123456", max: 4), "1234")
        XCTAssertEqual(VirtualDisplayForm.digits("１２０", max: 3), "", "全角数字不认")
        var f = VirtualDisplayForm()
        f.selectRefresh(.custom)
        f.setCustomRefresh("1440")
        XCTAssertEqual(f.customRefresh, "144")
        XCTAssertNil(VirtualDisplayForm.number(""))
        XCTAssertNil(VirtualDisplayForm.number("99999999999"))
        XCTAssertEqual(VirtualDisplayForm.number(" 60 "), 60)
    }

    func testQuotaAndSubmit() {
        let f = VirtualDisplayForm()
        XCTAssertFalse(VirtualDisplayQuota(added: 0, max: 0).available)
        XCTAssertTrue(f.canSubmit(quota: VirtualDisplayQuota(added: 2, max: 3), busy: false))
        XCTAssertFalse(f.canSubmit(quota: VirtualDisplayQuota(added: 2, max: 3), busy: true), "进行中不能再点")
        XCTAssertTrue(VirtualDisplayQuota(added: 3, max: 3).full)
        XCTAssertFalse(f.canSubmit(quota: VirtualDisplayQuota(added: 3, max: 3), busy: false), "加满了")
        XCTAssertFalse(f.canSubmit(quota: VirtualDisplayQuota(added: 0, max: 0), busy: false), "不支持")
        var bad = f
        bad.selectResolution(.custom)
        bad.setCustomWidth("100")
        XCTAssertFalse(bad.canSubmit(quota: VirtualDisplayQuota(added: 0, max: 3), busy: false), "不合法")
    }

    func testMyVirtualDisplays() {
        var m = MyVirtualDisplays()
        m.add(101)
        m.add(102)
        m.add(101)
        XCTAssertEqual(m.ids, [101, 102])
        XCTAssertTrue(m.contains(102))
        XCTAssertFalse(m.contains(3), "被控端自动加的、别的主控加的虚拟屏不算我加的")
        m.remove(101)
        XCTAssertEqual(m.ids, [102])
        // 屏幕列表里已经没有的不再算
        m.add(103)
        m.keepOnly(listed: [1, 2, 103])
        XCTAssertEqual(m.ids, [103])
        m.reset()
        XCTAssertEqual(m.count, 0)
    }

    func testFallbackAfterRemovingViewed() {
        func d(_ id: UInt32, primary: Bool = false, virtual: Bool = false) -> DisplayInfo {
            DisplayInfo(id: id, name: "D\(id)", width: 1920, height: 1080, scale: 1, primary: primary, isVirtual: virtual)
        }
        XCTAssertEqual(MyVirtualDisplays.fallback(removing: 101, in: [d(2), d(1, primary: true), d(101, virtual: true)]), 1)
        // 没有主屏：第一块
        XCTAssertEqual(MyVirtualDisplays.fallback(removing: 101, in: [d(101, virtual: true), d(5), d(6)]), 5)
        XCTAssertNil(MyVirtualDisplays.fallback(removing: 101, in: [d(101, virtual: true)]))
    }
}

/// 模拟内核的虚拟屏：上限 3、加屏追加 isVirtual 的屏并能开画面、只能删本会话加的
@MainActor
final class MockVirtualDisplayTests: XCTestCase {
    func makeAccount() -> GalaxyAccount {
        let config = CoreConfig(dataDir: NSTemporaryDirectory(), cacheDir: NSTemporaryDirectory(), logLevel: "info", platform: .ios,
                                deviceKind: .mobile, appVersion: "test", osVersion: "test", hostname: "test", locale: "en")
        return GalaxyAccount(config: config, store: MemorySecureStore(), forceMock: true, mockOptions: MockOptions(latencyScale: 0, liveSimulation: false))
    }

    func testAddListOpenRemove() async throws {
        let a = makeAccount()
        _ = try await a.login(server: "127.0.0.1:17443#sha256-vS0YoCAj63KxITahXYerlvFetFcrpl-Yz0rZJ7ecgw8", username: "alice", password: "x", totp: nil)
        let win = try await a.peer("dev-office")
        XCTAssertEqual(win.session.virtualDisplayMax(), 3)
        XCTAssertTrue(win.info.capabilities.contains("virtual_display"))
        let before = try await win.session.listDisplays()

        let d = try await win.session.addVirtualDisplay(width: 2560, height: 1440, refresh: 60)
        XCTAssertTrue(d.isVirtual)
        XCTAssertFalse(d.primary)
        XCTAssertEqual(d.width, 2560)
        let after = try await win.session.listDisplays()
        XCTAssertEqual(after.count, before.count + 1)
        XCTAssertEqual(after.last?.id, d.id)

        // 新屏能开画面
        let sink = VideoSinkAdapter()
        let ch = try await win.session.openDisplay(options: DisplayOptions(displayId: d.id, maxFps: 10, bitrateKbps: 2000, codecs: [.h264], viewOnly: false),
                                                   sink: sink)
        ch.stop()

        // 不合法的分辨率、加满都报错
        do { _ = try await win.session.addVirtualDisplay(width: 100, height: 100, refresh: 60); XCTFail("分辨率不合法应当报错") } catch {
            XCTAssertFalse(describeError(error).isEmpty)
        }
        _ = try await win.session.addVirtualDisplay(width: 1920, height: 1080, refresh: 0)
        _ = try await win.session.addVirtualDisplay(width: 1280, height: 720, refresh: 30)
        do { _ = try await win.session.addVirtualDisplay(width: 1280, height: 720, refresh: 30); XCTFail("加满了应当报错") } catch {
            XCTAssertFalse(describeError(error).isEmpty)
        }

        // 删掉后从列表里消失；再删报没有这块
        try await win.session.removeVirtualDisplay(displayId: d.id)
        let rest = try await win.session.listDisplays()
        XCTAssertFalse(rest.contains { $0.id == d.id })
        do { try await win.session.removeVirtualDisplay(displayId: d.id); XCTFail("已经删掉的应当报错") } catch {}
        // 断开时被控端删掉本会话加的
        win.session.disconnect()
        let gone = try await win.session.listDisplays()
        XCTAssertFalse(gone.contains { $0.isVirtual })
    }

    func testOnlyOwnVirtualDisplaysAndUnsupported() async throws {
        let a = makeAccount()
        _ = try await a.login(server: "127.0.0.1:17443#sha256-vS0YoCAj63KxITahXYerlvFetFcrpl-Yz0rZJ7ecgw8", username: "alice", password: "x", totp: nil)
        // macOS 上本来就有一块不是本会话加的虚拟屏：不能删
        let mac = try await a.peer("dev-mbp")
        let macDisplays = try await mac.session.listDisplays()
        let foreign = try XCTUnwrap(macDisplays.first { $0.isVirtual })
        do { try await mac.session.removeVirtualDisplay(displayId: foreign.id); XCTFail("别人加的不能删") } catch {
            XCTAssertFalse(describeError(error).isEmpty)
        }
        // 没装驱动的 Windows、Linux：上限为 0，不显示入口
        let reception = try XCTUnwrap(MockData.devices(zh: false).first { $0.id == "dev-reception" })
        XCTAssertEqual(MockData.virtualDisplayMax(for: reception), 0)
        XCTAssertFalse(MockData.capabilities(for: reception).contains("virtual_display"))
        let linux = try await a.peer("dev-build")
        XCTAssertEqual(linux.session.virtualDisplayMax(), 0)
        do { _ = try await linux.session.addVirtualDisplay(width: 1920, height: 1080, refresh: 60); XCTFail("不支持时应当报错") } catch {
            XCTAssertFalse(describeError(error).isEmpty)
        }
    }
}
