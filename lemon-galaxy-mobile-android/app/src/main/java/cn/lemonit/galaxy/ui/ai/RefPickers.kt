package cn.lemonit.galaxy.ui.ai

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.ArrowUpward
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Code
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.Dns
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.History
import androidx.compose.material.icons.rounded.Lan
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.MentionCandidate
import cn.lemonit.galaxy.app.Refs
import cn.lemonit.galaxy.app.SELECTABLE_MODES
import cn.lemonit.galaxy.app.TargetRef
import cn.lemonit.galaxy.app.filterCandidates
import cn.lemonit.galaxy.app.groupModels
import cn.lemonit.galaxy.app.modeAllowed
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.AiModel
import cn.lemonit.galaxy.core.bridge.ContextRef
import cn.lemonit.galaxy.core.bridge.ContextRefKind
import cn.lemonit.galaxy.core.bridge.FileEntry
import cn.lemonit.galaxy.core.bridge.FileKind
import cn.lemonit.galaxy.ui.components.InfoButton
import cn.lemonit.galaxy.ui.components.Pill
import cn.lemonit.galaxy.ui.theme.Galaxy

@Composable
private fun SheetTitle(text: String, trailing: @Composable () -> Unit = {}) {
    Row(Modifier.fillMaxWidth().padding(start = 24.dp, end = 12.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(text, style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
        trailing()
    }
}

@Composable
private fun MenuRow(icon: ImageVector?, text: String, enabled: Boolean = true, selected: Boolean = false, trailing: @Composable () -> Unit = {}, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 52.dp)
            .alpha(if (enabled) 1f else 0.38f)
            .clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = 24.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            Icon(icon, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(22.dp))
            Spacer(Modifier.width(16.dp))
        }
        Text(text, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
        trailing()
        if (selected) {
            Spacer(Modifier.width(8.dp))
            Icon(Icons.Rounded.Check, null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(20.dp))
        }
    }
}

/** 「+」菜单：设备或主机 / 文件或文件夹 / 命令片段 / 端口映射 / 连接记录。 */
@Composable
fun PlusMenu(onTargets: () -> Unit, onFiles: () -> Unit, onSnippets: () -> Unit, onForwards: () -> Unit, onActivity: () -> Unit) {
    Column(Modifier.navigationBarsPadding().padding(bottom = 8.dp)) {
        MenuRow(Icons.Rounded.Dns, stringResource(R.string.ai_ctx_targets), onClick = onTargets)
        MenuRow(Icons.Rounded.Folder, stringResource(R.string.ai_ctx_files), onClick = onFiles)
        MenuRow(Icons.Rounded.Code, stringResource(R.string.ai_ctx_snippets), onClick = onSnippets)
        MenuRow(Icons.Rounded.Lan, stringResource(R.string.ai_ctx_forwards), onClick = onForwards)
        MenuRow(Icons.Rounded.History, stringResource(R.string.ai_ctx_activity), onClick = onActivity)
    }
}

/** 模型菜单：按供应商分组，能看图的带「看图」标记。 */
@Composable
fun ModelMenu(models: List<AiModel>, selected: AiModel?, onPick: (AiModel) -> Unit) {
    LazyColumn(Modifier.navigationBarsPadding().padding(bottom = 8.dp)) {
        item { SheetTitle(stringResource(R.string.ai_model)) }
        groupModels(models).forEach { (provider, list) ->
            item("p-$provider") {
                Text(
                    provider, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(start = 24.dp, top = 8.dp, bottom = 4.dp),
                )
            }
            items(list, key = { "m-${it.providerId}/${it.id}" }) { m ->
                MenuRow(
                    null, m.name,
                    selected = selected?.providerId == m.providerId && selected.id == m.id,
                    trailing = { if (m.vision) Pill(stringResource(R.string.ai_vision), Galaxy.colors.online) },
                ) { onPick(m) }
            }
        }
    }
}

/** 档位菜单：四项（高于管理员上限的置灰），底部「先出计划」勾选。 */
@Composable
fun ModeMenu(mode: AiMode, maxMode: AiMode, planFirst: Boolean, planFirstEditable: Boolean, onMode: (AiMode) -> Unit, onPlanFirst: (Boolean) -> Unit) {
    Column(Modifier.navigationBarsPadding().padding(bottom = 8.dp)) {
        SheetTitle(stringResource(R.string.ai_mode)) { InfoButton(stringResource(R.string.ai_mode_info)) }
        SELECTABLE_MODES.forEach { m ->
            MenuRow(modeIcon(m), stringResource(modeLabel(m)), enabled = modeAllowed(m, maxMode), selected = m == mode) { onMode(m) }
        }
        HorizontalDivider(Modifier.padding(vertical = 4.dp), color = MaterialTheme.colorScheme.outlineVariant)
        Row(
            Modifier
                .fillMaxWidth()
                .heightIn(min = 52.dp)
                .alpha(if (planFirstEditable) 1f else 0.38f)
                .clickable(enabled = planFirstEditable) { onPlanFirst(!planFirst) }
                .padding(start = 12.dp, end = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Checkbox(checked = planFirst, onCheckedChange = { onPlanFirst(it) }, enabled = planFirstEditable)
            Text(stringResource(R.string.ai_plan_first), style = MaterialTheme.typography.bodyLarge)
            InfoButton(stringResource(if (planFirstEditable) R.string.ai_plan_first_info else R.string.ai_plan_first_locked_info))
        }
    }
}

/** 完全信任的时限：15 分钟 / 1 小时 / 本对话（null）。 */
@Composable
fun TrustMenu(onPick: (UInt?) -> Unit) {
    Column(Modifier.navigationBarsPadding().padding(bottom = 8.dp)) {
        SheetTitle(stringResource(R.string.ai_trust_for))
        MenuRow(null, stringResource(R.string.trust_15)) { onPick(15u) }
        MenuRow(null, stringResource(R.string.trust_60)) { onPick(60u) }
        MenuRow(null, stringResource(R.string.trust_conversation)) { onPick(null) }
    }
}

/** 列表多选（设备或主机、命令片段、端口映射）。已经引用的默认勾上。 */
@Composable
fun MultiPickSheet(title: String, items: List<MentionCandidate>, existing: List<ContextRef>, showStatus: Boolean = false, onAdd: (List<ContextRef>) -> Unit) {
    var query by remember { mutableStateOf("") }
    var picked by remember { mutableStateOf(items.filter { c -> existing.any { it.kind == c.ref.kind && it.id == c.ref.id } }.map { it.ref }.toSet()) }
    val shown = filterCandidates(items, query)
    Column(Modifier.navigationBarsPadding().padding(bottom = 12.dp)) {
        SheetTitle(title)
        if (items.size > 8) {
            OutlinedTextField(
                query, { query = it },
                placeholder = { Text(stringResource(R.string.search)) },
                leadingIcon = { Icon(Icons.Rounded.Search, null) },
                singleLine = true,
                shape = RoundedCornerShape(12.dp),
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
            )
        }
        if (items.isEmpty()) {
            Text(
                stringResource(R.string.ai_pick_empty), color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.fillMaxWidth().padding(24.dp),
            )
        }
        LazyColumn(Modifier.heightIn(max = 420.dp)) {
            items(shown, key = { it.ref.kind.name + it.ref.id }) { c ->
                val on = c.ref in picked
                CandidateRow(c, onClick = { picked = if (on) picked - c.ref else picked + c.ref }, showStatus = showStatus) {
                    Checkbox(checked = on, onCheckedChange = { picked = if (it) picked + c.ref else picked - c.ref })
                }
            }
        }
        Button(
            onClick = { onAdd(items.map { it.ref }.filter { it in picked }) },
            enabled = picked.isNotEmpty(),
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).height(48.dp),
        ) { Text(stringResource(R.string.ai_pick_add)) }
    }
}

/** 文件或文件夹：先选设备或主机，再浏览目录，选这个文件或当前文件夹。 */
@Composable
fun FilePickSheet(sources: RefSources, onPick: (ContextRef) -> Unit) {
    var target by remember { mutableStateOf<MentionCandidate?>(null) }
    val t = target
    Column(Modifier.fillMaxWidth().fillMaxHeight(0.85f).navigationBarsPadding()) {
        if (t == null || sources.browser == null) {
            SheetTitle(stringResource(R.string.ai_ctx_files))
            Text(
                stringResource(R.string.ai_pick_place), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(start = 24.dp, bottom = 4.dp),
            )
            LazyColumn(Modifier.weight(1f)) {
                items(sources.targets, key = { it.ref.kind.name + it.ref.id }) { c ->
                    Box(Modifier.alpha(if (c.online) 1f else 0.38f)) {
                        CandidateRow(c, onClick = { if (c.online) target = c })
                    }
                }
            }
        } else {
            val tr = if (t.ref.kind == ContextRefKind.DEVICE) TargetRef.Device(t.ref.id) else TargetRef.Ssh(t.ref.id)
            FileBrowserPane(sources, tr, t.ref.label, onBack = { target = null }, onPick = onPick)
        }
    }
}

@Composable
private fun ColumnScope.FileBrowserPane(sources: RefSources, target: TargetRef, placeName: String, onBack: () -> Unit, onPick: (ContextRef) -> Unit) {
    val browser = remember(target) { sources.browser!!(target).also { it.start() } }
    val st by browser.state.collectAsState()
    var chosen by remember(st.path) { mutableStateOf<FileEntry?>(null) }
    Column(Modifier.fillMaxWidth()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, stringResource(R.string.back)) }
            Column(Modifier.weight(1f)) {
                Text(placeName, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(st.path, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            IconButton(onClick = { browser.up() }, enabled = st.path.isNotEmpty() && st.path != "/") {
                Icon(Icons.Rounded.ArrowUpward, stringResource(R.string.files_up))
            }
        }
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
    }
    Box(Modifier.fillMaxWidth().weight(1f)) {
        when {
            st.loading -> CircularProgressIndicator(Modifier.align(Alignment.Center).size(28.dp))
            st.error != null -> Column(Modifier.align(Alignment.Center).padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Text(st.error!!, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium)
                Spacer(Modifier.height(8.dp))
                OutlinedButton(onClick = { browser.refresh() }) { Text(stringResource(R.string.retry)) }
            }
            else -> LazyColumn(Modifier.fillMaxWidth()) {
                items(st.visible, key = { it.path }) { e ->
                    val dir = e.kind == FileKind.DIR
                    val sel = chosen?.path == e.path
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .background(if (sel) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surface.copy(alpha = 0f))
                            .clickable { if (dir) browser.open(e.path) else chosen = e }
                            .padding(horizontal = 24.dp, vertical = 12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon(if (dir) Icons.Rounded.Folder else Icons.Rounded.Description, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(20.dp))
                        Spacer(Modifier.width(14.dp))
                        Text(e.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                        if (sel) Icon(Icons.Rounded.Check, null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(20.dp))
                    }
                }
            }
        }
    }
    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedButton(
            onClick = { onPick(Refs.path(target, st.path, folder = true)) },
            enabled = st.path.isNotEmpty() && !st.loading,
            modifier = Modifier.weight(1f).height(48.dp),
        ) { Text(stringResource(R.string.ai_pick_folder), maxLines = 1, overflow = TextOverflow.Ellipsis) }
        Button(
            onClick = { chosen?.let { onPick(Refs.path(target, it.path, folder = false)) } },
            enabled = chosen != null,
            modifier = Modifier.weight(1f).height(48.dp),
        ) { Text(stringResource(R.string.ai_pick_file), maxLines = 1, overflow = TextOverflow.Ellipsis) }
    }
}

