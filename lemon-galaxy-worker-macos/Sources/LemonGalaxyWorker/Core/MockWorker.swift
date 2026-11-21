import Foundation

/// 模拟内核的可调项（selftest 用：去掉延时、预置账号）。
struct MockWorkerOptions {
    /// 模拟网络延时倍率，0 表示立即返回
    var latencyScale: Double = 1
    /// 预置账号与服务器（selftest 用）；演示模式下从空白开始
    var presetAccounts = false
}

private var zh: Bool { Localization.shared.strings.isChinese }

/// 模拟的 WorkerHost：不连网络、不碰钥匙串和数据目录。
/// 登录规则（演示用）：任意密码可登录；用户名含 totp 先要两步验证码，含 new 先要改密码；密码为 wrong 时报错。
final class MockWorkerHost: WorkerHost, @unchecked Sendable {
    static let version = "0.0.0-mock"
    static let demoServer = "demo.lemongalaxy.test:7443#sha256-TW9ja0NvcmVEZW1vU2VydmVyRmluZ2VycHJpbnQwMDA"

    required init(unsafeFromHandle handle: UInt64) { fatalError("模拟对象不能由内核指针构造") }

    let options: MockWorkerOptions
    private let lock = NSLock()
    private var serverAddr: String?
    private var mock: MockWorkerService?
    private var nextUserId: UInt64 = 1001

    init(options: MockWorkerOptions) {
        self.options = options
        super.init(noHandle: NoHandle())
        if options.presetAccounts {
            serverAddr = Self.demoServer
            let s = ensureService()
            s.addAccount(userId: 7, username: "alice", displayName: zh ? "王爱丽" : "Alice Wang", status: .active,
                         policy: HostPolicy(systemUser: "alice", view: true, control: true, files: true, terminal: true, clipboard: true,
                                            audio: false, aiCap: .confirmDangerous, notify: true))
            s.addAccount(userId: 12, username: "ops", displayName: zh ? "运维值班" : "Ops on-call", status: .needsRelogin,
                         policy: HostPolicy(systemUser: "alice", view: true, control: false, files: true, terminal: true, clipboard: false,
                                            audio: false, aiCap: .readOnly, notify: true))
            s.seedEvents()
        }
    }

    /// 模拟服务（「模拟」菜单与 selftest 用）
    var mockService: MockWorkerService? { lock.locked { mock } }

    @discardableResult
    private func ensureService() -> MockWorkerService {
        lock.locked {
            if let m = mock { return m }
            let m = MockWorkerService(host: self)
            mock = m
            return m
        }
    }

    func delay(_ ms: Double) async {
        let ns = UInt64(max(0, ms * options.latencyScale) * 1_000_000)
        if ns > 0 { try? await Task.sleep(nanoseconds: ns) }
    }

    override func deviceId() -> String { "7c1e2f9a4b5d6e8f3a2b1c0d9e8f7a6b" }
    override func server() -> String? { lock.locked { serverAddr } }

    override func start() async throws -> HostService? {
        await delay(150)
        guard lock.locked({ serverAddr }) != nil else { return nil }
        return ensureService()
    }

    override func service() -> HostService? { lock.locked { mock } }

    override func login(server: String, username: String, password: String, totp: String?, newPassword: String?, systemUser: String?) async throws -> WorkerLoginResult {
        await delay(600)
        let addr = server.trimmingCharacters(in: .whitespaces)
        guard addr.contains("#sha256-") else {
            throw GalaxyError.Auth(code: "bad_server", detail: zh ? "服务器地址格式不对：应为 主机:端口#sha256-指纹" : "Invalid server address: expected host:port#sha256-fingerprint")
        }
        if let cur = lock.locked({ serverAddr }), cur != addr, !(mock?.accounts().isEmpty ?? true) {
            throw GalaxyError.Other(detail: zh ? "已登录的账号在另一台服务器上，更换服务器前请先退出这些账号" : "Signed-in accounts are on another server. Sign them out before changing servers.")
        }
        if password == "wrong" {
            throw GalaxyError.Auth(code: "bad_credentials", detail: zh ? "用户名或密码错误" : "Incorrect username or password")
        }
        let lower = username.lowercased()
        if lower.contains("totp") && (totp ?? "").isEmpty { return .needTotp }
        if lower.contains("new") && (newPassword ?? "").isEmpty { return .mustChangePassword }
        // 与真内核一致：没有账号时换服务器，旧服务停掉、按新服务器重新启动
        lock.locked {
            if let cur = serverAddr, cur != addr { mock = nil }
            serverAddr = addr
        }
        let s = ensureService()
        let existing = s.accounts().first { $0.username.caseInsensitiveCompare(username) == .orderedSame }
        let uid = existing?.userId ?? lock.locked { () -> UInt64 in nextUserId += 1; return nextUserId }
        let display = username.prefix(1).uppercased() + username.dropFirst()
        s.addAccount(userId: uid, username: username, displayName: display, status: .active,
                     policy: existing?.policy ?? HostPolicy(systemUser: systemUser ?? AppInfo.userName, view: true, control: true, files: true, terminal: true,
                                                            clipboard: true, audio: true, aiCap: .fullTrust, notify: true))
        return .success(userId: uid, username: username)
    }

    override func logout(userId: UInt64) async throws {
        await delay(300)
        mockService?.removeAccount(userId: userId)
    }

    override func setServer(server: String) async throws -> HostService? {
        await delay(300)
        let addr = server.trimmingCharacters(in: .whitespaces)
        guard addr.contains("#sha256-") else {
            throw GalaxyError.Auth(code: "bad_server", detail: zh ? "服务器地址格式不对：应为 主机:端口#sha256-指纹" : "Invalid server address: expected host:port#sha256-fingerprint")
        }
        if let cur = lock.locked({ serverAddr }), cur != addr, !(mock?.accounts().isEmpty ?? true) {
            throw GalaxyError.Other(detail: zh ? "已登录的账号在另一台服务器上，更换服务器前请先退出这些账号" : "Signed-in accounts are on another server. Sign them out before changing servers.")
        }
        lock.locked {
            if let cur = serverAddr, cur != addr { mock = nil }
            serverAddr = addr
        }
        return ensureService()
    }

    /// 演示用：当前版本低于 0.2.0 时报告有新版本
    static let demoUpdate = UpdateInfo(version: "0.2.0.48", fileName: "LemonGalaxyWorker-0.2.0.48-macOS.dmg", size: 16_384_000,
                                       sha256: String(repeating: "ab", count: 32), url: "/updates/files/LemonGalaxyWorker-0.2.0.48-macOS.dmg", notes: "")

    override func checkUpdate(platform: String, current: String) async throws -> UpdateInfo? {
        await delay(500)
        guard !(mock?.accounts().filter { $0.status == .active }.isEmpty ?? true) else {
            throw GalaxyError.Other(detail: zh ? "没有可用的账号" : "No active account")
        }
        var info = Self.demoUpdate
        info.notes = zh ? "- 会话可以单独断开\n- 没登录账号也能开临时协助\n- 关于页可以检查更新并下载安装包" : "- Disconnect a single session\n- Assistance works without an account\n- Check for updates from About"
        return current.compare("0.2.0", options: .numeric) == .orderedAscending ? info : nil
    }

    /// 演示下载：只报进度，不写文件
    override func downloadUpdate(info: UpdateInfo, dest: String, listener: TransferListener) {
        let total = info.size
        let steps: UInt64 = 20
        let interval = 0.12 * options.latencyScale
        DispatchQueue.global().async {
            for i in 1...steps {
                if interval > 0 { Thread.sleep(forTimeInterval: interval) }
                listener.onProgress(done: total * i / steps, total: total)
            }
            listener.onFinished(error: nil)
        }
    }
}

/// 模拟的被控服务：多个账号、被控会话、临时协助、事件日志。不调用 HostPlatform 里需要系统授权的接口。
final class MockWorkerService: HostService, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("模拟对象不能由内核指针构造") }
    private weak var host: MockWorkerHost?
    private let lock = NSLock()
    private var platform: HostPlatform?
    private var listener: HostListener?
    private var remoteEnabled = true
    private var lockOnEndOn = false
    private var accountList: [HostAccount] = []
    private var sessions: [HostSessionInfo] = []
    private var assist: AssistInfo?
    private var events: [HostEvent] = []
    private let queue = DispatchQueue(label: "mock.worker.host")

    init(host: MockWorkerHost) {
        self.host = host
        super.init(noHandle: NoHandle())
    }

    private static func level(_ kind: String) -> EventLevel {
        ["reject", "ai_denied", "auth_failed"].contains(kind) ? .warning : .info
    }

    func seedEvents() {
        let t = Clock.nowMs
        func ev(_ m: Int64, _ kind: String, _ user: String, _ ai: Bool, _ zhMsg: String, _ enMsg: String) -> HostEvent {
            HostEvent(at: t - m * 60_000, session: "hs-\(m)", user: user, kind: kind, detail: zh ? zhMsg : enMsg, ai: ai, level: Self.level(kind))
        }
        lock.locked {
            events = [
                ev(2, "connect", "alice", true, "从 OFFICE-PC-07 连接（局域网）", "Connected from OFFICE-PC-07 (LAN)"),
                ev(3, "exec", "alice", true, "AI 执行命令：df -h（退出码 0）", "AI ran: df -h (exit code 0)"),
                ev(4, "ai_denied", "alice", true, "AI 请求超出本机上限被拒绝：rm -rf ~/Library/Caches", "AI request refused by local limit: rm -rf ~/Library/Caches"),
                ev(18, "terminal", "alice", false, "打开终端（zsh，用户 alice）", "Opened a terminal (zsh, user alice)"),
                ev(25, "file", "alice", false, "上传 report.pdf 到 ~/Downloads（2.4 MB）", "Uploaded report.pdf to ~/Downloads (2.4 MB)"),
                ev(47, "disconnect", "alice", false, "会话结束，时长 21 分钟", "Session ended after 21 minutes"),
                ev(130, "reject", "", false, "拒绝连接：远程已被本机禁止", "Connection refused: remote access disabled locally"),
                ev(180, "auth_failed", "", false, "拒绝连接：证书已吊销（设备 7f3a…）", "Connection refused: revoked certificate (device 7f3a…)"),
                ev(240, "assist", "", false, "临时协助：设备码 482 913 657 已生成", "Assistance: device code 482 913 657 generated"),
                ev(1440, "policy", "", false, "本机修改了 ops 的权限：关闭键鼠控制", "Local change to ops's permissions: input control off"),
            ]
        }
    }

    private func notify() {
        let s = status()
        let l = lock.locked { listener }
        queue.async { l?.onStatus(status: s) }
    }

    private func log(_ kind: String, _ user: String, ai: Bool = false, _ detail: String) {
        lock.locked { events.insert(HostEvent(at: Clock.nowMs, session: "", user: user, kind: kind, detail: detail, ai: ai, level: Self.level(kind)), at: 0) }
    }

    func addAccount(userId: UInt64, username: String, displayName: String, status: AccountStatus, policy: HostPolicy) {
        lock.locked {
            let a = HostAccount(userId: userId, username: username, displayName: displayName, status: status, policy: policy)
            if let i = accountList.firstIndex(where: { $0.userId == userId }) { accountList[i] = a } else { accountList.append(a) }
        }
        notify()
    }

    func removeAccount(userId: UInt64) {
        let (name, ended) = lock.locked { () -> (String, [HostSessionInfo]) in
            let n = accountList.first { $0.userId == userId }?.username ?? ""
            accountList.removeAll { $0.userId == userId }
            let e = sessions.filter { $0.accountId == userId }
            sessions.removeAll { $0.accountId == userId }
            return (n, e)
        }
        log("policy", "", zh ? "本机退出了账号 \(name)" : "Signed out \(name) on this Mac")
        let l = lock.locked { listener }
        for s in ended { queue.async { l?.onSessionEnded(sessionId: s.id) } }
        notify()
    }

    override func status() -> HostStatus {
        lock.locked {
            let online = accountList.contains { $0.status == .active } || assist != nil
            return HostStatus(online: online, remoteEnabled: remoteEnabled, deviceId: host?.deviceId() ?? "", publicIp: online ? "203.0.113.7" : "",
                              accounts: accountList, sessions: sessions, assist: assist)
        }
    }

    override func setPlatform(platform: HostPlatform) { lock.locked { self.platform = platform } }
    override func setListener(listener: HostListener) { lock.locked { self.listener = listener } }
    override func accounts() -> [HostAccount] { lock.locked { accountList } }

    override func updateAccountPolicy(userId: UInt64, policy: HostPolicy) {
        let name = lock.locked { () -> String in
            guard let i = accountList.firstIndex(where: { $0.userId == userId }) else { return "" }
            accountList[i].policy = policy
            return accountList[i].username
        }
        log("policy", "", zh ? "本机修改了 \(name) 的权限" : "Local change to \(name)'s permissions")
        notify()
    }

    override func setRemoteEnabled(enabled: Bool) {
        let ended = lock.locked { () -> [HostSessionInfo] in
            remoteEnabled = enabled
            let e = enabled ? [] : sessions
            if !enabled { sessions.removeAll() }
            return e
        }
        log("policy", "", enabled ? (zh ? "已允许远程" : "Remote access enabled") : (zh ? "已禁止远程，断开所有连接" : "Remote access disabled; all sessions closed"))
        let l = lock.locked { listener }
        for s in ended { queue.async { l?.onSessionEnded(sessionId: s.id) } }
        notify()
    }

    override func lockOnEnd() -> Bool { lock.locked { lockOnEndOn } }

    override func setLockOnEnd(on: Bool) {
        lock.locked { lockOnEndOn = on }
        log("policy", "", on ? (zh ? "已开启远程结束后自动锁屏" : "Lock screen when remote sessions end: on")
                             : (zh ? "已关闭远程结束后自动锁屏" : "Lock screen when remote sessions end: off"))
        notify()
    }

    override func disconnectSession(id: String) -> Bool {
        let ended = lock.locked { () -> HostSessionInfo? in
            guard let i = sessions.firstIndex(where: { $0.id == id }) else { return nil }
            return sessions.remove(at: i)
        }
        guard let s = ended else { return false }
        log("disconnect", s.fromUser, zh ? "本机断开了 \(s.fromUser) 的连接" : "Disconnected \(s.fromUser) locally")
        let l = lock.locked { listener }
        queue.async { l?.onSessionEnded(sessionId: id) }
        notify()
        return true
    }

    override func disconnectAll() {
        let ended = lock.locked { () -> [HostSessionInfo] in let e = sessions; sessions.removeAll(); return e }
        log("disconnect", "", zh ? "本机一键断开了所有连接" : "All sessions disconnected locally")
        let l = lock.locked { listener }
        for s in ended { queue.async { l?.onSessionEnded(sessionId: s.id) } }
        notify()
    }

    override func stopAi() {
        lock.locked { for i in sessions.indices { sessions[i].aiActive = false } }
        log("ai", "", ai: true, zh ? "本机停止了所有 AI 操作" : "All AI operations stopped locally")
        notify()
    }

    override func startAssist() async throws -> AssistInfo {
        await host?.delay(200)
        let a = lock.locked { () -> AssistInfo in
            if assist == nil { assist = AssistInfo(deviceCode: "482913657", code: "K7QM4XPA") }
            return assist!
        }
        log("assist", "", zh ? "临时协助：设备码 482 913 657 已生成" : "Assistance: device code 482 913 657 generated")
        notify()
        return a
    }

    override func stopAssist() {
        lock.locked { assist = nil }
        notify()
    }

    override func setAssistCode(code: String?) {
        lock.locked {
            if var a = assist {
                a.code = code ?? Self.randomCode()
                assist = a
            }
        }
        notify()
    }

    static func randomCode() -> String {
        let chars = Array("ABCDEFGHJKLMNPQRSTUVWXYZ23456789")
        return String((0..<8).map { _ in chars.randomElement()! })
    }

    override func respondAssist(requestId: String, allow: Bool) {
        log("assist", "bob", allow ? (zh ? "允许了 bob 的协助请求" : "Allowed assistance request from bob") : (zh ? "拒绝了 bob 的协助请求" : "Rejected assistance request from bob"))
        if allow { simulateSession(user: "bob", device: "BOB-THINKPAD", mode: .assist, ai: false) } else { notify() }
    }

    override func recentEvents(limit: UInt32) -> [HostEvent] { lock.locked { Array(events.prefix(Int(limit))) } }

    // MARK: 模拟触发（「模拟」菜单与 selftest 用）

    func simulateAssistRequest() {
        let l = lock.locked { listener }
        queue.async { l?.onAssistRequest(requestId: "ar-1", fromUser: "bob (Bob Li)", fromDevice: "BOB-THINKPAD") }
    }

    func simulateSession(user: String = "alice", device: String = "OFFICE-PC-07", mode: ConnMode = .control, ai: Bool, accountId: UInt64? = 7) {
        guard lock.locked({ remoteEnabled }) else { return }
        let account = mode == .assist ? nil : lock.locked { accountList.first { $0.userId == accountId }?.userId ?? accountList.first?.userId }
        let info = HostSessionInfo(id: "hs-" + UUID().uuidString.prefix(6), fromUser: user, fromDevice: device, mode: mode,
                                   path: mode == .assist ? .relayUdp : .lan, startedAt: Clock.nowMs - 4 * 60_000, aiActive: ai, accountId: account)
        lock.locked { sessions.append(info) }
        log("connect", user, ai: ai, zh ? "\(user) 从 \(device) 连接" : "\(user) connected from \(device)")
        let l = lock.locked { listener }
        queue.async { l?.onSessionStarted(info: info) }
        if ai {
            let text = zh ? "AI 正在执行：du -sh ~/Library/Caches/*" : "AI is running: du -sh ~/Library/Caches/*"
            queue.asyncAfter(deadline: .now() + 0.2) { l?.onAiActivity(sessionId: info.id, description: text) }
        }
        notify()
    }
}
