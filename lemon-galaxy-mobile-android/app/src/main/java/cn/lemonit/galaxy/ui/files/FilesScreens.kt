package cn.lemonit.galaxy.ui.files

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
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
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.InsertDriveFile
import androidx.compose.material.icons.automirrored.rounded.Sort
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.rounded.Archive
import androidx.compose.material.icons.rounded.ArrowUpward
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.CreateNewFolder
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.Download
import androidx.compose.material.icons.rounded.DriveFileRenameOutline
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.FolderOff
import androidx.compose.material.icons.rounded.Home
import androidx.compose.material.icons.rounded.Image
import androidx.compose.material.icons.rounded.Link
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.MoreVert
import androidx.compose.material.icons.rounded.Pause
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.SwapVert
import androidx.compose.material.icons.rounded.Terminal
import androidx.compose.material.icons.rounded.Upload
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.HostItem
import cn.lemonit.galaxy.app.TransferDirection
import cn.lemonit.galaxy.app.TransferItem
import cn.lemonit.galaxy.app.TransferState
import cn.lemonit.galaxy.core.bridge.FileEntry
import cn.lemonit.galaxy.core.bridge.FileKind
import cn.lemonit.galaxy.ui.components.EmptyState
import cn.lemonit.galaxy.ui.components.EntryAvatar
import cn.lemonit.galaxy.ui.components.GalaxyTopBar
import cn.lemonit.galaxy.ui.components.SectionHeader
import cn.lemonit.galaxy.ui.components.formatBytes
import cn.lemonit.galaxy.ui.hosts.SheetRow
import cn.lemonit.galaxy.ui.hosts.avatarFor
import cn.lemonit.galaxy.ui.hosts.subtitleFor
import cn.lemonit.galaxy.ui.theme.Galaxy
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

interface FileBrowserActions {
    fun onBack()
    fun onOpen(entry: FileEntry)
    fun onMenu(entry: FileEntry)
    fun onUp()
    fun onHome()
    fun onRefresh()
    fun onUpload()
    fun onNewFolder()
    fun onSort(key: SortKey)
    fun onToggleHidden()
}

private val dateFmt = SimpleDateFormat("yyyy-MM-dd HH:mm", Locale.US)

fun fileIcon(e: FileEntry): ImageVector = when (e.kind) {
    FileKind.DIR -> Icons.Rounded.Folder
    FileKind.SYMLINK -> Icons.Rounded.Link
    FileKind.OTHER -> Icons.AutoMirrored.Rounded.InsertDriveFile
    FileKind.FILE -> when (e.name.substringAfterLast('.', "").lowercase()) {
        "jpg", "jpeg", "png", "gif", "webp", "heic", "svg" -> Icons.Rounded.Image
        "zip", "gz", "tgz", "xz", "bz2", "7z", "rar", "iso", "tar", "dmg" -> Icons.Rounded.Archive
        "sh", "bash", "zsh", "ps1", "bat" -> Icons.Rounded.Terminal
        "txt", "md", "log", "conf", "json", "yaml", "yml", "toml", "xml", "ini", "rs", "kt", "py", "js", "ts" -> Icons.Rounded.Description
        else -> Icons.AutoMirrored.Rounded.InsertDriveFile
    }
}

@Composable
fun FileBrowserScreen(
    title: String,
    state: FileBrowserState,
    actions: FileBrowserActions,
    showBack: Boolean = true,
) {
    Scaffold(
        topBar = {
            GalaxyTopBar(
                title = title,
                onBack = if (showBack) actions::onBack else null,
                subtitle = {
                    Text(
                        state.path.ifEmpty { "…" },
                        style = MaterialTheme.typography.labelMedium,
                        fontFamily = FontFamily.Monospace,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.StartEllipsis,
                    )
                },
                actions = {
                    IconButton(onClick = actions::onUpload) { Icon(Icons.Rounded.Upload, stringResource(R.string.files_upload)) }
                    IconButton(onClick = actions::onNewFolder) { Icon(Icons.Rounded.CreateNewFolder, stringResource(R.string.files_new_folder)) }
                    SortMenu(state, actions)
                },
            )
        },
    ) { pad ->
        Box(Modifier.padding(pad).fillMaxSize()) {
            LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 24.dp)) {
                item("nav") {
                    Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        NavChip(Icons.Rounded.ArrowUpward, stringResource(R.string.files_up), state.path != "/" && state.path.isNotEmpty(), actions::onUp)
                        NavChip(Icons.Rounded.Home, stringResource(R.string.files_home), state.path != state.home, actions::onHome)
                        NavChip(Icons.Rounded.Refresh, stringResource(R.string.refresh), !state.loading, actions::onRefresh)
                        Spacer(Modifier.weight(1f))
                        Text(
                            stringResource(R.string.files_items, state.visible.size),
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.align(Alignment.CenterVertically).padding(end = 8.dp),
                        )
                    }
                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                }
                if (!state.loading && state.error == null && state.visible.isEmpty()) {
                    item("empty") { EmptyState(Icons.Rounded.FolderOff, stringResource(R.string.files_empty)) }
                }
                state.error?.let { err ->
                    item("error") {
                        EmptyState(Icons.Rounded.FolderOff, err, action = { TextButton(onClick = actions::onRefresh) { Text(stringResource(R.string.retry)) } })
                    }
                }
                items(state.visible, key = { it.path }) { e -> FileRow(e, actions) }
            }
            if (state.loading) {
                LinearProgressIndicator(Modifier.fillMaxWidth().align(Alignment.TopCenter))
            }
        }
    }
}

@Composable
private fun NavChip(icon: ImageVector, label: String, enabled: Boolean, onClick: () -> Unit) {
    TextButton(onClick = onClick, enabled = enabled, contentPadding = PaddingValues(horizontal = 10.dp)) {
        Icon(icon, null, Modifier.size(18.dp))
        Spacer(Modifier.width(4.dp))
        Text(label, style = MaterialTheme.typography.labelLarge)
    }
}

@Composable
private fun SortMenu(state: FileBrowserState, actions: FileBrowserActions) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) { Icon(Icons.AutoMirrored.Rounded.Sort, stringResource(R.string.files_sort)) }
        DropdownMenu(open, { open = false }) {
            listOf(SortKey.NAME to R.string.sort_name, SortKey.SIZE to R.string.sort_size, SortKey.MODIFIED to R.string.sort_modified).forEach { (k, l) ->
                DropdownMenuItem(
                    text = { Text(stringResource(l)) },
                    leadingIcon = { if (state.sort == k) Icon(Icons.Rounded.Check, null) else Spacer(Modifier.size(24.dp)) },
                    trailingIcon = { if (state.sort == k) Icon(Icons.Rounded.SwapVert, stringResource(R.string.sort_descending), tint = if (state.descending) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline) },
                    onClick = { actions.onSort(k) },
                )
            }
            HorizontalDivider()
            DropdownMenuItem(
                text = { Text(stringResource(R.string.files_show_hidden)) },
                leadingIcon = { if (state.showHidden) Icon(Icons.Rounded.Check, null) else Icon(Icons.Rounded.Visibility, null, tint = MaterialTheme.colorScheme.outline) },
                onClick = {
                    actions.onToggleHidden()
                    open = false
                },
            )
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun FileRow(e: FileEntry, actions: FileBrowserActions) {
    val isDir = e.kind == FileKind.DIR
    Row(
        Modifier
            .fillMaxWidth()
            .combinedClickable(onClick = { actions.onOpen(e) }, onLongClick = { actions.onMenu(e) })
            .padding(start = 16.dp, end = 4.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        EntryAvatar(
            fileIcon(e),
            tint = if (isDir) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
            container = if (isDir) MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.6f) else MaterialTheme.colorScheme.surfaceContainerHigh,
            size = 38.dp,
        )
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(
                e.name + (e.linkTarget?.let { " → $it" } ?: ""),
                style = MaterialTheme.typography.bodyLarge,
                maxLines = 1,
                overflow = TextOverflow.MiddleEllipsis,
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    FileBrowserController.permString(e.kind, e.mode),
                    style = MaterialTheme.typography.labelSmall,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    "  " + (if (isDir) "" else formatBytes(e.size.toLong()) + "  ·  ") + dateFmt.format(Date(e.modifiedAt)) + "  ·  " + e.owner,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        IconButton(onClick = { actions.onMenu(e) }) {
            Icon(Icons.Rounded.MoreVert, stringResource(R.string.more), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

/** 文件操作面板内容（底部弹层）。 */
@Composable
fun FileMenuContent(
    e: FileEntry,
    onEdit: (() -> Unit)?,
    onDownload: (() -> Unit)?,
    onRename: () -> Unit,
    onChmod: () -> Unit,
    onDelete: () -> Unit,
) {
    Column(Modifier.fillMaxWidth().padding(bottom = 16.dp)) {
        Row(Modifier.padding(horizontal = 20.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            EntryAvatar(fileIcon(e), size = 44.dp)
            Spacer(Modifier.width(12.dp))
            Column {
                Text(e.name, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
                Text(
                    FileBrowserController.permString(e.kind, e.mode) + "  " + e.owner + ":" + e.group +
                        (if (e.kind == FileKind.FILE) "  ·  " + formatBytes(e.size.toLong()) else ""),
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        if (onEdit != null) SheetRow(Icons.Outlined.Edit, stringResource(R.string.files_edit), onClick = onEdit)
        if (onDownload != null) SheetRow(Icons.Rounded.Download, stringResource(R.string.files_download), onClick = onDownload)
        SheetRow(Icons.Rounded.DriveFileRenameOutline, stringResource(R.string.rename), onClick = onRename)
        SheetRow(Icons.Rounded.Lock, stringResource(R.string.files_permissions), onClick = onChmod)
        SheetRow(Icons.Rounded.Delete, stringResource(R.string.delete), color = MaterialTheme.colorScheme.error, onClick = onDelete)
    }
}

/** 改权限：三组读写执行 + 八进制。 */
@Composable
fun ChmodDialog(entry: FileEntry, onConfirm: (UInt) -> Unit, onDismiss: () -> Unit) {
    var mode by remember { mutableStateOf(entry.mode and 0x1FFu) }
    var octal by remember { mutableStateOf(mode.toString(8).padStart(3, '0')) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.chmod_title)) },
        text = {
            Column {
                Text(entry.name, style = MaterialTheme.typography.bodyMedium, fontFamily = FontFamily.Monospace, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.height(12.dp))
                Row {
                    Spacer(Modifier.width(72.dp))
                    listOf(R.string.chmod_read, R.string.chmod_write, R.string.chmod_exec).forEach {
                        Text(stringResource(it), style = MaterialTheme.typography.labelMedium, modifier = Modifier.width(56.dp), textAlign = TextAlign.Center)
                    }
                }
                listOf(R.string.chmod_owner to 6, R.string.chmod_group to 3, R.string.chmod_others to 0).forEach { (label, shift) ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(stringResource(label), modifier = Modifier.width(72.dp), style = MaterialTheme.typography.bodyMedium)
                        (2 downTo 0).forEach { b ->
                            val bit = 1u shl (shift + b)
                            Box(Modifier.width(56.dp), contentAlignment = Alignment.Center) {
                                Checkbox(checked = mode and bit != 0u, onCheckedChange = { on ->
                                    mode = if (on) mode or bit else mode and bit.inv()
                                    octal = mode.toString(8).padStart(3, '0')
                                })
                            }
                        }
                    }
                }
                Spacer(Modifier.height(8.dp))
                OutlinedTextField(
                    value = octal,
                    onValueChange = { v ->
                        val t = v.filter { it in '0'..'7' }.take(4)
                        octal = t
                        t.toUIntOrNull(8)?.let { mode = it and 0xFFFu }
                    },
                    label = { Text(stringResource(R.string.chmod_octal)) },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                    textStyle = MaterialTheme.typography.bodyLarge.copy(fontFamily = FontFamily.Monospace),
                    modifier = Modifier.width(140.dp),
                )
            }
        },
        confirmButton = {
            TextButton(onClick = {
                onConfirm(mode)
                onDismiss()
            }) { Text(stringResource(R.string.save)) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) } },
    )
}

/** 文本文件查看与编辑。 */
@Composable
fun FileEditorScreen(
    name: String,
    path: String,
    text: String?,
    error: String?,
    saving: Boolean,
    onChange: (String) -> Unit,
    dirty: Boolean,
    onSave: () -> Unit,
    onBack: () -> Unit,
) {
    var confirmDiscard by remember { mutableStateOf(false) }
    Scaffold(
        topBar = {
            GalaxyTopBar(
                title = name,
                onBack = { if (dirty) confirmDiscard = true else onBack() },
                subtitle = {
                    Text(path, style = MaterialTheme.typography.labelMedium, fontFamily = FontFamily.Monospace, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.StartEllipsis)
                },
                actions = {
                    if (saving) {
                        CircularProgressIndicator(Modifier.size(20.dp).padding(end = 4.dp), strokeWidth = 2.dp)
                        Spacer(Modifier.width(12.dp))
                    } else {
                        TextButton(onClick = onSave, enabled = dirty && text != null) { Text(stringResource(R.string.save)) }
                    }
                },
            )
        },
    ) { pad ->
        Box(Modifier.padding(pad).fillMaxSize().imePadding().background(Galaxy.colors.codeBackground)) {
            when {
                error != null -> EmptyState(Icons.Rounded.Description, error)
                text == null -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                else -> BasicTextField(
                    value = text,
                    onValueChange = onChange,
                    textStyle = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace, color = MaterialTheme.colorScheme.onSurface),
                    cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii, autoCorrectEnabled = false),
                    modifier = Modifier.fillMaxSize().padding(12.dp),
                )
            }
        }
    }
    if (confirmDiscard) {
        AlertDialog(
            onDismissRequest = { confirmDiscard = false },
            title = { Text(stringResource(R.string.editor_discard_title)) },
            confirmButton = {
                TextButton(onClick = {
                    confirmDiscard = false
                    onBack()
                }) { Text(stringResource(R.string.editor_discard), color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { confirmDiscard = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}

interface TransferActions {
    fun onPause(id: Long)
    fun onResume(id: Long)
    fun onCancel(id: Long)
    fun onRetry(id: Long)
    fun onClearFinished()
}

@Composable
fun TransferRow(t: TransferItem, actions: TransferActions) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        val (icon, tint) = when (t.direction) {
            TransferDirection.DOWNLOAD -> Icons.Rounded.Download to MaterialTheme.colorScheme.primary
            TransferDirection.UPLOAD -> Icons.Rounded.Upload to Galaxy.colors.online
        }
        EntryAvatar(icon, tint = tint, size = 38.dp)
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(t.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
            val status = when (val s = t.state) {
                TransferState.Queued -> stringResource(R.string.transfer_queued)
                TransferState.Running -> formatBytes(t.done) + " / " + formatBytes(t.total) + (if (t.bytesPerSecond > 0) "  ·  " + formatBytes(t.bytesPerSecond) + "/s" else "")
                TransferState.Paused -> stringResource(R.string.transfer_paused) + "  ·  " + formatBytes(t.done) + " / " + formatBytes(t.total)
                TransferState.Done -> stringResource(R.string.transfer_done) + "  ·  " + formatBytes(t.total)
                TransferState.Cancelled -> stringResource(R.string.transfer_cancelled)
                is TransferState.Failed -> stringResource(R.string.transfer_failed, s.message)
            }
            val dir = stringResource(if (t.direction == TransferDirection.DOWNLOAD) R.string.transfer_from else R.string.transfer_to, t.targetName)
            Text(
                "$status  ·  $dir",
                style = MaterialTheme.typography.bodySmall,
                color = if (t.state is TransferState.Failed) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (!t.state.finished) {
                Spacer(Modifier.height(6.dp))
                LinearProgressIndicator(
                    progress = { t.progress },
                    modifier = Modifier.fillMaxWidth().height(4.dp).clip(RoundedCornerShape(2.dp)),
                    color = if (t.state == TransferState.Paused) MaterialTheme.colorScheme.outline else MaterialTheme.colorScheme.primary,
                    trackColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                    drawStopIndicator = {},
                )
            }
        }
        Spacer(Modifier.width(4.dp))
        when (t.state) {
            TransferState.Running, TransferState.Queued -> {
                IconButton(onClick = { actions.onPause(t.id) }) { Icon(Icons.Rounded.Pause, stringResource(R.string.transfer_pause)) }
                IconButton(onClick = { actions.onCancel(t.id) }) { Icon(Icons.Rounded.Close, stringResource(R.string.transfer_cancel)) }
            }
            TransferState.Paused -> {
                IconButton(onClick = { actions.onResume(t.id) }) { Icon(Icons.Rounded.PlayArrow, stringResource(R.string.transfer_resume)) }
                IconButton(onClick = { actions.onCancel(t.id) }) { Icon(Icons.Rounded.Close, stringResource(R.string.transfer_cancel)) }
            }
            is TransferState.Failed, TransferState.Cancelled -> IconButton(onClick = { actions.onRetry(t.id) }) {
                Icon(Icons.Rounded.Refresh, stringResource(R.string.transfer_retry))
            }
            TransferState.Done -> Icon(Icons.Rounded.Check, null, tint = Galaxy.colors.success, modifier = Modifier.padding(12.dp))
        }
    }
}

/** 「文件」标签页：选择要浏览的设备 / 主机 + 传输队列。 */
@Composable
fun FilesHomeScreen(
    targets: List<HostItem>,
    transfers: List<TransferItem>,
    onBrowse: (HostItem) -> Unit,
    actions: TransferActions,
    selectedKey: String? = null,
) {
    Scaffold(topBar = { GalaxyTopBar(stringResource(R.string.files_tab_title)) }) { pad ->
        LazyColumn(Modifier.padding(pad).fillMaxSize(), contentPadding = PaddingValues(bottom = 24.dp)) {
            // 有传输任务时放在最上面（手机上一眼看到进度），没有时放在最下面
            if (transfers.isNotEmpty()) {
                item("h2") {
                    SectionHeader(stringResource(R.string.files_transfers)) {
                        if (transfers.any { it.state.finished }) {
                            TextButton(onClick = actions::onClearFinished) { Text(stringResource(R.string.transfer_clear_finished)) }
                        }
                    }
                }
                items(transfers.reversed(), key = { "tr-${it.id}" }) { TransferRow(it, actions) }
            }
            item("h1") { SectionHeader(stringResource(R.string.files_open_on)) }
            if (targets.isEmpty()) item("t-empty") { EmptyState(Icons.Rounded.FolderOff, stringResource(R.string.files_targets_empty)) }
            items(targets, key = { it.key }) { item ->
                Row(
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 8.dp, vertical = 1.dp)
                        .clip(RoundedCornerShape(12.dp))
                        .background(if (item.key == selectedKey) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent)
                        .clickable { onBrowse(item) }
                        .padding(horizontal = 8.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    val (icon, status) = avatarFor(item)
                    EntryAvatar(icon, status = status)
                    Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f)) {
                        Text(item.title, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(
                            subtitleFor(item),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            fontFamily = if (item is HostItem.Ssh) FontFamily.Monospace else null,
                        )
                    }
                    Icon(Icons.Rounded.Folder, null, tint = MaterialTheme.colorScheme.outline)
                }
            }
            if (transfers.isEmpty()) {
                item("h2-empty") { SectionHeader(stringResource(R.string.files_transfers)) }
                item("tr-empty") {
                    Text(
                        stringResource(R.string.transfers_empty),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
                    )
                }
            }
        }
    }
}
