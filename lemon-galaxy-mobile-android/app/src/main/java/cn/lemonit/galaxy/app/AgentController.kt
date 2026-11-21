package cn.lemonit.galaxy.app

import cn.lemonit.galaxy.core.bridge.AgentConversationInterface
import cn.lemonit.galaxy.core.bridge.AgentEvent
import cn.lemonit.galaxy.core.bridge.AgentListener
import cn.lemonit.galaxy.core.bridge.AgentMessage
import cn.lemonit.galaxy.core.bridge.AgentRole
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.AiModels
import cn.lemonit.galaxy.core.bridge.ApprovalDecision
import cn.lemonit.galaxy.core.bridge.ConversationConfig
import cn.lemonit.galaxy.core.bridge.ConversationSummary
import cn.lemonit.galaxy.core.bridge.GalaxyClientInterface
import cn.lemonit.galaxy.core.bridge.GalaxyClientProposedInterface
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
        scope.launch {
            try {
                conversation.send(text)
            } catch (e: Exception) {
                _ui.update { it.copy(running = false, items = it.items + ChatItem.Error(nextKey("err"), e.userMessage())) }
            }
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

    companion object {
        fun itemsFromHistory(history: List<AgentMessage>): List<ChatItem> = history.mapNotNull { m ->
            when (m.role) {
                AgentRole.USER -> ChatItem.User(m.id, m.text)
                AgentRole.ASSISTANT -> ChatItem.Assistant(m.id, m.text, m.thinking, false)
                AgentRole.TOOL -> m.toolCall?.let { c ->
                    val r = m.toolResult
                    ChatItem.Tool(
                        c.callId, c, m.approval != null,
                        when {
                            r == null -> ToolPhase.FAILED
                            r.denied -> ToolPhase.DENIED
                            r.ok -> ToolPhase.DONE
                            else -> ToolPhase.FAILED
                        },
                        r?.output.orEmpty(), r, m.approval,
                    )
                }
                AgentRole.SYSTEM -> null
            }
        }
    }
}

/** 对话列表、模型列表、新建与打开对话。 */
class AgentController(
    private val client: GalaxyClientInterface,
    private val scope: CoroutineScope,
) {
    private val proposed get() = client as? GalaxyClientProposedInterface
    private val _models = MutableStateFlow<AiModels?>(null)
    val models: StateFlow<AiModels?> = _models.asStateFlow()
    private val _summaries = MutableStateFlow<List<ConversationSummary>>(emptyList())
    val summaries: StateFlow<List<ConversationSummary>> = _summaries.asStateFlow()
    private val opened = mutableMapOf<String, ConversationController>()
    private val _modelsError = MutableStateFlow<String?>(null)
    val modelsError: StateFlow<String?> = _modelsError.asStateFlow()

    fun loadModels() {
        scope.launch {
            try {
                _models.value = client.aiModels()
                _modelsError.value = null
            } catch (e: Exception) {
                _modelsError.value = e.userMessage()
            }
        }
    }

    fun refreshList() {
        val p = proposed
        _summaries.value = if (p != null) {
            p.conversations()
        } else {
            opened.values.map { c ->
                val u = c.ui.value
                ConversationSummary(u.id, u.title, u.modelName, u.targetNames, u.mode, System.currentTimeMillis(), u.running, u.waitingForUser)
            }
        }
    }

    fun create(config: ConversationConfig, modelName: String, vision: Boolean, targetNames: List<String>, trustMinutes: UInt?): ConversationController {
        lateinit var controller: ConversationController
        val listener = object : AgentListener {
            override fun onEvent(event: AgentEvent) = controller.onEvent(event)
        }
        val conv = client.newConversation(config, listener)
        controller = ConversationController(
            conv,
            ConversationUi(conv.id(), "", modelName, vision, targetNames, config.mode),
            scope,
        )
        if (config.mode == AiMode.FULL_TRUST && trustMinutes != null) conv.setMode(AiMode.FULL_TRUST, trustMinutes)
        opened[conv.id()] = controller
        refreshList()
        return controller
    }

    fun open(id: String): ConversationController? {
        opened[id]?.let { return it }
        val p = proposed ?: return null
        val summary = p.conversations().firstOrNull { it.id == id } ?: return null
        lateinit var controller: ConversationController
        val listener = object : AgentListener {
            override fun onEvent(event: AgentEvent) = controller.onEvent(event)
        }
        val conv = p.openConversation(id, listener) ?: return null
        val vision = _models.value?.items?.firstOrNull { it.name == summary.modelName }?.vision ?: false
        controller = ConversationController(
            conv,
            ConversationUi(id, summary.title, summary.modelName, vision, summary.targetNames, summary.mode),
            scope,
        )
        opened[id] = controller
        return controller
    }

    fun delete(id: String) {
        opened.remove(id)?.stop()
        proposed?.deleteConversation(id)
        refreshList()
    }

    fun setForeground(foreground: Boolean) {
        proposed?.setForeground(foreground)
    }
}
