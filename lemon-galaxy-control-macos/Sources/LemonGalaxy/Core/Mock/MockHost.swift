import Foundation

/// 模拟本机被控服务。不会调用 HostPlatform 里需要系统授权的接口（截屏、注入），只保存注册的平台对象。
/// 「模拟」菜单（只在模拟内核下出现）可以触发协助请求、被控会话、AI 操作，用来演示提示条与弹窗。
final class MockHostService: HostService, @unchecked Sendable {
    required init(unsafeFromRawPointer pointer: UnsafeMutableRawPointer) { fatalError("模拟对象不能由内核指针构造") }
    private weak var client: MockGalaxyClient?
    private let lock = NSLock()
    private var platform: HostPlatform?
    private var listener: HostListener?
    private var remoteEnabled = true
    private var sessions: [HostSessionInfo] = []
    private var assist: AssistInfo?
    private var policies: [UInt64: HostPolicy] = [:]
    private var events: [HostEvent]
    private let queue = DispatchQueue(label: "mock.host")

    init(client: MockGalaxyClient) {
        self.client = client
        let zh = client.zh
        let t = MockData.fixedNow
        func ev(_ m: Int64, _ kind: String, _ user: String, _ ai: Bool, _ zhMsg: String, _ enMsg: String) -> HostEvent {
            HostEvent(at: t - m * 60_000, session: "hs-\(m)", user: user, kind: kind, detail: zh ? zhMsg : enMsg, ai: ai, level: MockHostEvents.level(kind))
        }
        events = [
            ev(3, "connect", "alice", false, "从 OFFICE-PC-07 连接（局域网）", "Connected from OFFICE-PC-07 (LAN)"),
            ev(5, "terminal", "alice", false, "打开终端（bash，用户 alice）", "Opened a terminal (bash, user alice)"),
            ev(12, "exec", "alice", true, "AI 执行命令：df -h（退出码 0）", "AI ran: df -h (exit code 0)"),
            ev(13, "denied", "alice", true, "AI 请求被本机上限拒绝：rm -rf ~/Library/Caches", "AI request refused by local limit: rm -rf ~/Library/Caches"),
            ev(40, "file", "alice", false, "上传 report.pdf 到 ~/Downloads（2.4 MB）", "Uploaded report.pdf to ~/Downloads (2.4 MB)"),
            ev(62, "disconnect", "alice", false, "会话结束，时长 21 分钟", "Session ended after 21 minutes"),
            ev(180, "auth_failed", "", false, "拒绝连接：证书已吊销（设备 7f3a…）", "Connection refused: revoked certificate (device 7f3a…)"),
            ev(240, "assist", "", false, "临时协助：设备码 482 913 657 已生成", "Assistance: device code 482 913 657 generated"),
            ev(242, "auth_failed", "", false, "临时协助：验证码错误 1 次", "Assistance: 1 incorrect verification code"),
            ev(1440, "policy", "", false, "本机修改了 alice 的权限：关闭声音", "Local change to alice's permissions: audio off"),
        ]
        super.init(noPointer: NoPointer())
    }

    private var zh: Bool { client?.zh ?? true }

    private func notify() {
        let s = status()
        let l = lock.locked { listener }
        queue.async { l?.onStatus(status: s) }
    }

    private func log(_ kind: String, _ user: String, ai: Bool = false, _ detail: String) {
        lock.locked { events.insert(HostEvent(at: MockData.now, session: "", user: user, kind: kind, detail: detail, ai: ai, level: MockHostEvents.level(kind)), at: 0) }
    }

    override func status() -> HostStatus {
        let accs = accounts()
        return lock.locked {
            HostStatus(online: client?.state() == .online, remoteEnabled: remoteEnabled, deviceId: "dev-self", publicIp: "203.0.113.7",
                       accounts: accs, sessions: sessions, assist: assist)
        }
    }

    override func setPlatform(platform: HostPlatform) { lock.locked { self.platform = platform } }
    override func setListener(listener: HostListener) { lock.locked { self.listener = listener } }

    override func accounts() -> [HostAccount] {
        guard let user = client?.currentUser() else { return [] }
        return lock.locked {
            let policy = policies[user.id] ?? HostPolicy(systemUser: user.username, view: true, control: true, files: true, terminal: true,
                                                         clipboard: true, audio: false, aiCap: .confirmDangerous, notify: true)
            return [HostAccount(userId: user.id, username: user.username, displayName: user.displayName, status: .active, policy: policy)]
        }
    }

    override func updateAccountPolicy(userId: UInt64, policy: HostPolicy) {
        lock.locked { policies[userId] = policy }
        log("policy", "", zh ? "本机修改了账号权限" : "Account permissions changed locally")
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
        await client?.delay(200)
        let a = lock.locked { () -> AssistInfo in
            if assist == nil { assist = AssistInfo(deviceCode: "482913657", code: Self.randomCode()) }
            return assist!
        }
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
        if allow {
            simulateSession(user: "bob", device: "BOB-THINKPAD", mode: .assist, ai: false)
        }
    }

    override func recentEvents(limit: UInt32) -> [HostEvent] { lock.locked { Array(events.prefix(Int(limit))) } }

    // MARK: 模拟触发（只在模拟内核的「模拟」菜单里调用）

    func simulateAssistRequest() {
        let l = lock.locked { listener }
        queue.async { l?.onAssistRequest(requestId: "ar-1", fromUser: "bob (Bob Li)", fromDevice: "BOB-THINKPAD") }
    }

    func simulateSession(user: String = "alice", device: String = "OFFICE-PC-07", mode: ConnMode = .control, ai: Bool) {
        let info = HostSessionInfo(id: "hs-" + UUID().uuidString.prefix(6), fromUser: user, fromDevice: device, mode: mode,
                                   path: .lan, startedAt: MockData.now, aiActive: ai)
        lock.locked { sessions.append(info) }
        log("connect", user, ai: ai, zh ? "\(user) 从 \(device) 连接" : "\(user) connected from \(device)")
        let l = lock.locked { listener }
        queue.async { l?.onSessionStarted(info: info) }
        if ai {
            let text = zh ? "AI 正在执行：df -h" : "AI is running: df -h"
            queue.asyncAfter(deadline: .now() + 0.3) { l?.onAiActivity(sessionId: info.id, description: text) }
        }
        notify()
    }
}

enum MockHostEvents {
    static func level(_ kind: String) -> EventLevel {
        ["reject", "ai_denied", "auth_failed"].contains(kind) ? .warning : .info
    }
}
