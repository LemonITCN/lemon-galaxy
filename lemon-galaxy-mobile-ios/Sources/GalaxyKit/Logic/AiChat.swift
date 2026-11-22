import Foundation

// AI 聊天界面里与界面无关的逻辑（docs/ai-chat-ui.md）：@ 引用的解析、对话按日期分组、默认模型与档位、引用的构造与去重。

/// 输入框里的 @ 引用：只认文本末尾的 `@过滤词`（@ 在开头或前面是空白，过滤词里没有空白）。
public enum MentionParser {
    /// 正在输入的 @ 过滤词；没有在输入 @ 时为 nil（刚敲下 @ 时是空串）
    public static func query(in text: String) -> String? {
        guard let at = text.lastIndex(of: "@") else { return nil }
        if at > text.startIndex {
            let before = text[text.index(before: at)]
            guard before.isWhitespace else { return nil }
        }
        let q = text[text.index(after: at)...]
        if q.contains(where: { $0.isWhitespace }) { return nil }
        return String(q)
    }

    /// 选中候选后：删掉 @ 及后面的过滤文字
    public static func removeQuery(from text: String) -> String {
        guard query(in: text) != nil, let at = text.lastIndex(of: "@") else { return text }
        return String(text[..<at])
    }

    /// 候选按名称过滤（不分大小写，名称或副标题包含过滤词）；在线的排前面，其余保持原顺序
    public static func filter<T>(_ items: [T], query: String, name: (T) -> String, detail: (T) -> String = { _ in "" }, online: (T) -> Bool) -> [T] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        let hit = q.isEmpty ? items : items.filter { name($0).lowercased().contains(q) || detail($0).lowercased().contains(q) }
        let indexed = hit.enumerated().map { ($0.offset, $0.element) }
        return indexed.sorted { a, b in
            let oa = online(a.1), ob = online(b.1)
            return oa != ob ? oa : a.0 < b.0
        }.map(\.1)
    }
}

/// 对话列表的日期分组：今天 / 昨天 / 最近 7 天 / 更早。
public enum ConversationGroup: String, CaseIterable {
    case today, yesterday, last7Days, earlier

    public var titleKey: String { "ai.group." + rawValue }

    public static func of(_ ms: Int64, now: Int64 = Clock.nowMs, calendar: Calendar = .current) -> ConversationGroup {
        let date = Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
        let today = calendar.startOfDay(for: Date(timeIntervalSince1970: TimeInterval(now) / 1000))
        let day = calendar.startOfDay(for: date)
        let days = calendar.dateComponents([.day], from: day, to: today).day ?? 0
        if days <= 0 { return .today }
        if days == 1 { return .yesterday }
        if days < 7 { return .last7Days }
        return .earlier
    }

    /// 按组排好（组内按时间新的在前），空组不出现
    public static func group<T>(_ items: [T], updatedAt: (T) -> Int64, now: Int64 = Clock.nowMs, calendar: Calendar = .current) -> [(ConversationGroup, [T])] {
        var map: [ConversationGroup: [T]] = [:]
        for it in items.sorted(by: { updatedAt($0) > updatedAt($1) }) {
            map[of(updatedAt(it), now: now, calendar: calendar), default: []].append(it)
        }
        return allCases.compactMap { g in map[g].map { (g, $0) } }
    }
}

/// 模型的本机记忆键：「供应商 ID/模型 ID」
public enum ModelChoice {
    public static func key(_ m: AiModel) -> String { key(providerId: m.providerId, modelId: m.id) }
    public static func key(providerId: UInt64, modelId: String) -> String { "\(providerId)/\(modelId)" }

    /// 默认模型：本机记住的上次选择；没有或已不可用时取列表第一个
    public static func pick(_ models: [AiModel], remembered: String?) -> AiModel? {
        if let r = remembered, let m = models.first(where: { key($0) == r }) { return m }
        return models.first
    }

    /// 按供应商分组（保持服务器给的顺序）
    public static func grouped(_ models: [AiModel]) -> [(provider: String, models: [AiModel])] {
        var order: [String] = []
        var map: [String: [AiModel]] = [:]
        for m in models {
            if map[m.providerName] == nil { order.append(m.providerName) }
            map[m.providerName, default: []].append(m)
        }
        return order.map { ($0, map[$0]!) }
    }
}

public extension AiMode {
    /// 输入框里能选的四档（关闭不出现）
    static let chatModes: [AiMode] = [.readOnly, .confirmAll, .confirmDangerous, .fullTrust]

    /// 新对话的默认档位：本机记住的（默认危险操作确认），不超过管理员上限
    static func initial(saved: AiMode?, maxMode: AiMode) -> AiMode {
        clamp(saved ?? .confirmDangerous, to: maxMode)
    }

    func allowed(maxMode: AiMode) -> Bool { rank <= maxMode.rank && self != .off }
}

/// 页面能不能聊（docs/ai-chat-ui.md 第 5 节）
public enum AiAvailability: Equatable {
    case loading
    case failed(String)
    case disabled
    case noModels
    case ready

    public static func of(models: AiModels?, error: String?, loading: Bool, userAiEnabled: Bool?) -> AiAvailability {
        if userAiEnabled == false { return .disabled }
        if let m = models {
            if !m.enabled { return .disabled }
            return m.items.isEmpty ? .noModels : .ready
        }
        if let e = error, !loading { return .failed(e) }
        return .loading
    }
}

public extension ContextRef {
    static func device(_ id: String, label: String) -> ContextRef { ContextRef(kind: .device, targetId: "", id: id, label: label) }
    static func sshHost(_ id: String, label: String) -> ContextRef { ContextRef(kind: .sshHost, targetId: "", id: id, label: label) }
    static func file(target: String, path: String) -> ContextRef { ContextRef(kind: .file, targetId: target, id: path, label: RemotePath.name(path)) }
    static func folder(target: String, path: String) -> ContextRef {
        let n = RemotePath.name(path)
        return ContextRef(kind: .folder, targetId: target, id: path, label: n.isEmpty ? path : n)
    }
    static func snippet(_ id: String, label: String) -> ContextRef { ContextRef(kind: .snippet, targetId: "", id: id, label: label) }
    static func portForward(_ id: String, label: String) -> ContextRef { ContextRef(kind: .portForward, targetId: "", id: id, label: label) }
    static func activity(label: String) -> ContextRef { ContextRef(kind: .activity, targetId: "", id: "", label: label) }

    /// 同一个引用（种类、所在目标、ID 都相同）只算一次
    var refKey: String { "\(kind)|\(targetId)|\(id)" }

    static func agentTarget(_ t: AgentTarget, label: String) -> ContextRef {
        t.kind == .device ? device(t.id, label: label) : sshHost(t.id, label: label)
    }
}

public extension Array where Element == ContextRef {
    /// 追加引用（已有的不重复加）
    mutating func addRef(_ r: ContextRef) {
        if !contains(where: { $0.refKey == r.refKey }) { append(r) }
    }
}
