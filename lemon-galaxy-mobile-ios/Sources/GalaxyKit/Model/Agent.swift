import Foundation
import Combine

/// 一段打开中的对话：把 AgentEvent 流归并成界面条目（ChatReducer），用户动作交给内核。
@MainActor
public final class ConversationController: ObservableObject, Identifiable {
    @Published public private(set) var state: ConversationState
    public let conversation: AgentConversation
    public let config: ConversationConfig?
    public nonisolated var id: String { conversationId }
    private let conversationId: String
    /// 状态变化（对话列表刷新运行状态用）
    var onChange: (() -> Void)?

    init(conversation: AgentConversation, initial: ConversationState, config: ConversationConfig?, adapter: AgentListenerAdapter) {
        self.conversation = conversation
        self.config = config
        self.conversationId = initial.id
        var s = initial
        let history = conversation.messages()
        if !history.isEmpty { s.items = ChatReducer.items(fromHistory: history) }
        let m = conversation.model()
        if !m.modelId.isEmpty {
            s.providerId = m.providerId
            s.modelId = m.modelId
        }
        state = s
        adapter.handler = { [weak self] e in onMain { self?.apply(e) } }
    }

    func refreshModel(_ m: AiModel) {
        state.modelName = m.name
        state.vision = m.vision
    }

    func apply(_ e: AgentEvent) {
        state = ChatReducer.reduce(state, e)
        onChange?()
    }

    public func send(_ text: String) { send(text, refs: []) }

    /// 发一条消息（可带引用）：文本与引用都为空时不发
    public func send(_ text: String, refs: [ContextRef]) {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty || !refs.isEmpty else { return }
        state = ChatReducer.userSent(state, text: t, refs: refs)
        conversation.sendWithRefs(text: t, refs: refs)
        onChange?()
    }

    /// 中途换模型（上下文保留）
    public func setModel(_ m: AiModel) throws {
        try conversation.setModel(providerId: m.providerId, modelId: m.id)
        state.providerId = m.providerId
        state.modelId = m.id
        state.modelName = m.name
        state.vision = m.vision
        onChange?()
    }

    public func decide(_ callId: String, _ decision: ApprovalDecision) {
        conversation.decide(callId: callId, decision: decision)
        state = ChatReducer.decided(state, callId: callId, decision: decision)
        onChange?()
    }

    public func answer(_ questionId: String, _ answer: String) {
        conversation.answer(questionId: questionId, answer: answer)
        state = ChatReducer.answered(state, questionId: questionId, answer: answer)
        onChange?()
    }

    /// 密码直接交给内核送到被控端，界面不保存
    public func provideSecret(_ requestId: String, _ secret: String) {
        conversation.provideSecret(requestId: requestId, secret: secret)
        state = ChatReducer.secretProvided(state, requestId: requestId)
        onChange?()
    }

    public func stop() {
        conversation.stop()
        state.running = false
        onChange?()
    }

    public func setMode(_ mode: AiMode, trustMinutes: UInt32?) {
        conversation.setMode(mode: mode, trustMinutes: trustMinutes)
        state.mode = mode
        onChange?()
    }

    /// 移动端切后台时 false：内核停止当前任务（ai-agent.md 第 1 节）
    public func setForeground(_ foreground: Bool) {
        conversation.setForeground(foreground: foreground)
        if !foreground { state.running = false }
    }
}

/// 对话列表里的一行（内核的摘要 + 打开中的运行状态）。
public struct ConversationListItem: Identifiable, Equatable {
    public let id: String
    public let title: String
    public let modelName: String
    public let targetNames: [String]
    public let mode: AiMode
    public let updatedAt: Int64
    public let running: Bool
    public let waitingForUser: Bool
}

/// 模型列表、对话列表、新建 / 打开 / 删除对话。
@MainActor
public final class AgentHub: ObservableObject {
    @Published public private(set) var models: AiModels?
    @Published public private(set) var modelsError: String?
    @Published public private(set) var items: [ConversationListItem] = []
    @Published public private(set) var loadingModels = false
    private var opened: [String: ConversationController] = [:]
    private var adapters: [String: AgentListenerAdapter] = [:]
    private let client: () -> GalaxyClient
    private let hosts: () -> HostsData
    public var defaultMode: () -> AiMode = { .confirmDangerous }

    public init(client: @escaping () -> GalaxyClient, hosts: @escaping () -> HostsData) {
        self.client = client
        self.hosts = hosts
    }

    public var aiEnabled: Bool { models?.enabled ?? true }

    public func loadModels() async {
        loadingModels = true
        defer { loadingModels = false }
        do {
            models = try await client().aiModels()
            modelsError = nil
        } catch {
            modelsError = describeError(error)
        }
        refreshList()
    }

    public func model(_ id: String) -> AiModel? { models?.items.first { $0.id == id } }
    public func model(providerId: UInt64, modelId: String) -> AiModel? { models?.items.first { $0.providerId == providerId && $0.id == modelId } }

    public func refreshList() {
        let saved = client().listConversations()
        let ids = Set(saved.map(\.id))
        var list: [ConversationListItem] = saved.map { c in
            let live = opened[c.id]?.state
            return ConversationListItem(id: c.id, title: (live?.title.isEmpty == false ? live!.title : c.title),
                                        modelName: model(c.modelId)?.name ?? c.modelId, targetNames: c.targetNames,
                                        mode: live?.mode ?? c.config?.mode ?? defaultMode(), updatedAt: c.updatedAt,
                                        running: live?.running ?? false, waitingForUser: live?.waitingForUser ?? false)
        }
        // 刚建还没落盘的对话也列出来（还没发出消息的不列）
        for (id, c) in opened where !ids.contains(id) && !c.state.items.isEmpty {
            let s = c.state
            list.append(ConversationListItem(id: id, title: s.title, modelName: s.modelName, targetNames: s.targetNames, mode: s.mode,
                                             updatedAt: Clock.nowMs, running: s.running, waitingForUser: s.waitingForUser))
        }
        items = list.sorted { $0.updatedAt > $1.updatedAt }
    }

    private func attach(_ c: ConversationController) {
        c.onChange = { [weak self] in self?.refreshListSoon() }
        opened[c.id] = c
    }

    private var refreshScheduled = false
    private func refreshListSoon() {
        guard !refreshScheduled else { return }
        refreshScheduled = true
        Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 300_000_000)
            self?.refreshScheduled = false
            self?.refreshList()
        }
    }

    /// 新建对话（聊天式：targets 为空，设备在消息里引用、用到时才连接）。发第一条消息时才调用
    public func create(_ config: ConversationConfig) async throws -> ConversationController {
        let adapter = AgentListenerAdapter()
        let conv = try await client().newConversation(config: config, listener: adapter)
        let m = models?.items.first { $0.providerId == config.providerId && $0.id == config.modelId }
        let names = config.targets.map { hosts().targetName($0) }
        var s = ConversationState(id: conv.id(), title: config.title ?? "", modelName: m?.name ?? config.modelId,
                                  vision: m?.vision ?? false, targetNames: names, mode: config.mode)
        s.providerId = config.providerId
        s.modelId = config.modelId
        s.planFirst = config.planFirst
        let c = ConversationController(conversation: conv, initial: s, config: config, adapter: adapter)
        adapters[c.id] = adapter
        attach(c)
        return c
    }

    /// 最近的登录与连接记录
    public func activity(limit: UInt32 = 50) async throws -> [ActivityItem] {
        try await client().accountActivity(limit: limit)
    }

    public func opened(_ id: String) -> ConversationController? { opened[id] }

    /// 打开历史对话：内核保存了创建时的配置（老记录没有，就按摘要里的模型与目标名称拼一个）
    public func open(_ id: String) async throws -> ConversationController {
        if let c = opened[id] { return c }
        guard let summary = client().listConversations().first(where: { $0.id == id }) else {
            throw GalaxyError.NotFound(detail: L("ai.notFound"))
        }
        if models == nil { await loadModels() }
        let m = model(summary.modelId)
        // 有保存的配置时传空，由内核用保存的配置；老记录没有，按摘要里的模型与目标名称拼一个
        var config: ConversationConfig? = nil
        if summary.config == nil {
            let h = hosts()
            let targets: [AgentTarget] = summary.targetNames.compactMap { name in
                if let d = h.devices.first(where: { h.deviceName($0.id) == name || $0.hostname == name }) { return AgentTarget(kind: .device, id: d.id) }
                if let s = h.sshHosts.first(where: { h.sshLabel($0.id) == name }) { return AgentTarget(kind: .sshHost, id: s.id) }
                return nil
            }
            config = ConversationConfig(providerId: m?.providerId ?? 0, modelId: summary.modelId, targets: targets, mode: defaultMode(),
                                        planFirst: false, trustMinutes: nil, title: summary.title)
        }
        let adapter = AgentListenerAdapter()
        let conv = try await client().openConversation(id: id, config: config, listener: adapter)
        let saved = summary.config ?? config
        var s = ConversationState(id: id, title: summary.title, modelName: m?.name ?? summary.modelId, vision: m?.vision ?? false,
                                  targetNames: summary.targetNames, mode: saved?.mode ?? defaultMode())
        s.providerId = saved?.providerId ?? m?.providerId ?? 0
        s.modelId = saved?.modelId ?? summary.modelId
        s.planFirst = saved?.planFirst ?? false
        let c = ConversationController(conversation: conv, initial: s, config: saved, adapter: adapter)
        // 内核给的当前模型（可能中途换过）
        if let cur = models?.items.first(where: { $0.providerId == c.state.providerId && $0.id == c.state.modelId }) {
            c.refreshModel(cur)
        }
        adapters[id] = adapter
        attach(c)
        return c
    }

    public func delete(_ id: String) {
        opened.removeValue(forKey: id)?.stop()
        adapters[id] = nil
        client().deleteConversation(id: id)
        refreshList()
    }

    /// 切到后台：内核停止正在进行的任务；回到前台恢复
    public func setForeground(_ foreground: Bool) {
        for c in opened.values { c.setForeground(foreground) }
        if !foreground { refreshList() }
    }

    public func reset() {
        for c in opened.values { c.stop() }
        opened.removeAll()
        adapters.removeAll()
        items = []
        models = nil
        modelsError = nil
    }
}
