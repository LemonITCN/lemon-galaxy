package cn.lemonit.galaxy.ui.ai

import android.graphics.BitmapFactory
import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Send
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material.icons.rounded.AdsClick
import androidx.compose.material.icons.rounded.Block
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.ExpandLess
import androidx.compose.material.icons.rounded.ExpandMore
import androidx.compose.material.icons.rounded.FolderOpen
import androidx.compose.material.icons.automirrored.rounded.HelpOutline
import androidx.compose.material.icons.rounded.Keyboard
import androidx.compose.material.icons.rounded.Lightbulb
import androidx.compose.material.icons.rounded.MoreVert
import androidx.compose.material.icons.rounded.Screenshot
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Stop
import androidx.compose.material.icons.rounded.Terminal
import androidx.compose.material.icons.rounded.Timer
import androidx.compose.material.icons.rounded.AccountTree
import androidx.compose.material.icons.rounded.SwapHoriz
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.ChatItem
import cn.lemonit.galaxy.app.ConversationUi
import cn.lemonit.galaxy.app.ToolPhase
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.ApprovalDecision
import cn.lemonit.galaxy.core.bridge.ImageData
import cn.lemonit.galaxy.core.bridge.RiskLevel
import cn.lemonit.galaxy.ui.components.GalaxyTopBar
import cn.lemonit.galaxy.ui.components.InfoButton
import cn.lemonit.galaxy.ui.components.Pill
import cn.lemonit.galaxy.ui.components.formatCount
import cn.lemonit.galaxy.ui.theme.Galaxy

interface ConversationActions {
    fun onBack()
    fun onSend(text: String)
    fun onStop()
    fun onDecide(callId: String, decision: ApprovalDecision)
    fun onAnswer(questionId: String, answer: String)
    fun onSecret(requestId: String, secret: CharArray)
    fun onMode(mode: AiMode, trustMinutes: UInt?)
    fun onDelete()
}

@Composable
fun riskColor(r: RiskLevel): Color = when (r) {
    RiskLevel.READ -> Galaxy.colors.riskRead
    RiskLevel.NORMAL -> Galaxy.colors.riskNormal
    RiskLevel.DANGEROUS -> Galaxy.colors.riskDangerous
    RiskLevel.UNKNOWN -> Galaxy.colors.riskUnknown
    RiskLevel.FORBIDDEN -> Galaxy.colors.riskForbidden
}

fun riskLabel(r: RiskLevel): Int = when (r) {
    RiskLevel.READ -> R.string.risk_read
    RiskLevel.NORMAL -> R.string.risk_normal
    RiskLevel.DANGEROUS -> R.string.risk_dangerous
    RiskLevel.UNKNOWN -> R.string.risk_unknown
    RiskLevel.FORBIDDEN -> R.string.risk_forbidden
}

private fun toolInfo(tool: String): Pair<Int?, ImageVector> = when (tool) {
    "run_command" -> R.string.tool_run_command to Icons.Rounded.Terminal
    "terminal_open", "terminal_send", "terminal_read", "terminal_close" -> R.string.tool_terminal to Icons.Rounded.Terminal
    "read_file" -> R.string.tool_read_file to Icons.Rounded.Description
    "write_file" -> R.string.tool_write_file to Icons.Rounded.Description
    "edit_file" -> R.string.tool_edit_file to Icons.Rounded.Description
    "list_dir" -> R.string.tool_list_dir to Icons.Rounded.FolderOpen
    "search_files" -> R.string.tool_search_files to Icons.Rounded.Search
    "upload_file", "download_file" -> R.string.tool_transfer to Icons.Rounded.SwapHoriz
    "screenshot" -> R.string.tool_screenshot to Icons.Rounded.Screenshot
    "ui_tree" -> R.string.tool_ui_tree to Icons.Rounded.AccountTree
    "mouse" -> R.string.tool_mouse to Icons.Rounded.AdsClick
    "keyboard" -> R.string.tool_keyboard to Icons.Rounded.Keyboard
    "wait" -> R.string.tool_wait to Icons.Rounded.Timer
    else -> null to Icons.AutoMirrored.Rounded.HelpOutline
}

@Composable
private fun rememberBitmap(img: ImageData?): ImageBitmap? = remember(img) {
    img?.let { runCatching { BitmapFactory.decodeByteArray(it.data, 0, it.data.size)?.asImageBitmap() }.getOrNull() }
}

@Composable
fun ConversationScreen(
    ui: ConversationUi,
    actions: ConversationActions,
    maxMode: AiMode = AiMode.FULL_TRUST,
    showBack: Boolean = true,
    initialInput: String = "",
) {
    var input by remember { mutableStateOf(initialInput) }
    var viewer by remember { mutableStateOf<ImageData?>(null) }
    var denyFor by remember { mutableStateOf<String?>(null) }
    var editFor by remember { mutableStateOf<ChatItem.Tool?>(null) }
    var secretFor by remember { mutableStateOf<ChatItem.Secret?>(null) }
    val listState = rememberLazyListState()
    LaunchedEffect(ui.items.size, (ui.items.lastOrNull() as? ChatItem.Assistant)?.text?.length) {
        if (ui.items.isNotEmpty()) listState.scrollToItem(ui.items.size)
    }
    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surface).imePadding()) {
        GalaxyTopBar(
            title = ui.title.ifEmpty { stringResource(R.string.ai_new) },
            onBack = if (showBack) actions::onBack else null,
            subtitle = {
                Text(
                    ui.modelName + "  ·  " + ui.targetNames.joinToString(", "),
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            },
            actions = {
                ModeMenu(ui.mode, maxMode, actions)
                MoreMenu(actions)
            },
        )
        StatusStrip(ui, actions)
        LazyColumn(
            state = listState,
            modifier = Modifier.weight(1f).fillMaxWidth(),
            contentPadding = PaddingValues(horizontal = 12.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            items(ui.items, key = { it.key }) { item ->
                Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
                    Box(Modifier.widthIn(max = 760.dp).fillMaxWidth()) {
                        when (item) {
                            is ChatItem.User -> UserBubble(item)
                            is ChatItem.Assistant -> AssistantBlock(item)
                            is ChatItem.Tool -> ToolCard(
                                item,
                                onImage = { viewer = it },
                                onAllowOnce = { actions.onDecide(item.key, ApprovalDecision.AllowOnce) },
                                onAllowSimilar = { actions.onDecide(item.key, ApprovalDecision.AllowSimilar) },
                                onDeny = { denyFor = item.key },
                                onEdit = { editFor = item },
                            )
                            is ChatItem.Ask -> AskCard(item) { actions.onAnswer(item.key, it) }
                            is ChatItem.Secret -> SecretCard(item) { secretFor = item }
                            is ChatItem.Error -> ErrorCard(item.message)
                        }
                    }
                }
            }
            item("end") { Spacer(Modifier.height(4.dp)) }
        }
        InputBar(input, { input = it }, ui.running, onSend = {
            actions.onSend(input.trim())
            input = ""
        }, onStop = actions::onStop)
    }
    viewer?.let { ImageViewer(it) { viewer = null } }
    denyFor?.let { id -> DenyDialog({ reason -> actions.onDecide(id, ApprovalDecision.Deny(reason.ifBlank { null })) }) { denyFor = null } }
    editFor?.let { t -> EditCommandDialog(t.call.summary, { cmd -> actions.onDecide(t.key, ApprovalDecision.EditAndRun(cmd)) }) { editFor = null } }
    secretFor?.let { s -> SecretDialog(s.prompt, { actions.onSecret(s.key, it) }) { secretFor = null } }
}

@Composable
private fun StatusStrip(ui: ConversationUi, actions: ConversationActions) {
    Surface(color = MaterialTheme.colorScheme.surfaceContainer) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            when {
                ui.waitingForUser -> Pill(stringResource(R.string.ai_waiting), Galaxy.colors.riskUnknown)
                ui.running -> {
                    CircularProgressIndicator(Modifier.size(12.dp), strokeWidth = 1.5.dp)
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.ai_running), style = MaterialTheme.typography.labelMedium)
                }
                else -> Text(stringResource(R.string.ai_idle), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (ui.step > 0) {
                Text("  ·  " + stringResource(R.string.ai_step, ui.step), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Spacer(Modifier.weight(1f))
            Text(
                stringResource(R.string.ai_tokens, formatCount(ui.promptTokens), formatCount(ui.completionTokens)),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun ModeMenu(mode: AiMode, maxMode: AiMode, actions: ConversationActions) {
    var open by remember { mutableStateOf(false) }
    Box {
        TextButton(onClick = { open = true }) {
            Box(Modifier.size(8.dp).clip(RoundedCornerShape(4.dp)).background(modeColor(mode)))
            Spacer(Modifier.width(6.dp))
            Text(
                stringResource(modeLabel(mode)),
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.widthIn(max = 104.dp),
            )
        }
        DropdownMenu(open, { open = false }) {
            SELECTABLE_MODES.filter { it.ordinal <= maxMode.ordinal }.forEach { m ->
                if (m == AiMode.FULL_TRUST) {
                    listOf<Pair<UInt?, Int>>(15u to R.string.trust_15, 60u to R.string.trust_60, null to R.string.trust_unlimited).forEach { (mins, l) ->
                        DropdownMenuItem(
                            text = { Text(stringResource(modeLabel(m)) + " · " + stringResource(l)) },
                            leadingIcon = { Box(Modifier.size(8.dp).clip(RoundedCornerShape(4.dp)).background(modeColor(m))) },
                            onClick = {
                                open = false
                                actions.onMode(m, mins)
                            },
                        )
                    }
                } else {
                    DropdownMenuItem(
                        text = { Text(stringResource(modeLabel(m))) },
                        leadingIcon = { Box(Modifier.size(8.dp).clip(RoundedCornerShape(4.dp)).background(modeColor(m))) },
                        onClick = {
                            open = false
                            actions.onMode(m, null)
                        },
                    )
                }
            }
        }
    }
}

@Composable
private fun MoreMenu(actions: ConversationActions) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) { Icon(Icons.Rounded.MoreVert, stringResource(R.string.more)) }
        DropdownMenu(open, { open = false }) {
            DropdownMenuItem(
                text = { Text(stringResource(R.string.ai_delete), color = MaterialTheme.colorScheme.error) },
                leadingIcon = { Icon(Icons.Rounded.Delete, null, tint = MaterialTheme.colorScheme.error) },
                onClick = {
                    open = false
                    actions.onDelete()
                },
            )
        }
    }
}

@Composable
private fun UserBubble(item: ChatItem.User) {
    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.CenterEnd) {
        Text(
            item.text,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onPrimaryContainer,
            modifier = Modifier
                .widthIn(max = 520.dp)
                .padding(start = 48.dp)
                .clip(RoundedCornerShape(16.dp, 16.dp, 4.dp, 16.dp))
                .background(MaterialTheme.colorScheme.primaryContainer)
                .padding(horizontal = 14.dp, vertical = 10.dp),
        )
    }
}

@Composable
private fun AssistantBlock(item: ChatItem.Assistant) {
    var showThinking by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxWidth().padding(end = 24.dp)) {
        if (!item.thinking.isNullOrBlank()) {
            Row(
                Modifier.clip(RoundedCornerShape(8.dp)).clickable { showThinking = !showThinking }.padding(vertical = 4.dp, horizontal = 2.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(Icons.Rounded.Lightbulb, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(14.dp))
                Spacer(Modifier.width(4.dp))
                Text(stringResource(R.string.ai_thinking), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Icon(if (showThinking) Icons.Rounded.ExpandLess else Icons.Rounded.ExpandMore, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(16.dp))
            }
            if (showThinking) {
                Text(
                    item.thinking,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(start = 8.dp, bottom = 6.dp).border(width = 0.dp, color = Color.Transparent),
                )
            }
        }
        if (item.text.isNotEmpty()) {
            MarkdownText(item.text + if (item.streaming) " ▍" else "")
        } else if (item.streaming) {
            CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 1.5.dp)
        }
    }
}

@Composable
fun ToolCard(
    item: ChatItem.Tool,
    onImage: (ImageData) -> Unit,
    onAllowOnce: () -> Unit,
    onAllowSimilar: () -> Unit,
    onDeny: () -> Unit,
    onEdit: () -> Unit,
) {
    val call = item.call
    val color = riskColor(call.risk)
    var showOutput by remember { mutableStateOf(false) }
    var showDetail by remember { mutableStateOf(false) }
    val awaiting = item.phase == ToolPhase.AWAITING
    val (toolName, icon) = toolInfo(call.tool)
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(14.dp))
            .border(if (awaiting) 1.5.dp else 1.dp, if (awaiting) color else MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(14.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerLow)
            .animateContentSize(),
    ) {
        // 左侧风险色条 + 标题行
        Row(
            Modifier.fillMaxWidth().background(color.copy(alpha = if (awaiting) 0.14f else 0.07f)).padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(icon, null, tint = color, modifier = Modifier.size(18.dp))
            Spacer(Modifier.width(8.dp))
            Text(
                (toolName?.let { stringResource(it) } ?: call.tool) + "  ·  " + call.targetName,
                style = MaterialTheme.typography.labelLarge,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            Pill(stringResource(riskLabel(call.risk)), color, filled = awaiting)
            Spacer(Modifier.width(6.dp))
            PhaseBadge(item)
        }
        Column(Modifier.padding(horizontal = 12.dp, vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(8.dp))
                    .background(Galaxy.colors.codeBackground)
                    .clickable(enabled = call.detail.isNotBlank()) { showDetail = !showDetail }
                    .padding(start = 10.dp, end = 4.dp, top = 8.dp, bottom = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    call.summary,
                    style = MaterialTheme.typography.bodyMedium,
                    fontFamily = FontFamily.Monospace,
                    modifier = Modifier.weight(1f).horizontalScroll(rememberScrollState()),
                )
                if (call.detail.isNotBlank()) {
                    Icon(
                        if (showDetail) Icons.Rounded.ExpandLess else Icons.Rounded.ExpandMore,
                        stringResource(R.string.tool_detail),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.size(18.dp),
                    )
                }
            }
            if (call.modelReason.isNotBlank()) {
                LabeledLine(stringResource(R.string.tool_model_reason), call.modelReason)
            }
            if (call.riskReasons.isNotEmpty() && call.risk != RiskLevel.READ) {
                LabeledLine(stringResource(R.string.tool_reasons), call.riskReasons.joinToString("  ·  "), color)
            }
            val img = call.image ?: item.result?.image
            rememberBitmap(img)?.let { bmp ->
                Image(
                    bmp, stringResource(R.string.image_viewer),
                    contentScale = ContentScale.Crop,
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(140.dp)
                        .clip(RoundedCornerShape(10.dp))
                        .border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(10.dp))
                        .clickable { onImage(img!!) },
                )
            }
            if (showDetail) {
                Text(
                    call.detail,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp)).background(Galaxy.colors.codeBackground).padding(8.dp),
                )
            }
            val output = item.output.trimEnd()
            if (output.isNotEmpty() || item.result?.summary?.isNotEmpty() == true) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    item.result?.summary?.takeIf { it.isNotEmpty() }?.let {
                        Text(
                            it,
                            style = MaterialTheme.typography.labelMedium,
                            color = if (item.phase == ToolPhase.DENIED || item.phase == ToolPhase.FAILED) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.weight(1f),
                            maxLines = 2,
                        )
                    } ?: Spacer(Modifier.weight(1f))
                    if (output.isNotEmpty()) {
                        TextButton(onClick = { showOutput = !showOutput }, contentPadding = PaddingValues(horizontal = 8.dp)) {
                            Text(stringResource(if (showOutput) R.string.tool_hide_output else R.string.tool_show_output), style = MaterialTheme.typography.labelMedium)
                            Icon(if (showOutput) Icons.Rounded.ExpandLess else Icons.Rounded.ExpandMore, null, Modifier.size(16.dp))
                        }
                    }
                }
            }
            if (showOutput && output.isNotEmpty()) {
                Text(
                    output,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(max = 280.dp)
                        .clip(RoundedCornerShape(8.dp))
                        .background(Galaxy.colors.codeBackground)
                        .horizontalScroll(rememberScrollState())
                        .padding(10.dp),
                )
            }
            if (awaiting) {
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Button(onClick = onAllowOnce, colors = ButtonDefaults.buttonColors(containerColor = color, contentColor = MaterialTheme.colorScheme.surface)) {
                        Text(stringResource(R.string.approve_once))
                    }
                    FilledTonalButton(onClick = onAllowSimilar) { Text(stringResource(R.string.approve_similar)) }
                    if (call.tool == "run_command" || call.tool == "terminal_send") {
                        OutlinedButton(onClick = onEdit) { Text(stringResource(R.string.approve_edit)) }
                    }
                    TextButton(onClick = onDeny) { Text(stringResource(R.string.approve_deny), color = MaterialTheme.colorScheme.error) }
                    Box(Modifier.align(Alignment.CenterVertically)) { InfoButton(stringResource(R.string.approve_similar_info)) }
                }
            }
        }
    }
}

@Composable
private fun PhaseBadge(item: ChatItem.Tool) {
    when (item.phase) {
        ToolPhase.AWAITING -> Unit
        ToolPhase.RUNNING -> CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 1.5.dp)
        ToolPhase.DONE -> Icon(Icons.Rounded.CheckCircle, stringResource(R.string.tool_done), tint = Galaxy.colors.success, modifier = Modifier.size(18.dp))
        ToolPhase.FAILED -> Icon(Icons.Rounded.ErrorOutline, stringResource(R.string.tool_failed), tint = MaterialTheme.colorScheme.error, modifier = Modifier.size(18.dp))
        ToolPhase.DENIED -> Icon(Icons.Rounded.Block, stringResource(R.string.tool_denied), tint = MaterialTheme.colorScheme.error, modifier = Modifier.size(18.dp))
    }
}

@Composable
private fun LabeledLine(label: String, text: String, color: Color = MaterialTheme.colorScheme.onSurfaceVariant) {
    Row {
        Text(label, style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.SemiBold, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.widthIn(min = 96.dp))
        Spacer(Modifier.width(8.dp))
        Text(text, style = MaterialTheme.typography.bodySmall, color = color)
    }
}

@Composable
private fun AskCard(item: ChatItem.Ask, onAnswer: (String) -> Unit) {
    var text by remember { mutableStateOf("") }
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(14.dp))
            .border(1.dp, if (item.answer == null) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(14.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerLow)
            .padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(item.question, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
        if (item.answer != null) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Rounded.CheckCircle, null, tint = Galaxy.colors.success, modifier = Modifier.size(16.dp))
                Spacer(Modifier.width(6.dp))
                Text(item.answer, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        } else {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                item.options.forEachIndexed { i, o ->
                    if (i == 0) Button(onClick = { onAnswer(o) }) { Text(o) } else OutlinedButton(onClick = { onAnswer(o) }) { Text(o) }
                }
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                OutlinedTextField(
                    text, { text = it },
                    placeholder = { Text(stringResource(R.string.ask_answer_hint)) },
                    singleLine = true,
                    modifier = Modifier.weight(1f),
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
                    keyboardActions = KeyboardActions(onSend = { if (text.isNotBlank()) onAnswer(text.trim()) }),
                )
                IconButton(onClick = { if (text.isNotBlank()) onAnswer(text.trim()) }, enabled = text.isNotBlank()) {
                    Icon(Icons.AutoMirrored.Rounded.Send, stringResource(R.string.ai_send))
                }
            }
        }
    }
}

@Composable
private fun SecretCard(item: ChatItem.Secret, onProvide: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(14.dp))
            .border(1.dp, if (item.provided) MaterialTheme.colorScheme.outlineVariant else Galaxy.colors.riskUnknown, RoundedCornerShape(14.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerLow)
            .padding(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Outlined.Lock, null, tint = Galaxy.colors.riskUnknown, modifier = Modifier.size(22.dp))
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            Text(stringResource(R.string.secret_title), style = MaterialTheme.typography.labelLarge)
            Text(item.prompt, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        if (item.provided) {
            Pill(stringResource(R.string.secret_provided), Galaxy.colors.success)
        } else {
            Button(onClick = onProvide) { Text(stringResource(R.string.secret_provide)) }
        }
    }
}

@Composable
private fun ErrorCard(message: String) {
    Row(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(MaterialTheme.colorScheme.errorContainer).padding(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Rounded.ErrorOutline, null, tint = MaterialTheme.colorScheme.onErrorContainer)
        Spacer(Modifier.width(8.dp))
        Text(message, color = MaterialTheme.colorScheme.onErrorContainer, style = MaterialTheme.typography.bodyMedium)
    }
}

@Composable
private fun InputBar(value: String, onChange: (String) -> Unit, running: Boolean, onSend: () -> Unit, onStop: () -> Unit) {
    Surface(color = MaterialTheme.colorScheme.surfaceContainer, modifier = Modifier.navigationBarsPadding()) {
        Row(Modifier.fillMaxWidth().padding(8.dp), verticalAlignment = Alignment.Bottom) {
            TextField(
                value = value,
                onValueChange = onChange,
                placeholder = { Text(stringResource(R.string.ai_input_hint)) },
                maxLines = 6,
                shape = RoundedCornerShape(20.dp),
                colors = TextFieldDefaults.colors(
                    focusedIndicatorColor = Color.Transparent,
                    unfocusedIndicatorColor = Color.Transparent,
                    focusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                    unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                ),
                modifier = Modifier.weight(1f).heightIn(min = 48.dp),
            )
            Spacer(Modifier.width(6.dp))
            if (running) {
                IconButton(
                    onClick = onStop,
                    modifier = Modifier.size(48.dp).clip(RoundedCornerShape(24.dp)).background(MaterialTheme.colorScheme.errorContainer),
                ) { Icon(Icons.Rounded.Stop, stringResource(R.string.ai_stop), tint = MaterialTheme.colorScheme.onErrorContainer) }
            } else {
                IconButton(
                    onClick = onSend,
                    enabled = value.isNotBlank(),
                    modifier = Modifier.size(48.dp).clip(RoundedCornerShape(24.dp)).background(
                        if (value.isNotBlank()) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceContainerHighest,
                    ),
                ) {
                    Icon(
                        Icons.AutoMirrored.Rounded.Send, stringResource(R.string.ai_send),
                        tint = if (value.isNotBlank()) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.outline,
                    )
                }
            }
        }
    }
}

@Composable
private fun ImageViewer(img: ImageData, onClose: () -> Unit) {
    val bmp = rememberBitmap(img)
    Dialog(onDismissRequest = onClose, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Box(Modifier.fillMaxSize().background(Color.Black).clickable(onClick = onClose), contentAlignment = Alignment.Center) {
            bmp?.let { Image(it, stringResource(R.string.image_viewer), contentScale = ContentScale.Fit, modifier = Modifier.fillMaxWidth()) }
            IconButton(onClick = onClose, modifier = Modifier.align(Alignment.TopEnd).padding(16.dp)) {
                Icon(Icons.Rounded.Close, stringResource(R.string.close), tint = Color.White)
            }
        }
    }
}

@Composable
private fun DenyDialog(onDeny: (String) -> Unit, onDismiss: () -> Unit) {
    var reason by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.approve_deny)) },
        text = { OutlinedTextField(reason, { reason = it }, placeholder = { Text(stringResource(R.string.deny_reason_hint)) }, minLines = 2, modifier = Modifier.fillMaxWidth()) },
        confirmButton = {
            TextButton(onClick = {
                onDeny(reason.trim())
                onDismiss()
            }) { Text(stringResource(R.string.approve_deny), color = MaterialTheme.colorScheme.error) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) } },
    )
}

@Composable
private fun EditCommandDialog(initial: String, onRun: (String) -> Unit, onDismiss: () -> Unit) {
    var cmd by remember { mutableStateOf(initial) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.edit_command_title)) },
        text = {
            OutlinedTextField(
                cmd, { cmd = it },
                textStyle = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
                minLines = 3,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii, autoCorrectEnabled = false),
                modifier = Modifier.fillMaxWidth(),
            )
        },
        confirmButton = {
            TextButton(enabled = cmd.isNotBlank(), onClick = {
                onRun(cmd.trim())
                onDismiss()
            }) { Text(stringResource(R.string.run)) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) } },
    )
}

/** request_secret 的安全输入框：密码只交给 provide_secret，不进对话、不进模型。 */
@Composable
fun SecretDialog(prompt: String, onSubmit: (CharArray) -> Unit, onDismiss: () -> Unit) {
    var value by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = { Icon(Icons.Outlined.Lock, null) },
        title = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.secret_title))
                InfoButton(stringResource(R.string.secret_info))
            }
        },
        text = {
            Column {
                Text(prompt, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.height(12.dp))
                OutlinedTextField(
                    value, { value = it },
                    label = { Text(stringResource(R.string.secret_hint)) },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false),
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        },
        confirmButton = {
            TextButton(enabled = value.isNotEmpty(), onClick = {
                onSubmit(value.toCharArray())
                value = ""
                onDismiss()
            }) { Text(stringResource(R.string.ok)) }
        },
        dismissButton = {
            TextButton(onClick = {
                value = ""
                onDismiss()
            }) { Text(stringResource(R.string.cancel)) }
        },
    )
}
