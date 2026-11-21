package cn.lemonit.galaxy.ui.hosts

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.StarOutline
import androidx.compose.material.icons.rounded.AutoAwesome
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.automirrored.rounded.DriveFileMove
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.FolderOpen
import androidx.compose.material.icons.rounded.Route
import androidx.compose.material.icons.rounded.Star
import androidx.compose.material.icons.rounded.Terminal
import androidx.compose.material.icons.rounded.Tv
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.HostItem
import cn.lemonit.galaxy.app.HostsData
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.VaultData
import cn.lemonit.galaxy.ui.components.EntryAvatar
import cn.lemonit.galaxy.ui.components.InfoButton
import cn.lemonit.galaxy.ui.components.Tag
import cn.lemonit.galaxy.ui.theme.Galaxy

enum class HostAction { REMOTE, TERMINAL, FILES, AI, EDIT, FAVORITE, MOVE, DELETE }

/** 设备有没有图形界面（能远程桌面）。 */
fun HostItem.Device.hasDisplay() = device.platform == Platform.MACOS || device.platform == Platform.WINDOWS
fun HostItem.Device.reachable() = device.online || device.lanSeen

/**
 * SSH 主机的连接路径文字：直连 / 经设备 X / 经 SSH 主机 Y。
 * 有直连会话时用内核报告的 [liveVia]（direct、device:<名>、ssh:<名>），否则按条目里配置的跳转推断。
 */
@Composable
fun sshViaText(data: HostsData, host: VaultData.SshHost, liveVia: String?): String {
    val (kind, name) = when {
        liveVia != null -> when {
            liveVia.startsWith("device:") -> "device" to liveVia.removePrefix("device:")
            liveVia.startsWith("ssh:") -> "ssh" to liveVia.removePrefix("ssh:")
            else -> "direct" to ""
        }
        !host.jumpDeviceId.isNullOrEmpty() -> "device" to data.deviceName(host.jumpDeviceId)
        !host.jumpHostId.isNullOrEmpty() -> "ssh" to ((data.entry(host.jumpHostId)?.data as? VaultData.SshHost)?.label ?: host.jumpHostId)
        else -> "direct" to ""
    }
    return when (kind) {
        "device" -> stringResource(R.string.ssh_via_device, name)
        "ssh" -> stringResource(R.string.ssh_via_host, name)
        else -> stringResource(R.string.ssh_via_direct)
    }
}

/**
 * 点设备 / 主机弹出的操作面板内容（底部弹层与平板详情栏共用）。
 * [sshVia] 是 SSH 主机的连接路径（见 [sshViaText]），[sshConnected] 表示当前有这台主机的直连会话。
 */
@Composable
fun HostActionsContent(
    item: HostItem,
    onAction: (HostAction) -> Unit,
    modifier: Modifier = Modifier,
    sshVia: String? = null,
    sshConnected: Boolean = false,
) {
    Column(modifier.fillMaxWidth().padding(bottom = 16.dp)) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            val (icon, status) = avatarFor(item)
            EntryAvatar(icon, status = status, size = 48.dp)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text(item.title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(
                    subtitleFor(item),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 2,
                    fontFamily = if (item is HostItem.Device) null else FontFamily.Monospace,
                )
            }
        }
        if (item is HostItem.Device) {
            Row(Modifier.padding(horizontal = 20.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Tag(item.device.hostname)
                Tag(item.device.arch)
                Tag("v" + item.device.appVersion)
                item.tags.forEach { Tag("#$it") }
            }
        } else if (item.tags.isNotEmpty()) {
            Row(Modifier.padding(horizontal = 20.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                item.tags.forEach { Tag("#$it") }
            }
        }
        if (item is HostItem.Ssh && sshVia != null) {
            Row(Modifier.padding(horizontal = 20.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Rounded.Route, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(16.dp))
                Spacer(Modifier.width(8.dp))
                Text(
                    stringResource(R.string.ssh_via) + "  " + sshVia,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.weight(1f, fill = false),
                )
                if (sshConnected) {
                    Spacer(Modifier.width(8.dp))
                    Tag(stringResource(R.string.ssh_connected), color = Galaxy.colors.online)
                }
            }
        }
        Spacer(Modifier.height(12.dp))
        when (item) {
            is HostItem.Device -> {
                val ok = item.reachable()
                Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    if (item.hasDisplay()) ActionTile(Icons.Rounded.Tv, stringResource(R.string.action_remote_desktop), ok) { onAction(HostAction.REMOTE) }
                    ActionTile(Icons.Rounded.Terminal, stringResource(R.string.action_terminal), ok) { onAction(HostAction.TERMINAL) }
                    ActionTile(Icons.Rounded.FolderOpen, stringResource(R.string.action_files), ok) { onAction(HostAction.FILES) }
                    ActionTile(Icons.Rounded.AutoAwesome, stringResource(R.string.action_ai), ok) { onAction(HostAction.AI) }
                }
                if (!ok) {
                    Text(
                        stringResource(R.string.device_offline),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 20.dp, vertical = 8.dp),
                    )
                }
            }
            is HostItem.Ssh -> Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                ActionTile(Icons.Rounded.Terminal, stringResource(R.string.action_terminal), true) { onAction(HostAction.TERMINAL) }
                ActionTile(Icons.Rounded.FolderOpen, stringResource(R.string.action_files), true) { onAction(HostAction.FILES) }
                ActionTile(Icons.Rounded.AutoAwesome, stringResource(R.string.action_ai), true) { onAction(HostAction.AI) }
            }
            is HostItem.Rdp -> Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                ActionTile(Icons.Rounded.Tv, stringResource(R.string.action_remote_desktop), false) {}
                Spacer(Modifier.width(4.dp))
                InfoButton(stringResource(R.string.rdp_unsupported))
            }
        }
        Spacer(Modifier.height(12.dp))
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        SheetRow(Icons.Outlined.Edit, stringResource(R.string.edit)) { onAction(HostAction.EDIT) }
        SheetRow(
            if (item.favorite) Icons.Rounded.Star else Icons.Outlined.StarOutline,
            stringResource(if (item.favorite) R.string.action_unfavorite else R.string.action_favorite),
        ) { onAction(HostAction.FAVORITE) }
        SheetRow(Icons.AutoMirrored.Rounded.DriveFileMove, stringResource(R.string.action_move)) { onAction(HostAction.MOVE) }
        SheetRow(
            Icons.Rounded.Delete,
            stringResource(if (item is HostItem.Device) R.string.action_remove_device else R.string.delete),
            color = MaterialTheme.colorScheme.error,
        ) { onAction(HostAction.DELETE) }
    }
}

@Composable
private fun androidx.compose.foundation.layout.RowScope.ActionTile(icon: ImageVector, label: String, enabled: Boolean, onClick: () -> Unit) {
    Column(
        Modifier
            .weight(1f)
            .clip(RoundedCornerShape(14.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .clickable(enabled = enabled, onClick = onClick)
            .alpha(if (enabled) 1f else 0.4f)
            .padding(vertical = 14.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(icon, null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(24.dp))
        Spacer(Modifier.height(6.dp))
        Text(label, style = MaterialTheme.typography.labelMedium, maxLines = 1, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Center)
    }
}

@Composable
fun SheetRow(icon: ImageVector, text: String, color: Color = MaterialTheme.colorScheme.onSurface, onClick: () -> Unit) {
    ListItem(
        headlineContent = { Text(text, color = color) },
        leadingContent = { Icon(icon, null, tint = color) },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = Modifier.clickable(onClick = onClick).heightIn(min = 48.dp),
    )
}

/** 移动到文件夹：列出所有文件夹（缩进表示层级），排除自身与子孙。 */
@Composable
fun MoveDialog(data: HostsData, excluded: Set<String>, current: String?, onPick: (String?) -> Unit, onDismiss: () -> Unit) {
    val rows = remember(data) {
        val out = mutableListOf<Pair<Int, cn.lemonit.galaxy.core.bridge.VaultEntry>>()
        fun walk(parent: String?, depth: Int) {
            data.folders.filter { it.folderId == parent }.sortedBy { it.sort }.forEach {
                out.add(depth to it)
                walk(it.id, depth + 1)
            }
        }
        walk(null, 0)
        out
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.move_title)) },
        text = {
            LazyColumn(Modifier.heightIn(max = 360.dp)) {
                item {
                    FolderPickRow(0, stringResource(R.string.move_root), current == null, true) { onPick(null) }
                }
                items(rows, key = { it.second.id }) { (depth, e) ->
                    FolderPickRow(depth + 1, (e.data as VaultData.Folder).name, current == e.id, e.id !in excluded) { onPick(e.id) }
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) } },
    )
}

@Composable
private fun FolderPickRow(depth: Int, name: String, selected: Boolean, enabled: Boolean, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(8.dp))
            .background(if (selected) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent)
            .clickable(enabled = enabled && !selected, onClick = onClick)
            .alpha(if (enabled) 1f else 0.4f)
            .padding(start = 8.dp + 18.dp * depth, top = 10.dp, bottom = 10.dp, end = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Rounded.Folder, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(20.dp))
        Spacer(Modifier.width(10.dp))
        Text(name, style = MaterialTheme.typography.bodyLarge)
    }
}

/** 临时协助：输入设备码与验证码。 */
@Composable
fun AssistDialog(onConnect: (String, String) -> Unit, onDismiss: () -> Unit) {
    var code by remember { mutableStateOf("") }
    var verify by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.assist_title))
                InfoButton(stringResource(R.string.assist_info))
            }
        },
        text = {
            Column {
                OutlinedTextField(
                    code, { v -> code = v.filter { it.isDigit() || it == ' ' }.take(11) },
                    label = { Text(stringResource(R.string.assist_device_code)) }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                    textStyle = MaterialTheme.typography.bodyLarge.copy(fontFamily = FontFamily.Monospace),
                )
                Spacer(Modifier.height(8.dp))
                OutlinedTextField(
                    verify, { verify = it.trim() }, label = { Text(stringResource(R.string.assist_verification)) }, singleLine = true,
                    textStyle = MaterialTheme.typography.bodyLarge.copy(fontFamily = FontFamily.Monospace),
                )
            }
        },
        confirmButton = {
            TextButton(enabled = code.count { it.isDigit() } == 9 && verify.length >= 4, onClick = {
                onConnect(code.filter { it.isDigit() }, verify)
                onDismiss()
            }) { Text(stringResource(R.string.assist_connect)) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) } },
    )
}
