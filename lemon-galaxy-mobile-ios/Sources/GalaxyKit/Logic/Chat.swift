import Foundation

public enum ToolPhase: Equatable {
    case awaiting, running, done, failed, denied
}

/// 对话页里的一条。
public enum ChatItem: Identifiable {
    case user(key: String, text: String)
    case assistant(key: String, text: String, thinking: String, streaming: Bool)
    case tool(ToolEntry)
    case ask(key: String, question: String, options: [String], answer: String?)
    case secret(key: String, prompt: String, targetId: String, provided: Bool)
    case error(key: String, message: String)

    public var id: String {
        switch self {
        case .user(let k, _), .assistant(let k, _, _, _), .ask(let k, _, _, _), .secret(let k, _, _, _), .error(let k, _): return k
        case .tool(let t): return "tool-" + t.call.callId
        }
    }
}

/// 一次工具调用的卡片状态。
public struct ToolEntry {
    public var call: ToolCallView
    public var needsApproval: Bool
    public var phase: ToolPhase
    public var output: String
    public var result: ToolResultView?
    public var decision: ApprovalDecision?

    public init(call: ToolCallView, needsApproval: Bool, phase: ToolPhase, output: String = "", result: ToolResultView? = nil, decision: ApprovalDecision? = nil) {
        self.call = call
        self.needsApproval = needsApproval
        self.phase = phase
        self.output = output
        self.result = result
        self.decision = decision
    }
}

/// 一段对话的界面状态。AgentEvent 经 ChatReducer.reduce 归并进来。
public struct ConversationState {
    public var id: String
    public var title: String
    public var modelName: String
    public var vision: Bool
    public var targetNames: [String]
    public var mode: AiMode
    public var items: [ChatItem] = []
    public var running = false
    public var step: UInt32 = 0
    public var promptTokens: UInt64 = 0
    public var completionTokens: UInt64 = 0
    /// 完全信任到期时间（毫秒），不限时为空
    public var trustUntilMs: Int64?
    var seq = 0

    public init(id: String, title: String, modelName: String, vision: Bool, targetNames: [String], mode: AiMode) {
        self.id = id
        self.title = title
        self.modelName = modelName
        self.vision = vision
        self.targetNames = targetNames
        self.mode = mode
    }

    /// 有等用户处理的卡片（确认、提问、密码）
    public var waitingForUser: Bool {
        items.contains {
            switch $0 {
            case .tool(let t): return t.phase == .awaiting
            case .ask(_, _, _, let a): return a == nil
            case .secret(_, _, _, let p): return !p
            default: return false
            }
        }
    }

    mutating func nextKey(_ prefix: String) -> String {
        seq += 1
        return "\(prefix)-\(seq)"
    }
}

public enum ChatReducer {
    public static func reduce(_ state: ConversationState, _ event: AgentEvent) -> ConversationState {
        var s = state
        switch event {
        case .assistantDelta(let id, let text):
            upsertAssistant(&s, id) { t, th, _ in (t + text, th, true) }
        case .assistantDone(let id):
            upsertAssistant(&s, id) { t, th, _ in (t, th, false) }
        case .thinking(let id, let text):
            upsertAssistant(&s, id) { t, th, st in (t, th + text, st) }
        case .toolProposed(let call, let needsApproval):
            if let i = toolIndex(s, call.callId) {
                if case .tool(var t) = s.items[i] {
                    t.call = call
                    t.needsApproval = needsApproval
                    if needsApproval && t.phase == .running && t.result == nil { t.phase = .awaiting }
                    s.items[i] = .tool(t)
                }
            } else {
                s.items.append(.tool(ToolEntry(call: call, needsApproval: needsApproval, phase: needsApproval ? .awaiting : .running)))
            }
        case .toolStarted(let callId):
            updateTool(&s, callId) { t in if t.phase == .awaiting { t.phase = .running } }
        case .toolOutput(let callId, let text):
            updateTool(&s, callId) { t in t.output += text }
        case .toolFinished(let callId, let result):
            updateTool(&s, callId) { t in
                t.result = result
                t.phase = result.denied ? .denied : (result.ok ? .done : .failed)
                if t.output.isEmpty { t.output = result.output }
            }
        case .askUser(let qid, let question, let options):
            if !s.items.contains(where: { $0.id == qid }) {
                s.items.append(.ask(key: qid, question: question, options: options, answer: nil))
            }
        case .secretRequested(let rid, let prompt, let targetId):
            if !s.items.contains(where: { $0.id == rid }) {
                s.items.append(.secret(key: rid, prompt: prompt, targetId: targetId, provided: false))
            }
        case .usage(let p, let c):
            s.promptTokens = p
            s.completionTokens = c
        case .status(let running, let step):
            s.running = running
            s.step = step
        case .modeChanged(_, let effective, let untilMs):
            s.mode = effective
            s.trustUntilMs = untilMs
        case .error(let message):
            let k = s.nextKey("err")
            s.items.append(.error(key: k, message: message))
        }
        return s
    }

    static func toolIndex(_ s: ConversationState, _ callId: String) -> Int? {
        s.items.firstIndex { if case .tool(let t) = $0 { return t.call.callId == callId }; return false }
    }

    static func updateTool(_ s: inout ConversationState, _ callId: String, _ change: (inout ToolEntry) -> Void) {
        guard let i = toolIndex(s, callId), case .tool(var t) = s.items[i] else { return }
        change(&t)
        s.items[i] = .tool(t)
    }

    static func upsertAssistant(_ s: inout ConversationState, _ id: String, _ change: (String, String, Bool) -> (String, String, Bool)) {
        if let i = s.items.firstIndex(where: { if case .assistant(let k, _, _, _) = $0 { return k == id }; return false }),
           case .assistant(_, let text, let thinking, let streaming) = s.items[i] {
            let (t, th, st) = change(text, thinking, streaming)
            s.items[i] = .assistant(key: id, text: t, thinking: th, streaming: st)
        } else {
            let (t, th, st) = change("", "", true)
            s.items.append(.assistant(key: id, text: t, thinking: th, streaming: st))
        }
    }

    // MARK: 用户动作（界面先改本地，再交给内核）

    public static func userSent(_ state: ConversationState, text: String) -> ConversationState {
        var s = state
        let k = s.nextKey("u")
        s.items.append(.user(key: k, text: text))
        if s.title.isEmpty { s.title = String(text.prefix(40)) }
        s.running = true
        return s
    }

    public static func decided(_ state: ConversationState, callId: String, decision: ApprovalDecision) -> ConversationState {
        var s = state
        updateTool(&s, callId) { t in
            t.decision = decision
            if case .deny = decision { t.phase = .denied } else { t.phase = .running }
        }
        return s
    }

    public static func answered(_ state: ConversationState, questionId: String, answer: String) -> ConversationState {
        var s = state
        s.items = s.items.map {
            if case .ask(let k, let q, let o, nil) = $0, k == questionId { return .ask(key: k, question: q, options: o, answer: answer) }
            return $0
        }
        return s
    }

    public static func secretProvided(_ state: ConversationState, requestId: String) -> ConversationState {
        var s = state
        s.items = s.items.map {
            if case .secret(let k, let p, let t, false) = $0, k == requestId { return .secret(key: k, prompt: p, targetId: t, provided: true) }
            return $0
        }
        return s
    }

    /// 历史对话（内核本机加密保存的消息）→ 界面条目
    public static func items(fromHistory history: [AgentMessage]) -> [ChatItem] {
        history.enumerated().map { i, m in
            switch m {
            case .user(let text):
                return .user(key: "h-u-\(i)", text: text)
            case .assistant(let id, let text, let thinking):
                return .assistant(key: id.isEmpty ? "h-a-\(i)" : id, text: text, thinking: thinking, streaming: false)
            case .tool(let call, let result):
                let phase: ToolPhase
                if let r = result { phase = r.denied ? .denied : (r.ok ? .done : .failed) } else { phase = .failed }
                return .tool(ToolEntry(call: call, needsApproval: false, phase: phase, output: result?.output ?? "", result: result))
            }
        }
    }
}
