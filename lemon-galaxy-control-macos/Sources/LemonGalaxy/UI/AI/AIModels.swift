import SwiftUI
import AppKit

/// AI 分区的总状态：可用模型、新建对话草稿、已打开的对话。
@MainActor
final class AIHubModel: ObservableObject {
    weak var app: AppModel?
    @Published var models: AiModels?
    @Published var loadingModels = false
    @Published var modelsError: String?
    @Published var draftTargets: [AgentTarget] = []
    @Published var draftModel = ""          // "<provider_id>:<model_id>"
    @Published var draftMode: AiMode = .confirmDangerous
    @Published var draftTrustMinutes: Int = 0      // 0 = 不限时
    @Published var draftPlanFirst = false
    @Published var draftPrompt = ""
    @Published var conversations: [AIConversationModel] = []
    @Published var summaries: [ConversationSummary] = []

    func loadModels() {
        guard models == nil, !loadingModels, let app else { return }
        loadingModels = true
        draftMode = app.settings.aiMode
        draftPlanFirst = app.settings.aiPlanFirst
        Task {
            do {
                let m = try await app.client.aiModels()
                models = m
                if draftModel.isEmpty {
                    let last = app.settings.aiLastModel
                    draftModel = m.items.contains(where: { key($0) == last }) ? last : (m.items.first.map(key) ?? "")
                }
                if draftMode.rank > m.maxMode.rank { draftMode = m.maxMode }
            } catch {
                modelsError = AppModel.describe(error)
            }
            loadingModels = false
        }
    }

    func key(_ m: AiModel) -> String { "\(m.providerId):\(m.id)" }
    var selectedModel: AiModel? { models?.items.first { key($0) == draftModel } }

    func reloadSummaries() {
        guard let app else { summaries = []; return }
        let open = Set(conversations.compactMap(\.conversationId))
        summaries = app.client.listConversations().filter { !open.contains($0.id) }
    }

    func targetName(_ t: AgentTarget) -> String {
        guard let app else { return t.id }
        return t.kind == .device ? app.deviceName(t.id) : app.sshLabel(t.id)
    }

    func toggleTarget(_ t: AgentTarget) {
        if let i = draftTargets.firstIndex(of: t) { draftTargets.remove(at: i) } else { draftTargets.append(t) }
    }

    var canStart: Bool { selectedModel != nil && !draftTargets.isEmpty && !draftPrompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

    /// 新建对话（内核会先连上所有目标设备）并发出第一条消息
    @discardableResult
    func start() -> AIConversationModel? {
        guard let app, let m = selectedModel, canStart else { return nil }
        app.settings.aiLastModel = draftModel
        let trust: UInt32? = draftMode == .fullTrust && draftTrustMinutes > 0 ? UInt32(draftTrustMinutes) : nil
        let prompt = draftPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        let config = ConversationConfig(providerId: m.providerId, modelId: m.id, targets: draftTargets, mode: draftMode, planFirst: draftPlanFirst,
                                        trustMinutes: trust, title: String(prompt.prefix(40)))
        let tab = AIConversationModel(app: app, config: config, model: m)
        tab.trustMinutes = draftTrustMinutes
        conversations.insert(tab, at: 0)
        app.addTab(tab)
        draftPrompt = ""
        tab.create(firstMessage: prompt)
        return tab
    }

    /// 重新打开本机保存的对话：优先用创建时保存的配置；老记录没有配置时按目标名称找回设备 id
    func reopen(_ s: ConversationSummary) {
        guard let app else { return }
        let saved = s.config
        let model = models?.items.first { $0.id == (saved?.modelId ?? s.modelId) && (saved == nil || $0.providerId == saved!.providerId) }
            ?? AiModel(providerId: saved?.providerId ?? 0, providerName: "", id: s.modelId, name: s.modelId, vision: false, tools: true, contextLength: 0)
        let config: ConversationConfig
        if let saved {
            config = ConversationConfig(providerId: saved.providerId, modelId: saved.modelId, targets: saved.targets, mode: saved.mode,
                                        planFirst: saved.planFirst, trustMinutes: saved.trustMinutes, title: s.title)
        } else {
            let targets: [AgentTarget] = s.targetNames.compactMap { name in
                app.devices.first { $0.hostname == name || app.deviceName($0.id) == name }.map { AgentTarget(kind: .device, id: $0.id) }
            }
            config = ConversationConfig(providerId: model.providerId, modelId: model.id, targets: targets, mode: app.settings.aiMode,
                                        planFirst: false, trustMinutes: nil, title: s.title)
        }
        let tab = AIConversationModel(app: app, config: config, model: model)
        tab.title = s.title
        tab.savedTargetNames = s.targetNames
        conversations.insert(tab, at: 0)
        app.addTab(tab)
        tab.open(savedId: s.id)
        reloadSummaries()
    }

    func delete(_ s: ConversationSummary) {
        app?.client.deleteConversation(id: s.id)
        reloadSummaries()
    }

    func focus(_ c: AIConversationModel) {
        guard let app else { return }
        if !app.tabs.contains(where: { $0.id == c.id }) { app.tabs.append(c) }
        app.activeTabId = c.id
    }
}

/// 对话里的一个条目
enum ChatItem: Identifiable {
    case user(id: String, text: String)
    case assistant(id: String)
    case thinking(id: String)
    case tool(ToolCardModel)
    case ask(AskModel)
    case secret(SecretRequestModel)
    case error(id: String, text: String)

    var id: String {
        switch self {
        case .user(let id, _), .assistant(let id), .thinking(let id), .error(let id, _): return id
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

/// 一个 AI 对话标签。内核对象要异步创建（先连上目标设备），创建完成前发出的消息先排队。
@MainActor
final class AIConversationModel: TabModel {
    private(set) var conversation: AgentConversation?
    let config: ConversationConfig
    let model: AiModel
    let listener: AgentListenerAdapter
    @Published var preparing = false
    @Published var items: [ChatItem] = []
    @Published var texts: [String: String] = [:]         // assistant / thinking 消息的累计文字
    @Published var doneMessages: Set<String> = []
    @Published var running = false
    @Published var step: UInt32 = 0
    @Published var promptTokens: UInt64 = 0
    @Published var completionTokens: UInt64 = 0
    @Published var mode: AiMode
    @Published var effectiveMode: AiMode?
    @Published var trustUntil: Date?
    @Published var trustMinutes = 0
    @Published var composer = ""
    @Published var activeSecret: SecretRequestModel?
    @Published var zoomImage: ImageData?
    var savedTargetNames: [String]?
    private var pending: [String] = []
    var conversationId: String? { conversation?.id() }

    init(app: AppModel, config: ConversationConfig, model: AiModel) {
        self.config = config
        self.model = model
        self.mode = config.mode
        self.listener = AgentListenerAdapter()
        super.init(app: app, kind: .ai, title: config.title?.isEmpty == false ? config.title! : L("ai.newConversation"))
        listener.owner = self
    }

    var targetNames: [String] { savedTargetNames ?? config.targets.map { app?.ai.targetName($0) ?? $0.id } }

    /// 新对话：内核建对话（连目标设备）后发出第一条消息
    func create(firstMessage: String) {
        appendUser(firstMessage)
        pending.append(firstMessage)
        attach { [config, listener] client in try await client.newConversation(config: config, listener: listener) }
    }

    /// 打开本机保存的对话并恢复历史
    func open(savedId: String) {
        attach { [config, listener] client in try await client.openConversation(id: savedId, config: config, listener: listener) }
    }

    private func attach(_ make: @escaping (GalaxyClient) async throws -> AgentConversation) {
        guard let client = app?.client else { return }
        preparing = true
        running = true
        live = true
        Task {
            do {
                let conv = try await make(client)
                conversation = conv
                preparing = false
                if items.isEmpty { restoreHistory() }
                let t = conv.title()
                if !t.isEmpty { title = t }
                let queued = pending
                pending.removeAll()
                if queued.isEmpty { running = false }
                for text in queued { conv.send(text: text) }
            } catch {
                preparing = false
                running = false
                live = false
                items.append(.error(id: UUID().uuidString, text: AppModel.describe(error)))
            }
        }
    }

    private func appendUser(_ t: String) {
        if title == L("ai.newConversation") { title = String(t.prefix(24)) }
        items.append(.user(id: UUID().uuidString, text: t))
    }

    func send(_ text: String) {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return }
        appendUser(t)
        running = true
        live = true
        if let conversation { conversation.send(text: t) } else { pending.append(t) }
    }

    func stop() {
        conversation?.stop()
        running = false
        for case .tool(let card) in items where card.decision == .pending {
            card.decision = .denied(nil)
        }
    }

    func changeMode(_ m: AiMode, minutes: Int) {
        mode = m
        trustMinutes = minutes
        conversation?.setMode(mode: m, trustMinutes: m == .fullTrust && minutes > 0 ? UInt32(minutes) : nil)
    }

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
        activeSecret = nil
    }

    func cancelSecret(_ req: SecretRequestModel) {
        conversation?.provideSecret(requestId: req.id, secret: "")
        req.cancelled = true
        activeSecret = nil
    }

    func restoreHistory() {
        guard let conversation else { return }
        for m in conversation.messages() {
            switch m {
            case .user(let text): items.append(.user(id: UUID().uuidString, text: text))
            case .assistant(let id, let text, let thinking):
                if !thinking.isEmpty {
                    texts[id + "-thinking"] = thinking
                    items.append(.thinking(id: id + "-thinking"))
                }
                texts[id] = text
                doneMessages.insert(id)
                items.append(.assistant(id: id))
            case .tool(let call, let result):
                let card = ToolCardModel(call: call, needsApproval: false)
                card.result = result
                card.output = result?.output ?? ""
                card.phase = .finished
                items.append(.tool(card))
            }
        }
    }

    private func card(_ id: String) -> ToolCardModel? {
        for case .tool(let c) in items where c.call.callId == id { return c }
        return nil
    }

    func handle(_ e: AgentEvent) {
        switch e {
        case .assistantDelta(let id, let text):
            if texts[id] == nil { items.append(.assistant(id: id)) }
            texts[id, default: ""] += text
        case .assistantDone(let id):
            doneMessages.insert(id)
        case .thinking(let id, let text):
            if texts[id] == nil { items.append(.thinking(id: id)) }
            texts[id, default: ""] += text
        case .toolProposed(let call, let needs):
            items.append(.tool(ToolCardModel(call: call, needsApproval: needs)))
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
            let r = SecretRequestModel(id: rid, prompt: prompt, targetId: target)
            items.append(.secret(r))
            activeSecret = r
        case .usage(let p, let c):
            promptTokens = p
            completionTokens = c
        case .status(let r, let s):
            running = r
            step = s
        case .modeChanged(let m, let effective, let until):
            mode = m
            effectiveMode = effective
            trustUntil = until.map { Date(timeIntervalSince1970: TimeInterval($0) / 1000) }
        case .error(let m):
            items.append(.error(id: UUID().uuidString, text: m))
        }
    }

    override func close() {
        if running { conversation?.stop() }
        app?.ai.conversations.removeAll { $0 === self }
        app?.ai.reloadSummaries()
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
