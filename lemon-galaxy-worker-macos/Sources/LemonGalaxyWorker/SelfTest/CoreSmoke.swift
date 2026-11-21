import Foundation
import AppKit

/// --core-smoke <服务器地址> <用户名> <密码> [--hold 秒]
/// 不开窗口、不碰钥匙串（内存 SecureStore）、数据目录用临时目录，按顺序验证真内核的 WorkerHost / HostService 绑定：
/// core_init(worker) → WorkerHost.start（应为空）→ login → service → set_platform / set_listener → 等上线 →
/// 状态与账号 → 改权限再改回 → 临时协助开 / 换码 / 关 →（--hold 期间可用 galaxy-dev 连进来）→ 事件 → logout → 账号清空。
/// 每步打印结果与耗时；全部通过退出码 0，任何一步失败为 1。
/// logout 会让服务器删掉这台测试设备与该账号的关联（设备从账号的设备列表里消失）；服务器 devices 表里的设备记录本身会保留，
/// 可在管理后台「设备」里删除。
enum CoreSmoke {
    static func run(_ args: [String]) -> Int32 {
        #if LEMONGALAXY_REAL_CORE
        guard let i = args.firstIndex(of: "--core-smoke"), i + 3 < args.count else {
            print("用法：LemonGalaxyWorker --core-smoke <服务器地址> <用户名> <密码> [--hold 秒]")
            return 2
        }
        let server = args[i + 1], user = args[i + 2], pass = args[i + 3]
        var hold: Double = 0
        if let j = args.firstIndex(of: "--hold") {
            guard j + 1 < args.count, let v = Double(args[j + 1]), v >= 0 else {
                print("--hold 后面要跟秒数")
                return 2
            }
            hold = v
        }
        // 平台层在主线程上建（它读 NSScreen），之后主线程阻塞等测试结束
        _ = NSApplication.shared
        let platform = MacHostPlatform()
        let sem = DispatchSemaphore(value: 0)
        nonisolated(unsafe) var code: Int32 = 1
        Task.detached {
            code = await SmokeRunner(server: server, user: user, pass: pass, hold: hold, platform: platform).run()
            sem.signal()
        }
        sem.wait()
        return code
        #else
        print("这个构建没有接入 Rust 内核（缺 Vendor/LemonGalaxyCore.xcframework），无法做冒烟测试")
        return 2
        #endif
    }
}

#if LEMONGALAXY_REAL_CORE
private let started = Date()
private func log(_ s: String) {
    let t = String(format: "%7.3f", Date().timeIntervalSince(started))
    print("[\(t)s] \(s)")
    fflush(stdout)
}

private final class SmokeHostListener: HostListener, @unchecked Sendable {
    private let lock = NSLock()
    private var _statusCount = 0
    private var _sessions: [String] = []
    var statusCount: Int { lock.locked { _statusCount } }
    var sessionsSeen: [String] { lock.locked { _sessions } }

    func onStatus(status: HostStatus) {
        let n = lock.locked { () -> Int in _statusCount += 1; return _statusCount }
        if n <= 3 || n % 10 == 0 {
            log("  [回调] on_status #\(n) online=\(status.online) remote=\(status.remoteEnabled) 账号 \(status.accounts.count) 会话 \(status.sessions.count)")
        }
    }
    func onAssistRequest(requestId: String, fromUser: String, fromDevice: String) {
        log("  [回调] on_assist_request \(fromUser) \(fromDevice)（冒烟测试不处理，按超时拒绝）")
    }
    func onSessionStarted(info: HostSessionInfo) {
        lock.locked { _sessions.append(info.id) }
        log("  [回调] on_session_started \(info.id) 来自 \(info.fromUser) 设备 \(info.fromDevice.prefix(8)) \(info.mode) 路径 \(info.path)")
    }
    func onSessionEnded(sessionId: String) { log("  [回调] on_session_ended \(sessionId)") }
    func onAiActivity(sessionId: String, description: String) { log("  [回调] on_ai_activity \(sessionId) \(description)") }
}

private struct SmokeRunner {
    let server: String
    let user: String
    let pass: String
    let hold: Double
    let platform: MacHostPlatform

    /// 执行一步并打印结果与耗时；失败返回 nil
    func step<T>(_ name: String, _ body: () async throws -> T) async -> T? {
        let t0 = Date()
        log("开始 \(name)")
        do {
            let v = try await body()
            log(String(format: "通过 %@（%.0f ms）", name, Date().timeIntervalSince(t0) * 1000))
            return v
        } catch {
            log(String(format: "失败 %@（%.0f ms）：%@", name, Date().timeIntervalSince(t0) * 1000, describeError(error)))
            return nil
        }
    }

    func fail(_ s: String) -> GalaxyError { GalaxyError.Other(detail: s) }

    func run() async -> Int32 {
        var failures = 0
        let base = FileManager.default.temporaryDirectory.appendingPathComponent("lemongalaxy-worker-smoke-\(ProcessInfo.processInfo.processIdentifier)", isDirectory: true)
        let dataDir = base.appendingPathComponent("data"), cacheDir = base.appendingPathComponent("cache")
        try? FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        try? FileManager.default.createDirectory(at: cacheDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: base) }

        log("内核版本 \(coreVersion())；数据目录 \(base.path)")
        WorkerCore.initCoreOnce(WorkerCore.config(locale: "zh-CN", dataDir: dataDir, cacheDir: cacheDir, hostname: "LemonGalaxyWorker-smoke"))
        let worker = WorkerHost(store: MemorySecureStore())
        let deviceId = worker.deviceId()
        log("device_id \(deviceId)（前 8 位 \(deviceId.prefix(8))），server \(worker.server() ?? "无")")

        // 1. 还没有账号：start 应返回空
        if let s = await step("start（无账号）", { try await worker.start() }) {
            if s != nil { log("失败 没有账号时 start 不应启动被控服务"); failures += 1 }
        } else { failures += 1 }

        // 2. 登录
        guard let result = await step("login", { try await worker.login(server: server, username: user, password: pass, totp: nil, newPassword: nil, systemUser: nil) }) else {
            return 1
        }
        log("  登录结果 \(result)")
        guard case .success(let userId, let username) = result else {
            log("  需要两步验证或改密码，冒烟测试不处理")
            return 1
        }
        log("  已登录 \(username)（user_id \(userId)），server \(worker.server() ?? "无")")
        if worker.server() == nil { log("失败 登录后 server() 仍为空"); failures += 1 }

        // 3. 被控服务
        guard let service = worker.service() else {
            log("失败 登录后 service() 为空")
            return 1
        }
        let listener = SmokeHostListener()
        service.setListener(listener: listener)
        service.setPlatform(platform: platform)
        log("  平台能力 \(platform.capabilities())（未授权的能力不会出现）")
        if let again = await step("start（已有账号，应返回同一服务）", { try await worker.start() }) {
            if again == nil { log("失败 有账号时 start 返回空"); failures += 1 }
        } else { failures += 1 }

        let online = await step("等待上线") { () async throws -> HostStatus in
            for _ in 0..<200 {
                let s = service.status()
                if s.online { return s }
                try await Task.sleep(nanoseconds: 100_000_000)
            }
            throw fail("20 秒内没有上线")
        }
        if online == nil { failures += 1 }

        // 4. 状态与账号
        let st = service.status()
        log("  状态 online=\(st.online) remote_enabled=\(st.remoteEnabled) device_id=\(st.deviceId) public_ip=\(st.publicIp.isEmpty ? "（未知）" : st.publicIp)")
        if st.deviceId != deviceId { log("失败 HostStatus.device_id 与 WorkerHost.device_id 不一致"); failures += 1 }
        let accounts = service.accounts()
        for a in accounts {
            let pl = a.policy
            log("  账号 \(a.username)（\(a.displayName)）user_id=\(a.userId) \(a.status) 系统用户 \(pl.systemUser) 画面=\(pl.view) 控制=\(pl.control) 文件=\(pl.files) 终端=\(pl.terminal) 剪贴板=\(pl.clipboard) 声音=\(pl.audio) AI上限=\(pl.aiCap) 提示=\(pl.notify)")
        }
        guard let acct = accounts.first(where: { $0.userId == userId }) else {
            log("失败 accounts() 里没有刚登录的账号")
            return 1
        }
        if acct.policy.systemUser != NSUserName() { log("  注意：系统用户 \(acct.policy.systemUser) 与当前用户 \(NSUserName()) 不同") }

        // 5. 改权限再改回
        if await step("update_account_policy", { () async throws -> Void in
            var p = acct.policy
            p.aiCap = .readOnly
            p.clipboard = false
            service.updateAccountPolicy(userId: userId, policy: p)
            guard let now = service.accounts().first(where: { $0.userId == userId })?.policy, now.aiCap == .readOnly, !now.clipboard else {
                throw fail("改完读回的权限不对")
            }
            service.updateAccountPolicy(userId: userId, policy: acct.policy)
            guard service.accounts().first(where: { $0.userId == userId })?.policy == acct.policy else { throw fail("改回原值失败") }
        }) == nil { failures += 1 }

        // 6. 禁止 / 允许远程
        if await step("set_remote_enabled(false → true)", { () async throws -> Void in
            service.setRemoteEnabled(enabled: false)
            guard !service.status().remoteEnabled else { throw fail("禁止后状态仍为允许") }
            service.setRemoteEnabled(enabled: true)
            guard service.status().remoteEnabled else { throw fail("允许后状态仍为禁止") }
        }) == nil { failures += 1 }

        // 7. 临时协助
        if let a = await step("start_assist", { try await service.startAssist() }) {
            log("  设备码 \(a.deviceCode) 验证码 \(a.code)")
            if await step("set_assist_code（自定义）", { () async throws -> Void in
                service.setAssistCode(code: "SMOKE2026TEST")
                try await Task.sleep(nanoseconds: 300_000_000)
                guard service.status().assist?.code == "SMOKE2026TEST" else { throw fail("自定义验证码没有生效：\(service.status().assist?.code ?? "无")") }
                service.setAssistCode(code: nil)
            }) == nil { failures += 1 }
            service.stopAssist()
            let stopped: Void? = await step("stop_assist") { () async throws -> Void in
                for _ in 0..<50 {
                    if service.status().assist == nil { return }
                    try await Task.sleep(nanoseconds: 100_000_000)
                }
                throw fail("5 秒内临时协助没有关闭")
            }
            if stopped == nil { failures += 1 }
            // 协助关掉后信令会重连，等回到在线
            _ = await step("协助关闭后重新上线") { () async throws -> Void in
                for _ in 0..<150 {
                    if service.status().online { return }
                    try await Task.sleep(nanoseconds: 100_000_000)
                }
                throw fail("15 秒内没有重新上线")
            }
        } else { failures += 1 }

        // 8. 保持在线，供 galaxy-dev 等对端连进来验证
        if hold > 0 {
            log("保持在线 \(Int(hold)) 秒：可用 galaxy-dev --data <主控数据目录> exec \(deviceId.prefix(8)) \"echo hi\" 连进来")
            let end = Date().addingTimeInterval(hold)
            var lastSessions = -1
            while Date() < end {
                let n = service.status().sessions.count
                if n != lastSessions { log("  当前会话 \(n) 个"); lastSessions = n }
                try? await Task.sleep(nanoseconds: 500_000_000)
            }
            log("  保持期间连进来的会话：\(listener.sessionsSeen.count) 个")
        }

        // 9. 事件
        let events = service.recentEvents(limit: 50)
        log("最近事件 \(events.count) 条：")
        for e in events.suffix(15) {
            log("    \(e.kind) \(e.level == .warning ? "[警告] " : "")\(e.ai ? "[AI] " : "")\(e.user) \(e.detail)")
        }
        log("状态回调共 \(listener.statusCount) 次")
        if listener.statusCount == 0 { log("失败 没有收到任何 on_status 回调"); failures += 1 }

        // 10. 退出账号
        if await step("logout", { try await worker.logout(userId: userId) }) == nil { failures += 1 }
        let left = service.accounts()
        if !left.isEmpty { log("失败 退出后仍有账号 \(left.map(\.username))"); failures += 1 } else { log("  退出后本机账号已清空") }
        service.disconnectAll()

        log(failures == 0 ? "全部通过" : "失败 \(failures) 项")
        return failures == 0 ? 0 : 1
    }
}
#endif
