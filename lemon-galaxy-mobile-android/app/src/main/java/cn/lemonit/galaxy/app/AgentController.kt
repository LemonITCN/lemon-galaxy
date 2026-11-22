package cn.lemonit.galaxy.app

import cn.lemonit.galaxy.core.bridge.AgentConversationInterface
import cn.lemonit.galaxy.core.bridge.AgentEvent
import cn.lemonit.galaxy.core.bridge.AgentListener
import cn.lemonit.galaxy.core.bridge.AgentMessage
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.AiModels
import cn.lemonit.galaxy.core.bridge.ApprovalDecision
import cn.lemonit.galaxy.core.bridge.ContextRef
import cn.lemonit.galaxy.core.bridge.ConversationConfig
import cn.lemonit.galaxy.core.bridge.GalaxyClientInterface
import cn.lemonit.galaxy.core.bridge.ToolCallView
import cn.lemonit.galaxy.core.bridge.ToolResultView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

enum class ToolPhase { AWAITING, RUNNING, DONE, FAILED, DENIED }

/** 对话页消息流里的一条。 */
sealed class ChatItem {
    abstract val key: String

    data class User(override val key: String, val text: String, val refs: List<ContextRef> = emptyList()) : ChatItem()
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
    val providerId: ULong,
    val modelId: String,
    /** 界面选的档位（输入框里显示的）。 */
    val mode: AiMode,
    val planFirst: Boolean = false,
    val trustMinutes: UInt? = null,
    val items: List<ChatItem> = emptyList(),
    val running: Boolean = false,
    val step: Int = 0,
    val promptTokens: Long = 0,
    val completionTokens: Long = 0,
    /** 与管理员、机主上限取最严后的实际档位（内核 ModeChanged 告知）。 */
    val effectiveMode: AiMode? = null,
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
    /** 运行 / 等用户确认状态变化（对话列表行尾的标记、刷新标题）。 */
    private val onStatus: (id: String, running: Boolean, waiting: Boolean) -> Unit = { _, _, _ -> },
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
        val before = _ui.value
        _ui.update { reduce(it, event) }
        report(before)
    }

    /** 运行或等确认的状态变了才通知。 */
    private fun report(before: ConversationUi) {
        val after = _ui.value
        if (before.running != after.running || before.waitingForUser != after.waitingForUser) {
            onStatus(after.id, after.running, after.waitingForUser)
        }
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
        is AgentEvent.Status -> s.copy(
            running = e.running,
            step = e.step.toInt(),
            // 任务结束时还在流式输出的回复（被停止）不再显示光标
            items = if (e.running) s.items else s.items.map { if (it is ChatItem.Assistant && it.streaming) it.copy(streaming = false) else it },
        )
        is AgentEvent.Error -> s.copy(items = s.items + ChatItem.Error(nextKey("err"), e.message))
        is AgentEvent.ModeChanged -> s.copy(mode = e.mode, effectiveMode = e.effective, trustUntil = e.untilMs)
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

    /** 发送一条消息（带引用）。 */
    fun send(text: String, refs: List<ContextRef> = emptyList()) {
        if (!canSend(text, refs)) return
        val t = text.trim()
        _ui.update {
            it.copy(
                items = it.items + ChatItem.User(nextKey("u"), t, refs),
                title = it.title.ifEmpty { t.split(Regex("\\s+")).filter { w -> w.isNotEmpty() }.joinToString(" ").take(40).ifEmpty { refs.firstOrNull()?.label.orEmpty() } },
                running = true,
            )
        }
        onStatus(_ui.value.id, true, false)
        try {
            conversation.sendWithRefs(t, refs)
        } catch (e: Exception) {
            _ui.update { it.copy(running = false, items = it.items + ChatItem.Error(nextKey("err"), e.userMessage())) }
            onStatus(_ui.value.id, false, false)
        }
    }

    fun decide(callId: String, decision: ApprovalDecision) {
        conversation.decide(callId, decision)
        val before = _ui.value
        _ui.update { s ->
            s.updateTool(callId) {
                it.copy(decision = decision, phase = if (decision is ApprovalDecision.Deny) ToolPhase.DENIED else ToolPhase.RUNNING)
            }
        }
        report(before)
    }

    fun answer(questionId: String, answer: String) {
        conversation.answer(questionId, answer)
        val before = _ui.value
        _ui.update { s -> s.copy(items = s.items.map { if (it is ChatItem.Ask && it.key == questionId) it.copy(answer = answer) else it }) }
        report(before)
    }

    /** 密码直接交给内核，界面不保存。 */
    fun provideSecret(requestId: String, secret: CharArray) {
        conversation.provideSecret(requestId, String(secret))
        secret.fill('\u0000')
        val before = _ui.value
        _ui.update { s -> s.copy(items = s.items.map { if (it is ChatItem.Secret && it.key == requestId) it.copy(provided = true) else it }) }
        report(before)
    }

    fun stop() {
        conversation.stop()
        _ui.update { s -> s.copy(running = false, items = s.items.map { if (it is ChatItem.Assistant && it.streaming) it.copy(streaming = false) else it }) }
        onStatus(_ui.value.id, false, false)
    }

    fun setMode(mode: AiMode, trustMinutes: UInt?) {
        conversation.setMode(mode, trustMinutes)
        _ui.update { it.copy(mode = mode, trustMinutes = trustMinutes) }
    }

    /** 中途换模型（上下文保留）。失败时返回错误文字。 */
    fun setModel(providerId: ULong, modelId: String): String? = try {
        conversation.setModel(providerId, modelId)
        _ui.update { it.copy(providerId = providerId, modelId = modelId) }
        null
    } catch (e: Exception) {
        e.userMessage()
    }

    fun setForeground(foreground: Boolean) {
        conversation.setForeground(foreground)
        if (!foreground && _ui.value.running) {
            _ui.update { it.copy(running = false) }
            onStatus(_ui.value.id, false, false)
        }
    }

    companion object {
        fun itemsFromHistory(history: List<AgentMessage>): List<ChatItem> = history.mapIndexed { i, m ->
            when (m) {
                is AgentMessage.User -> ChatItem.User("h-u-$i", m.text, m.refs)
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

/** 对话列表里的一行：标题（内核取首条消息前 40 字）、时间、是否正在进行任务。 */
data class ConversationListItem(
    val id: String,
    val title: String,
    val updatedAt: Long,
    val running: Boolean = false,
    /** 正在等用户确认、回答或输入密码。 */
    val waiting: Boolean = false,
)

/** 模型列表、对话列表、新建 / 打开 / 删除对话（docs/ai-chat-ui.md 第 1、3、5 节）。 */
class AgentController(
    private val client: GalaxyClientInterface,
    private val scope: CoroutineScope,
    /** 本机记住的档位（新对话与老记录没有配置时用）。 */
    private val defaultMode: () -> AiMode = { AiMode.CONFIRM_DANGEROUS },
) {
    private val _state = MutableStateFlow<AiModelsState>(AiModelsState.Loading)

    /** ai_models() 的结果；进入 AI 页面时调用 [loadModels]。 */
    val state: StateFlow<AiModelsState> = _state.asStateFlow()

    /** 已加载的模型列表（还没加载或失败时为 null）。 */
    val models: StateFlow<AiModels?> = _state.map { (it as? AiModelsState.Loaded)?.models }
        .stateIn(scope, SharingStarted.Eagerly, null)

    private val saved = MutableStateFlow<List<ConversationListItem>>(emptyList())
    private val running = MutableStateFlow<Set<String>>(emptySet())
    private val waiting = MutableStateFlow<Set<String>>(emptySet())

    /** 对话列表（按更新时间倒序），正在进行任务的带 running。 */
    val items: StateFlow<List<ConversationListItem>> = combine(saved, running, waiting) { list, run, wait ->
        list.map { it.copy(running = it.id in run, waiting = it.id in wait) }
    }.stateIn(scope, SharingStarted.Eagerly, emptyList())

    private val opened = java.util.concurrent.ConcurrentHashMap<String, ConversationController>()
    private var loading = false

    /**
     * 调用 ai_models()（进入 AI 页面、点「重试」时）。已经加载过的在后台刷新、不闪加载状态，
     * 刷新失败也保留上次的结果；没加载过或上次失败时先显示加载中。
     */
    fun loadModels() {
        if (loading) return
        loading = true
        val had = _state.value as? AiModelsState.Loaded
        if (had == null) _state.value = AiModelsState.Loading
        scope.launch {
            try {
                _state.value = AiModelsState.Loaded(client.aiModels())
            } catch (e: Exception) {
                if (had == null) _state.value = AiModelsState.Failed(e.userMessage())
            }
            loading = false
            refreshList()
        }
    }

    /** 刷新对话列表（内核要解密本机文件，放到后台线程）。 */
    fun refreshList() {
        scope.launch(Dispatchers.IO) { rebuildList() }
    }

    private fun rebuildList() {
        val fromCore = runCatching { client.listConversations() }.getOrDefault(emptyList()).map { c ->
            val live = opened[c.id]?.ui?.value
            ConversationListItem(c.id, live?.title?.ifEmpty { null } ?: c.title, c.updatedAt)
        }
        val ids = fromCore.map { it.id }.toSet()
        // 刚建还没落盘的对话也列出来
        val pending = opened.values.map { it.ui.value }.filter { it.id !in ids && it.items.isNotEmpty() }
            .map { ConversationListItem(it.id, it.title, System.currentTimeMillis()) }
        saved.value = (pending + fromCore).sortedByDescending { it.updatedAt }
    }

    private fun onStatus(id: String, isRunning: Boolean, isWaiting: Boolean) {
        running.update { if (isRunning) it + id else it - id }
        waiting.update { if (isWaiting) it + id else it - id }
        refreshList()
    }

    private fun listenerFor(holder: () -> ConversationController?) = object : AgentListener {
        override fun onEvent(event: AgentEvent) {
            holder()?.onEvent(event)
        }
    }

    /** 新对话发第一条消息时才创建（第 3 节）。 */
    suspend fun create(config: ConversationConfig): ConversationController {
        var controller: ConversationController? = null
        val conv = client.newConversation(config, listenerFor { controller })
        controller = ConversationController(
            conv,
            ConversationUi(conv.id(), config.title.orEmpty(), config.providerId, config.modelId, config.mode, config.planFirst, config.trustMinutes),
            ::onStatus,
        )
        opened[conv.id()] = controller
        return controller
    }

    fun opened(id: String): ConversationController? = opened[id]

    /** 打开历史对话：open_conversation(id, nil)，用创建时保存的配置；老记录没有配置时用当前默认值拼一个。 */
    suspend fun open(id: String): ConversationController? {
        opened[id]?.let { return it }
        val summary = withContext(Dispatchers.IO) {
            runCatching { client.listConversations() }.getOrDefault(emptyList()).firstOrNull { it.id == id }
        } ?: return null
        var config = summary.config
        if (config == null) {
            val items = models.value?.items.orEmpty()
            val m = items.firstOrNull { it.id == summary.modelId } ?: items.firstOrNull()
            config = ConversationConfig(
                providerId = m?.providerId ?: 0u,
                modelId = m?.id ?: summary.modelId,
                targets = emptyList(),
                mode = capMode(defaultMode(), models.value?.maxMode ?: AiMode.FULL_TRUST),
                planFirst = false,
                trustMinutes = null,
                title = summary.title,
            )
        }
        var controller: ConversationController? = null
        val conv = client.openConversation(id, if (summary.config == null) config else null, listenerFor { controller })
        val model = runCatching { conv.model() }.getOrNull()
        controller = ConversationController(
            conv,
            ConversationUi(
                id, summary.title, model?.providerId ?: config.providerId, model?.modelId ?: config.modelId,
                config.mode, config.planFirst, config.trustMinutes,
            ),
            ::onStatus,
        )
        opened[id] = controller
        return controller
    }

    fun delete(id: String) {
        opened.remove(id)?.stop()
        running.update { it - id }
        waiting.update { it - id }
        runCatching { client.deleteConversation(id) }
        saved.update { list -> list.filter { it.id != id } }
        refreshList()
    }

    /** 切到后台：内核停止正在进行的任务（ai-agent.md 第 1 节）。 */
    fun setForeground(foreground: Boolean) {
        opened.values.forEach { it.setForeground(foreground) }
        if (!foreground) refreshList()
    }
}
