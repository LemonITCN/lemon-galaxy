import SwiftUI
import AppKit
import UserNotifications

// 本机 MCP 服务的界面状态（docs/mcp.md 第 5 节）。服务本身在内核里（GalaxyClient.mcp_*），这里只管：
// 开关与端口、档位与时限、外部客户端的待批准卡片与要密码、连接方式、已连接的客户端、操作记录、系统通知。

/// MCP 页的分区；中间列点哪一项，页面就滚到哪一段
enum MCPSection: String, CaseIterable, Identifiable {
    case service, pending, connection, permission, clients, records
    var id: String { rawValue }
    var titleKey: String { "mcp.section." + rawValue }
    var icon: String {
        switch self {
        case .service: return "power"
        case .pending: return "hand.raised"
        case .connection: return "link"
        case .permission: return "lock.shield"
        case .clients: return "person.2"
        case .records: return "clock.arrow.circlepath"
        }
    }
}

/// 外部客户端的配置写法（一键复制）
enum MCPConfigKind: String, CaseIterable, Identifiable {
    case claudeCode, codex, cursor, claudeDesktop
    var id: String { rawValue }
    var title: String {
        switch self {
        case .claudeCode: return "Claude Code"
        case .codex: return "Codex"
        case .cursor: return "Cursor"
        case .claudeDesktop: return "Claude Desktop"
        }
    }
    var infoKey: String { "mcp.config.info." + rawValue }
}

/// 某个外部客户端的一次工具调用：待批准时显示成卡片，同时是操作记录里的一行
@MainActor
final class MCPCall: ObservableObject, Identifiable {
    enum Outcome: Equatable { case pending, running, success, failed, denied, cancelled }
    /// 内核在客户端断开（取消）时给结果的摘要（galaxy-agent cancel_external）
    static let cancelledSummary = "调用方已取消"

    nonisolated let id: String
    let clientKey: String
    /// 收到时间（毫秒）；从 mcp_history 补回来的记录取相邻记录或客户端最近活动的时间
    let at: Int64
    /// 排序用：越大越新
    var order: Double
    @Published var call: ToolCallView
    @Published var decision: ToolCardModel.Decision
    @Published var phase: ToolCardModel.Phase = .proposed
    @Published var output = ""
    @Published var result: ToolResultView?
    /// 服务关闭、退出登录时还没结束的调用（之后再没有结果过来）
    @Published var abandoned = false
    @Published var expanded = false
    @Published var denying = false
    @Published var denyReason = ""

    init(clientKey: String, call: ToolCallView, needsApproval: Bool, at: Int64, order: Double) {
        self.clientKey = clientKey
        self.call = call
        self.id = clientKey + "/" + call.callId
        self.at = at
        self.order = order
        self.decision = needsApproval ? .pending : .auto
    }

    var isUseTarget: Bool { call.tool == "use_target" }
    var isUseApp: Bool { call.tool == "use_app" }
    /// 连接设备、操作应用：只有允许 / 拒绝
    var isSimpleApproval: Bool { isUseTarget || isUseApp }
    var isPending: Bool { decision == .pending }

    var outcome: Outcome {
        if decision == .pending { return .pending }
        if let r = result {
            if r.denied && r.summary == Self.cancelledSummary { return .cancelled }
            if r.denied { return .denied }
            return r.ok ? .success : .failed
        }
        if case .denied = decision { return abandoned ? .cancelled : .denied }
        return abandoned ? .cancelled : .running
    }

    func apply(result r: ToolResultView) {
        result = r
        phase = .finished
        if r.output.count > output.count { output = r.output }
        if decision == .pending { decision = r.denied ? .denied(nil) : .approved }
    }
}

/// 外部智能体请用户输入密码（request_secret）：内容直接送到设备，不经过外部智能体
struct MCPSecretRequest: Identifiable, Equatable {
    let id: String
    let clientKey: String
    let prompt: String
    let targetId: String
    /// 所属的 request_secret 调用（它结束时卡片收起）
    let callId: String?
    let at: Int64
}

@MainActor
final class MCPModel: ObservableObject {
    weak var app: AppModel?
    @Published private(set) var status: McpStatus
    @Published private(set) var busy = false
    /// 上次开启失败的原因（如端口被占用）；开起来后清掉
    @Published private(set) var startError: String?
    /// 全部调用，最新在前；结束了的最多留 maxRecords 条
    @Published private(set) var calls: [MCPCall] = []
    @Published private(set) var secrets: [MCPSecretRequest] = []
    @Published var portText = ""
    @Published var tokenShown = false
    @Published var configKind: MCPConfigKind = .claudeCode
    /// 中间列选中的分区；scrollTarget 由页面消费后清空
    @Published var navSelection: MCPSection = .service
    @Published var scrollTarget: MCPSection?

    static let maxRecords = 200
    /// selftest 用私有剪贴板，不碰用户的
    static var pasteboardOverride: NSPasteboard?
    /// selftest 截图里的可执行文件路径固定写成安装位置
    static var executablePathOverride: String?
    /// 发过（selftest 下为「该发」）的系统通知次数
    private(set) var notifyCount = 0

    private let listener = MCPListenerAdapter()
    private var index: [String: MCPCall] = [:]
    private var nextOrder: Double = 1
    private var announced = Set<String>()
    private var trustTimer: Timer?

    init() {
        status = McpStatus(supported: true, running: false, url: "", port: mcpDefaultPort(), token: "", endpointFile: "", mode: nil, error: nil, clients: [])
        listener.owner = self
    }

    private var settings: AppSettings? { app?.settings }

    // MARK: 登录、退出登录

    /// 登录后：读出端口与令牌；设置里开着就开启服务（已经在跑的不重启）
    func didLogin() {
        guard let app else { return }
        status = app.client.mcpStatus()
        portText = "\(app.settings.mcpPort)"
        expireTrustIfDue()
        scheduleTrustTimer()
        if app.settings.mcpEnabled && !status.running { start() }
    }

    /// 退出登录：内核把外部客户端的智能体全部作废（服务继续开着），界面上的卡片与记录清掉
    func didLogout() {
        MCPNotifier.remove(Array(announced))
        announced.removeAll()
        calls.removeAll()
        index.removeAll()
        secrets.removeAll()
        if let app { status = app.client.mcpStatus() }
    }

    // MARK: 服务

    var running: Bool { status.running }

    func setEnabled(_ on: Bool) {
        settings?.mcpEnabled = on
        if on { start() } else { stop() }
    }

    /// 开启，或按新设置重启（端口改了）
    func start() {
        guard let app, !busy else { return }
        busy = true
        startError = nil
        let config = McpConfig(port: UInt16(clamping: app.settings.mcpPort), mode: mode, trustMinutes: trustValue)
        let l = listener
        Task {
            do {
                let s = try await app.client.mcpStart(config: config, listener: l)
                apply(s)
                if !app.selfTest { MCPNotifier.requestAuthorization() }
            } catch {
                let msg = AppModel.describe(error)
                status = app.client.mcpStatus()
                startError = msg
            }
            busy = false
        }
    }

    func stop() {
        guard let app else { return }
        app.client.mcpStop()
        status = app.client.mcpStatus()
        startError = nil
        abandonOpen()
    }

    /// 页面打开：刷新状态，用 mcp_history 补齐记录
    func pageAppeared() {
        guard let app, app.phase == .main else { return }
        let s = app.client.mcpStatus()
        apply(s)
        syncHistory()
    }

    func apply(_ s: McpStatus) {
        status = s
        if s.running { startError = nil } else if let e = s.error, !e.isEmpty { startError = e }
    }

    // MARK: 端口

    /// 输入框里的端口，不合法为 nil
    var portValue: Int? {
        guard let p = Int(portText.trimmingCharacters(in: .whitespaces)), (1024...65535).contains(p) else { return nil }
        return p
    }

    var portChanged: Bool {
        guard let p = portValue, let settings else { return false }
        return p != settings.mcpPort || (settings.mcpEnabled && !running && startError != nil)
    }

    /// 应用新端口：开着就按新端口重启
    func applyPort() {
        guard let app, let p = portValue else { return }
        app.settings.mcpPort = p
        portText = "\(p)"
        if app.settings.mcpEnabled { start() }
    }

    // MARK: 地址、令牌、配置

    var url: String {
        if running, !status.url.isEmpty { return status.url }
        return "http://127.0.0.1:\(settings?.mcpPort ?? Int(mcpDefaultPort()))/mcp"
    }

    var token: String { status.token }

    var maskedToken: String { token.isEmpty ? "" : String(token.prefix(4)) + String(repeating: "•", count: 20) }

    var executablePath: String { Self.executablePathOverride ?? Bundle.main.executablePath ?? CommandLine.arguments[0] }

    /// 某种客户端的配置文字；token 传 nil 时用真令牌
    func configText(_ kind: MCPConfigKind, token tok: String? = nil) -> String {
        let t = tok ?? token
        let u = url
        switch kind {
        case .claudeCode:
            return "claude mcp add --transport http --scope user lemongalaxy \(u) --header \"Authorization: Bearer \(t)\""
        case .codex:
            return "[mcp_servers.lemongalaxy]\nurl = \(Self.quoted(u))\nhttp_headers = { \"Authorization\" = \(Self.quoted("Bearer " + t)) }"
        case .cursor:
            return """
            {
              "mcpServers": {
                "lemongalaxy": {
                  "url": \(Self.quoted(u)),
                  "headers": { "Authorization": \(Self.quoted("Bearer " + t)) }
                }
              }
            }
            """
        case .claudeDesktop:
            return """
            {
              "mcpServers": {
                "lemongalaxy": {
                  "command": \(Self.quoted(executablePath)),
                  "args": ["--mcp-stdio"]
                }
              }
            }
            """
        }
    }

    /// JSON / TOML 都认的双引号字符串（转义反斜杠、引号与控制字符）
    static func quoted(_ s: String) -> String {
        var out = "\""
        for ch in s.unicodeScalars {
            switch ch {
            case "\"": out += "\\\""
            case "\\": out += "\\\\"
            case "\n": out += "\\n"
            case "\t": out += "\\t"
            default:
                if ch.value < 0x20 { out += String(format: "\\u%04x", ch.value) } else { out.unicodeScalars.append(ch) }
            }
        }
        return out + "\""
    }

    private func copy(_ text: String, toast: String) {
        let pb = Self.pasteboardOverride ?? .general
        pb.clearContents()
        pb.setString(text, forType: .string)
        app?.showToast(toast)
    }

    func copyURL() { copy(url, toast: L("common.copied")) }
    func copyToken() { guard !token.isEmpty else { return }; copy(token, toast: L("common.copied")) }
    func copyConfig(_ kind: MCPConfigKind) {
        guard !token.isEmpty else { return }
        copy(configText(kind), toast: L("mcp.config.copied", kind.title))
    }

    func askRegenerate() {
        app?.confirm(L("mcp.token.regenerate.title"), L("mcp.token.regenerate.message"), button: L("mcp.token.regenerate")) { [weak self] in
            self?.regenerate()
        }
    }

    func regenerate() {
        guard let app else { return }
        Task {
            do {
                apply(try await app.client.mcpRegenerateToken())
                app.showToast(L("mcp.token.regenerated"))
            } catch {
                app.showError(error)
            }
        }
    }

    // MARK: 档位

    var mode: AiMode {
        let m = AiMode.from(settings?.mcpMode ?? "confirmDangerous")
        return m == .off ? .confirmDangerous : m
    }

    /// 限时完全信任的到期时间
    var trustUntilMs: Int64? {
        guard mode == .fullTrust, let u = settings?.mcpTrustUntil, u > 0 else { return nil }
        return Int64(u)
    }

    /// 交给内核的时限：限时完全信任时取剩余分钟（至少 1 分钟）
    var trustValue: UInt32? {
        guard let until = trustUntilMs else { return nil }
        let left = (Double(until - Clock.nowMs) / 60_000).rounded(.up)
        return UInt32(max(1, left))
    }

    /// 选档位：完全信任沿用上次选的时限
    func setMode(_ m: AiMode) {
        setMode(m, minutes: settings?.mcpTrustMinutes ?? 60)
    }

    func setMode(_ m: AiMode, minutes: Int) {
        guard let app else { return }
        app.settings.mcpMode = m.key
        if m == .fullTrust {
            app.settings.mcpTrustMinutes = minutes
            app.settings.mcpTrustUntil = minutes > 0 ? Double(Clock.nowMs + Int64(minutes) * 60_000) : 0
        } else {
            app.settings.mcpTrustUntil = 0
        }
        // 立即作用于所有外部客户端；没开着时下次开启带上
        if running { app.client.mcpSetMode(mode: m, trustMinutes: trustValue) }
        scheduleTrustTimer()
        objectWillChange.send()
    }

    /// 限时完全信任到了：回到危险操作确认
    func expireTrustIfDue() {
        guard let until = trustUntilMs, Clock.nowMs >= until else { return }
        setMode(.confirmDangerous, minutes: 0)
        if app?.phase == .main { app?.showToast(L("mcp.trust.expired")) }
    }

    private func scheduleTrustTimer() {
        trustTimer?.invalidate()
        trustTimer = nil
        guard let until = trustUntilMs, app?.selfTest == false else { return }
        let delay = max(0, Double(until - Clock.nowMs) / 1000) + 0.5
        trustTimer = Timer.scheduledTimer(withTimeInterval: delay, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated { self?.expireTrustIfDue() }
        }
    }

    // MARK: 客户端

    func clientName(_ key: String) -> String {
        status.clients.first { $0.key == key }?.name ?? key
    }

    /// 界面上这个客户端等你处理的数量（批准卡片 + 要密码）
    func pendingCount(for key: String) -> Int {
        calls.reduce(0) { $0 + ($1.clientKey == key && $1.isPending ? 1 : 0) } + secrets.filter { $0.clientKey == key }.count
    }

    func stopClient(_ key: String) {
        guard let app else { return }
        app.client.mcpStopClient(clientKey: key)
        app.showToast(L("mcp.client.stopped", clientName(key)))
    }

    // MARK: 待批准

    /// 待批准的卡片：先来的在上面
    var pendingCalls: [MCPCall] { calls.filter(\.isPending).reversed() }

    /// 侧栏角标：待批准 + 要密码
    var pendingCount: Int { calls.reduce(0) { $0 + ($1.isPending ? 1 : 0) } + secrets.count }

    func decide(_ c: MCPCall, _ d: ApprovalDecision) {
        guard let app, c.isPending else { return }
        app.client.mcpDecide(clientKey: c.clientKey, callId: c.call.callId, decision: d)
        objectWillChange.send()
        switch d {
        case .allowOnce: c.decision = .approved
        case .allowSimilar: c.decision = .approvedSimilar
        case .deny(let r): c.decision = .denied(r)
        case .editAndRun(let cmd): c.decision = .edited(cmd)
        }
        c.denying = false
        MCPNotifier.remove(["mcp/" + c.id])
    }

    func provideSecret(_ r: MCPSecretRequest, _ secret: String) {
        guard let app else { return }
        // 密码只交给内核送到设备，不留在界面状态里
        app.client.mcpProvideSecret(clientKey: r.clientKey, requestId: r.id, secret: secret)
        removeSecret(r.id)
    }

    func cancelSecret(_ r: MCPSecretRequest) {
        guard let app else { return }
        app.client.mcpProvideSecret(clientKey: r.clientKey, requestId: r.id, secret: "")
        removeSecret(r.id)
    }

    private func removeSecret(_ id: String) {
        secrets.removeAll { $0.id == id }
        MCPNotifier.remove(["mcp/secret/" + id])
    }

    // MARK: 事件

    private func find(_ key: String, _ callId: String) -> MCPCall? { index[key + "/" + callId] }

    func handle(_ key: String, _ e: AgentEvent) {
        if case .toolOutput(let id, let text) = e {
            // 输出只影响那一行，不惊动整页
            if let c = find(key, id), c.output.utf8.count < 256 * 1024 { c.output += text }
            return
        }
        objectWillChange.send()
        switch e {
        case .toolProposed(let call, let needs):
            if let c = find(key, call.callId) {
                c.call = call
                if needs {
                    c.decision = .pending
                    c.phase = .proposed
                    announce(c)
                }
            } else {
                let c = MCPCall(clientKey: key, call: call, needsApproval: needs, at: Clock.nowMs, order: nextOrder)
                nextOrder += 1
                calls.insert(c, at: 0)
                index[c.id] = c
                trim()
                if needs { announce(c) }
            }
        case .toolStarted(let id):
            if let c = find(key, id) {
                c.phase = .running
                if c.decision == .pending { c.decision = .approved }
            }
        case .toolFinished(let id, let r):
            if let c = find(key, id) {
                c.apply(result: r)
                MCPNotifier.remove(["mcp/" + c.id])
            }
            for s in secrets where s.clientKey == key && s.callId == id { removeSecret(s.id) }
            trim()
        case .secretRequested(let rid, let prompt, let target):
            let used = Set(secrets.compactMap(\.callId))
            let owner = calls.first { $0.clientKey == key && $0.call.tool == "request_secret" && $0.result == nil && !used.contains($0.call.callId) }
            let r = MCPSecretRequest(id: rid, clientKey: key, prompt: prompt, targetId: target, callId: owner?.call.callId, at: Clock.nowMs)
            secrets.append(r)
            announce(id: "mcp/secret/" + rid, title: L("mcp.notify.secret", clientName(key)), body: prompt)
        case .status(let r, _):
            // 这个客户端没有在执行的调用了：认不出归属的要密码卡片也收起
            if !r { for s in secrets where s.clientKey == key && s.callId == nil { removeSecret(s.id) } }
        default:
            break
        }
    }

    /// 只保留最近 maxRecords 条；待批准、还在执行的不删
    private func trim() {
        var i = calls.count - 1
        while calls.count > Self.maxRecords && i >= 0 {
            let c = calls[i]
            if c.result != nil || c.abandoned {
                calls.remove(at: i)
                index[c.id] = nil
            }
            i -= 1
        }
    }

    /// 服务关了：还没结束的调用再不会有结果
    private func abandonOpen() {
        objectWillChange.send()
        for c in calls where c.result == nil {
            if c.isPending { c.decision = .denied(nil) }
            c.abandoned = true
            c.phase = .finished
        }
        for s in secrets { removeSecret(s.id) }
        MCPNotifier.remove(Array(announced))
    }

    /// 用内核里各客户端的最近记录补齐（界面可能错过了事件，或记录被裁掉）
    func syncHistory() {
        guard let app, running else { return }
        objectWillChange.send()
        for client in status.clients {
            var prevOrder: Double = 0
            var prevAt = client.lastActiveMs
            var step = 0.0
            for m in app.client.mcpHistory(clientKey: client.key) {
                guard case .tool(let call, let result) = m else { continue }
                if let c = find(client.key, call.callId) {
                    if let result, c.result == nil { c.apply(result: result) }
                    prevOrder = c.order
                    prevAt = c.at
                    step = 0
                    continue
                }
                // 不认识的记录：已结束的排在同一客户端前一条已知记录之后；还没结束的（事件还在路上就先被补齐收进来）
                // 是正在进行的，按最新排，不然新客户端的第一条待批准会因为前面没有已知记录而排到最早
                let order: Double
                if result == nil {
                    order = nextOrder
                    nextOrder += 1
                } else {
                    step += 0.001
                    order = prevOrder + step
                }
                let c = MCPCall(clientKey: client.key, call: call, needsApproval: false, at: result == nil ? Clock.nowMs : prevAt, order: order)
                if let result {
                    c.apply(result: result)
                    if result.denied { c.decision = .denied(nil) }
                } else {
                    c.phase = .running
                }
                calls.append(c)
                index[c.id] = c
            }
        }
        calls.sort { $0.order > $1.order }
        trim()
    }

    // MARK: 系统通知

    private func announce(_ c: MCPCall) {
        let what: String
        if c.isUseTarget { what = L("mcp.useTarget.ask", c.call.targetName) }
        else if c.isUseApp { what = c.call.summary }
        else { what = c.call.summary.isEmpty ? L("ai.tool." + c.call.tool) : c.call.summary }
        let target = c.isUseTarget || c.call.targetName.isEmpty ? "" : " → " + c.call.targetName
        announce(id: "mcp/" + c.id, title: L("mcp.notify.approval", clientName(c.clientKey)), body: what + target)
    }

    /// 有新的待批准：窗口不在前台或不在 MCP 页时发系统通知并在程序坞提醒
    private func announce(id: String, title: String, body: String) {
        guard let app, !announced.contains(id) else { return }
        announced.insert(id)
        let onPage = app.phase == .main && app.section == .mcp && app.activeTabId == nil
        let window = (NSApp.delegate as? AppDelegate)?.window
        let front = NSApp.isActive && (window?.isVisible ?? false) && !(window?.isMiniaturized ?? true)
        if front && onPage { return }
        notifyCount += 1
        guard !app.selfTest else { return }
        MCPNotifier.post(id: id, title: title, body: body)
        NSApp.requestUserAttention(.informationalRequest)
    }

    // MARK: 演示（模拟内核）

    var mock: MockMcpService? { (app?.client as? MockGalaxyClient)?.mockMcp }
}

// MARK: - 回调适配：内核线程 → 主线程

final class MCPListenerAdapter: McpListener, @unchecked Sendable {
    weak var owner: MCPModel?
    func onStatus(status: McpStatus) {
        DispatchQueue.main.async { [weak self] in MainActor.assumeIsolated { self?.owner?.apply(status) } }
    }
    func onEvent(clientKey: String, event: AgentEvent) {
        DispatchQueue.main.async { [weak self] in MainActor.assumeIsolated { self?.owner?.handle(clientKey, event) } }
    }
}

// MARK: - 系统通知

/// 有新的待批准时的系统通知，点开回到 MCP 页。只在正式 .app 里可用：裸可执行文件调用通知中心会直接崩溃。
enum MCPNotifier {
    static var available: Bool { Bundle.main.bundleIdentifier != nil && Bundle.main.bundlePath.hasSuffix(".app") }
    private static let handler = MCPNotificationHandler()

    /// 启动时接管通知的点击
    static func install() {
        guard available else { return }
        UNUserNotificationCenter.current().delegate = handler
    }

    /// 开启 MCP 服务时才请求通知权限（不用 MCP 的人不会被问）
    static func requestAuthorization() {
        guard available else { return }
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }

    static func post(id: String, title: String, body: String) {
        guard available else { return }
        let c = UNMutableNotificationContent()
        c.title = title
        c.body = body
        c.sound = .default
        c.userInfo = ["section": "mcp"]
        UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: id, content: c, trigger: nil)) { _ in }
    }

    /// 已经处理了的待批准：收回通知
    static func remove(_ ids: [String]) {
        guard available, !ids.isEmpty else { return }
        UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: ids)
    }
}

final class MCPNotificationHandler: NSObject, UNUserNotificationCenterDelegate, @unchecked Sendable {
    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse, withCompletionHandler done: @escaping () -> Void) {
        let open = response.actionIdentifier != UNNotificationDismissActionIdentifier
        DispatchQueue.main.async {
            if open { MainActor.assumeIsolated { (NSApp.delegate as? AppDelegate)?.showMCP() } }
            done()
        }
    }

    /// 主窗口在前台但不在 MCP 页时也弹横幅（默认前台应用的通知不显示）
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification, withCompletionHandler done: @escaping (UNNotificationPresentationOptions) -> Void) {
        done([.banner, .sound])
    }
}
