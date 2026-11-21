package cn.lemonit.galaxy.ui.ai

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.AutoAwesome
import androidx.compose.material.icons.rounded.Block
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.HostItem
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.AiModel
import cn.lemonit.galaxy.core.bridge.AiModels
import cn.lemonit.galaxy.core.bridge.ConversationSummary
import cn.lemonit.galaxy.ui.components.ConfirmDialog
import cn.lemonit.galaxy.ui.components.EmptyState
import cn.lemonit.galaxy.ui.components.EntryAvatar
import cn.lemonit.galaxy.ui.components.GalaxyTopBar
import cn.lemonit.galaxy.ui.components.InfoButton
import cn.lemonit.galaxy.ui.components.Pill
import cn.lemonit.galaxy.ui.components.SectionHeader
import cn.lemonit.galaxy.ui.components.relativeTime
import cn.lemonit.galaxy.ui.hosts.avatarFor
import cn.lemonit.galaxy.ui.theme.Galaxy

fun modeLabel(m: AiMode): Int = when (m) {
    AiMode.OFF -> R.string.mode_off
    AiMode.READ_ONLY -> R.string.mode_read_only
    AiMode.CONFIRM_ALL -> R.string.mode_confirm_all
    AiMode.CONFIRM_DANGEROUS -> R.string.mode_confirm_dangerous
    AiMode.FULL_TRUST -> R.string.mode_full_trust
}

@Composable
fun modeColor(m: AiMode): Color = when (m) {
    AiMode.OFF, AiMode.READ_ONLY -> Galaxy.colors.riskRead
    AiMode.CONFIRM_ALL -> Galaxy.colors.riskNormal
    AiMode.CONFIRM_DANGEROUS -> Galaxy.colors.riskUnknown
    AiMode.FULL_TRUST -> Galaxy.colors.riskDangerous
}

/** 档位顺序（越往后越宽松），实际可选不超过管理员上限。 */
val SELECTABLE_MODES = listOf(AiMode.READ_ONLY, AiMode.CONFIRM_ALL, AiMode.CONFIRM_DANGEROUS, AiMode.FULL_TRUST)

@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ConversationListScreen(
    items: List<ConversationSummary>,
    enabled: Boolean,
    onOpen: (String) -> Unit,
    onNew: () -> Unit,
    onDelete: (String) -> Unit,
    selectedId: String? = null,
) {
    var deleting by remember { mutableStateOf<ConversationSummary?>(null) }
    Scaffold(
        topBar = {
            GalaxyTopBar(
                stringResource(R.string.ai_title),
                actions = {
                    InfoButton(stringResource(R.string.ai_background_info))
                    if (enabled) IconButton(onClick = onNew) { Icon(Icons.Rounded.Add, stringResource(R.string.ai_new)) }
                },
            )
        },
    ) { pad ->
        when {
            !enabled -> EmptyState(Icons.Rounded.Block, stringResource(R.string.ai_disabled), Modifier.padding(pad))
            items.isEmpty() -> EmptyState(
                Icons.Rounded.AutoAwesome, stringResource(R.string.ai_empty), Modifier.padding(pad),
                action = { Button(onClick = onNew) { Text(stringResource(R.string.ai_new)) } },
            )
            else -> LazyColumn(Modifier.padding(pad).fillMaxSize(), contentPadding = PaddingValues(vertical = 8.dp)) {
                items(items, key = { it.id }) { c ->
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 8.dp, vertical = 1.dp)
                            .clip(RoundedCornerShape(12.dp))
                            .background(if (c.id == selectedId) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent)
                            .combinedClickable(onClick = { onOpen(c.id) }, onLongClick = { deleting = c })
                            .padding(horizontal = 8.dp, vertical = 10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        EntryAvatar(Icons.Rounded.AutoAwesome, tint = MaterialTheme.colorScheme.primary, container = MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.6f))
                        Spacer(Modifier.width(12.dp))
                        Column(Modifier.weight(1f)) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text(c.title, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                                Spacer(Modifier.width(8.dp))
                                Text(relativeTime(c.updatedAt), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                            Spacer(Modifier.height(2.dp))
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text(
                                    c.modelName + "  ·  " + c.targetNames.joinToString(", "),
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                    modifier = Modifier.weight(1f),
                                )
                                when {
                                    c.waitingForUser -> Pill(stringResource(R.string.ai_waiting), Galaxy.colors.riskUnknown)
                                    c.running -> Pill(stringResource(R.string.ai_running), Galaxy.colors.riskNormal)
                                    else -> Pill(stringResource(modeLabel(c.mode)), modeColor(c.mode))
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    deleting?.let { c ->
        ConfirmDialog(
            title = stringResource(R.string.ai_delete),
            message = stringResource(R.string.ai_delete_message),
            confirmText = stringResource(R.string.delete),
            destructive = true,
            onConfirm = { onDelete(c.id) },
            onDismiss = { deleting = null },
        )
    }
}

/** 新对话的选择。 */
data class NewConversationDraft(
    val modelKey: String? = null,
    val targets: Set<String> = emptySet(),
    val mode: AiMode = AiMode.CONFIRM_DANGEROUS,
    val planFirst: Boolean = false,
    val trustMinutes: UInt? = 60u,
)

fun AiModel.key() = "$providerId/$id"

@Composable
fun NewConversationScreen(
    models: AiModels?,
    modelsError: String?,
    targets: List<HostItem>,
    draft: NewConversationDraft,
    onDraft: (NewConversationDraft) -> Unit,
    onStart: () -> Unit,
    onBack: () -> Unit,
    showBack: Boolean = true,
) {
    val maxMode = models?.maxMode ?: AiMode.FULL_TRUST
    val canStart = draft.modelKey != null && draft.targets.isNotEmpty()
    Scaffold(
        topBar = { GalaxyTopBar(stringResource(R.string.ai_new), onBack = if (showBack) onBack else null) },
        bottomBar = {
            Surface(color = MaterialTheme.colorScheme.surface, tonalElevation = 2.dp) {
                Box(Modifier.fillMaxWidth().navigationBarsPadding().padding(16.dp), contentAlignment = Alignment.Center) {
                    Button(onClick = onStart, enabled = canStart, modifier = Modifier.widthIn(max = 480.dp).fillMaxWidth().height(48.dp)) {
                        Text(if (draft.targets.isEmpty()) stringResource(R.string.ai_targets_required) else stringResource(R.string.ai_start))
                    }
                }
            }
        },
    ) { pad ->
        LazyColumn(Modifier.padding(pad).fillMaxSize(), contentPadding = PaddingValues(bottom = 16.dp)) {
            item("m") {
                SectionHeader(stringResource(R.string.ai_model)) { InfoButton(stringResource(R.string.ai_vision_info)) }
            }
            if (models == null) {
                item("m-loading") {
                    Text(modelsError ?: stringResource(R.string.loading), color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(16.dp))
                }
            }
            items(models?.items.orEmpty(), key = { "model-" + it.key() }) { m ->
                val selected = draft.modelKey == m.key()
                Row(
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 12.dp, vertical = 3.dp)
                        .clip(RoundedCornerShape(12.dp))
                        .border(1.dp, if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(12.dp))
                        .clickable { onDraft(draft.copy(modelKey = m.key())) }
                        .padding(horizontal = 8.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RadioButton(selected = selected, onClick = { onDraft(draft.copy(modelKey = m.key())) })
                    Column(Modifier.weight(1f)) {
                        Text(m.name, style = MaterialTheme.typography.bodyLarge)
                        Text(
                            m.providerName + "  ·  " + stringResource(R.string.ai_context, (m.contextLength / 1000u).toInt()),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Pill(
                        stringResource(if (m.vision) R.string.ai_vision else R.string.ai_text_only),
                        if (m.vision) Galaxy.colors.online else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            item("t") { SectionHeader(stringResource(R.string.ai_targets)) }
            items(targets, key = { "target-" + it.key }) { t ->
                val checked = t.key in draft.targets
                Row(
                    Modifier
                        .fillMaxWidth()
                        .clickable { onDraft(draft.copy(targets = if (checked) draft.targets - t.key else draft.targets + t.key)) }
                        .padding(horizontal = 16.dp, vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    val (icon, status) = avatarFor(t)
                    EntryAvatar(icon, status = status, size = 36.dp)
                    Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f)) {
                        Text(t.title, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(
                            when (t) {
                                is HostItem.Device -> t.device.osVersion
                                is HostItem.Ssh -> t.address
                                is HostItem.Rdp -> t.address
                            },
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            fontFamily = if (t is HostItem.Ssh) FontFamily.Monospace else null,
                        )
                    }
                    Checkbox(checked = checked, onCheckedChange = { onDraft(draft.copy(targets = if (it) draft.targets + t.key else draft.targets - t.key)) })
                }
            }
            item("mode") { SectionHeader(stringResource(R.string.ai_mode)) { InfoButton(stringResource(R.string.ai_mode_info)) } }
            items(SELECTABLE_MODES, key = { "mode-" + it.name }) { m ->
                val allowed = m.ordinal <= maxMode.ordinal
                Row(
                    Modifier
                        .fillMaxWidth()
                        .alpha(if (allowed) 1f else 0.4f)
                        .clickable(enabled = allowed) { onDraft(draft.copy(mode = m)) }
                        .padding(horizontal = 8.dp, vertical = 2.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RadioButton(selected = draft.mode == m, onClick = { onDraft(draft.copy(mode = m)) }, enabled = allowed)
                    Text(stringResource(modeLabel(m)), style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
                    Box(Modifier.size(10.dp).clip(RoundedCornerShape(5.dp)).background(modeColor(m)))
                    Spacer(Modifier.width(12.dp))
                }
            }
            if (draft.mode == AiMode.FULL_TRUST) {
                item("trust") {
                    Row(
                        Modifier.fillMaxWidth().padding(start = 56.dp, end = 16.dp, top = 2.dp, bottom = 6.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(stringResource(R.string.ai_trust_for), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        listOf<Pair<UInt?, Int>>(15u to R.string.trust_15, 60u to R.string.trust_60, null to R.string.trust_unlimited).forEach { (v, l) ->
                            FilterChip(selected = draft.trustMinutes == v, onClick = { onDraft(draft.copy(trustMinutes = v)) }, label = { Text(stringResource(l)) })
                        }
                    }
                }
            }
            item("plan") {
                Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(stringResource(R.string.ai_plan_first), style = MaterialTheme.typography.bodyLarge)
                    InfoButton(stringResource(R.string.ai_plan_first_info))
                    Spacer(Modifier.weight(1f))
                    Switch(checked = draft.planFirst, onCheckedChange = { onDraft(draft.copy(planFirst = it)) })
                }
            }
        }
    }
}
