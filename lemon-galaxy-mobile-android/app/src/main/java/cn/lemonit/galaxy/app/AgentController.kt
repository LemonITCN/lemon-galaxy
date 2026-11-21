package cn.lemonit.galaxy.app

import cn.lemonit.galaxy.core.bridge.AgentConversationInterface
import cn.lemonit.galaxy.core.bridge.AgentEvent
import cn.lemonit.galaxy.core.bridge.AgentListener
import cn.lemonit.galaxy.core.bridge.AgentMessage
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.AiModels
import cn.lemonit.galaxy.core.bridge.ApprovalDecision
import cn.lemonit.galaxy.core.bridge.ConversationConfig
import cn.lemonit.galaxy.core.bridge.ConversationSummary
import cn.lemonit.galaxy.core.bridge.GalaxyClientInterface
import cn.lemonit.galaxy.core.bridge.AgentTarget
import cn.lemonit.galaxy.core.bridge.ToolCallView
import cn.lemonit.galaxy.core.bridge.ToolResultView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

enum class ToolPhase { AWAITING, RUNNING, DONE, FAILED, DENIED }

/** 对话页里的一条。 */
sealed class ChatItem {
    abstract val key: String

    data class User(override val key: String, val text: String) : ChatItem()
    data class Assistant(override val key: String, val text: String, val thinking: String?, val streaming: Boolean) : ChatItem()
    data class Tool(
        override val key: String,
        val call: ToolCallView,
        val needsApproval: Boolean,
        val phase: ToolPhase,
        val output: String,
        val result: ToolResultView?,
        val decision: ApprovalDecision?,
    ) : ChatItem()

    data class Ask(override val key: String, val question: String, val options: List<String>, val answer: String?) : ChatItem()
    data class Secret(override val key: String, val prompt: String, val targetId: String, val provided: Boolean) : ChatItem()
    data class Error(override val key: String, val message: String) : ChatItem()
}

data class ConversationUi(
    val id: String,
    val title: String,
    val modelName: String,
    val vision: Boolean,
    val targetNames: List<String>,
    val mode: AiMode,
    val items: List<ChatItem> = emptyList(),
    val running: Boolean = false,
    val step: Int = 0,
    val promptTokens: Long = 0,
    val completionTokens: Long = 0,
    /** 完全信任到期时间（毫秒），不限时为空。 */
    val trustUntil: Long? = null,
) {
    val waitingForUser: Boolean
        get() = items.any {
            (it is ChatItem.Tool && it.phase == ToolPhase.AWAITING) ||
                (it is ChatItem.Ask && it.answer == null) ||
                (it is ChatItem.Secret && !it.provided)
        }
}

/** 一段对话：把 AgentEvent 流归并成界面条目。事件来自内核线程，用原子 update 归并。 */
class ConversationController(
    val conversation: AgentConversationInterface,
    initial: ConversationUi,
    private val scope: CoroutineScope,
) : AgentListener {
    private val _ui = MutableStateFlow(initial)
    val ui: StateFlow<ConversationUi> = _ui.asStateFlow()
    private var seq = 0L

    init {
        val history = runCatching { conversation.messages() }.getOrDefault(emptyList())
        if (history.isNotEmpty()) _ui.update { it.copy(items = itemsFromHistory(history)) }
    }

    private fun nextKey(prefix: String) = "$prefix-${seq++}"

    override fun onEvent(event: AgentEvent) {
        _ui.update { reduce(it, event) }
    }

    private fun reduce(s: ConversationUi, e: AgentEvent): ConversationUi = when (e) {
        is AgentEvent.AssistantDelta -> s.upsertAssistant(e.messageId) { it.copy(text = it.text + e.text, streaming = true) }
        is AgentEvent.AssistantDone -> s.upsertAssistant(e.messageId) { it.copy(streaming = false) }
        is AgentEvent.Thinking -> s.upsertAssistant(e.messageId) { it.copy(thinking = (it.thinking ?: "") + e.text) }
        is AgentEvent.ToolProposed -> s.copy(
            items = s.items + ChatItem.Tool(
                e.call.callId, e.call, e.needsApproval,
                if (e.needsApproval) ToolPhase.AWAITING else ToolPhase.RUNNING, "", null, null,
            ),
        )
        is AgentEvent.ToolStarted -> s.updateTool(e.callId) { if (it.phase == ToolPhase.AWAITING) it.copy(phase = ToolPhase.RUNNING) else it }
        is AgentEvent.ToolOutput -> s.updateTool(e.callId) { it.copy(output = it.output + e.text) }
        is AgentEvent.ToolFinished -> s.updateTool(e.callId) {
            it.copy(
                result = e.result,
                phase = when {
                    e.result.denied -> ToolPhase.DENIED
                    e.result.ok -> ToolPhase.DONE
                    else -> ToolPhase.FAILED
                },
                output = it.output.ifEmpty { e.result.output },
            )
        }
        is AgentEvent.AskUser -> s.copy(items = s.items + ChatItem.Ask(e.questionId, e.question, e.options, null))
        is AgentEvent.SecretRequested -> s.copy(items = s.items + ChatItem.Secret(e.requestId, e.prompt, e.targetId, false))
        is AgentEvent.Usage -> s.copy(promptTokens = e.promptTokens.toLong(), completionTokens = e.completionTokens.toLong())
        is AgentEvent.Status -> s.copy(running = e.running, step = e.step.toInt())
        is AgentEvent.Error -> s.copy(items = s.items + ChatItem.Error(nextKey("err"), e.message))
        is AgentEvent.ModeChanged -> s.copy(mode = e.effective, trustUntil = e.untilMs)
    }

    private fun ConversationUi.upsertAssistant(id: String, block: (ChatItem.Assistant) -> ChatItem.Assistant): ConversationUi {
        val idx = items.indexOfFirst { it is ChatItem.Assistant && it.key == id }
        return if (idx >= 0) {
            copy(items = items.toMutableList().also { it[idx] = block(it[idx] as ChatItem.Assistant) })
        } else {
            copy(items = items + block(ChatItem.Assistant(id, "", null, true)))
        }
    }

    private fun ConversationUi.updateTool(id: String, block: (ChatItem.Tool) -> ChatItem.Tool): ConversationUi =
        copy(items = items.map { if (it is ChatItem.Tool && it.key == id) block(it) else it })

    fun send(text: String) {
        if (text.isBlank()) return
        _ui.update { it.copy(items = it.items + ChatItem.User(nextKey("u"), text), title = it.title.ifEmpty { text.take(40) }, running = true) }
        try {
            conversation.send(text)
        } catch (e: Exception) {
            _ui.update { it.copy(running = false, items = it.items + ChatItem.Error(nextKey("err"), e.userMessage())) }
        }
    }

    fun decide(callId: String, decision: ApprovalDecision) {
        conversation.decide(callId, decision)
        _ui.update { s ->
            s.updateTool(callId) {
                it.copy(decision = decision, phase = if (decision is ApprovalDecision.Deny) ToolPhase.DENIED else ToolPhase.RUNNING)
            }
        }
    }

    fun answer(questionId: String, answer: String) {
        conversation.answer(questionId, answer)
        _ui.update { s -> s.copy(items = s.items.map { if (it is ChatItem.Ask && it.key == questionId) it.copy(answer = answer) else it }) }
    }

    /** 密码直接交给内核，界面不保存。 */
    fun provideSecret(requestId: String, secret: CharArray) {
        conversation.provideSecret(requestId, String(secret))
        secret.fill('\u0000')
        _ui.update { s -> s.copy(items = s.items.map { if (it is ChatItem.Secret && it.key == requestId) it.copy(provided = true) else it }) }
    }

    fun stop() {
        conversation.stop()
        _ui.update { it.copy(running = false) }
    }

    fun setMode(mode: AiMode, trustMinutes: UInt?) {
        conversation.setMode(mode, trustMinutes)
        _ui.update { it.copy(mode = mode) }
    }

    fun setForeground(foreground: Boolean) {
        conversation.setForeground(foreground)
        if (!foreground) _ui.update { it.copy(running = false) }
    }

    companion object {
        fun itemsFromHistory(history: List<AgentMessage>): List<ChatItem> = history.mapIndexed { i, m ->
            when (m) {
                is AgentMessage.User -> ChatItem.User("h-u-$i", m.text)
                is AgentMessage.Assistant -> ChatItem.Assistant(m.id.ifEmpty { "h-a-$i" }, m.text, m.thinking.ifEmpty { null }, false)
                is AgentMessage.Tool -> {
                    val r = m.result
                    ChatItem.Tool(
                        m.call.callId.ifEmpty { "h-t-$i" }, m.call, false,
                        when {
                            r == null -> ToolPhase.FAILED
                            r.denied -> ToolPhase.DENIED
                            r.ok -> ToolPhase.DONE
                            else -> ToolPhase.FAILED
                        },
                        r?.output.orEmpty(), r, null,
                    )
                }
            }
        }
    }
}

/** 对话列表里的一行（内核的摘要 + 本机记下的档位 + 打开中的运行状态）。 */
data class ConversationListItem(
    val id: String,
    val title: String,
    val modelName: String,
    val targetNames: List<String>,
    val mode: AiMode,
    val updatedAt: Long,
    val running: Boolean,
    val waitingForUser: Boolean,
)

/**
 * 每个对话创建时的配置（模型、目标、档位）记在本机：内核的 open_conversation 需要界面再给一次配置，
 * 而对话摘要里只有模型 id 与目标名称。
 */
interface ConversationConfigStore {
    fun get(id: String): ConversationConfig?
    fun put(id: String, config: ConversationConfig)
    fun remove(id: String)
}

class MemoryConversationConfigStore : ConversationConfigStore {
    private val map = java.util.concurrent.ConcurrentHashMap<String, ConversationConfig>()
    override fun get(id: String) = map[id]
    override fun put(id: String, config: ConversationConfig) {
        map[id] = config
    }

    override fun remove(id: String) {
        map.remove(id)
    }
}

/** 对话列表、模型列表、新建与打开对话。 */
class AgentController(
    private val client: GalaxyClientInterface,
    private val scope: CoroutineScope,
    private val configs: ConversationConfigStore = MemoryConversationConfigStore(),
    /** 目标名称 → 目标（重新打开没有本机配置的历史对话时按名称找回）。 */
    private val resolveTargets: (List<String>) -> List<AgentTarget> = { emptyList() },
    private val defaultMode: () -> AiMode = { AiMode.CONFIRM_DANGEROUS },
) {
    private val _models = MutableStateFlow<AiModels?>(null)
    val models: StateFlow<AiModels?> = _models.asStateFlow()
    private val _items = MutableStateFlow<List<ConversationListItem>>(emptyList())
    val items: StateFlow<List<ConversationListItem>> = _items.asStateFlow()
    private val opened = java.util.concurrent.ConcurrentHashMap<String, ConversationController>()
    private val _modelsError = MutableStateFlow<String?>(null)
    val modelsError: StateFlow<String?> = _modelsError.asStateFlow()

    fun loadModels() {
        scope.launch {
            try {
                _models.value = client.aiModels()
                _modelsError.value = null
                refreshList()
            } catch (e: Exception) {
                _modelsError.value = e.userMessage()
            }
        }
    }

    private fun modelName(id: String) = _models.value?.items?.firstOrNull { it.id == id }?.name ?: id

    /** 刷新对话列表（内核要解密本机文件，放到后台线程）。 */
    fun refreshList() {
        scope.launch(kotlinx.coroutines.Dispatchers.IO) { rebuildList() }
    }

    private fun rebuildList() {
        val saved = runCatching { client.listConversations() }.getOrDefault(emptyList())
        val ids = saved.map { it.id }.toSet()
        val fromCore = saved.map { c ->
            val live = opened[c.id]?.ui?.value
            ConversationListItem(
                c.id, live?.title?.ifEmpty { null } ?: c.title, modelName(c.modelId), c.targetNames,
                live?.mode ?: configs.get(c.id)?.mode ?: defaultMode(), c.updatedAt,
                live?.running ?: false, live?.waitingForUser ?: false,
            )
        }
        // 刚建还没落盘的对话也列出来
        val pending = opened.values.map { it.ui.value }.filter { it.id !in ids }.map { u ->
            ConversationListItem(u.id, u.title, u.modelName, u.targetNames, u.mode, System.currentTimeMillis(), u.running, u.waitingForUser)
        }
        _items.value = (pending + fromCore).sortedByDescending { it.updatedAt }
    }

    private fun listenerFor(holder: () -> ConversationController?) = object : AgentListener {
        override fun onEvent(event: AgentEvent) {
            holder()?.onEvent(event)
        }
    }

    suspend fun create(config: ConversationConfig, modelName: String, vision: Boolean, targetNames: List<String>): ConversationController {
        var controller: ConversationController? = null
        val conv = client.newConversation(config, listenerFor { controller })
        controller = ConversationController(
            conv,
            ConversationUi(conv.id(), config.title.orEmpty(), modelName, vision, targetNames, config.mode),
            scope,
        )
        configs.put(conv.id(), config)
        opened[conv.id()] = controller
        refreshList()
        return controller
    }

    fun opened(id: String): ConversationController? = opened[id]

    /** 打开历史对话：配置优先用本机记下的，没有就按摘要里的模型与目标名称拼一个。 */
    suspend fun open(id: String): ConversationController? {
        opened[id]?.let { return it }
        val summary = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
            runCatching { client.listConversations() }.getOrDefault(emptyList()).firstOrNull { it.id == id }
        } ?: return null
        val models = _models.value ?: runCatching { client.aiModels() }.getOrNull()?.also { _models.value = it }
        val model = models?.items?.firstOrNull { it.id == summary.modelId }
        val config = configs.get(id) ?: ConversationConfig(
            providerId = model?.providerId ?: 0u,
            modelId = summary.modelId,
            targets = resolveTargets(summary.targetNames),
            mode = defaultMode(),
            planFirst = false,
            trustMinutes = null,
            title = summary.title,
        )
        var controller: ConversationController? = null
        val conv = client.openConversation(id, config, listenerFor { controller })
        controller = ConversationController(
            conv,
            ConversationUi(id, summary.title, model?.name ?: summary.modelId, model?.vision ?: false, summary.targetNames, config.mode),
            scope,
        )
        opened[id] = controller
        return controller
    }

    fun delete(id: String) {
        opened.remove(id)?.stop()
        runCatching { client.deleteConversation(id) }
        configs.remove(id)
        refreshList()
    }

    /** 切到后台：内核停止正在进行的任务（ai-agent.md 第 1 节）。 */
    fun setForeground(foreground: Boolean) {
        opened.values.forEach { it.setForeground(foreground) }
        if (!foreground) refreshList()
    }
}
