import Foundation

/// 模拟本机被控服务。不会调用 HostPlatform 里需要系统授权的接口（截屏、注入），只保存注册的平台对象。
/// 「模拟」菜单（只在模拟内核下出现）可以触发协助请求、被控会话、AI 操作，用来演示提示条与弹窗。
final class MockHostService: HostService {
    private weak var client: MockGalaxyClient?
    private let lock = NSLock()
    private var platform: HostPlatform?
    private weak var listener: HostListener?
    private var remoteEnabled = true
    private var sessions: [HostSessionInfo] = []
    private var aiActive = false
    private var assist: AssistInfo?
    private var policies: [UInt64: HostPolicy] = [:]
    private var events: [HostEvent]
    private let queue = DispatchQueue(label: "mock.host")

    init(client: MockGalaxyClient) {
        self.client = client
        let zh = client.zh
        let t = MockData.fixedNow
        func ev(_ m: Int64, _ level: HostEventLevel, _ cat: String, _ user: String?, _ ai: Bool, _ zhMsg: String, _ enMsg: String) -> HostEvent {
            HostEvent(time: t - m * 60_000, level: level, category: cat, user: user, ai: ai, message: zh ? zhMsg : enMsg)
        }
        events = [
            ev(3, .info, "connect", "alice", false, "从 OFFICE-PC-07 连接（局域网）", "Connected from OFFICE-PC-07 (LAN)"),
            ev(5, .info, "terminal", "alice", false, "打开终端（bash，用户 alice）", "Opened a terminal (bash, user alice)"),
            ev(12, .info, "ai", "alice", true, "AI 执行命令：df -h（退出码 0）", "AI ran: df -h (exit code 0)"),
            ev(13, .warning, "ai", "alice", true, "AI 请求被本机上限拒绝：rm -rf ~/Library/Caches", "AI request refused by local limit: rm -rf ~/Library/Caches"),
            ev(40, .info, "file", "alice", false, "上传 report.pdf 到 ~/Downloads（2.4 MB）", "Uploaded report.pdf to ~/Downloads (2.4 MB)"),
            ev(62, .info, "disconnect", "alice", false, "会话结束，时长 21 分钟", "Session ended after 21 minutes"),
            ev(180, .error, "auth", nil, false, "拒绝连接：证书已吊销（设备 7f3a…）", "Connection refused: revoked certificate (device 7f3a…)"),
            ev(240, .info, "assist", nil, false, "临时协助：设备码 482 913 657 已生成", "Assistance: device code 482 913 657 generated"),
            ev(242, .warning, "assist", nil, false, "临时协助：验证码错误 1 次", "Assistance: 1 incorrect verification code"),
            ev(1440, .info, "policy", nil, false, "本机修改了 alice 的权限：关闭声音", "Local change to alice's permissions: audio off"),
        ]
        super.init(noPointer: NoPointer())
    }

    private var zh: Bool { client?.zh ?? true }

    private func notify() {
        let s = status()
        let l = listener
        queue.async { l?.onStatus(status: s) }
    }

    private func log(_ level: HostEventLevel, _ cat: String, _ user: String?, ai: Bool = false, _ message: String) {
        lock.lock()
        events.insert(HostEvent(time: MockData.now, level: level, category: cat, user: user, ai: ai, message: message), at: 0)
        lock.unlock()
    }

    override func status() -> HostStatus {
        let user = client?.currentUser()
        lock.lock(); defer { lock.unlock() }
        return HostStatus(remoteEnabled: remoteEnabled, online: client?.state() == .online,
                          accounts: user.map { [$0.username] } ?? [], sessions: sessions, aiActive: aiActive, assist: assist)
    }

    override func setPlatform(platform: HostPlatform) { lock.locked { self.platform = platform } }
    override func setListener(listener: HostListener) { lock.locked { self.listener = listener } }

    override func accounts() -> [HostAccount] {
        guard let user = client?.currentUser() else { return [] }
        lock.lock(); defer { lock.unlock() }
        let policy = policies[user.id] ?? HostPolicy(
            permissions: Permissions(view: true, control: true, files: true, terminal: true, clipboard: true, audio: false, aiCap: .confirmDangerous),
            systemUser: user.username)
        return [HostAccount(userId: user.id, username: user.username, displayName: user.displayName,
                            serverAddress: client?.server()?.address ?? "", policy: policy)]
    }

    override func updateAccountPolicy(userId: UInt64, policy: HostPolicy) {
        lock.locked { policies[userId] = policy }
        log(.info, "policy", nil, zh ? "本机修改了账号权限" : "Account permissions changed locally")
        notify()
    }

    override func setRemoteEnabled(enabled: Bool) {
        lock.lock()
        remoteEnabled = enabled
        let ended = enabled ? [] : sessions
        if !enabled { sessions.removeAll(); aiActive = false }
        lock.unlock()
        log(enabled ? .info : .warning, "policy", nil, enabled ? (zh ? "已允许远程" : "Remote access enabled") : (zh ? "已禁止远程，断开所有连接" : "Remote access disabled; all sessions closed"))
        let l = listener
        for s in ended { queue.async { l?.onSessionEnded(sessionId: s.sessionId) } }
        notify()
    }

    override func disconnectAll() {
        let ended = lock.locked { () -> [HostSessionInfo] in let e = sessions; sessions.removeAll(); aiActive = false; return e }
        log(.info, "disconnect", nil, zh ? "本机一键断开了所有连接" : "All sessions disconnected locally")
        let l = listener
        for s in ended { queue.async { l?.onSessionEnded(sessionId: s.sessionId) } }
        notify()
    }

    override func stopAi() {
        lock.lock()
        aiActive = false
        for i in sessions.indices { sessions[i].ai = false }
        lock.unlock()
        log(.warning, "ai", nil, ai: true, zh ? "本机停止了所有 AI 操作" : "All AI operations stopped locally")
        notify()
    }

    override func startAssist() -> AssistInfo {
        lock.lock()
        if assist == nil {
            assist = AssistInfo(deviceCode: "482913657", verificationCode: Self.randomCode(), customCode: false)
        }
        let a = assist!
        lock.unlock()
        notify()
        return a
    }

    override func stopAssist() {
        lock.locked { assist = nil }
        notify()
    }

    override func setAssistCode(code: String?) {
        lock.lock()
        if var a = assist {
            a.verificationCode = code ?? Self.randomCode()
            a.customCode = code != nil
            assist = a
        }
        lock.unlock()
        notify()
    }

    static func randomCode() -> String {
        let chars = Array("ABCDEFGHJKLMNPQRSTUVWXYZ23456789")
        return String((0..<8).map { _ in chars.randomElement()! })
    }

    override func respondAssist(requestId: String, allow: Bool) {
        log(allow ? .info : .warning, "assist", "bob", allow ? (zh ? "允许了 bob 的协助请求" : "Allowed assistance request from bob") : (zh ? "拒绝了 bob 的协助请求" : "Rejected assistance request from bob"))
        if allow {
            simulateSession(user: "bob", display: "Bob", device: "BOB-THINKPAD", platform: .windows, ai: false)
        }
    }

    override func recentEvents() -> [HostEvent] { lock.lock(); defer { lock.unlock() }; return events }

    // MARK: 模拟触发（只在模拟内核的「模拟」菜单里调用）

    func simulateAssistRequest() {
        let l = listener
        queue.async { l?.onAssistRequest(requestId: "ar-1", fromUser: "bob (Bob Li)", fromDevice: "BOB-THINKPAD") }
    }

    func simulateSession(user: String = "alice", display: String = "Alice", device: String = "OFFICE-PC-07", platform: Platform = .windows, ai: Bool) {
        let info = HostSessionInfo(sessionId: "hs-" + UUID().uuidString.prefix(6), userId: 3, username: user, displayName: display,
                                   fromDevice: device, fromPlatform: platform, features: ["display", "terminal"], ai: ai, startedAt: MockData.now)
        lock.locked { sessions.append(info); if ai { aiActive = true } }
        log(.info, "connect", user, ai: ai, zh ? "\(display) 从 \(device) 连接" : "\(display) connected from \(device)")
        let l = listener
        queue.async { l?.onSessionStarted(info: info) }
        if ai {
            queue.asyncAfter(deadline: .now() + 0.3) { l?.onAiActivity(sessionId: info.sessionId, description: self.zh ? "AI 正在执行：df -h" : "AI is running: df -h") }
        }
        notify()
    }
}
