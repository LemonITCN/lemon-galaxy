import Foundation
import AppKit
import CoreVideo

/// 虚拟屏真机联调（真内核，不开窗口、不碰钥匙串，数据目录用临时目录）。两个进程配合：
///
///   --vd-host <服务器地址> <用户名> <密码> [--hold 秒]
///       本机当被控端（MacHostPlatform，真 CGVirtualDisplay），上线后保持 hold 秒（默认 300），结束时退出登录并删掉本进程加的屏。
///       要「屏幕录制」授权：没有就直接退出（不去触发授权框）。授权跟着应用签名走，要经 open 启动，从终端直接跑算在终端头上。
///       主线程只跑 RunLoop（建屏、删屏在主线程上做）。
///
///   --vd-smoke <服务器地址> <用户名> <密码> [--target 主机名] [--shot 输出文件]
///       主控端按界面同样的调用走一遍：连上在线的 Mac / Windows 被控端 → virtual_display_max → 加 2560x1440@60 →
///       列屏里有它且标为虚拟屏 → 开画面并真解码若干帧 → 截图（给了 --shot 时存下来）→ 删掉 → 列屏里没有了 → 再删报错。
///       --target 只连这个主机名的设备。全部通过退出码 0，否则 1。
///
/// 两个命令都要账号已完成首次登录（改过初始密码），例如先用 galaxy-dev login。
enum VirtualDisplaySmoke {
    static func host(_ args: [String]) -> Int32 {
        #if LEMONGALAXY_REAL_CORE
        guard let i = args.firstIndex(of: "--vd-host"), i + 3 < args.count else {
            print("用法：LemonGalaxy --vd-host <服务器地址> <用户名> <密码> [--hold 秒]")
            return 2
        }
        let hold = args.firstIndex(of: "--hold").flatMap { $0 + 1 < args.count ? Double(args[$0 + 1]) : nil } ?? 300
        // 要连窗口服务器（建虚拟屏），但不在程序坞出图标
        NSApplication.shared.setActivationPolicy(.prohibited)
        guard SystemPermissions.screenRecordingGranted else {
            vdLog("失败 没有「屏幕录制」授权，不当被控端（不触发授权框）。要经 open 启动这个应用，授权才算在它自己头上")
            return 3
        }
        guard VirtualDisplays.shared.available else {
            vdLog("失败 这台 Mac 的系统没有 CGVirtualDisplay")
            return 3
        }
        let platform = MacHostPlatform()
        let result = ResultBox()
        Task.detached {
            result.set(await VDHostRunner(server: args[i + 1], user: args[i + 2], pass: args[i + 3], hold: hold, platform: platform).run())
        }
        while result.value == nil { RunLoop.main.run(until: Date().addingTimeInterval(0.05)) }
        return result.value ?? 1
        #else
        print("这个构建没有接入 Rust 内核，无法联调")
        return 2
        #endif
    }

    static func controller(_ args: [String]) -> Int32 {
        #if LEMONGALAXY_REAL_CORE
        guard let i = args.firstIndex(of: "--vd-smoke"), i + 3 < args.count else {
            print("用法：LemonGalaxy --vd-smoke <服务器地址> <用户名> <密码> [--target 主机名] [--shot 输出文件]")
            return 2
        }
        let opt = { (name: String) in args.firstIndex(of: name).flatMap { $0 + 1 < args.count ? args[$0 + 1] : nil } }
        let result = ResultBox()
        Task.detached {
            result.set(await VDControllerRunner(server: args[i + 1], user: args[i + 2], pass: args[i + 3],
                                                target: opt("--target"), shot: opt("--shot")).run())
        }
        while result.value == nil { RunLoop.main.run(until: Date().addingTimeInterval(0.05)) }
        return result.value ?? 1
        #else
        print("这个构建没有接入 Rust 内核，无法联调")
        return 2
        #endif
    }
}

private final class ResultBox: @unchecked Sendable {
    private let lock = NSLock()
    private var code: Int32?
    var value: Int32? { lock.locked { code } }
    func set(_ v: Int32) { lock.locked { code = v } }
}

private let vdStarted = Date()
private func vdLog(_ s: String) {
    print(String(format: "[%7.3fs] ", Date().timeIntervalSince(vdStarted)) + s)
    fflush(stdout)
}

#if LEMONGALAXY_REAL_CORE
private final class QuietListener: ClientListener, @unchecked Sendable {
    func onState(state: ClientState) { vdLog("  [回调] 状态 \(state)") }
    func onDevicesChanged() {}
    func onVaultChanged() {}
    func onSessionRevoked() { vdLog("  [回调] 会话被吊销") }
}

private struct VDError: Error, CustomStringConvertible {
    let description: String
}

/// 执行一步并打印结果与耗时；失败返回 nil
private func vdStep<T>(_ name: String, _ body: () async throws -> T) async -> T? {
    let t0 = Date()
    vdLog("开始 \(name)")
    do {
        let v = try await body()
        vdLog(String(format: "通过 %@（%.0f ms）", name, Date().timeIntervalSince(t0) * 1000))
        return v
    } catch {
        let why = (error as? VDError)?.description ?? AppModel.describe(error)
        vdLog(String(format: "失败 %@（%.0f ms）：%@", name, Date().timeIntervalSince(t0) * 1000, why))
        return nil
    }
}

/// 临时数据目录 + 内核初始化 + 登录 + 等上线
private func vdLogin(server: String, user: String, pass: String, hostname: String) async -> (GalaxyClient, URL)? {
    let base = FileManager.default.temporaryDirectory.appendingPathComponent("lemongalaxy-\(hostname)-\(ProcessInfo.processInfo.processIdentifier)", isDirectory: true)
    let dataDir = base.appendingPathComponent("data"), cacheDir = base.appendingPathComponent("cache")
    try? FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
    try? FileManager.default.createDirectory(at: cacheDir, withIntermediateDirectories: true)
    vdLog("内核版本 \(coreVersion())；数据目录 \(base.path)")
    coreInit(config: CoreConfig(dataDir: dataDir.path, cacheDir: cacheDir.path, logLevel: "info", platform: .macos, deviceKind: .control,
                                appVersion: AppInfo.version, osVersion: AppInfo.osVersion, hostname: hostname, locale: "zh-CN"))
    let client = GalaxyClient(store: MemorySecureStore(), listener: QuietListener())
    guard let r = await vdStep("login", { try await client.login(server: server, username: user, password: pass, totp: nil) }), r == .success else {
        vdLog("失败 登录没成功（需要两步验证或改密码的账号先用 galaxy-dev login 处理）")
        try? FileManager.default.removeItem(at: base)
        return nil
    }
    let online = await vdStep("等待上线") { () async throws -> Bool in
        for _ in 0..<200 {
            if client.state() == .online { return true }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
        throw VDError(description: "20 秒内没有上线，当前状态 \(client.state())")
    }
    guard online != nil else {
        _ = try? await client.logout()
        try? FileManager.default.removeItem(at: base)
        return nil
    }
    return (client, base)
}

private struct VDHostRunner {
    let server: String
    let user: String
    let pass: String
    let hold: Double
    let platform: MacHostPlatform

    func run() async -> Int32 {
        guard let (client, base) = await vdLogin(server: server, user: user, pass: pass, hostname: "LemonGalaxy-vd-host") else { return 1 }
        defer { try? FileManager.default.removeItem(at: base) }
        guard let service = client.host() else {
            vdLog("失败 没有本机被控服务")
            _ = try? await client.logout()
            return 1
        }
        service.setPlatform(platform: platform)
        let up = await vdStep("被控服务上线") { () async throws -> Bool in
            for _ in 0..<200 {
                if service.status().online { return true }
                try await Task.sleep(nanoseconds: 100_000_000)
            }
            throw VDError(description: "20 秒内被控服务没有上线")
        }
        let code: Int32 = up == nil ? 1 : 0
        let status = service.status()
        vdLog("  设备 \(status.deviceId)；平台层虚拟屏上限 \(platform.virtualDisplayMax())；无头 \(platform.needsHeadlessDisplay())")
        vdLog("VD-HOST-READY \(status.deviceId)")
        let end = Date().addingTimeInterval(hold)
        var lastSessions = -1
        var lastIds: [UInt32] = []
        while Date() < end, code == 0 {
            let sessions = service.status().sessions.count
            if sessions != lastSessions { vdLog("  当前会话 \(sessions) 个"); lastSessions = sessions }
            let ids = VirtualDisplays.shared.ids.sorted()
            if ids != lastIds { vdLog("  本进程的虚拟屏 \(ids)"); lastIds = ids }
            try? await Task.sleep(nanoseconds: 200_000_000)
        }
        _ = await vdStep("logout") { try await client.logout() }
        VirtualDisplays.shared.removeAll()
        vdLog("结束：本进程的虚拟屏 \(VirtualDisplays.shared.ids)")
        return code
    }
}

/// 画面回调：真解码，数解出来的帧
private final class DecodeSink: VideoSink, @unchecked Sendable {
    let decoder = VideoDecoder()
    private let lock = NSLock()
    private var _decoded = 0
    private var _size: (UInt32, UInt32)?
    private var _closed: String?
    var decoded: Int { lock.locked { _decoded } }
    var size: (UInt32, UInt32)? { lock.locked { _size } }
    var closed: String? { lock.locked { _closed } }

    init() {
        decoder.onFrame = { [weak self] _, _ in self?.lock.locked { self?._decoded += 1 } }
    }
    func onConfig(codec: VideoCodec, width: UInt32, height: UInt32) {
        lock.locked { _size = (width, height) }
        decoder.configure(codec: codec)
    }
    func onFrame(data: Data, ptsUs: UInt64, keyframe: Bool) { decoder.decode(data, ptsUs: ptsUs, keyframe: keyframe) }
    func onClosed(reason: String) { lock.locked { _closed = reason } }
    func onCursor(x: Int32, y: Int32, visible: Bool) {}
}

private struct VDControllerRunner {
    let server: String
    let user: String
    let pass: String
    let target: String?
    let shot: String?

    func run() async -> Int32 {
        guard let (client, base) = await vdLogin(server: server, user: user, pass: pass, hostname: "LemonGalaxy-vd-smoke") else { return 1 }
        defer { try? FileManager.default.removeItem(at: base) }
        var failures = 0
        func finish() async -> Int32 {
            _ = await vdStep("logout") { try await client.logout() }
            vdLog(failures == 0 ? "全部通过" : "失败 \(failures) 项")
            return failures == 0 ? 0 : 1
        }

        _ = await vdStep("refresh_devices") { try await client.refreshDevices() }
        let candidates = client.devices().filter { d in
            d.online && !d.isSelf && (d.platform == .macos || d.platform == .windows) && (target == nil || d.hostname == target)
        }
        guard let device = candidates.first else {
            vdLog("失败 没有在线的 Mac / Windows 被控端\(target.map { "（主机名 \($0)）" } ?? "")")
            failures += 1
            return await finish()
        }
        vdLog("  目标 \(device.hostname) \(device.platform) id=\(device.id)")
        guard let session = await vdStep("connect", { try await client.connect(deviceId: device.id) }) else {
            failures += 1
            return await finish()
        }
        let info = session.info()
        vdLog("  控制权限 \(info.permissions.control)；能力 \(info.capabilities.joined(separator: ","))")
        let max = session.virtualDisplayMax()
        vdLog("  virtual_display_max = \(max)")
        guard max > 0 else {
            vdLog("失败 对端不支持虚拟屏或没有控制权限，界面不会显示入口")
            failures += 1
            session.disconnect()
            return await finish()
        }

        let before = await vdStep("list_displays（加屏前）") { try await session.listDisplays() } ?? []
        for d in before { vdLog("  屏幕 \(d.id) \(d.name) \(d.width)x\(d.height) 主屏 \(d.primary) 虚拟 \(d.isVirtual)") }

        guard let added = await vdStep("add_virtual_display(2560, 1440, 60)", { try await session.addVirtualDisplay(width: 2560, height: 1440, refresh: 60) }) else {
            failures += 1
            session.disconnect()
            return await finish()
        }
        vdLog("  新屏 id \(added.id) \(added.name) \(added.width)x\(added.height) 虚拟 \(added.isVirtual)")
        if !added.isVirtual { vdLog("失败 新屏没有标为虚拟屏"); failures += 1 }
        if before.contains(where: { $0.id == added.id }) { vdLog("失败 新屏的 id 与已有屏幕重复"); failures += 1 }

        if let list = await vdStep("list_displays（加屏后）", { try await session.listDisplays() }) {
            for d in list { vdLog("  屏幕 \(d.id) \(d.name) \(d.width)x\(d.height) 主屏 \(d.primary) 虚拟 \(d.isVirtual)") }
            if list.first(where: { $0.id == added.id })?.isVirtual != true { vdLog("失败 列屏里没有新屏或没标虚拟"); failures += 1 }
        } else { failures += 1 }

        // 看画面：与远程桌面界面同样的参数开画面，真解码
        let sink = DecodeSink()
        if let ch = await vdStep("open_display(\(added.id))", {
            try await session.openDisplay(options: DisplayOptions(displayId: added.id, maxFps: 30, bitrateKbps: 8_000, codecs: [.h264], viewOnly: false), sink: sink)
        }) {
            let ok = await vdStep("解码 10 帧") { () async throws -> Int in
                for _ in 0..<150 {
                    if sink.decoded >= 10 { return sink.decoded }
                    if let why = sink.closed { throw VDError(description: "画面被关掉：\(why)") }
                    if sink.decoded == 0 { ch.requestKeyframe() }
                    try await Task.sleep(nanoseconds: 100_000_000)
                }
                throw VDError(description: "15 秒内只解出 \(sink.decoded) 帧")
            }
            if let s = sink.size { vdLog("  画面 \(s.0)x\(s.1)，已解 \(sink.decoded) 帧") }
            if ok == nil { failures += 1 }
            if let s = sink.size, s.0 != added.width || s.1 != added.height { vdLog("失败 画面大小与新屏不一致"); failures += 1 }
            ch.stop()
            sink.decoder.invalidate()
        } else { failures += 1 }

        if let path = shot {
            if let img = await vdStep("screenshot(\(added.id))", { try await session.screenshot(displayId: added.id, maxWidth: 1280) }) {
                do {
                    try img.data.write(to: URL(fileURLWithPath: path))
                    vdLog("  截图 \(img.mime) \(img.data.count) 字节 → \(path)")
                } catch { vdLog("失败 写截图：\(error)"); failures += 1 }
            } else { failures += 1 }
        }

        if await vdStep("remove_virtual_display(\(added.id))", { try await session.removeVirtualDisplay(displayId: added.id) }) == nil { failures += 1 }
        if let list = await vdStep("list_displays（删屏后）", { try await session.listDisplays() }) {
            for d in list { vdLog("  屏幕 \(d.id) \(d.name) \(d.width)x\(d.height) 主屏 \(d.primary) 虚拟 \(d.isVirtual)") }
            if list.contains(where: { $0.id == added.id }) { vdLog("失败 删掉的屏还在列表里"); failures += 1 }
        } else { failures += 1 }
        let again = await vdStep("再删一次（应报错）") { () async throws -> String in
            do {
                try await session.removeVirtualDisplay(displayId: added.id)
            } catch {
                return AppModel.describe(error)
            }
            throw VDError(description: "删掉的屏再删没有报错")
        }
        if let why = again { vdLog("  报错：\(why)") } else { failures += 1 }

        session.disconnect()
        return await finish()
    }
}
#endif
