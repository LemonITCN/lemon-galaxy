// 建议内核补充的接口（docs/ffi.md 目前没有）。
// 界面用 `client as? GalaxyClientProposed` 判断：MockCore 实现了这些方法，真内核没实现前对应界面自动置灰。
// 这些不属于 uniffi 生成代码，真内核接入后本文件保留；内核加上同名方法后，只需给生成的类补一个空的协议遵循扩展。
import Foundation

/// 两步验证自助管理（security.md 第 3 节：普通用户可自行开启）。
public struct TotpSetup: Equatable, Hashable {
    public var secret: String
    public var otpauthUri: String
    public init(secret: String, otpauthUri: String) { self.secret = secret; self.otpauthUri = otpauthUri }
}

/// 本地保存的 AI 对话摘要（ai-agent.md：对话记录只存在主控端本地，界面需要列出历史对话）。
public struct ConversationSummary: Equatable, Hashable {
    public var id: String
    public var title: String
    public var modelName: String
    public var targets: [AgentTarget]
    public var updatedAt: Int64
    public init(id: String, title: String, modelName: String, targets: [AgentTarget], updatedAt: Int64) {
        self.id = id; self.title = title; self.modelName = modelName; self.targets = targets; self.updatedAt = updatedAt
    }
}

public protocol GalaxyClientProposed: AnyObject {
    func totpEnabled() -> Bool
    func totpBeginSetup() async throws -> TotpSetup
    func totpConfirm(code: String) async throws
    func totpDisable(code: String) async throws
    /// 历史对话（本地存储），以及按 id 重新打开继续对话
    func conversations() -> [ConversationSummary]
    func openConversation(id: String, listener: AgentListener) throws -> AgentConversation
    func deleteConversation(id: String)
}
