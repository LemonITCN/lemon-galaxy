package cn.lemonit.galaxy.ui.hosts

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.SwapHoriz
import androidx.compose.material.icons.rounded.Tv
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.RdpForward
import cn.lemonit.galaxy.ui.components.InfoButton

/**
 * 经设备转发中的 RDP（主界面底部）：Windows App 连的是本机端口，用完在这里停止转发。
 * 退出登录、应用退出时全部自动关闭。
 */
@Composable
fun RdpForwardBanner(forwards: List<RdpForward>, onStop: (String) -> Unit, onStopAll: () -> Unit, modifier: Modifier = Modifier) {
    if (forwards.isEmpty()) return
    Column(modifier.fillMaxWidth().background(MaterialTheme.colorScheme.secondaryContainer)) {
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp, top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Rounded.SwapHoriz, null, tint = MaterialTheme.colorScheme.onSecondaryContainer, modifier = Modifier.size(18.dp))
            Spacer(Modifier.width(8.dp))
            Text(
                stringResource(R.string.rdp_bar_title),
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
            )
            InfoButton(stringResource(R.string.rdp_bar_info), size = 16.dp)
            Spacer(Modifier.weight(1f))
            if (forwards.size > 1) {
                TextButton(onClick = onStopAll) { Text(stringResource(R.string.rdp_bar_stop_all), color = MaterialTheme.colorScheme.error) }
            }
        }
        forwards.forEach { f ->
            Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(
                        f.label,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSecondaryContainer,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    SelectionContainer {
                        Text(
                            "127.0.0.1:${f.port}",
                            style = MaterialTheme.typography.bodySmall,
                            fontFamily = FontFamily.Monospace,
                            color = MaterialTheme.colorScheme.onSecondaryContainer.copy(alpha = 0.75f),
                        )
                    }
                }
                TextButton(onClick = { onStop(f.id) }) { Text(stringResource(R.string.rdp_bar_stop), color = MaterialTheme.colorScheme.error) }
            }
        }
    }
}

/** 没有能打开 rdp:// 的应用：提示从 Play 商店安装 Windows App。 */
@Composable
fun RdpMissingClientDialog(onInstall: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = { Icon(Icons.Rounded.Tv, null) },
        title = { Text(stringResource(R.string.rdp_noclient_title)) },
        text = { Text(stringResource(R.string.rdp_noclient_body)) },
        confirmButton = {
            TextButton(onClick = {
                onDismiss()
                onInstall()
            }) { Text(stringResource(R.string.rdp_noclient_store)) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) } },
    )
}
