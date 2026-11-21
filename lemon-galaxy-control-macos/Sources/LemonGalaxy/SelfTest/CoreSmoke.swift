import Foundation

/// --core-smoke <服务器地址> <用户名> <密码>
/// 不开窗口、不碰钥匙串（内存 SecureStore）、数据目录用临时目录，按顺序验证真内核绑定：
/// core_init → login → 等上线 → devices → connect 第一台在线 worker → exec → 终端收发 → 列目录 → ai_models → logout。
/// 每步打印结果与耗时；全部通过退出码 0，否则 1。
enum CoreSmoke {
    static func run(_ args: [String]) -> Int32 {
        #if LEMONGALAXY_REAL_CORE
        guard let i = args.firstIndex(of: "--core-smoke"), i + 3 < args.count else {
            print("用法：LemonGalaxy --core-smoke <服务器地址> <用户名> <密码>")
            return 2
        }
        let server = args[i + 1], user = args[i + 2], pass = args[i + 3]
        let sem = DispatchSemaphore(value: 0)
        nonisolated(unsafe) var code: Int32 = 1
        Task.detached {
            code = await Runner(server: server, user: user, pass: pass).run()
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
private final class SmokeListener: ClientListener, @unchecked Sendable {
    func onState(state: ClientState) { log("  [回调] on_state \(state)") }
    func onDevicesChanged() { log("  [回调] on_devices_changed") }
    func onVaultChanged() { log("  [回调] on_vault_changed") }
    func onSessionRevoked() { log("  [回调] on_session_revoked") }
}

private final class SmokeTerminalSink: TerminalSink, @unchecked Sendable {
    private let lock = NSLock()
    private var buffer = Data()
    private(set) var exited: Int32??
    func onOutput(data: Data) { lock.locked { buffer.append(data) } }
    func onExit(code: Int32?) { lock.locked { exited = .some(code) } }
    var text: String { lock.locked { String(decoding: buffer, as: UTF8.self) } }
}

private let started = Date()
private func log(_ s: String) {
    let t = String(format: "%7.3f", Date().timeIntervalSince(started))
    print("[\(t)s] \(s)")
    fflush(stdout)
}

/// 去掉终端输出里的 ANSI 控制序列，便于打印
private func plain(_ s: String) -> String {
    s.replacingOccurrences(of: "\u{1b}\\[[0-9;?]*[ -/]*[@-~]", with: "", options: .regularExpression)
        .replacingOccurrences(of: "\u{1b}\\][^\u{07}]*\u{07}", with: "", options: .regularExpression)
        .replacingOccurrences(of: "\r", with: "")
}

private struct Runner {
    let server: String
    let user: String
    let pass: String

    func step<T>(_ name: String, _ body: () async throws -> T) async -> T? {
        let t0 = Date()
        log("开始 \(name)")
        do {
            let v = try await body()
            log(String(format: "通过 %@（%.0f ms）", name, Date().timeIntervalSince(t0) * 1000))
            return v
        } catch {
            log(String(format: "失败 %@（%.0f ms）：%@", name, Date().timeIntervalSince(t0) * 1000, AppModel.describe(error)))
            return nil
        }
    }

    func run() async -> Int32 {
        var failures = 0
        let base = FileManager.default.temporaryDirectory.appendingPathComponent("lemongalaxy-smoke-\(ProcessInfo.processInfo.processIdentifier)", isDirectory: true)
        let dataDir = base.appendingPathComponent("data"), cacheDir = base.appendingPathComponent("cache")
        try? FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        try? FileManager.default.createDirectory(at: cacheDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: base) }

        log("内核版本 \(coreVersion())；数据目录 \(base.path)")
        coreInit(config: CoreConfig(dataDir: dataDir.path, cacheDir: cacheDir.path, logLevel: "info", platform: .macos, deviceKind: .control,
                                    appVersion: AppInfo.version, osVersion: AppInfo.osVersion, hostname: "LemonGalaxy-smoke", locale: "zh-CN"))
        let listener = SmokeListener()
        let client = GalaxyClient(store: MemorySecureStore(), listener: listener)
        log("device_id \(client.deviceId())，状态 \(client.state())")

        guard let result = await step("login", { try await client.login(server: server, username: user, password: pass, totp: nil) }) else {
            return 1
        }
        log("  登录结果 \(result)")
        guard result == .success else { log("  需要两步验证或改密码，冒烟测试不处理"); return 1 }
        if let u = client.currentUser() { log("  用户 \(u.username)（\(u.displayName)）AI \(u.aiEnabled) TOTP \(u.totpEnabled)") }
        if let s = client.server() { log("  服务器 \(s.name) \(s.version)") }

        let online = await step("等待上线") { () async throws -> Bool in
            for _ in 0..<200 {
                if client.state() == .online { return true }
                try await Task.sleep(nanoseconds: 100_000_000)
            }
            throw GalaxyError.Network(message: "20 秒内没有上线，当前状态 \(client.state())")
        }
        if online == nil { failures += 1 }

        _ = await step("refresh_devices") { try await client.refreshDevices() }
        let devices = client.devices()
        log("  设备 \(devices.count) 台：")
        for d in devices {
            log("    \(d.isSelf ? "*" : " ") \(d.hostname) \(d.kind) \(d.platform) \(d.osVersion) online=\(d.online) lan=\(d.lanSeen) id=\(d.id)")
        }
        log("  本机被控服务：\(client.host() == nil ? "无" : "有")")

        guard let target = devices.first(where: { $0.kind == .worker && $0.online && !$0.isSelf }) else {
            log("失败 没有在线的 worker 可以连接")
            _ = await step("logout") { try await client.logout() }
            return 1
        }
        log("  目标 \(target.hostname)（\(target.id)）")

        guard let session = await step("connect", { try await client.connect(deviceId: target.id) }) else {
            _ = await step("logout") { try await client.logout() }
            return 1
        }
        let info = session.info()
        log("  路径 \(session.path().kind) rtt=\(session.path().rttMs)ms \(session.path().remoteAddr)")
        log("  对端 \(info.hostname) \(info.platform) \(info.osVersion) \(info.arch) 用户 \(info.systemUser) shell \(info.shell)")
        log("  权限 view=\(info.permissions.view) control=\(info.permissions.control) files=\(info.permissions.files) terminal=\(info.permissions.terminal) ai=\(info.permissions.aiCap)")
        log("  能力 \(info.capabilities.joined(separator: ","))")

        if let r = await step("exec", {
            try await session.exec(request: ExecRequest(command: "echo hello && uname -a", shell: nil, cwd: nil, timeoutMs: 15_000, maxOutput: 65_536))
        }) {
            log("  退出码 \(r.code.map(String.init) ?? "无") 超时 \(r.timedOut) 截断 \(r.truncated) 耗时 \(r.durationMs)ms")
            for l in r.stdout.split(separator: "\n") { log("  stdout| \(l)") }
            for l in r.stderr.split(separator: "\n") { log("  stderr| \(l)") }
            if !r.stdout.contains("hello") { log("失败 exec 输出里没有 hello"); failures += 1 }
        } else { failures += 1 }

        let sink = SmokeTerminalSink()
        if let term = await step("open_terminal", {
            try await session.openTerminal(options: TerminalOptions(attachSession: nil, shell: nil, cols: 100, rows: 30,
                                                                     env: [EnvVar(name: "TERM", value: "xterm-256color")]), sink: sink)
        }) {
            log("  会话 \(term.sessionId()) 标题 \(term.title())")
            let ok = await step("终端收发") { () async throws -> Bool in
                try await Task.sleep(nanoseconds: 300_000_000)
                term.write(data: Data("echo smoke-$((40+2))\n".utf8))
                for _ in 0..<100 {
                    if sink.text.contains("smoke-42") { return true }
                    try await Task.sleep(nanoseconds: 100_000_000)
                }
                throw GalaxyError.Other(message: "10 秒内没有收到命令输出")
            }
            let lines = plain(sink.text).split(separator: "\n").suffix(6)
            for l in lines { log("  term| \(l)") }
            if ok == nil { failures += 1 }
            if let list = try? await session.listTerminals() {
                log("  被控端终端会话 \(list.count) 个：" + list.map { "\($0.id)(\($0.attached ? "已连" : "保留"))" }.joined(separator: " "))
            }
            term.kill()
        } else { failures += 1 }

        let files = session.files()
        var entries = await step("files.list(\"~\")") { try await files.list(path: "~") }
        if entries == nil {
            if let home = await step("files.home", { try await files.home() }) {
                log("  主目录 \(home)")
                entries = await step("files.list(home)") { try await files.list(path: home) }
            }
        }
        if let entries {
            log("  共 \(entries.count) 项，前 10 项：")
            for e in entries.prefix(10) { log("    \(e.kind) \(Fmt.mode(e.mode)) \(e.size) \(e.name)") }
        } else { failures += 1 }

        if let m = await step("ai_models", { try await client.aiModels() }) {
            log("  AI 开通 \(m.enabled) 上限 \(m.maxMode) 模型 \(m.items.count) 个")
            for it in m.items.prefix(10) { log("    \(it.providerName) / \(it.name)（\(it.id)）看图=\(it.vision) 上下文=\(it.contextLength)") }
        } else { failures += 1 }

        session.disconnect()
        if await step("logout", { try await client.logout() }) == nil { failures += 1 }
        log("  最终状态 \(client.state())")
        log(failures == 0 ? "全部通过" : "有 \(failures) 步失败")
        return failures == 0 ? 0 : 1
    }
}
#endif
