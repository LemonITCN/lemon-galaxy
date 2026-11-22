package cn.lemonit.galaxy.ui.ai

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Send
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material.icons.rounded.Block
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.ExpandLess
import androidx.compose.material.icons.rounded.ExpandMore
import androidx.compose.material.icons.rounded.Lightbulb
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.PlainTooltip
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TooltipBox
import androidx.compose.material3.TooltipDefaults
import androidx.compose.material3.rememberTooltipState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
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
import cn.lemonit.galaxy.app.ToolPhase
import cn.lemonit.galaxy.core.bridge.ContextRef
import cn.lemonit.galaxy.core.bridge.ImageData
import cn.lemonit.galaxy.core.bridge.RiskLevel
import cn.lemonit.galaxy.ui.components.Pill
import cn.lemonit.galaxy.ui.theme.Galaxy

/** 输出默认最多显示的行数，更多的点「显示全部」。 */
private const val OUTPUT_PREVIEW_LINES = 12

/** 引用标签：图标 + 名称（+ 删除叉）。[detail] 不为空时长按显示（文件的完整路径与所在设备）。 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RefChip(ref: ContextRef, icon: ImageVector, detail: String?, onRemove: (() -> Unit)?, modifier: Modifier = Modifier) {
    val chip = @Composable {
        Row(
            modifier
                .heightIn(min = 30.dp)
                .clip(RoundedCornerShape(8.dp))
                .background(MaterialTheme.colorScheme.surfaceContainerHighest)
                .padding(start = 8.dp, end = if (onRemove != null) 2.dp else 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(icon, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(15.dp))
            Spacer(Modifier.width(6.dp))
            Text(
                ref.label,
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.widthIn(max = 200.dp),
            )
            if (onRemove != null) {
                Box(
                    Modifier.size(28.dp).clip(CircleShape).clickable(onClick = onRemove),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(Icons.Rounded.Close, stringResource(R.string.ai_ref_remove), tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(15.dp))
                }
            }
        }
    }
    if (detail == null) {
        chip()
    } else {
        TooltipBox(
            positionProvider = TooltipDefaults.rememberPlainTooltipPositionProvider(),
            tooltip = { PlainTooltip { Text(detail) } },
            state = rememberTooltipState(),
        ) { chip() }
    }
}

/** 用户消息：靠右的气泡，引用标签在文字上方。 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun UserMessage(item: ChatItem.User, refIcon: (ContextRef) -> ImageVector, refDetail: (ContextRef) -> String?) {
    Column(Modifier.fillMaxWidth().padding(start = 48.dp), horizontalAlignment = Alignment.End) {
        if (item.refs.isNotEmpty()) {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp, Alignment.End), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                item.refs.forEach { RefChip(it, refIcon(it), refDetail(it), null) }
            }
            if (item.text.isNotEmpty()) Spacer(Modifier.height(6.dp))
        }
        if (item.text.isNotEmpty()) {
            Text(
                item.text,
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurface,
                modifier = Modifier
                    .widthIn(max = 560.dp)
                    .clip(RoundedCornerShape(20.dp))
                    .background(MaterialTheme.colorScheme.surfaceContainerHigh)
                    .padding(horizontal = 16.dp, vertical = 10.dp),
            )
        }
    }
}

/** 模型回复：不用气泡、占满宽度；上方一行可折叠的思考过程。 */
@Composable
fun AssistantMessage(item: ChatItem.Assistant) {
    var showThinking by remember { mutableStateOf(false) }
    val thinkingNow = item.streaming && item.text.isEmpty()
    Column(Modifier.fillMaxWidth()) {
        if (!item.thinking.isNullOrBlank()) {
            Row(
                Modifier.clip(RoundedCornerShape(8.dp)).clickable { showThinking = !showThinking }.padding(vertical = 4.dp, horizontal = 2.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (thinkingNow) {
                    CircularProgressIndicator(Modifier.size(13.dp), strokeWidth = 1.5.dp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                } else {
                    Icon(Icons.Rounded.Lightbulb, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(15.dp))
                }
                Spacer(Modifier.width(6.dp))
                Text(
                    stringResource(if (thinkingNow) R.string.ai_thinking_now else R.string.ai_thinking),
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Icon(if (showThinking) Icons.Rounded.ExpandLess else Icons.Rounded.ExpandMore, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(18.dp))
            }
            if (showThinking) {
                Row(Modifier.padding(start = 8.dp, top = 4.dp, bottom = 8.dp)) {
                    Box(Modifier.width(2.dp).heightIn(min = 16.dp).background(MaterialTheme.colorScheme.outlineVariant))
                    Spacer(Modifier.width(10.dp))
                    Text(item.thinking, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        }
        if (item.text.isNotEmpty()) {
            MarkdownText(item.text, streaming = item.streaming)
        } else if (item.streaming && item.thinking.isNullOrBlank()) {
            TypingIndicator()
        }
    }
}

/** 等模型开口时的小指示。 */
@Composable
fun TypingIndicator() {
    CircularProgressIndicator(Modifier.padding(vertical = 6.dp).size(16.dp), strokeWidth = 2.dp, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

/** 工具调用：需要确认时是醒目的卡片，否则是一行紧凑的条目（点击展开）。 */
@Composable
fun ToolEntry(
    item: ChatItem.Tool,
    onImage: (ImageData) -> Unit,
    onAllowOnce: () -> Unit,
    onAllowSimilar: () -> Unit,
    onDeny: () -> Unit,
    onEdit: () -> Unit,
) {
    if (item.phase == ToolPhase.AWAITING && item.needsApproval) {
        if (item.call.tool == "use_target") ConnectCard(item, onAllow = onAllowOnce, onDeny = onDeny) else ApprovalCard(item, onImage, onAllowOnce, onAllowSimilar, onDeny, onEdit)
    } else {
        ToolRow(item, onImage)
    }
}

@Composable
private fun ToolRow(item: ChatItem.Tool, onImage: (ImageData) -> Unit) {
    val call = item.call
    var expanded by remember { mutableStateOf(false) }
    val (label, icon) = toolInfo(call.tool)
    val bad = item.phase == ToolPhase.FAILED || item.phase == ToolPhase.DENIED
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(10.dp))
            .border(1.dp, if (expanded) MaterialTheme.colorScheme.outlineVariant else Color.Transparent, RoundedCornerShape(10.dp))
            .animateContentSize(),
    ) {
        Row(
            Modifier.fillMaxWidth().clickable { expanded = !expanded }.padding(horizontal = 8.dp, vertical = 7.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(icon, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(16.dp))
            Spacer(Modifier.width(8.dp))
            Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
                Text(
                    call.summary.ifBlank { label?.let { stringResource(it) } ?: call.tool },
                    style = MaterialTheme.typography.bodyMedium,
                    fontFamily = if (call.tool == "run_command") FontFamily.Monospace else null,
                    color = if (bad) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                )
                if (call.targetName.isNotBlank()) {
                    Spacer(Modifier.width(8.dp))
                    Text(
                        call.targetName,
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.outline,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.widthIn(max = 120.dp),
                    )
                }
            }
            Spacer(Modifier.width(8.dp))
            PhaseIcon(item.phase)
        }
        if (expanded) ToolDetail(item, onImage, Modifier.padding(start = 32.dp, end = 10.dp, bottom = 10.dp))
    }
}

@Composable
private fun PhaseIcon(phase: ToolPhase) {
    when (phase) {
        ToolPhase.AWAITING, ToolPhase.RUNNING -> CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 1.5.dp, color = MaterialTheme.colorScheme.onSurfaceVariant)
        ToolPhase.DONE -> Icon(Icons.Rounded.CheckCircle, stringResource(R.string.tool_done), tint = Galaxy.colors.success, modifier = Modifier.size(16.dp))
        ToolPhase.FAILED -> Icon(Icons.Rounded.ErrorOutline, stringResource(R.string.tool_failed), tint = MaterialTheme.colorScheme.error, modifier = Modifier.size(16.dp))
        ToolPhase.DENIED -> Icon(Icons.Rounded.Block, stringResource(R.string.tool_denied), tint = MaterialTheme.colorScheme.error, modifier = Modifier.size(16.dp))
    }
}

/** 展开后的内容：完整命令或参数、理由、风险原因、输出、截图。 */
@Composable
private fun ToolDetail(item: ChatItem.Tool, onImage: (ImageData) -> Unit, modifier: Modifier = Modifier) {
    val call = item.call
    Column(modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (call.detail.isNotBlank()) CodeBox(call.detail)
        if (call.modelReason.isNotBlank()) LabeledLine(stringResource(R.string.tool_model_reason), call.modelReason)
        if (call.risk != RiskLevel.READ) {
            LabeledLine(
                stringResource(R.string.tool_risk),
                (listOf(stringResource(riskLabel(call.risk))) + call.riskReasons).joinToString("  ·  "),
                riskColor(call.risk),
            )
        }
        item.result?.summary?.takeIf { it.isNotBlank() }?.let {
            val bad = item.phase == ToolPhase.FAILED || item.phase == ToolPhase.DENIED
            LabeledLine(stringResource(R.string.tool_result), it, if (bad) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant)
        }
        val output = item.output.trimEnd()
        if (output.isNotEmpty()) OutputBox(output)
        val img = item.result?.image ?: call.image
        Thumbnail(img, onImage)
    }
}

@Composable
private fun Thumbnail(img: ImageData?, onImage: (ImageData) -> Unit) {
    val bmp = rememberBitmap(img) ?: return
    Image(
        bmp, stringResource(R.string.image_viewer),
        contentScale = ContentScale.Crop,
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(max = 160.dp)
            .clip(RoundedCornerShape(10.dp))
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(10.dp))
            .clickable { onImage(img!!) },
    )
}

@Composable
private fun CodeBox(text: String, color: Color = MaterialTheme.colorScheme.onSurface) {
    Text(
        text,
        style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
        color = color,
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(8.dp))
            .background(Galaxy.colors.codeBackground)
            .heightIn(max = 320.dp)
            .horizontalScroll(rememberScrollState())
            .padding(10.dp),
    )
}

@Composable
private fun OutputBox(output: String) {
    var all by remember { mutableStateOf(false) }
    val lines = output.lines()
    val cut = !all && lines.size > OUTPUT_PREVIEW_LINES
    Column {
        CodeBox(if (cut) lines.take(OUTPUT_PREVIEW_LINES).joinToString("\n") else output, MaterialTheme.colorScheme.onSurfaceVariant)
        if (lines.size > OUTPUT_PREVIEW_LINES) {
            TextButton(onClick = { all = !all }, contentPadding = PaddingValues(horizontal = 8.dp)) {
                Text(
                    if (all) stringResource(R.string.tool_hide_output) else stringResource(R.string.tool_show_all, lines.size),
                    style = MaterialTheme.typography.labelMedium,
                )
            }
        }
    }
}

@Composable
private fun LabeledLine(label: String, text: String, color: Color = MaterialTheme.colorScheme.onSurfaceVariant) {
    Row {
        Text(label, style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.SemiBold, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.widthIn(min = 72.dp))
        Spacer(Modifier.width(8.dp))
        Text(text, style = MaterialTheme.typography.bodySmall, color = color)
    }
}

@Composable
private fun CardFrame(accent: Color, content: @Composable () -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(16.dp))
            .border(1.5.dp, accent.copy(alpha = 0.7f), RoundedCornerShape(16.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerLow)
            .padding(14.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) { content() }
}

/** 需要确认的操作：目标、完整命令 / 改动 / 点击位置、风险与原因、模型理由，四个按钮。 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ApprovalCard(
    item: ChatItem.Tool,
    onImage: (ImageData) -> Unit,
    onAllowOnce: () -> Unit,
    onAllowSimilar: () -> Unit,
    onDeny: () -> Unit,
    onEdit: () -> Unit,
) {
    val call = item.call
    val color = riskColor(call.risk)
    val (label, icon) = toolInfo(call.tool)
    CardFrame(color) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(30.dp).clip(RoundedCornerShape(8.dp)).background(color.copy(alpha = 0.14f)), contentAlignment = Alignment.Center) {
                Icon(icon, null, tint = color, modifier = Modifier.size(17.dp))
            }
            Spacer(Modifier.width(10.dp))
            Column(Modifier.weight(1f)) {
                Text(label?.let { stringResource(it) } ?: call.tool, style = MaterialTheme.typography.titleSmall)
                if (call.targetName.isNotBlank()) {
                    Text(call.targetName, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            Pill(stringResource(riskLabel(call.risk)), color, filled = true)
        }
        CodeBox(call.detail.ifBlank { call.summary })
        if (call.riskReasons.isNotEmpty()) {
            Row(verticalAlignment = Alignment.Top) {
                Icon(Icons.Rounded.Warning, null, tint = color, modifier = Modifier.padding(top = 1.dp).size(15.dp))
                Spacer(Modifier.width(6.dp))
                Text(call.riskReasons.joinToString("  ·  "), style = MaterialTheme.typography.bodySmall, color = color)
            }
        }
        if (call.modelReason.isNotBlank()) LabeledLine(stringResource(R.string.tool_model_reason), call.modelReason)
        Thumbnail(call.image, onImage)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Button(onClick = onAllowOnce, colors = ButtonDefaults.buttonColors(containerColor = color, contentColor = Color.White)) {
                Text(stringResource(R.string.approve_once))
            }
            FilledTonalButton(onClick = onAllowSimilar) { Text(stringResource(R.string.approve_similar)) }
            if (call.tool == "run_command") OutlinedButton(onClick = onEdit) { Text(stringResource(R.string.approve_edit)) }
            TextButton(onClick = onDeny) { Text(stringResource(R.string.approve_deny), color = MaterialTheme.colorScheme.error) }
        }
    }
}

/** use_target：「AI 想连接 某设备」，只有允许与拒绝。 */
@Composable
private fun ConnectCard(item: ChatItem.Tool, onAllow: () -> Unit, onDeny: () -> Unit) {
    val call = item.call
    val accent = MaterialTheme.colorScheme.primary
    CardFrame(accent) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(30.dp).clip(RoundedCornerShape(8.dp)).background(accent.copy(alpha = 0.14f)), contentAlignment = Alignment.Center) {
                Icon(toolInfo(call.tool).second, null, tint = accent, modifier = Modifier.size(17.dp))
            }
            Spacer(Modifier.width(10.dp))
            Text(stringResource(R.string.ai_wants_connect, call.targetName), style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f))
        }
        if (call.detail.isNotBlank()) Text(call.detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        if (call.modelReason.isNotBlank()) LabeledLine(stringResource(R.string.tool_model_reason), call.modelReason)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = onAllow) { Text(stringResource(R.string.approve_allow)) }
            TextButton(onClick = onDeny) { Text(stringResource(R.string.approve_deny), color = MaterialTheme.colorScheme.error) }
        }
    }
}

/** ask_user：选项按钮 + 自由回答。 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun AskCard(item: ChatItem.Ask, onAnswer: (String) -> Unit) {
    var text by remember { mutableStateOf("") }
    val accent = if (item.answer == null) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outlineVariant
    CardFrame(accent) {
        Text(item.question, style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.Medium)
        if (item.answer != null) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Rounded.CheckCircle, null, tint = Galaxy.colors.success, modifier = Modifier.size(16.dp))
                Spacer(Modifier.width(6.dp))
                Text(item.answer, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        } else {
            if (item.options.isNotEmpty()) {
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    item.options.forEach { o -> OutlinedButton(onClick = { onAnswer(o) }) { Text(o) } }
                }
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                OutlinedTextField(
                    text, { text = it },
                    placeholder = { Text(stringResource(R.string.ask_answer_hint)) },
                    singleLine = true,
                    shape = RoundedCornerShape(12.dp),
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

/** request_secret：卡片里一个不回显的输入框，内容走 provide_secret、不进入消息流。 */
@Composable
fun SecretCard(item: ChatItem.Secret, onSubmit: (CharArray) -> Unit) {
    var value by remember { mutableStateOf("") }
    val accent = if (item.provided) MaterialTheme.colorScheme.outlineVariant else Galaxy.colors.riskUnknown
    CardFrame(accent) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Outlined.Lock, null, tint = Galaxy.colors.riskUnknown, modifier = Modifier.size(18.dp))
            Spacer(Modifier.width(8.dp))
            Text(item.prompt.ifBlank { stringResource(R.string.secret_title) }, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
        }
        if (item.provided) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Rounded.CheckCircle, null, tint = Galaxy.colors.success, modifier = Modifier.size(16.dp))
                Spacer(Modifier.width(6.dp))
                Text(stringResource(R.string.secret_provided), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        } else {
            val submit = {
                if (value.isNotEmpty()) {
                    onSubmit(value.toCharArray())
                    value = ""
                }
            }
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    value, { value = it },
                    placeholder = { Text(stringResource(R.string.secret_hint)) },
                    singleLine = true,
                    shape = RoundedCornerShape(12.dp),
                    visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false, imeAction = ImeAction.Send),
                    keyboardActions = KeyboardActions(onSend = { submit() }),
                    modifier = Modifier.weight(1f),
                )
                Button(onClick = submit, enabled = value.isNotEmpty()) { Text(stringResource(R.string.ai_send)) }
            }
        }
    }
}

/** 错误：消息流里一行红色提示。 */
@Composable
fun ErrorLine(message: String) {
    Row(Modifier.fillMaxWidth().padding(vertical = 2.dp), verticalAlignment = Alignment.Top) {
        Icon(Icons.Rounded.ErrorOutline, null, tint = MaterialTheme.colorScheme.error, modifier = Modifier.padding(top = 2.dp).size(16.dp))
        Spacer(Modifier.width(6.dp))
        Text(message, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium)
    }
}

@Composable
fun ImageViewer(img: ImageData, onClose: () -> Unit) {
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
fun DenyDialog(onDeny: (String) -> Unit, onDismiss: () -> Unit) {
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
fun EditCommandDialog(initial: String, onRun: (String) -> Unit, onDismiss: () -> Unit) {
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
