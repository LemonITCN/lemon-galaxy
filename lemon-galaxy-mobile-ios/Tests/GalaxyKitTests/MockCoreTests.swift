import XCTest
import CoreVideo
@testable import GalaxyKit

/// 用模拟内核走一遍 GalaxyKit 的模型层（与 App 用的同一套代码）：登录规则、加密库、连接池、终端、传输、AI、SSH 认证弹窗。
@MainActor
final class MockCoreTests: XCTestCase {
    let server = "127.0.0.1:17443#sha256-vS0YoCAj63KxITahXYerlvFetFcrpl-Yz0rZJ7ecgw8"

    func makeAccount() -> GalaxyAccount {
        let config = CoreConfig(dataDir: NSTemporaryDirectory(), cacheDir: NSTemporaryDirectory(), logLevel: "info", platform: .ios,
                                deviceKind: .mobile, appVersion: "test", osVersion: "test", hostname: "test", locale: "en")
        return GalaxyAccount(config: config, store: MemorySecureStore(), forceMock: true, mockOptions: MockOptions(latencyScale: 0, liveSimulation: false))
    }

    func waitFor(_ what: String, timeout seconds: Double = 5, _ cond: @escaping () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(seconds)
        while !cond() {
            if Date() > deadline { XCTFail("超时：\(what)"); return }
            try await Task.sleep(nanoseconds: 20_000_000)
        }
    }

    func testLoginRules() async throws {
        let a = makeAccount()
        XCTAssertTrue(a.isMock)
        do {
            _ = try await a.login(server: server, username: "alice", password: "wrong", totp: nil)
            XCTFail("密码错误应当报错")
        } catch {
            XCTAssertFalse(describeError(error).isEmpty)
        }
        let r1 = try await a.login(server: server, username: "totp-alice", password: "x", totp: nil)
        XCTAssertEqual(r1, .needTotp)
        XCTAssertEqual(a.phase, .login)
        let r2 = try await a.login(server: server, username: "totp-alice", password: "x", totp: "123456")
        XCTAssertEqual(r2, .success)
        XCTAssertEqual(a.phase, .main)
        XCTAssertFalse(a.devices.isEmpty)
        try await waitFor("on_state 回调切到主线程") { a.clientState == .online }
        var loggedOut = false
        a.onLoggedOut = { loggedOut = true }
        await a.logout()
        XCTAssertEqual(a.phase, .login)
        XCTAssertTrue(loggedOut)

        let b = makeAccount()
        let r3 = try await b.login(server: server, username: "new-bob", password: "x", totp: nil)
        XCTAssertEqual(r3, .mustChangePassword)
        do { try await b.completeFirstLogin(newPassword: "short"); XCTFail() } catch {}
        try await b.completeFirstLogin(newPassword: "long-enough-password")
        XCTAssertEqual(b.phase, .main)
    }

    func testVaultEditing() async throws {
        let a = makeAccount()
        _ = try await a.login(server: server, username: "alice", password: "x", totp: nil)
        try await a.saveData(id: nil, folderId: nil, data: .folder(name: "Lab", color: nil, expanded: true))
        let lab = try XCTUnwrap(a.vault.first { if case .folder("Lab", _, _) = $0.data { return true }; return false })
        // 设备没有 DeviceMeta 时新建一条
        try await a.updateDeviceMeta("dev-mbp", folder: .some(lab.id)) { alias, tags, _, fav in
            alias = "MacBook"
            tags = ["mac"]
            fav = true
        }
        XCTAssertEqual(a.deviceName("dev-mbp"), "MacBook")
        let item = try XCTUnwrap(HostTree.items(a.hosts).first { $0.device?.id == "dev-mbp" })
        XCTAssertEqual(item.folderId, lab.id)
        XCTAssertTrue(item.favorite)
        try await a.toggleFavorite(item)
        XCTAssertFalse(HostTree.items(a.hosts).first { $0.device?.id == "dev-mbp" }!.favorite)
        // SSH 主机移到顶层
        let ssh = try XCTUnwrap(HostTree.items(a.hosts).first { $0.key == "ssh:s-web1" })
        try await a.move(ssh, to: nil)
        XCTAssertNil(a.entry("s-web1")?.folderId)
        // 文件夹不能移进自己的子文件夹
        let prod = try XCTUnwrap(a.entry("f-prod"))
        try await a.moveFolder(prod, to: "f-db")
        XCTAssertNil(a.entry("f-prod")?.folderId)
        // 折叠状态先改本地
        a.setFolderExpanded(prod, false)
        if case .folder(_, _, let e) = a.entry("f-prod")!.data { XCTAssertFalse(e) }
        try await a.delete(lab.id)
        XCTAssertNil(a.entry(lab.id))
    }

    func testPeerPoolExecAndTerminal() async throws {
        let a = makeAccount()
        _ = try await a.login(server: server, username: "alice", password: "x", totp: nil)
        let p = try await a.peer("dev-build")
        let again = try await a.peer("dev-build")
        XCTAssertTrue(p === again, "同一台设备共用一条会话")
        XCTAssertEqual(p.info.platform, .linux)
        let r = try await a.exec(.device("dev-build"), ExecRequest(command: "uname -a", shell: nil, cwd: nil, timeoutMs: 5000, maxOutput: 4096))
        XCTAssertEqual(r.code, 0)

        var output = Data()
        var exited = false
        let sink = TerminalSinkAdapter(onData: { d in DispatchQueue.main.async { output.append(d) } },
                                       onExit: { _ in DispatchQueue.main.async { exited = true } })
        let ch = try await a.openTerminal(.device("dev-build"), options: TerminalOptions(attachSession: nil, shell: nil, cols: 80, rows: 24, env: []), sink: sink)
        ch.write(data: Data("echo hello-ios".utf8) + Data(TerminalKeys.bytes(.enter)))
        try await waitFor("终端回显") { String(decoding: output, as: UTF8.self).contains("hello-ios") }
        ch.detach()
        let kept = try await p.session.listTerminals()
        XCTAssertTrue(kept.contains { $0.id == ch.sessionId() && !$0.attached }, "分离后被控端保留会话")
        _ = exited
        a.release("dev-build")
        XCTAssertNil(a.existingPeer("dev-build"))
        do { _ = try await a.peer("dev-nas"); XCTFail("离线设备应连不上") } catch {}
    }

    func testTransfersRoundTrip() async throws {
        let a = makeAccount()
        _ = try await a.login(server: server, username: "alice", password: "x", totp: nil)
        let files = try await a.files(.device("dev-build"))
        let home = try await files.home()
        let list = try await files.list(path: home)
        let file = try XCTUnwrap(list.first { $0.kind == .file && !$0.name.contains("fail") })
        let center = TransferCenter()
        var finished: [TransferItem] = []
        center.onFinished = { finished.append($0) }
        let local = FileManager.default.temporaryDirectory.appendingPathComponent("galaxy-test-\(UUID().uuidString)/" + file.name)
        let item = center.download(files, deviceId: "dev-build", deviceName: "build", remote: file.path, to: local)
        try await waitFor("下载完成", timeout: 10) { !item.isActive }
        XCTAssertEqual(item.status, .done)
        XCTAssertEqual(finished.count, 1)
        XCTAssertEqual(center.activeCount, 0)
        center.clearFinished()
        XCTAssertTrue(center.items.isEmpty)
    }

    func testAgentConversationNeedsApproval() async throws {
        let a = makeAccount()
        _ = try await a.login(server: server, username: "alice", password: "x", totp: nil)
        let hub = AgentHub(client: { a.client }, hosts: { a.hosts })
        await hub.loadModels()
        let m = try XCTUnwrap(hub.models?.items.first)
        let c = try await hub.create(ConversationConfig(providerId: m.providerId, modelId: m.id, targets: [AgentTarget(kind: .device, id: "dev-build")],
                                                         mode: .confirmDangerous, planFirst: false, trustMinutes: nil, title: nil))
        XCTAssertEqual(c.state.targetNames, ["Build server"])
        c.send("Free up disk space")
        XCTAssertEqual(c.state.title, "Free up disk space")
        // 脚本里第三个工具是危险操作：在「危险操作确认」档要等用户确认
        try await waitFor("危险操作的确认卡片", timeout: 15) {
            c.state.items.contains { if case .tool(let t) = $0 { return t.phase == .awaiting }; return false }
        }
        XCTAssertTrue(c.state.waitingForUser)
        let pending = c.state.items.compactMap { item -> ToolEntry? in if case .tool(let t) = item, t.phase == .awaiting { return t }; return nil }.first!
        XCTAssertEqual(pending.call.risk, .dangerous)
        c.decide(pending.call.callId, .deny(reason: "keep backups"))
        try await waitFor("对话结束", timeout: 10) { !c.state.running }
        guard case .tool(let t)? = c.state.items.first(where: { $0.id == "tool-" + pending.call.callId }) else { return XCTFail() }
        XCTAssertEqual(t.phase, .denied)
        hub.refreshList()
        XCTAssertEqual(hub.items.first?.id, c.id)
        // 重新打开：从内核的历史恢复条目
        let reopened = try await hub.open(c.id)
        XCTAssertTrue(reopened === c)
        hub.delete(c.id)
        XCTAssertTrue(hub.items.isEmpty)
    }

    func testSshPromptsGoThroughPresenter() async throws {
        let a = makeAccount()
        _ = try await a.login(server: server, username: "alice", password: "x", totp: nil)
        var asked: [String] = []
        a.secretPresenter = { req in
            switch req {
            case .hostKey(_, let host, _, _, _, let changed, let reply):
                asked.append("hostKey:\(host):\(changed)")
                reply(true)
            case .ask(_, _, let prompt, let echo, let reply):
                asked.append("ask:\(echo)")
                XCTAssertTrue(prompt.contains("postgres@10.0.2.10"))
                reply("secret")
            }
        }
        // db-primary 没存密码：先确认主机密钥（首次连接），再问密码
        let ssh = try await a.ssh("s-dbp")
        XCTAssertEqual(asked, ["hostKey:10.0.2.10:false", "ask:false"])
        XCTAssertEqual(ssh.info.host, "10.0.2.10")
        XCTAssertTrue(a.vault.contains { if case .knownHost("10.0.2.10", 22, _, _) = $0.data { return true }; return false }, "确认后的主机密钥存进加密库")
        let r = try await a.exec(.ssh("s-dbp"), ExecRequest(command: "whoami", shell: nil, cwd: nil, timeoutMs: 1000, maxOutput: 1024))
        XCTAssertEqual(r.code, 0)
        _ = try await a.files(.ssh("s-dbp")).home()
        // 取消：返回 nil 表示取消
        a.releaseSsh("s-dbp")
        a.secretPresenter = { req in req.cancel() }
        do { _ = try await a.ssh("s-dbp"); XCTFail("取消应当报错") } catch { XCTAssertTrue(isCancellation(error)) }
        XCTAssertEqual(a.targetName(.ssh("s-vps")), "vps-tokyo")
    }

    /// 视频链路：模拟被控端用 VideoToolbox 编码出 H.264 Annex B → VideoSinkAdapter → VideoDecoder 解出像素
    func testVideoPipelineDecodesMockDisplay() async throws {
        let a = makeAccount()
        _ = try await a.login(server: server, username: "alice", password: "x", totp: nil)
        let p = try await a.peer("dev-office")
        let displays = try await p.session.listDisplays()
        let d = try XCTUnwrap(displays.first { $0.primary })
        let decoder = VideoDecoder()
        let box = FrameBox()
        decoder.onFrame = { pb, _ in box.add(CVPixelBufferGetWidth(pb), CVPixelBufferGetHeight(pb)) }
        let sink = VideoSinkAdapter()
        sink.onConfigure = { codec, _, _ in decoder.configure(codec: codec) }
        sink.onData = { data, pts, key in decoder.decode(data, ptsUs: pts, keyframe: key) }
        let ch = try await p.session.openDisplay(options: DisplayOptions(displayId: d.id, maxFps: 20, bitrateKbps: 4000, codecs: [.h264], viewOnly: false), sink: sink)
        decoder.onNeedKeyframe = { ch.requestKeyframe() }
        try await waitFor("解出画面", timeout: 15) { box.count >= 3 }
        XCTAssertGreaterThan(box.width, 0)
        ch.sendInput(event: .mouseMove(x: 10, y: 10))
        ch.stop()
        decoder.invalidate()
    }
}

final class FrameBox: @unchecked Sendable {
    private let lock = NSLock()
    private var _count = 0
    private var _width = 0
    func add(_ w: Int, _ h: Int) { lock.locked { _count += 1; _width = w } }
    var count: Int { lock.locked { _count } }
    var width: Int { lock.locked { _width } }
}
