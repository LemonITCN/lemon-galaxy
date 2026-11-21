package cn.lemonit.galaxy.core.bridge

/*
 * 界面需要、但 docs/ffi.md 目前没有的接口（建议补进 ffi.md，见报告）。
 *
 * 不和 ffi.md 的对照类型混在一起：界面通过 `client as? GalaxyClientProposedInterface` 取用，
 * 真内核没实现时对应入口自动隐藏或降级（例如两步验证管理、对话历史只保留本次运行）。
 */

/** 两步验证（对应 api.md 的 /auth/totp/setup、verify、disable）。 */
data class TotpSetup(
    val secret: String,
    val otpauthUri: String,
)

/** 本地保存的对话摘要（ai-agent.md：对话记录只存在主控端本地）。 */
data class ConversationSummary(
    val id: String,
    val title: String,
    val modelName: String,
    val targetNames: List<String>,
    val mode: AiMode,
    val updatedAt: Long,
    val running: Boolean,
    val waitingForUser: Boolean,
)

interface GalaxyClientProposedInterface {
    /** 当前账号是否已开启两步验证（UserInfo 里没有这个字段）。 */
    suspend fun totpEnabled(): Boolean
    suspend fun totpSetup(): TotpSetup
    suspend fun totpVerify(code: String)
    suspend fun totpDisable(code: String)

    /** 本地对话列表（按更新时间倒序）。 */
    fun conversations(): List<ConversationSummary>

    /** 重新打开历史对话，接上监听器；找不到返回 null。 */
    fun openConversation(id: String, listener: AgentListener): AgentConversationInterface?
    fun deleteConversation(id: String)

    /** 应用前后台切换：后台时暂停所有 AI 任务（ai-agent.md 第 1 节），回前台继续。 */
    fun setForeground(foreground: Boolean)
}
