package cn.lemonit.galaxy.ui.ai

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.ArrowUpward
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.ExpandMore
import androidx.compose.material.icons.rounded.Stop
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.MentionCandidate
import cn.lemonit.galaxy.app.MentionQuery
import cn.lemonit.galaxy.app.Refs
import cn.lemonit.galaxy.app.TargetRef
import cn.lemonit.galaxy.app.canSend
import cn.lemonit.galaxy.app.filterCandidates
import cn.lemonit.galaxy.app.findMention
import cn.lemonit.galaxy.app.removeMention
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.AiModel
import cn.lemonit.galaxy.core.bridge.ContextRef
import cn.lemonit.galaxy.core.bridge.ContextRefKind
import cn.lemonit.galaxy.ui.components.StatusDot
import cn.lemonit.galaxy.ui.files.FileBrowserController
import cn.lemonit.galaxy.ui.theme.Galaxy

/** 输入框的内容：文字（含光标）、引用、错误。由页面持有，发送成功后清空。 */
@Stable
class ComposerState(text: String = "", refs: List<ContextRef> = emptyList()) {
    var value by mutableStateOf(TextFieldValue(text, TextRange(text.length)))
    var refs by mutableStateOf(refs)

    /** 新对话创建失败等：显示在输入框上方。 */
    var error by mutableStateOf<String?>(null)

    /** 正在创建对话（发送按钮转圈、不能重复发）。 */
    var busy by mutableStateOf(false)
    private var dismissedAt by mutableStateOf<Int?>(null)

    val canSend: Boolean get() = canSend(value.text, refs) && !busy

    /** 正在输入的 @ 引用（被关掉的不算）。 */
    val mention: MentionQuery?
        get() {
            if (!value.selection.collapsed) return null
            val m = findMention(value.text, value.selection.end) ?: return null
            return if (m.start == dismissedAt) null else m
        }

    /** Esc / 点空白处关闭 @ 候选。 */
    fun dismissMention() {
        dismissedAt = mention?.start
    }

    fun addRefs(more: List<ContextRef>) {
        refs = Refs.addAll(refs, more)
    }

    fun removeRef(ref: ContextRef) {
        refs = refs.filterNot { it == ref }
    }

    /** 选中 @ 候选：删掉 @ 与过滤文字，加一个引用标签。 */
    fun pickMention(m: MentionQuery, ref: ContextRef) {
        val (t, c) = removeMention(value.text, m)
        value = TextFieldValue(t, TextRange(c))
        addRefs(listOf(ref))
    }

    fun clear() {
        value = TextFieldValue("")
        refs = emptyList()
        error = null
    }
}

/** 「+」菜单与 @ 候选的数据来源。 */
class RefSources(
    /** 设备（不含本机与手机）与 SSH 主机。 */
    val targets: List<MentionCandidate> = emptyList(),
    val snippets: List<MentionCandidate> = emptyList(),
    val forwards: List<MentionCandidate> = emptyList(),
    /** 文件、文件夹所在设备或主机的名字。 */
    val placeName: (ContextRef) -> String = { it.targetId },
    /** 浏览某台设备或主机的文件（复用文件页的 SFTP / 设备文件浏览）。 */
    val browser: ((TargetRef) -> FileBrowserController)? = null,
) {
    fun icon(ref: ContextRef): ImageVector = refIcon(ref, targets.firstOrNull { it.ref.kind == ContextRefKind.DEVICE && it.ref.id == ref.id }?.platform)

    /** 长按标签显示的内容：文件 / 文件夹的完整路径与所在设备。 */
    fun detail(ref: ContextRef): String? = when (ref.kind) {
        ContextRefKind.FILE, ContextRefKind.FOLDER -> ref.id + "\n" + placeName(ref)
        else -> null
    }
}

enum class ComposerSheet { NONE, PLUS, MODEL, MODE, TRUST, TARGETS, FILES, SNIPPETS, FORWARDS }

/** 输入框（docs/ai-chat-ui.md 第 2 节）。[enabled] 为假时（模型不可用）整体置灰。 */
@OptIn(ExperimentalLayoutApi::class, ExperimentalMaterial3Api::class)
@Composable
fun Composer(
    state: ComposerState,
    enabled: Boolean,
    running: Boolean,
    models: List<AiModel>,
    model: AiModel?,
    mode: AiMode,
    maxMode: AiMode,
    planFirst: Boolean,
    sources: RefSources,
    /** 已有对话里「先出计划」不能再改（只在新建时生效）。 */
    planFirstEditable: Boolean,
    onSend: () -> Unit,
    onStop: () -> Unit,
    onModel: (AiModel) -> Unit,
    onMode: (AiMode, UInt?) -> Unit,
    onPlanFirst: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    initialSheet: ComposerSheet = ComposerSheet.NONE,
) {
    var sheet by remember { mutableStateOf(initialSheet) }
    val activityLabel = stringResource(R.string.ai_ref_activity)
    val mention = if (enabled) state.mention else null
    val candidates = mention?.let { filterCandidates(sources.targets, it.query) }.orEmpty()
    Column(modifier.fillMaxWidth()) {
        state.error?.let { err ->
            Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 6.dp), verticalAlignment = Alignment.Top) {
                Icon(Icons.Rounded.ErrorOutline, null, tint = MaterialTheme.colorScheme.error, modifier = Modifier.padding(top = 1.dp).size(16.dp))
                Spacer(Modifier.width(6.dp))
                Text(err, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
            }
        }
        if (mention != null && candidates.isNotEmpty()) {
            MentionList(candidates) { state.pickMention(mention, it.ref) }
            Spacer(Modifier.size(6.dp))
        }
        Column(
            Modifier
                .fillMaxWidth()
                .alpha(if (enabled) 1f else 0.5f)
                .clip(RoundedCornerShape(24.dp))
                .border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(24.dp))
                .background(MaterialTheme.colorScheme.surfaceContainerLow)
                .padding(start = 6.dp, end = 6.dp, top = 8.dp, bottom = 6.dp),
        ) {
            if (state.refs.isNotEmpty()) {
                FlowRow(
                    Modifier.fillMaxWidth().padding(horizontal = 6.dp, vertical = 2.dp),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    state.refs.forEach { r -> RefChip(r, sources.icon(r), sources.detail(r), onRemove = { state.removeRef(r) }) }
                }
                Spacer(Modifier.size(4.dp))
            }
            val textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface)
            BasicTextField(
                value = state.value,
                onValueChange = { state.value = it },
                enabled = enabled,
                textStyle = textStyle,
                cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                // 手机端回车换行、点发送按钮发送；最多长到约 8 行后内部滚动
                maxLines = 8,
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                modifier = Modifier.fillMaxWidth().padding(horizontal = 10.dp, vertical = 6.dp),
                decorationBox = { inner ->
                    Box {
                        if (state.value.text.isEmpty()) {
                            Text(stringResource(R.string.ai_input_hint), style = textStyle, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        inner()
                    }
                },
            )
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
              Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
                ToolbarButton(enabled, onClick = { sheet = ComposerSheet.PLUS }) {
                    Icon(Icons.Rounded.Add, stringResource(R.string.ai_add_context), modifier = Modifier.size(22.dp))
                }
                ToolbarButton(enabled, onClick = { sheet = ComposerSheet.MODEL }, modifier = Modifier.weight(1f, fill = false)) {
                    Text(
                        model?.name ?: stringResource(R.string.ai_model),
                        style = MaterialTheme.typography.labelLarge,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f, fill = false),
                    )
                    Icon(Icons.Rounded.ExpandMore, null, modifier = Modifier.size(18.dp))
                }
                ToolbarButton(enabled, onClick = { sheet = ComposerSheet.MODE }, modifier = Modifier.weight(1f, fill = false)) {
                    Icon(modeIcon(mode), null, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.width(4.dp))
                    Text(
                        stringResource(modeLabel(mode)),
                        style = MaterialTheme.typography.labelLarge,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f, fill = false),
                    )
                    Icon(Icons.Rounded.ExpandMore, null, modifier = Modifier.size(18.dp))
                }
              }
                Spacer(Modifier.width(4.dp))
                SendButton(
                    running = running,
                    busy = state.busy,
                    enabled = enabled && state.canSend,
                    onSend = onSend,
                    onStop = onStop,
                )
            }
        }
    }
    if (sheet != ComposerSheet.NONE) {
        val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = sheet == ComposerSheet.FILES)
        ModalBottomSheet(onDismissRequest = { sheet = ComposerSheet.NONE }, sheetState = sheetState) {
            val close = { sheet = ComposerSheet.NONE }
            when (sheet) {
                ComposerSheet.PLUS -> PlusMenu(
                    onTargets = { sheet = ComposerSheet.TARGETS },
                    onFiles = { sheet = ComposerSheet.FILES },
                    onSnippets = { sheet = ComposerSheet.SNIPPETS },
                    onForwards = { sheet = ComposerSheet.FORWARDS },
                    onActivity = {
                        state.addRefs(listOf(Refs.activity(activityLabel)))
                        close()
                    },
                )
                ComposerSheet.MODEL -> ModelMenu(models, model) {
                    onModel(it)
                    close()
                }
                ComposerSheet.MODE -> ModeMenu(
                    mode, maxMode, planFirst, planFirstEditable,
                    onMode = { m ->
                        if (m == AiMode.FULL_TRUST) {
                            sheet = ComposerSheet.TRUST
                        } else {
                            onMode(m, null)
                            close()
                        }
                    },
                    onPlanFirst = onPlanFirst,
                )
                ComposerSheet.TRUST -> TrustMenu { minutes ->
                    onMode(AiMode.FULL_TRUST, minutes)
                    close()
                }
                ComposerSheet.TARGETS -> MultiPickSheet(stringResource(R.string.ai_ctx_targets), sources.targets, state.refs, showStatus = true) {
                    state.addRefs(it)
                    close()
                }
                ComposerSheet.SNIPPETS -> MultiPickSheet(stringResource(R.string.ai_ctx_snippets), sources.snippets, state.refs) {
                    state.addRefs(it)
                    close()
                }
                ComposerSheet.FORWARDS -> MultiPickSheet(stringResource(R.string.ai_ctx_forwards), sources.forwards, state.refs) {
                    state.addRefs(it)
                    close()
                }
                ComposerSheet.FILES -> FilePickSheet(sources) {
                    state.addRefs(listOf(it))
                    close()
                }
                ComposerSheet.NONE -> Unit
            }
        }
    }
}

@Composable
private fun ToolbarButton(enabled: Boolean, onClick: () -> Unit, modifier: Modifier = Modifier, content: @Composable RowScope.() -> Unit) {
    Row(
        modifier
            .heightIn(min = 36.dp)
            .clip(RoundedCornerShape(18.dp))
            .clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        androidx.compose.runtime.CompositionLocalProvider(androidx.compose.material3.LocalContentColor provides MaterialTheme.colorScheme.onSurfaceVariant) {
            content()
        }
    }
}

/** 右侧圆形按钮：发送（箭头）/ 停止（方块）。 */
@Composable
private fun SendButton(running: Boolean, busy: Boolean, enabled: Boolean, onSend: () -> Unit, onStop: () -> Unit) {
    val primary = MaterialTheme.colorScheme.primary
    when {
        running -> Box(
            Modifier.size(36.dp).clip(CircleShape).background(MaterialTheme.colorScheme.onSurface).clickable(onClick = onStop),
            contentAlignment = Alignment.Center,
        ) { Icon(Icons.Rounded.Stop, stringResource(R.string.ai_stop), tint = MaterialTheme.colorScheme.surface, modifier = Modifier.size(20.dp)) }
        busy -> Box(Modifier.size(36.dp).clip(CircleShape).background(primary), contentAlignment = Alignment.Center) {
            CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp, color = MaterialTheme.colorScheme.onPrimary)
        }
        else -> Box(
            Modifier
                .size(36.dp)
                .clip(CircleShape)
                .background(if (enabled) primary else MaterialTheme.colorScheme.surfaceContainerHighest)
                .clickable(enabled = enabled, onClick = onSend),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                Icons.Rounded.ArrowUpward, stringResource(R.string.ai_send),
                tint = if (enabled) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.outline,
                modifier = Modifier.size(20.dp),
            )
        }
    }
}

/** @ 候选：显示在输入框上方。 */
@Composable
private fun MentionList(items: List<MentionCandidate>, onPick: (MentionCandidate) -> Unit) {
    Surface(
        shape = RoundedCornerShape(16.dp),
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        shadowElevation = 4.dp,
        modifier = Modifier.fillMaxWidth(),
    ) {
        LazyColumn(Modifier.heightIn(max = 232.dp).padding(vertical = 4.dp)) {
            items(items, key = { it.ref.kind.name + it.ref.id }) { c ->
                CandidateRow(c, onClick = { onPick(c) })
            }
        }
    }
}

@Composable
fun CandidateRow(c: MentionCandidate, onClick: () -> Unit, showStatus: Boolean = true, trailing: @Composable () -> Unit = {}) {
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 9.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(refIcon(c.ref, c.platform), null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(20.dp))
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(c.ref.label, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (c.subtitle.isNotBlank()) {
                Text(c.subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
        if (showStatus && c.ref.kind == ContextRefKind.DEVICE) {
            Spacer(Modifier.width(8.dp))
            StatusDot(if (c.online) Galaxy.colors.online else Galaxy.colors.offline)
            Spacer(Modifier.width(4.dp))
            Text(
                stringResource(if (c.online) R.string.status_online else R.string.status_offline),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.widthIn(min = 32.dp),
            )
        }
        trailing()
    }
}
