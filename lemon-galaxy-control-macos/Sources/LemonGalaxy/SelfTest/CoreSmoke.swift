import Foundation

/// --core-smoke <服务器地址> <用户名> <密码> [--ssh <主机:端口> <SSH 用户名> <私钥文件>]
/// 不开窗口、不碰钥匙串（内存 SecureStore）、数据目录用临时目录，按顺序验证真内核绑定：
/// core_init → login → 等上线 → devices → connect 第一台在线 worker → exec → 终端收发 → 列目录 → ai_models →（SSH 直连一段）→ logout。
/// 带 --ssh 时：在加密库建临时 credential（私钥）与 ssh_host 条目 → connect_ssh（自动信任主机密钥）→ exec → 终端收发 →
/// SFTP 列目录、上传、下载、比对、删除 → 端口转发 → 再连一次确认主机密钥已记录 → AI 对话以 SSH 主机为目标 → 删掉临时条目。
/// 每步打印结果与耗时；全部通过退出码 0，否则 1。
enum CoreSmoke {
    static func run(_ args: [String]) -> Int32 {
        #if LEMONGALAXY_REAL_CORE
        guard let i = args.firstIndex(of: "--core-smoke"), i + 3 < args.count else {
            print("用法：LemonGalaxy --core-smoke <服务器地址> <用户名> <密码> [--ssh <主机:端口> <SSH 用户名> <私钥文件>]")
            return 2
        }
        let server = args[i + 1], user = args[i + 2], pass = args[i + 3]
        var ssh: SshSpec?
        if let j = args.firstIndex(of: "--ssh") {
            guard j + 3 < args.count, let spec = SshSpec(address: args[j + 1], username: args[j + 2], keyFile: args[j + 3]) else {
                print("--ssh 参数不对：--ssh <主机:端口> <SSH 用户名> <私钥文件>")
                return 2
            }
            ssh = spec
        }
        let sem = DispatchSemaphore(value: 0)
        nonisolated(unsafe) var code: Int32 = 1
        Task.detached {
            code = await Runner(server: server, user: user, pass: pass, ssh: ssh).run()
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

/// --ssh 段的目标：主机、端口、用户名、私钥
struct SshSpec {
    let host: String
    let port: UInt16
    let username: String
    let keyFile: String

    init?(address: String, username: String, keyFile: String) {
        let parts = address.split(separator: ":", omittingEmptySubsequences: false)
        guard parts.count <= 2, let h = parts.first, !h.isEmpty else { return nil }
        host = String(h)
        if parts.count == 2 { guard let p = UInt16(parts[1]) else { return nil }; port = p } else { port = 22 }
        self.username = username
        self.keyFile = keyFile
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

/// 执行一步并打印结果与耗时；失败返回 nil
private func smokeStep<T>(_ name: String, _ body: () async throws -> T) async -> T? {
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

private struct Runner {
    let server: String
    let user: String
    let pass: String
    let ssh: SshSpec?

    func step<T>(_ name: String, _ body: () async throws -> T) async -> T? { await smokeStep(name, body) }

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
            throw GalaxyError.Network(detail: "20 秒内没有上线，当前状态 \(client.state())")
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
                throw GalaxyError.Other(detail: "10 秒内没有收到命令输出")
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

        log("  对端隐私屏能力：\(info.capabilities.contains("privacy") ? "有" : "没有")（冒烟测试不切换，免得把真机屏幕黑掉）")
        session.disconnect()
        failures += await updateCheck(client: client)
        if let ssh {
            failures += await SshSmoke(client: client, spec: ssh).run()
            failures += await rdpForward(client: client, viaDevice: target.id, port: ssh.port)
        }
        if await step("logout", { try await client.logout() }) == nil { failures += 1 }
        log("  最终状态 \(client.state())")
        log(failures == 0 ? "全部通过" : "有 \(failures) 步失败")
        return failures == 0 ? 0 : 1
    }
}

// MARK: - 检查更新

/// check_update：有新版本时下载到临时目录（内核核对 SHA-256）
private func updateCheck(client: GalaxyClient) async -> Int {
    guard let r = await smokeStep("check_update(control, macos, 0.0.1)", {
        try await client.checkUpdate(component: "control", platform: "macos", arch: "", current: "0.0.1")
    }) else { return 1 }
    guard let info = r else {
        log("  服务器上没有比 0.0.1 新的 control/macos 发布（或未配置更新源）")
        return 0
    }
    log("  新版本 \(info.version) \(info.fileName) \(info.size) 字节 sha256=\(info.sha256.prefix(16))… 地址 \(info.url)")
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("lemongalaxy-update-smoke-\(ProcessInfo.processInfo.processIdentifier)", isDirectory: true)
    try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    let dest = dir.appendingPathComponent((info.fileName as NSString).lastPathComponent)
    let l = SmokeTransferListener()
    let ok = await smokeStep("download_update") { () async throws -> Bool in
        client.downloadUpdate(info: info, dest: dest.path, listener: l)
        if let e = try await l.wait(seconds: 300) { throw GalaxyError.Other(detail: e) }
        let size = (try? FileManager.default.attributesOfItem(atPath: dest.path)[.size] as? NSNumber)?.uint64Value ?? 0
        guard size == info.size else { throw GalaxyError.Other(detail: "下载大小 \(size) 与清单 \(info.size) 不一致") }
        return true
    }
    return ok == nil ? 1 : 0
}

// MARK: - RDP 经设备转发

/// 建临时 rdp_host（经第一台在线 worker、目标是本机的 SSH 端口代替 RDP 服务），open_rdp 后从转发端口读到 SSH 标识，
/// close_rdp 后端口应连不上；再测不经设备的直连地址；最后删掉临时条目。
private func rdpForward(client: GalaxyClient, viaDevice: String, port: UInt16) async -> Int {
    var failures = 0
    let tag = "smoke-\(ProcessInfo.processInfo.processIdentifier)"
    var created: [String] = []
    guard let viaEntry = await smokeStep("vault_save(rdp_host 经设备)", {
        try await client.vaultSave(entry: VaultEntry(id: "", folderId: nil, sort: 0,
                                                     data: .rdpHost(label: tag + "-rdp", host: "127.0.0.1", port: port, username: "smoke", password: "不该出现在链接里",
                                                                    domain: "CORP", jumpDeviceId: viaDevice, tags: [], note: "", favorite: false),
                                                     updatedAt: 0))
    }) else { return 1 }
    created.append(viaEntry.id)
    if let t = await smokeStep("open_rdp（经设备）", { try await client.openRdp(hostId: viaEntry.id) }) {
        log("  \(t.label) → \(t.host):\(t.port) 用户 \(t.username) 域 \(t.domain ?? "无") 转发 \(t.forwarded) id=\(t.forwardId)")
        if let url = RdpLauncher.url(for: t) { log("  交给远程桌面应用的链接 \(url.absoluteString)") }
        if !t.forwarded || t.host != "127.0.0.1" || t.forwardId.isEmpty { log("失败 经设备时应在本机开转发端口"); failures += 1 }
        if RdpLauncher.url(for: t)?.absoluteString.contains("该出现") == true { log("失败 链接里出现了密码"); failures += 1 }
        if let banner = await smokeStep("经 RDP 转发读标识", { try await Task.detached { try readBanner(port: t.port) }.value }) {
            log("  读到 \(banner.trimmingCharacters(in: .whitespacesAndNewlines))")
            if !banner.hasPrefix("SSH-") { log("失败 转发过去读到的不对"); failures += 1 }
        } else { failures += 1 }
        client.closeRdp(forwardId: t.forwardId)
        let closed = await smokeStep("close_rdp 后端口关闭") { () async throws -> Bool in
            for _ in 0..<20 {
                if (try? await Task.detached { try readBanner(port: t.port) }.value) == nil { return true }
                try await Task.sleep(nanoseconds: 100_000_000)
            }
            throw GalaxyError.Other(detail: "关掉转发 2 秒后端口仍能连上")
        }
        if closed == nil { failures += 1 }
    } else { failures += 1 }

    if let direct = await smokeStep("vault_save(rdp_host 直连)", {
        try await client.vaultSave(entry: VaultEntry(id: "", folderId: nil, sort: 0,
                                                     data: .rdpHost(label: tag + "-rdp-direct", host: "10.0.3.5", port: 0, username: "Administrator", password: nil,
                                                                    domain: nil, jumpDeviceId: nil, tags: [], note: "", favorite: false),
                                                     updatedAt: 0))
    }) {
        created.append(direct.id)
        if let t = await smokeStep("open_rdp（直连）", { try await client.openRdp(hostId: direct.id) }) {
            log("  \(t.host):\(t.port) 转发 \(t.forwarded) 链接 \(RdpLauncher.url(for: t)?.absoluteString ?? "无")")
            if t.forwarded || t.host != "10.0.3.5" || t.port != 3389 { log("失败 直连应原样给出地址，端口 0 按 3389"); failures += 1 }
        } else { failures += 1 }
    } else { failures += 1 }

    for id in created {
        if await smokeStep("vault_delete(\(id))", { try await client.vaultDelete(id: id) }) == nil { failures += 1 }
    }
    return failures
}

// MARK: - SSH 直连

/// 冒烟测试里的 SecretPrompt：主机密钥自动信任（记下被问了几次），要密码一律取消（私钥登录不该问）
private final class SmokePrompt: SecretPrompt, @unchecked Sendable {
    private let lock = NSLock()
    private var _hostKeyAsks = 0
    private var _asks = 0
    var hostKeyAsks: Int { lock.locked { _hostKeyAsks } }
    var asks: Int { lock.locked { _asks } }

    func ask(prompt: String, echo: Bool) -> String? {
        lock.locked { _asks += 1 }
        log("  [询问] ask echo=\(echo)：\(prompt.trimmingCharacters(in: .whitespacesAndNewlines))，冒烟测试回答取消")
        return nil
    }

    func confirmHostKey(host: String, port: UInt16, keyType: String, fingerprint: String, changed: Bool) -> Bool {
        lock.locked { _hostKeyAsks += 1 }
        log("  [询问] confirm_host_key \(host):\(port) \(keyType) \(fingerprint) changed=\(changed)，自动信任")
        return !changed
    }
}

private final class SmokeTransferListener: TransferListener, @unchecked Sendable {
    private let lock = NSLock()
    private var result: String??
    private var last: (UInt64, UInt64) = (0, 0)
    func onProgress(done: UInt64, total: UInt64) { lock.locked { last = (done, total) } }
    func onFinished(error: String?) { lock.locked { result = .some(error) } }
    var finished: String?? { lock.locked { result } }
    var progress: (UInt64, UInt64) { lock.locked { last } }

    /// 等传输结束，返回错误文字（nil 为成功）
    func wait(seconds: Double = 30) async throws -> String? {
        let end = Date().addingTimeInterval(seconds)
        while Date() < end {
            if let r = finished { return r }
            try await Task.sleep(nanoseconds: 50_000_000)
        }
        throw GalaxyError.Other(detail: "\(Int(seconds)) 秒内传输没有结束")
    }
}

private final class SmokeAgentListener: AgentListener, @unchecked Sendable {
    func onEvent(event: AgentEvent) { log("  [AI 事件] \(String(describing: event).prefix(160))") }
}

private struct SshSmoke {
    let client: GalaxyClient
    let spec: SshSpec

    func step<T>(_ name: String, _ body: () async throws -> T) async -> T? { await smokeStep(name, body) }

    func knownHostIds() -> Set<String> {
        Set(client.vault().compactMap { e -> String? in
            if case .knownHost(let h, let p, _, _) = e.data, h.caseInsensitiveCompare(spec.host) == .orderedSame, p == spec.port { return e.id }
            return nil
        })
    }

    /// 返回失败步数
    func run() async -> Int {
        var failures = 0
        log("SSH 直连段：\(spec.username)@\(spec.host):\(spec.port)，私钥 \(spec.keyFile)")
        guard let key = try? String(contentsOfFile: spec.keyFile, encoding: .utf8), !key.isEmpty else {
            log("失败 读不到私钥文件 \(spec.keyFile)")
            return 1
        }
        let knownBefore = knownHostIds()
        let tag = "smoke-\(ProcessInfo.processInfo.processIdentifier)"

        guard let cred = await step("vault_save(credential)", {
            try await client.vaultSave(entry: VaultEntry(id: "", folderId: nil, sort: 0,
                                                         data: .credential(label: tag + "-key", username: spec.username, password: nil, privateKey: key, passphrase: nil),
                                                         updatedAt: 0))
        }) else { return 1 }
        log("  凭据条目 \(cred.id)")
        var created = [cred.id]

        let hostEntry = await step("vault_save(ssh_host)") {
            try await client.vaultSave(entry: VaultEntry(id: "", folderId: nil, sort: 0,
                                                         data: .sshHost(label: tag + "-host", host: spec.host, port: spec.port, username: spec.username,
                                                                        auth: .privateKey(credentialId: cred.id), jumpDeviceId: nil, jumpHostId: nil,
                                                                        tags: ["smoke"], note: "", favorite: false, terminalTheme: nil),
                                                         updatedAt: 0))
        }
        if let hostEntry {
            created.append(hostEntry.id)
            log("  主机条目 \(hostEntry.id)")
            failures += await exercise(hostId: hostEntry.id, knownBefore: knownBefore)
        } else { failures += 1 }

        // 清理：删掉临时条目，以及这次测试新记下的主机密钥
        let newKnown = knownHostIds().subtracting(knownBefore)
        for id in created.reversed() + Array(newKnown) {
            if await step("vault_delete(\(id))", { try await client.vaultDelete(id: id) }) == nil { failures += 1 }
        }
        let left = client.vault().filter { created.contains($0.id) || newKnown.contains($0.id) }
        log("  清理后残留 \(left.count) 条")
        if !left.isEmpty { failures += 1 }
        return failures
    }

    func exercise(hostId: String, knownBefore: Set<String>) async -> Int {
        var failures = 0
        let prompt = SmokePrompt()
        guard let session = await step("connect_ssh", { try await client.connectSsh(hostId: hostId, prompt: prompt) }) else {
            return 1
        }
        let info = session.info()
        log("  会话 \(info.username)@\(info.host):\(info.port) 标签 \(info.label) via=\(info.via) 已关闭=\(session.isClosed())")
        log("  主机密钥确认 \(prompt.hostKeyAsks) 次，其他询问 \(prompt.asks) 次")
        if info.via != "direct" { log("失败 via 应为 direct"); failures += 1 }
        if knownBefore.isEmpty && prompt.hostKeyAsks != 1 { log("失败 首次连接应确认一次主机密钥"); failures += 1 }
        if prompt.asks != 0 { log("失败 私钥登录不应再问密码"); failures += 1 }
        let known = knownHostIds()
        log("  加密库里这台主机的 known_host \(known.count) 条")
        if known.isEmpty { log("失败 确认过的主机密钥没有存进加密库"); failures += 1 }

        if let r = await step("ssh.exec", {
            try await session.exec(request: ExecRequest(command: "echo hello-ssh && uname -sr && pwd", shell: nil, cwd: nil, timeoutMs: 15_000, maxOutput: 65_536))
        }) {
            log("  退出码 \(r.code.map(String.init) ?? "无") 超时 \(r.timedOut) 截断 \(r.truncated) 耗时 \(r.durationMs)ms")
            for l in r.stdout.split(separator: "\n") { log("  stdout| \(l)") }
            for l in r.stderr.split(separator: "\n") { log("  stderr| \(l)") }
            if !r.stdout.contains("hello-ssh") || r.code != 0 { log("失败 exec 输出不对"); failures += 1 }
        } else { failures += 1 }

        if let r = await step("ssh.exec(cwd=/tmp)", {
            try await session.exec(request: ExecRequest(command: "pwd", shell: nil, cwd: "/tmp", timeoutMs: 15_000, maxOutput: 4096))
        }) {
            log("  stdout| \(r.stdout.trimmingCharacters(in: .whitespacesAndNewlines))")
            if !r.stdout.contains("tmp") { log("失败 cwd 没有生效"); failures += 1 }
        } else { failures += 1 }

        let sink = SmokeTerminalSink()
        if let term = await step("ssh.open_terminal", {
            try await session.openTerminal(options: TerminalOptions(attachSession: nil, shell: nil, cols: 100, rows: 30,
                                                                     env: [EnvVar(name: "TERM", value: "xterm-256color")]), sink: sink)
        }) {
            log("  终端 \(term.sessionId()) 标题 \(term.title())")
            let ok = await step("ssh 终端收发") { () async throws -> Bool in
                try await Task.sleep(nanoseconds: 300_000_000)
                term.resize(cols: 120, rows: 32)
                term.write(data: Data("echo ssh-smoke-$((40+2)) && stty size\n".utf8))
                for _ in 0..<100 {
                    if sink.text.contains("ssh-smoke-42") && sink.text.contains("32 120") { return true }
                    try await Task.sleep(nanoseconds: 100_000_000)
                }
                throw GalaxyError.Other(detail: "10 秒内没有收到命令输出（或窗口大小没变成 32 120）")
            }
            for l in plain(sink.text).split(separator: "\n").suffix(4) { log("  term| \(l)") }
            if ok == nil { failures += 1 }
            term.write(data: Data("exit\n".utf8))
            let exited = await step("ssh 终端退出") { () async throws -> Bool in
                for _ in 0..<50 {
                    if sink.exited != nil { return true }
                    try await Task.sleep(nanoseconds: 100_000_000)
                }
                throw GalaxyError.Other(detail: "5 秒内没有收到 on_exit")
            }
            if exited == nil { failures += 1 } else { log("  on_exit 退出码 \(sink.exited!.map(String.init) ?? "无")") }
        } else { failures += 1 }

        let files = session.files()
        if let home = await step("sftp.home", { try await files.home() }) {
            log("  主目录 \(home)")
            if let entries = await step("sftp.list(home)", { try await files.list(path: home) }) {
                log("  共 \(entries.count) 项，前 6 项：")
                for e in entries.prefix(6) { log("    \(e.kind) \(Fmt.mode(e.mode)) \(e.size) \(e.name)") }
            } else { failures += 1 }
            failures += await transfers(files: files, home: home)
        } else { failures += 1 }

        if let fwd = await step("ssh.forward_port", { try await session.forwardPort(localPort: 0, remoteHost: "127.0.0.1", remotePort: spec.port) }) {
            let port = fwd.localPort()
            log("  本机 127.0.0.1:\(port) → 远端 127.0.0.1:\(spec.port)")
            if let banner = await step("经转发读 SSH 标识", { try await Task.detached { try readBanner(port: port) }.value }) {
                log("  读到 \(banner.trimmingCharacters(in: .whitespacesAndNewlines))")
                if !banner.hasPrefix("SSH-") { log("失败 转发过去读到的不是 SSH 标识"); failures += 1 }
            } else { failures += 1 }
            fwd.stop()
        } else { failures += 1 }

        session.disconnect()
        let closed = await step("ssh.disconnect") { () async throws -> Bool in
            for _ in 0..<50 {
                if session.isClosed() { return true }
                try await Task.sleep(nanoseconds: 100_000_000)
            }
            throw GalaxyError.Other(detail: "5 秒后 is_closed 仍为假")
        }
        if closed == nil { failures += 1 }

        // 再连一次：主机密钥已记录，不应再问
        let again = SmokePrompt()
        if let s2 = await step("connect_ssh（第二次）", { try await client.connectSsh(hostId: hostId, prompt: again) }) {
            log("  主机密钥确认 \(again.hostKeyAsks) 次")
            if again.hostKeyAsks != 0 { log("失败 已记录的主机密钥又被要求确认"); failures += 1 }
            s2.disconnect()
        } else { failures += 1 }

        failures += await aiTarget(hostId: hostId)
        return failures
    }

    func transfers(files: FileClient, home: String) async -> Int {
        var failures = 0
        let tmp = FileManager.default.temporaryDirectory.appendingPathComponent("lemongalaxy-ssh-smoke-\(ProcessInfo.processInfo.processIdentifier)", isDirectory: true)
        try? FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tmp) }
        let up = tmp.appendingPathComponent("up.bin"), down = tmp.appendingPathComponent("down.bin")
        var payload = Data("LemonGalaxy SSH smoke \(Date())\n".utf8)
        for i in 0..<20_000 { payload.append(UInt8(truncatingIfNeeded: i &* 31)) }
        try? payload.write(to: up)
        let remote = (home.hasSuffix("/") ? home : home + "/") + "lemongalaxy-smoke-\(ProcessInfo.processInfo.processIdentifier).bin"

        let ul = SmokeTransferListener()
        let upErr = await step("sftp.upload（\(payload.count) 字节）") { () async throws -> Bool in
            let t = files.upload(local: up.path, remote: remote, listener: ul)
            _ = t
            if let e = try await ul.wait() { throw GalaxyError.Other(detail: e) }
            return true
        }
        if upErr == nil { failures += 1 }
        if let st = await step("sftp.stat", { try await files.stat(path: remote) }) {
            log("  远端 \(st.path) \(st.size) 字节 \(Fmt.mode(st.mode))")
            if st.size != UInt64(payload.count) { log("失败 远端大小不对"); failures += 1 }
        } else { failures += 1 }

        let dl = SmokeTransferListener()
        let downOk = await step("sftp.download") { () async throws -> Bool in
            let t = files.download(remote: remote, local: down.path, listener: dl)
            _ = t
            if let e = try await dl.wait() { throw GalaxyError.Other(detail: e) }
            return true
        }
        if downOk == nil { failures += 1 }
        let same = (try? Data(contentsOf: down)) == payload
        log(same ? "  下载内容与上传一致" : "失败 下载内容与上传不一致")
        if !same { failures += 1 }

        let textPath = remote + ".txt"
        if await step("sftp.write_text / read_text", { () async throws -> Bool in
            try await files.writeText(path: textPath, text: "你好，SSH\n")
            let t = try await files.readText(path: textPath, maxBytes: 0)
            guard t == "你好，SSH\n" else { throw GalaxyError.Other(detail: "读回内容不对：\(t)") }
            return true
        }) == nil { failures += 1 }
        for path in [remote, textPath] {
            if await step("sftp.remove(\((path as NSString).lastPathComponent))", { try await files.remove(path: path, recursive: false) }) == nil { failures += 1 }
        }
        return failures
    }

    /// AI 对话以 SSH 主机为目标：内核用保存的私钥与已记录的主机密钥连接（不弹询问）。只建对话不发消息
    func aiTarget(hostId: String) async -> Int {
        guard let models = try? await client.aiModels(), models.enabled, let m = models.items.first else {
            log("  AI 未开通或没有模型，跳过 AI 目标检查")
            return 0
        }
        let config = ConversationConfig(providerId: m.providerId, modelId: m.id, targets: [AgentTarget(kind: .sshHost, id: hostId)],
                                        mode: .readOnly, planFirst: false, trustMinutes: nil, title: "ssh smoke")
        guard let conv = await step("new_conversation(SshHost)", { try await client.newConversation(config: config, listener: SmokeAgentListener()) }) else {
            return 1
        }
        log("  对话 \(conv.id())")
        conv.stop()
        client.deleteConversation(id: conv.id())
        return 0
    }
}

/// 连到本机端口读一行（SSH 服务端连上就先发标识 "SSH-2.0-..."）
private func readBanner(port: UInt16) throws -> String {
    let fd = socket(AF_INET, SOCK_STREAM, 0)
    guard fd >= 0 else { throw GalaxyError.Network(detail: "socket 失败") }
    defer { Darwin.close(fd) }
    var tv = timeval(tv_sec: 5, tv_usec: 0)
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
    var addr = sockaddr_in()
    addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
    addr.sin_family = sa_family_t(AF_INET)
    addr.sin_port = port.bigEndian
    addr.sin_addr.s_addr = inet_addr("127.0.0.1")
    let r = withUnsafePointer(to: &addr) { $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) } }
    guard r == 0 else { throw GalaxyError.Network(detail: "连不上 127.0.0.1:\(port)（errno \(errno)）") }
    var buf = [UInt8](repeating: 0, count: 256)
    let n = recv(fd, &buf, buf.count, 0)
    guard n > 0 else { throw GalaxyError.Network(detail: "5 秒内没有读到数据") }
    return String(decoding: buf[0..<n], as: UTF8.self)
}
#endif
