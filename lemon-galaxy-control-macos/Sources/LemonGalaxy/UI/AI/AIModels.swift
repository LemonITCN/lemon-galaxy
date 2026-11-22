import SwiftUI
import AppKit

/// AI 页面能不能用：进入页面就查 ai_models()，按结果决定显示什么（docs/ai-chat-ui.md 第 5 节）
enum AIAvailability: Equatable {
    case loading
    case failed(String)
    /// 账号没开通 AI
    case disabled
    /// 开通了 AI 但管理员没给任何模型
    case noModels
    case ready
}

/// 对话列表里的一行：本次打开过的对话（可能正在进行）或本机保存的历史
struct AIChatRow: Identifiable {
    let id: String
    let title: String
    let updatedAt: Int64
    let live: AIConversationModel?
    let summary: ConversationSummary?
}

/// @ 候选与「设备或主机」选择列表里的一项
struct RefCandidate: Identifiable {
    let ref: ContextRef
    let platform: Platform?
    let online: Bool
    var id: String { ref.key }
}

extension ContextRef {
    /// 同一个资源只引用一次
    var key: String { "\(kind)|\(targetId)|\(id)" }
}

/// AI 分区的总状态：模型列表、对话列表、当前显示的对话。
@MainActor
final class AIHubModel: ObservableObject {
    weak var app: AppModel?
    @Published var models: AiModels?
    @Published var loadingModels = false
    @Published var modelsError: String?
    /// 本次运行里打开或新建过的对话（正在进行的任务不因切换对话而中断）
    @Published var conversations: [AIConversationModel] = []
    /// 本机保存的对话（内核加密保存）
    @Published var summaries: [ConversationSummary] = []
    /// 右侧显示的对话；还没发消息的新对话也是一个对象（没有内核对话）
    @Published var current: AIConversationModel?
    @Published var search = ""
    /// selftest 直接摆出各种「不可用」状态时不让后台加载覆盖掉
    var pinModels = false

    var availability: AIAvailability {
        if app?.user?.aiEnabled == false { return .disabled }
        if let m = models {
            if !m.enabled { return .disabled }
            return m.items.isEmpty ? .noModels : .ready
        }
        if let e = modelsError, !loadingModels { return .failed(e) }
        return .loading
    }

    /// 进入 AI 页面就调用；已有结果时在后台刷新（管理员刚开通的模型能马上看到）
    func loadModels() {
        guard !loadingModels, !pinModels, let app else { return }
        loadingModels = true
        if models == nil { modelsError = nil }
        Task {
            do {
                let m = try await app.client.aiModels()
                if !pinModels {
                    models = m
                    modelsError = nil
                    for c in conversations + [current].compactMap({ $0 }) { c.applyDefaultsIfNeeded() }
                }
            } catch {
                if !pinModels, models == nil { modelsError = AppModel.describe(error) }
            }
            loadingModels = false
        }
    }

    func retry() {
        models = nil
        modelsError = nil
        loadModels()
    }

    static func key(_ providerId: UInt64, _ modelId: String) -> String { "\(providerId):\(modelId)" }

    func model(_ providerId: UInt64, _ modelId: String) -> AiModel? {
        models?.items.first { $0.providerId == providerId && $0.id == modelId }
    }

    /// 默认模型：本机记住的上次选择，没有或已不可用时取列表第一个
    var defaultModel: AiModel? {
        guard let items = models?.items, !items.isEmpty else { return nil }
        let last = app?.settings.aiLastModel ?? ""
        return items.first { Self.key($0.providerId, $0.id) == last } ?? items.first
    }

    /// 默认档位：上次选的（默认「危险操作确认」），不超过管理员上限
    var defaultMode: AiMode {
        let m = app?.settings.aiMode ?? .confirmDangerous
        let cap = models?.maxMode ?? .fullTrust
        return m.rank > cap.rank ? cap : m
    }

    var maxMode: AiMode { models?.maxMode ?? .fullTrust }

    // MARK: 对话

    /// 当前对话；第一次进入时建一个空的新对话
    func ensureCurrent() {
        if current == nil, let app { current = AIConversationModel(app: app) }
    }

    /// 新对话：当前已经是空的新对话就沿用（只补上引用）
    func newChat(refs: [ContextRef] = []) {
        guard let app else { return }
        if let c = current, c.isDraft {
            for r in refs { c.addRef(r) }
            c.focusToken += 1
            return
        }
        let c = AIConversationModel(app: app)
        for r in refs { c.addRef(r) }
        current = c
    }

    func select(_ c: AIConversationModel) {
        current = c
        app?.selectHome(.ai)
    }

    /// 打开历史对话：已经打开过的直接切过去
    func open(_ s: ConversationSummary) {
        guard let app else { return }
        if let live = conversations.first(where: { $0.conversationId == s.id }) {
            select(live)
            return
        }
        let c = AIConversationModel(app: app)
        conversations.insert(c, at: 0)
        current = c
        app.selectHome(.ai)
        c.open(summary: s)
    }

    /// 新对话建好后登记到列表
    func didCreate(_ c: AIConversationModel) {
        if !conversations.contains(where: { $0 === c }) { conversations.insert(c, at: 0) }
        reloadSummaries()
    }

    func delete(_ row: AIChatRow) {
        if let live = row.live {
            if live.running { live.stop() }
            conversations.removeAll { $0 === live }
            if current === live { current = nil; ensureCurrent() }
        }
        app?.client.deleteConversation(id: row.id)
        reloadSummaries()
    }

    func reloadSummaries() {
        guard let app else { summaries = []; return }
        summaries = app.client.listConversations()
    }

    /// 退出登录、会话被吊销：停掉所有对话
    func closeAll() {
        for c in conversations where c.running { c.stop() }
        conversations.removeAll()
        current = nil
        summaries = []
        models = nil
        modelsError = nil
    }

    /// 左侧列表：打开过的对话 + 本机保存的历史，按更新时间倒序，按搜索词过滤
    var rows: [AIChatRow] {
        var out: [AIChatRow] = []
        var seen = Set<String>()
        for c in conversations {
            guard let id = c.conversationId else { continue }
            seen.insert(id)
            let s = summaries.first { $0.id == id }
            out.append(AIChatRow(id: id, title: c.title.isEmpty ? (s?.title ?? "") : c.title, updatedAt: max(c.updatedAt, s?.updatedAt ?? 0), live: c, summary: s))
        }
        for s in summaries where !seen.contains(s.id) {
            out.append(AIChatRow(id: s.id, title: s.title, updatedAt: s.updatedAt, live: nil, summary: s))
        }
        let q = search.trimmingCharacters(in: .whitespaces)
        if !q.isEmpty { out = out.filter { $0.title.localizedCaseInsensitiveContains(q) } }
        return out.sorted { $0.updatedAt > $1.updatedAt }
    }

    // MARK: 引用

    /// 可引用的设备（不含本机与手机）与 SSH 主机，按名称过滤
    func candidates(_ query: String = "") -> [RefCandidate] {
        guard let app else { return [] }
        var out: [RefCandidate] = []
        for d in app.devices where !d.isSelf && d.kind != .mobile {
            let name = app.deviceName(d.id)
            out.append(RefCandidate(ref: ContextRef(kind: .device, targetId: "", id: d.id, label: name), platform: d.platform, online: d.online || d.lanSeen))
        }
        for e in app.vault {
            if case .sshHost = e.data {
                out.append(RefCandidate(ref: ContextRef(kind: .sshHost, targetId: "", id: e.id, label: app.sshLabel(e.id)), platform: nil, online: true))
            }
        }
        let q = query.trimmingCharacters(in: .whitespaces)
        if !q.isEmpty {
            out = out.filter { c in
                if c.ref.label.localizedCaseInsensitiveContains(q) { return true }
                if c.ref.kind == .device, let d = app.device(c.ref.id) { return d.hostname.localizedCaseInsensitiveContains(q) }
                return false
            }
        }
        // 在线的排前面，其余按名称
        return out.sorted { a, b in a.online != b.online ? a.online : a.ref.label.localizedStandardCompare(b.ref.label) == .orderedAscending }
    }

    /// 从主机页「AI」入口带过来的目标
    func ref(for t: AgentTarget) -> ContextRef {
        switch t.kind {
        case .device: return ContextRef(kind: .device, targetId: "", id: t.id, label: app?.deviceName(t.id) ?? t.id)
        case .sshHost: return ContextRef(kind: .sshHost, targetId: "", id: t.id, label: app?.sshLabel(t.id) ?? t.id)
        }
    }

    /// 设备 ID 或 SSH 主机条目 ID → 显示名
    func targetName(_ id: String) -> String {
        guard let app else { return id }
        if app.device(id) != nil { return app.deviceName(id) }
        if app.entry(id) != nil { return app.sshLabel(id) }
        return id
    }
}

/// 对话里的一个条目
enum ChatItem: Identifiable {
    case user(id: String, text: String, refs: [ContextRef])
    case assistant(id: String)
    case thinking(id: String)
    case tool(ToolCardModel)
    case ask(AskModel)
    case secret(SecretRequestModel)
    case error(id: String, text: String)

    var id: String {
        switch self {
        case .user(let id, _, _), .assistant(let id), .thinking(let id), .error(let id, _): return id
        case .tool(let t): return t.id
        case .ask(let a): return a.id
        case .secret(let s): return s.id
        }
    }
}

@MainActor
final class ToolCardModel: ObservableObject, Identifiable {
    enum Decision: Equatable { case none, pending, approved, approvedSimilar, denied(String?), edited(String), auto }
    enum Phase: Equatable { case proposed, running, finished }
    let call: ToolCallView
    nonisolated let id: String
    @Published var decision: Decision
    @Published var phase: Phase = .proposed
    @Published var output = ""
    @Published var result: ToolResultView?
    @Published var expanded = false
    @Published var outputExpanded = false
    @Published var editing = false
    @Published var editText = ""
    @Published var denying = false
    @Published var denyReason = ""

    init(call: ToolCallView, needsApproval: Bool) {
        self.call = call
        self.id = call.callId
        self.decision = needsApproval ? .pending : .auto
        self.editText = call.summary
    }

    var isUseTarget: Bool { call.tool == "use_target" }
    var failed: Bool {
        if case .denied = decision { return true }
        guard let r = result else { return false }
        return r.denied || !r.ok
    }
}

@MainActor
final class AskModel: ObservableObject, Identifiable {
    let id: String
    let question: String
    let options: [String]
    @Published var answer: String?
    @Published var freeText = ""
    init(id: String, question: String, options: [String]) { self.id = id; self.question = question; self.options = options }
}

@MainActor
final class SecretRequestModel: ObservableObject, Identifiable {
    let id: String
    let prompt: String
    let targetId: String
    @Published var done = false
    @Published var cancelled = false
    init(id: String, prompt: String, targetId: String) { self.id = id; self.prompt = prompt; self.targetId = targetId }
}

/// 输入框里正在输入的 @ 引用：range 是「@过滤文字」在文本里的位置，caret 是 @ 在输入框里的坐标（弹出候选用）
struct MentionState: Equatable {
    var range: NSRange
    var query: String
    var caret: CGPoint
    var selected = 0
}

/// 一个 AI 对话。还没发第一条消息时只是输入框草稿；发出第一条时才建内核对话（docs/ai-chat-ui.md 第 3 节）。
@MainActor
final class AIConversationModel: ObservableObject, Identifiable {
    let id = UUID().uuidString
    weak var app: AppModel?
    private(set) var conversation: AgentConversation?
    let listener = AgentListenerAdapter()
    @Published private(set) var conversationId: String?
    @Published var title = ""
    @Published var updatedAt: Int64 = 0
    @Published var items: [ChatItem] = []
    @Published var texts: [String: String] = [:]         // assistant / thinking 消息的累计文字
    @Published var doneMessages: Set<String> = []
    @Published var running = false
    /// 正在建对话或打开历史对话
    @Published var connecting = false
    @Published var step: UInt32 = 0
    @Published var promptTokens: UInt64 = 0
    @Published var completionTokens: UInt64 = 0
    @Published var effectiveMode: AiMode?
    @Published var trustUntil: Date?
    @Published var zoomImage: ImageData?
    /// 每来一个事件加一，消息流据此决定要不要滚到底
    @Published var contentVersion = 0

    // 输入框
    @Published var text = ""
    @Published var refs: [ContextRef] = []
    @Published var providerId: UInt64 = 0
    @Published var modelId = ""
    @Published var mode: AiMode = .confirmDangerous
    /// 完全信任的时限（分钟），0 为本对话内一直有效
    @Published var trustMinutes = 0
    @Published var planFirst = false
    @Published var composerError: String?
    @Published var mention: MentionState?
    /// 输入框要求把光标放到这里（选中 @ 候选后）
    @Published var caretRequest: Int?
    /// 加一时输入框获得焦点
    @Published var focusToken = 0
    private var modelChosen = false

    init(app: AppModel) {
        self.app = app
        listener.owner = self
        applyDefaultsIfNeeded()
        let hub = app.ai
        mode = hub.defaultMode
        planFirst = app.settings.aiPlanFirst
    }

    var hub: AIHubModel? { app?.ai }
    /// 还没发过消息的新对话
    var isDraft: Bool { conversation == nil && items.isEmpty && !connecting }
    var model: AiModel? { hub?.model(providerId, modelId) }
    var modelName: String { model?.name ?? modelId }

    /// 模型还没定（新对话、模型列表刚到）时取默认值
    func applyDefaultsIfNeeded() {
        guard !modelChosen, conversation == nil, !connecting, let m = hub?.defaultModel else { return }
        providerId = m.providerId
        modelId = m.id
        if let hub, mode.rank > hub.maxMode.rank { mode = hub.maxMode }
    }

    var canSend: Bool {
        guard !running, !connecting, hub?.availability == .ready, model != nil || conversation != nil else { return false }
        return !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !refs.isEmpty
    }

    func addRef(_ r: ContextRef) {
        if !refs.contains(where: { $0.key == r.key }) { refs.append(r) }
    }

    func removeRef(_ r: ContextRef) { refs.removeAll { $0.key == r.key } }

    /// 选中 @ 候选：删掉「@过滤文字」，加一个引用标签
    func pickMention(_ r: ContextRef) {
        if let m = mention {
            let ns = text as NSString
            if m.range.location + m.range.length <= ns.length {
                text = ns.replacingCharacters(in: m.range, with: "")
                caretRequest = m.range.location
            }
        }
        mention = nil
        addRef(r)
        focusToken += 1
    }

    private var trustValue: UInt32? { mode == .fullTrust && trustMinutes > 0 ? UInt32(trustMinutes) : nil }

    // MARK: 发送

    func send() {
        guard canSend, let app else { return }
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let r = refs
        composerError = nil
        text = ""
        refs = []
        mention = nil
        let userId = appendUser(t, r)
        app.settings.aiLastModel = AIHubModel.key(providerId, modelId)
        if let conversation {
            running = true
            conversation.sendWithRefs(text: t, refs: r)
            return
        }
        // 第一条消息：这时才建内核对话，不预先选设备
        connecting = true
        running = true
        let config = ConversationConfig(providerId: providerId, modelId: modelId, targets: [], mode: mode, planFirst: planFirst,
                                        trustMinutes: trustValue, title: nil)
        let listener = listener
        Task {
            do {
                let conv = try await app.client.newConversation(config: config, listener: listener)
                conversation = conv
                conversationId = conv.id()
                connecting = false
                modelChosen = true
                let ct = conv.title()
                if !ct.isEmpty { title = ct }
                hub?.didCreate(self)
                conv.sendWithRefs(text: t, refs: r)
            } catch {
                // 建对话失败：消息退回输入框，错误显示在输入框上方
                connecting = false
                running = false
                items.removeAll { $0.id == userId }
                if text.isEmpty { text = t }
                for x in r { addRef(x) }
                if items.isEmpty { title = "" }
                composerError = AppModel.describe(error)
            }
        }
    }

    @discardableResult
    private func appendUser(_ t: String, _ r: [ContextRef]) -> String {
        if title.isEmpty { title = String((t.isEmpty ? r.map(\.label).joined(separator: ", ") : t).prefix(40)) }
        let id = UUID().uuidString
        items.append(.user(id: id, text: t, refs: r))
        updatedAt = Clock.nowMs
        contentVersion += 1
        return id
    }

    /// 打开本机保存的对话：配置取对话保存的；老记录没有配置时用当前默认模型
    func open(summary s: ConversationSummary) {
        guard let app else { return }
        title = s.title
        updatedAt = s.updatedAt
        conversationId = s.id
        modelChosen = true
        if let c = s.config {
            providerId = c.providerId
            modelId = c.modelId
            mode = c.mode
            planFirst = c.planFirst
            trustMinutes = Int(c.trustMinutes ?? 0)
        } else {
            modelId = s.modelId
            if let m = hub?.models?.items.first(where: { $0.id == s.modelId }) ?? hub?.defaultModel { providerId = m.providerId; modelId = m.id }
        }
        let fallback: ConversationConfig? = s.config == nil
            ? ConversationConfig(providerId: providerId, modelId: modelId, targets: [], mode: mode, planFirst: planFirst, trustMinutes: nil, title: s.title)
            : nil
        connecting = true
        let listener = listener
        Task {
            do {
                let conv = try await app.client.openConversation(id: s.id, config: fallback, listener: listener)
                conversation = conv
                connecting = false
                restoreHistory()
                let m = conv.model()
                providerId = m.providerId
                modelId = m.modelId
                let ct = conv.title()
                if !ct.isEmpty { title = ct }
            } catch {
                connecting = false
                items.append(.error(id: UUID().uuidString, text: AppModel.describe(error)))
            }
        }
    }

    func stop() {
        conversation?.stop()
        running = false
        for case .tool(let card) in items where card.decision == .pending {
            card.decision = .denied(nil)
        }
    }

    // MARK: 中途换模型、档位

    func setModel(_ m: AiModel) {
        if let conversation {
            do {
                try conversation.setModel(providerId: m.providerId, modelId: m.id)
            } catch {
                composerError = AppModel.describe(error)
                return
            }
        }
        modelChosen = true
        composerError = nil
        providerId = m.providerId
        modelId = m.id
        app?.settings.aiLastModel = AIHubModel.key(m.providerId, m.id)
    }

    func setMode(_ m: AiMode, minutes: Int = 0) {
        mode = m
        trustMinutes = m == .fullTrust ? minutes : 0
        app?.settings.aiMode = m
        conversation?.setMode(mode: m, trustMinutes: trustValue)
    }

    func setPlanFirst(_ on: Bool) {
        planFirst = on
        app?.settings.aiPlanFirst = on
    }

    // MARK: 卡片回应

    func decide(_ card: ToolCardModel, _ d: ApprovalDecision) {
        conversation?.decide(callId: card.call.callId, decision: d)
        switch d {
        case .allowOnce: card.decision = .approved
        case .allowSimilar: card.decision = .approvedSimilar
        case .deny(let r): card.decision = .denied(r)
        case .editAndRun(let c): card.decision = .edited(c)
        }
        card.editing = false
        card.denying = false
    }

    func answer(_ ask: AskModel, _ text: String) {
        ask.answer = text
        conversation?.answer(questionId: ask.id, answer: text)
    }

    func provideSecret(_ req: SecretRequestModel, _ secret: String) {
        // 密码只交给内核送到被控端，不留在界面状态里，也不进对话历史
        conversation?.provideSecret(requestId: req.id, secret: secret)
        req.done = true
    }

    func cancelSecret(_ req: SecretRequestModel) {
        conversation?.provideSecret(requestId: req.id, secret: "")
        req.cancelled = true
    }

    var pendingSecret: SecretRequestModel? {
        for case .secret(let s) in items.reversed() where !s.done && !s.cancelled { return s }
        return nil
    }

    var hasPendingApproval: Bool {
        items.contains { if case .tool(let t) = $0 { return t.decision == .pending }; return false }
    }

    // MARK: 事件

    func restoreHistory() {
        guard let conversation else { return }
        items.removeAll()
        for m in conversation.messages() {
            switch m {
            case .user(let text, let refs): items.append(.user(id: UUID().uuidString, text: text, refs: refs))
            case .assistant(let id, let text, let thinking):
                if !thinking.isEmpty {
                    texts[id + "-thinking"] = thinking
                    items.append(.thinking(id: id + "-thinking"))
                }
                texts[id] = text
                doneMessages.insert(id)
                if !text.isEmpty { items.append(.assistant(id: id)) }
            case .tool(let call, let result):
                let card = ToolCardModel(call: call, needsApproval: false)
                card.result = result
                card.output = result?.output ?? ""
                card.phase = .finished
                if result?.denied == true { card.decision = .denied(nil) }
                items.append(.tool(card))
            }
        }
        contentVersion += 1
    }

    private func card(_ id: String) -> ToolCardModel? {
        for case .tool(let c) in items where c.call.callId == id { return c }
        return nil
    }

    func handle(_ e: AgentEvent) {
        contentVersion += 1
        switch e {
        case .assistantDelta(let id, let text):
            if texts[id] == nil { items.append(.assistant(id: id)) }
            texts[id, default: ""] += text
        case .assistantDone(let id):
            doneMessages.insert(id)
            updatedAt = Clock.nowMs
        case .thinking(let id, let text):
            if texts[id] == nil { items.append(.thinking(id: id)) }
            texts[id, default: ""] += text
        case .toolProposed(let call, let needs):
            if let c = card(call.callId) {
                // 同一调用再次提出（例如连接设备后重新确认）：更新状态
                if needs { c.decision = .pending }
            } else {
                items.append(.tool(ToolCardModel(call: call, needsApproval: needs)))
            }
        case .toolStarted(let id):
            card(id)?.phase = .running
        case .toolOutput(let id, let text):
            card(id)?.output += text
        case .toolFinished(let id, let result):
            if let c = card(id) {
                c.result = result
                c.phase = .finished
                if result.denied, case .auto = c.decision { c.decision = .denied(nil) }
                if result.output.count > c.output.count { c.output = result.output }
            }
        case .askUser(let qid, let q, let options):
            items.append(.ask(AskModel(id: qid, question: q, options: options)))
        case .secretRequested(let rid, let prompt, let target):
            items.append(.secret(SecretRequestModel(id: rid, prompt: prompt, targetId: target)))
        case .usage(let p, let c):
            promptTokens = p
            completionTokens = c
        case .status(let r, let s):
            running = r || connecting
            step = s
            if !r {
                updatedAt = Clock.nowMs
                hub?.reloadSummaries()
            }
        case .modeChanged(let m, let effective, let until):
            mode = m
            effectiveMode = effective
            trustUntil = until.map { Date(timeIntervalSince1970: TimeInterval($0) / 1000) }
        case .error(let m):
            items.append(.error(id: UUID().uuidString, text: m))
        }
    }
}

final class AgentListenerAdapter: AgentListener, @unchecked Sendable {
    weak var owner: AIConversationModel?
    func onEvent(event: AgentEvent) {
        DispatchQueue.main.async { [weak self] in MainActor.assumeIsolated { self?.owner?.handle(event) } }
    }
}

extension RiskLevel {
    var key: String {
        switch self {
        case .read: return "read"
        case .normal: return "normal"
        case .dangerous: return "dangerous"
        case .unknown: return "unknown"
        case .forbidden: return "forbidden"
        }
    }
    func color(_ p: Palette) -> Color {
        switch self {
        case .read: return p.success
        case .normal: return p.info
        case .dangerous: return p.danger
        case .unknown: return p.warning
        case .forbidden: return p.danger
        }
    }
    var icon: String {
        switch self {
        case .read: return "eye"
        case .normal: return "pencil"
        case .dangerous: return "exclamationmark.triangle.fill"
        case .unknown: return "questionmark.diamond"
        case .forbidden: return "nosign"
        }
    }
}

extension ContextRefKind {
    var icon: String {
        switch self {
        case .device: return "desktopcomputer"
        case .sshHost: return "server.rack"
        case .file: return "doc"
        case .folder: return "folder"
        case .snippet: return "chevron.left.forwardslash.chevron.right"
        case .portForward: return "arrow.left.arrow.right"
        case .activity: return "clock.arrow.circlepath"
        }
    }
}

extension AiMode {
    var icon: String {
        switch self {
        case .readOnly: return "eye"
        case .confirmAll: return "hand.raised"
        case .confirmDangerous: return "exclamationmark.shield"
        case .fullTrust: return "bolt"
        case .off: return "nosign"
        }
    }
}
