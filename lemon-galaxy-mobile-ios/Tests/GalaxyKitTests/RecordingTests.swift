import XCTest
import AVFoundation
@testable import GalaxyKit

/// 远程开机、会话录像、端口映射条目（手机端忽略）。
@MainActor
final class WakeAndRecordingTests: XCTestCase {
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

    func testWakeOnlyForOfflineWakeableDevices() async throws {
        let a = makeAccount()
        _ = try await a.login(server: server, username: "alice", password: "x", totp: nil)
        XCTAssertTrue(a.canWake("dev-reception"), "离线且上报过网卡地址")
        XCTAssertFalse(a.canWake("dev-nas"), "离线但没上报网卡地址")
        XCTAssertFalse(a.canWake("dev-office"), "在线设备不显示远程开机")
        XCTAssertFalse(a.canWake("dev-phone"), "手机不能被控")
        XCTAssertFalse(a.canWake("no-such-device"))
        let via = try await a.wake("dev-reception")
        XCTAssertEqual(via, ["OFFICE-PC-07"])
        do { _ = try await a.wake("dev-nas"); XCTFail("没上报网卡地址时应当报错") } catch {
            XCTAssertFalse(describeError(error).isEmpty)
        }
    }

    func testWakeText() throws {
        let url = AppResources.url("Locales/zh-Hans.json")
        let zh = Strings(pack: try XCTUnwrap(LanguagePack.load(url)), fallback: nil)
        let en = Strings(pack: try XCTUnwrap(LanguagePack.load(AppResources.url("Locales/en.json"))), fallback: nil)
        XCTAssertEqual(WakeText.sent([""], strings: zh), "已从本机发送唤醒包")
        XCTAssertEqual(WakeText.sent(["build-01"], strings: zh), "已发送唤醒包（经 build-01）")
        XCTAssertEqual(WakeText.sent(["", "build-01", "build-01"], strings: zh), "已发送唤醒包（经 本机、build-01）")
        XCTAssertEqual(WakeText.sent(["a", "b"], strings: en), "Wake-up packet sent via a, b")
    }

    func testRecordingFileNameAndClock() {
        var c = DateComponents()
        (c.year, c.month, c.day, c.hour, c.minute, c.second) = (2026, 11, 22, 9, 5, 7)
        let date = Calendar(identifier: .gregorian).date(from: c)!
        XCTAssertEqual(RecordingFiles.fileName(device: "OFFICE-PC-07", date: date), "LemonGalaxy-OFFICE-PC-07-20261122-090507.mp4")
        XCTAssertEqual(RecordingFiles.fileName(device: "a/b:c*?", date: date), "LemonGalaxy-a-b-c---20261122-090507.mp4")
        XCTAssertEqual(RecordingFiles.fileName(device: "  ", date: date, suffix: 2), "LemonGalaxy-device-20261122-090507-2.mp4")
        XCTAssertEqual(Fmt.elapsed(0), "00:00")
        XCTAssertEqual(Fmt.elapsed(75), "01:15")
        XCTAssertEqual(Fmt.elapsed(3_725), "1:02:05")
    }

    func testNewURLAvoidsOverwriting() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("rec-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let date = Date(timeIntervalSince1970: 1_800_000_000)
        let first = try RecordingFiles.newURL(device: "pc", in: dir, date: date)
        FileManager.default.createFile(atPath: first.path, contents: Data([1]))
        let second = try RecordingFiles.newURL(device: "pc", in: dir, date: date)
        XCTAssertNotEqual(first, second)
        XCTAssertTrue(second.lastPathComponent.hasSuffix("-1.mp4"))
    }

    /// 模拟内核的录像写出能播放的 MP4；关画面时自动结束
    func testMockRecordingWritesPlayableFile() async throws {
        let a = makeAccount()
        _ = try await a.login(server: server, username: "alice", password: "x", totp: nil)
        let p = try await a.peer("dev-office")
        let sink = VideoSinkAdapter()
        let frames = FrameBox()
        sink.onData = { _, _, _ in frames.add(1, 1) }
        let ch = try await p.session.openDisplay(options: DisplayOptions(displayId: 1, maxFps: 20, bitrateKbps: 4000, codecs: [.h264], viewOnly: false), sink: sink)
        try await waitFor("出画面", timeout: 15) { frames.count >= 2 }

        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("rec-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = try RecordingFiles.newURL(device: "OFFICE-PC-07", in: dir)
        XCTAssertFalse(ch.isRecording())
        try await ch.startRecording(path: url.path, withAudio: false)
        XCTAssertTrue(ch.isRecording())
        do { try await ch.startRecording(path: url.path, withAudio: false); XCTFail("重复开始应当报错") } catch {}
        let start = frames.count
        try await waitFor("录到几帧", timeout: 15) { frames.count >= start + 8 }
        let info = try ch.stopRecording()
        XCTAssertFalse(ch.isRecording())
        XCTAssertEqual(info.paths, [url.path])
        XCTAssertGreaterThan(info.bytes, 0)
        XCTAssertGreaterThan(info.durationMs, 0)
        let asset = AVURLAsset(url: url)
        let tracks = try await asset.loadTracks(withMediaType: .video)
        XCTAssertEqual(tracks.count, 1)
        let duration = try await asset.load(.duration)
        XCTAssertGreaterThan(duration.seconds, 0)
        do { _ = try ch.stopRecording(); XCTFail("没在录时停止应当报错") } catch {}

        // 录着就关画面：自动结束，不再处于录像中
        let url2 = try RecordingFiles.newURL(device: "OFFICE-PC-07", in: dir, date: Date().addingTimeInterval(1))
        try await ch.startRecording(path: url2.path, withAudio: false)
        ch.stop()
        try await waitFor("关画面后自动结束录像") { !ch.isRecording() }
    }

    func testPortForwardEntriesAreIgnoredByLists() {
        let data = HostsData(devices: MockData.devices(zh: false), vault: MockData.vault(zh: false))
        XCTAssertTrue(data.vault.contains { if case .portForward = $0.data { return true }; return false }, "模拟数据里带一条端口映射规则")
        let keys = HostTree.items(data).map(\.key)
        XCTAssertFalse(keys.contains { $0.contains("p-pg") })
        XCTAssertFalse(data.credentials.contains { $0.id == "p-pg" })
        XCTAssertFalse(data.snippets.contains { $0.id == "p-pg" })
        XCTAssertFalse(data.knownHosts.contains { $0.id == "p-pg" })
        // 所在文件夹的计数不包含它
        let db = HostTree.rows(data).first { $0.folder?.id == "f-db" }
        XCTAssertEqual(db?.count, 2)
    }
}
