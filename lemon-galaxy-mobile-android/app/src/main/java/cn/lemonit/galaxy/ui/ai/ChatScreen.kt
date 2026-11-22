package cn.lemonit.galaxy.ui.ai

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.interaction.collectIsDraggedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.AddComment
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.MoreVert
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.SmallFloatingActionButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.AiAvailability
import cn.lemonit.galaxy.app.CONTINUE_TEXT
import cn.lemonit.galaxy.app.ChatItem
import cn.lemonit.galaxy.app.continueOfferKey
import cn.lemonit.galaxy.app.isSimpleApproval
import cn.lemonit.galaxy.app.latestPlanKeys
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.AiModel
import cn.lemonit.galaxy.core.bridge.ApprovalDecision
import cn.lemonit.galaxy.core.bridge.ImageData
import cn.lemonit.galaxy.ui.components.GalaxyTopBar

interface ChatActions {
    fun onBack()
    fun onNewChat()
    fun onDelete()

    /** 重新调用 ai_models()。 */
    fun onRetry()
    fun onSend()

    /** 不经输入框直接发一条消息（错误行上的「继续」）。 */
    fun onQuickReply(text: String)
    fun onStop()
    fun onModel(model: AiModel)
    fun onMode(mode: AiMode, trustMinutes: UInt?)
    fun onPlanFirst(on: Boolean)
    fun onDecide(callId: String, decision: ApprovalDecision)
    fun onAnswer(questionId: String, answer: String)
    fun onSecret(requestId: String, secret: CharArray)
}

/** 输入框上方要显示的内容（模型、档位、先出计划）。 */
data class ChatSettings(
    val models: List<AiModel>,
    val model: AiModel?,
    val mode: AiMode,
    val maxMode: AiMode,
    val planFirst: Boolean,
)

/**
 * 对话页（docs/ai-chat-ui.md 第 1–5 节，手机布局）：消息流 + 底部输入框。
 * 新对话（还没有消息）时问候语居中、输入框在问候语下方；有消息后输入框回到底部。
 * [conversationId] 为空表示还没发消息的新对话。
 */
@Composable
fun ChatScreen(
    conversationId: String?,
    title: String,
    availability: AiAvailability,
    loadError: String?,
    settings: ChatSettings,
    items: List<ChatItem>,
    running: Boolean,
    composer: ComposerState,
    sources: RefSources,
    actions: ChatActions,
    showBack: Boolean = true,
    initialSheet: ComposerSheet = ComposerSheet.NONE,
) {
    var viewer by remember { mutableStateOf<ImageData?>(null) }
    var denyFor by remember { mutableStateOf<String?>(null) }
    var editFor by remember { mutableStateOf<ChatItem.Tool?>(null) }
    val ready = availability == AiAvailability.READY
    val composerView = @Composable { modifier: Modifier ->
        Composer(
            state = composer,
            enabled = ready,
            running = running,
            models = settings.models,
            model = settings.model,
            mode = settings.mode,
            maxMode = settings.maxMode,
            planFirst = settings.planFirst,
            sources = sources,
            planFirstEditable = conversationId == null,
            onSend = actions::onSend,
            onStop = actions::onStop,
            onModel = actions::onModel,
            onMode = actions::onMode,
            onPlanFirst = actions::onPlanFirst,
            modifier = modifier,
            initialSheet = initialSheet,
        )
    }
    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surface).imePadding()) {
        GalaxyTopBar(
            title = title.ifBlank { stringResource(R.string.ai_new) },
            onBack = if (showBack) actions::onBack else null,
            actions = {
                if (conversationId != null) {
                    IconButton(onClick = actions::onNewChat) { Icon(Icons.Outlined.AddComment, stringResource(R.string.ai_new)) }
                    MoreMenu(actions::onDelete)
                }
            },
        )
        if (items.isEmpty()) {
            // 新对话：问候语居中、输入框在下方；不可用时居中显示原因，输入框置灰放在底部
            if (ready) {
                Box(
                    Modifier.weight(1f).fillMaxWidth().pointerInput(Unit) { detectTapGestures { composer.dismissMention() } }.padding(horizontal = 16.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Column(Modifier.widthIn(max = 720.dp).fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
                        Text(stringResource(R.string.ai_greeting), style = MaterialTheme.typography.headlineSmall, textAlign = TextAlign.Center)
                        Spacer(Modifier.height(24.dp))
                        composerView(Modifier)
                    }
                }
            } else {
                Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                    AvailabilityState(availability, loadError, actions::onRetry)
                }
                BottomComposer { composerView(it) }
            }
        } else {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                MessageList(
                    items, running, sources, composer,
                    onImage = { viewer = it },
                    onDecide = actions::onDecide,
                    onDeny = { denyFor = it },
                    onEdit = { editFor = it },
                    onAnswer = actions::onAnswer,
                    onSecret = actions::onSecret,
                    onQuickReply = actions::onQuickReply,
                )
            }
            if (!ready) AvailabilityLine(availability, loadError, actions::onRetry)
            BottomComposer { composerView(it) }
        }
    }
    viewer?.let { ImageViewer(it) { viewer = null } }
    denyFor?.let { id -> DenyDialog({ reason -> actions.onDecide(id, ApprovalDecision.Deny(reason.ifBlank { null })) }) { denyFor = null } }
    editFor?.let { t -> EditCommandDialog(t.call.detail.ifBlank { t.call.summary }, { cmd -> actions.onDecide(t.key, ApprovalDecision.EditAndRun(cmd)) }) { editFor = null } }
}

@Composable
private fun BottomComposer(content: @Composable (Modifier) -> Unit) {
    Box(Modifier.fillMaxWidth().navigationBarsPadding().padding(start = 12.dp, end = 12.dp, top = 4.dp, bottom = 10.dp), contentAlignment = Alignment.Center) {
        content(Modifier.widthIn(max = 760.dp))
    }
}

@Composable
private fun MessageList(
    items: List<ChatItem>,
    running: Boolean,
    sources: RefSources,
    composer: ComposerState,
    onImage: (ImageData) -> Unit,
    onDecide: (String, ApprovalDecision) -> Unit,
    onDeny: (String) -> Unit,
    onEdit: (ChatItem.Tool) -> Unit,
    onAnswer: (String, String) -> Unit,
    onSecret: (String, CharArray) -> Unit,
    onQuickReply: (String) -> Unit,
) {
    val listState = rememberLazyListState()
    val latestPlans = remember(items) { latestPlanKeys(items) }
    val continueKey = continueOfferKey(items, running)
    // 用户往上翻看时不自动滚动；回到底部后恢复跟随
    var follow by remember { mutableStateOf(true) }
    val dragged by listState.interactionSource.collectIsDraggedAsState()
    LaunchedEffect(dragged) { if (dragged && listState.canScrollForward) follow = false }
    LaunchedEffect(listState) {
        snapshotFlow { listState.canScrollForward }.collect { canForward -> if (!canForward) follow = true else if (dragged) follow = false }
    }
    val pending = running && items.lastOrNull() is ChatItem.User
    val total = items.size + (if (pending) 1 else 0) + 1
    val last = items.lastOrNull()
    val contentVersion = Triple(items.size, (last as? ChatItem.Assistant)?.text?.length ?: 0, (last as? ChatItem.Tool)?.let { it.output.length + it.phase.ordinal } ?: 0)
    LaunchedEffect(contentVersion, pending) {
        if (follow) listState.scrollToItem(total - 1)
    }
    val showJump by remember { derivedStateOf { !follow && listState.canScrollForward } }
    Box(Modifier.fillMaxSize().pointerInput(Unit) { detectTapGestures { composer.dismissMention() } }) {
        LazyColumn(
            state = listState,
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(horizontal = 16.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            items(items, key = { it.key }) { item ->
                Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
                    Box(Modifier.widthIn(max = 760.dp).fillMaxWidth()) {
                        when (item) {
                            is ChatItem.User -> UserMessage(item, sources::icon, sources::detail)
                            is ChatItem.Assistant -> AssistantMessage(item)
                            is ChatItem.Tool -> ToolEntry(
                                item,
                                latestPlan = item.key in latestPlans,
                                onImage = onImage,
                                onAllowOnce = { onDecide(item.key, ApprovalDecision.AllowOnce) },
                                onAllowSimilar = { onDecide(item.key, ApprovalDecision.AllowSimilar) },
                                // use_target / use_app 的拒绝不问理由
                                onDeny = { if (isSimpleApproval(item.call.tool)) onDecide(item.key, ApprovalDecision.Deny(null)) else onDeny(item.key) },
                                onEdit = { onEdit(item) },
                            )
                            is ChatItem.Ask -> AskCard(item) { onAnswer(item.key, it) }
                            is ChatItem.Secret -> SecretCard(item) { onSecret(item.key, it) }
                            is ChatItem.Error -> ErrorLine(item.message, if (item.key == continueKey) ({ onQuickReply(CONTINUE_TEXT) }) else null)
                        }
                    }
                }
            }
            if (pending) {
                item("pending") {
                    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
                        Box(Modifier.widthIn(max = 760.dp).fillMaxWidth()) { TypingIndicator() }
                    }
                }
            }
            item("end") { Spacer(Modifier.height(4.dp)) }
        }
        if (showJump) {
            SmallFloatingActionButton(
                onClick = {
                    follow = true
                },
                shape = CircleShape,
                containerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                modifier = Modifier.align(Alignment.BottomEnd).padding(end = 16.dp, bottom = 12.dp),
            ) { Icon(Icons.Rounded.KeyboardArrowDown, stringResource(R.string.ai_jump_bottom)) }
        }
    }
    LaunchedEffect(follow) { if (follow) listState.scrollToItem(total - 1) }
}

@Composable
private fun MoreMenu(onDelete: () -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) { Icon(Icons.Rounded.MoreVert, stringResource(R.string.more)) }
        DropdownMenu(open, { open = false }) {
            DropdownMenuItem(
                text = { Text(stringResource(R.string.ai_delete), color = MaterialTheme.colorScheme.error) },
                leadingIcon = { Icon(Icons.Rounded.Delete, null, tint = MaterialTheme.colorScheme.error) },
                onClick = {
                    open = false
                    onDelete()
                },
            )
        }
    }
}

/** 不可用时消息流位置的提示（第 5 节）。 */
@Composable
fun AvailabilityState(availability: AiAvailability, loadError: String?, onRetry: () -> Unit) {
    Column(Modifier.widthIn(max = 420.dp).padding(horizontal = 32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        when (availability) {
            AiAvailability.READY -> Unit
            AiAvailability.LOADING -> CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
            else -> {
                Text(
                    availabilityText(availability, loadError),
                    style = MaterialTheme.typography.bodyLarge,
                    color = if (availability == AiAvailability.FAILED) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
                if (availability != AiAvailability.DISABLED) {
                    Spacer(Modifier.height(16.dp))
                    OutlinedButton(onClick = onRetry) { Text(stringResource(R.string.retry)) }
                }
            }
        }
    }
}

/** 已有消息的历史对话：输入框上方一行紧凑提示。 */
@Composable
private fun AvailabilityLine(availability: AiAvailability, loadError: String?, onRetry: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (availability == AiAvailability.LOADING) {
            CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 1.5.dp)
            return@Row
        }
        Icon(Icons.Rounded.ErrorOutline, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(16.dp))
        Spacer(Modifier.width(6.dp))
        Text(
            availabilityText(availability, loadError),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 2,
            modifier = Modifier.weight(1f),
        )
        if (availability != AiAvailability.DISABLED) TextButton(onClick = onRetry) { Text(stringResource(R.string.retry)) }
    }
}

@Composable
private fun availabilityText(a: AiAvailability, loadError: String?): String = when (a) {
    AiAvailability.DISABLED -> stringResource(R.string.ai_disabled)
    AiAvailability.NO_MODELS -> stringResource(R.string.ai_no_models)
    AiAvailability.FAILED -> loadError?.takeIf { it.isNotBlank() } ?: stringResource(R.string.ai_models_failed)
    else -> ""
}
