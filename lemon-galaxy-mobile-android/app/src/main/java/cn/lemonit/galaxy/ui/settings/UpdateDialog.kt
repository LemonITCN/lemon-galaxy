package cn.lemonit.galaxy.ui.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.Download
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.UpdateState
import cn.lemonit.galaxy.ui.components.formatBytes
import cn.lemonit.galaxy.ui.theme.Galaxy

interface UpdateActions {
    fun onDownload()
    fun onInstall()
    fun onDismiss()
}

/** 新版本弹框：版本号、更新说明、下载进度、交给系统安装器。 */
@Composable
fun UpdateDialog(state: UpdateState, current: String, actions: UpdateActions) {
    val info = when (state) {
        is UpdateState.Available -> state.info
        is UpdateState.Downloading -> state.info
        is UpdateState.Downloaded -> state.info
        is UpdateState.Failed -> state.info
        else -> null
    } ?: return
    val downloading = state is UpdateState.Downloading
    AlertDialog(
        onDismissRequest = { if (!downloading) actions.onDismiss() },
        properties = DialogProperties(dismissOnClickOutside = !downloading),
        icon = { Icon(Icons.Rounded.Download, null) },
        title = { Text(stringResource(R.string.update_title)) },
        text = {
            Column {
                Text(
                    stringResource(R.string.update_versions, current, info.version) + "  ·  " + formatBytes(info.size.toLong()),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(12.dp))
                Text(stringResource(R.string.update_notes), style = MaterialTheme.typography.labelLarge)
                Spacer(Modifier.height(6.dp))
                Column(
                    Modifier
                        .fillMaxWidth()
                        .heightIn(max = 200.dp)
                        .background(MaterialTheme.colorScheme.surfaceContainerHigh, RoundedCornerShape(10.dp))
                        .verticalScroll(rememberScrollState())
                        .padding(12.dp),
                ) {
                    Text(
                        info.notes.ifBlank { stringResource(R.string.update_no_notes) },
                        style = MaterialTheme.typography.bodyMedium,
                        color = if (info.notes.isBlank()) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
                    )
                }
                Spacer(Modifier.height(12.dp))
                when (state) {
                    is UpdateState.Downloading -> {
                        LinearProgressIndicator(
                            progress = { if (state.total <= 0) 0f else (state.done.toFloat() / state.total).coerceIn(0f, 1f) },
                            modifier = Modifier.fillMaxWidth(),
                        )
                        Spacer(Modifier.height(6.dp))
                        Text(
                            stringResource(R.string.update_downloading, formatBytes(state.done), formatBytes(state.total)),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    is UpdateState.Downloaded -> StatusLine(Icons.Rounded.CheckCircle, Galaxy.colors.online, stringResource(R.string.update_downloaded))
                    is UpdateState.Failed -> StatusLine(Icons.Rounded.Warning, MaterialTheme.colorScheme.error, state.message)
                    else -> Text(stringResource(R.string.update_verified), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        },
        confirmButton = {
            when (state) {
                is UpdateState.Available -> TextButton(onClick = actions::onDownload) { Text(stringResource(R.string.update_download)) }
                is UpdateState.Downloaded -> TextButton(onClick = actions::onInstall) { Text(stringResource(R.string.update_install)) }
                is UpdateState.Failed -> TextButton(onClick = actions::onDownload) { Text(stringResource(R.string.retry)) }
                else -> Unit
            }
        },
        dismissButton = {
            if (!downloading) TextButton(onClick = actions::onDismiss) { Text(stringResource(R.string.update_later)) }
        },
    )
}

@Composable
private fun StatusLine(icon: androidx.compose.ui.graphics.vector.ImageVector, color: androidx.compose.ui.graphics.Color, text: String) {
    Row(verticalAlignment = Alignment.Top) {
        Icon(icon, null, tint = color, modifier = Modifier.size(18.dp))
        Spacer(Modifier.width(8.dp))
        Text(text, style = MaterialTheme.typography.bodySmall, color = color)
    }
}

/** 系统不允许本应用安装应用时：引导去设置里打开「安装未知应用」。 */
@Composable
fun UnknownSourcesDialog(onOpenSettings: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = { Icon(Icons.Rounded.Warning, null) },
        title = { Text(stringResource(R.string.update_unknown_title)) },
        text = { Text(stringResource(R.string.update_unknown_body)) },
        confirmButton = {
            TextButton(onClick = {
                onDismiss()
                onOpenSettings()
            }) { Text(stringResource(R.string.update_unknown_settings)) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) } },
    )
}
