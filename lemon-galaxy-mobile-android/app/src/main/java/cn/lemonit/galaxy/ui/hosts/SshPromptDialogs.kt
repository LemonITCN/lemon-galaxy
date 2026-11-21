package cn.lemonit.galaxy.ui.hosts

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import cn.lemonit.galaxy.R

/**
 * SSH 连接时内核要的输入：密码、私钥口令、键盘交互问题。[echo] 为 false 时遮挡输入。
 * [key] 每个新问题不同，输入框随之清空。
 */
@Composable
fun SecretAskDialog(key: Long, title: String?, prompt: String, echo: Boolean, onSubmit: (String) -> Unit, onCancel: () -> Unit) {
    var value by remember(key) { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onCancel,
        icon = { Icon(Icons.Rounded.Key, null) },
        title = { Text(title ?: stringResource(R.string.secret_prompt_title)) },
        text = {
            Column {
                Text(prompt.trim(), style = MaterialTheme.typography.bodyMedium)
                Spacer(Modifier.height(12.dp))
                OutlinedTextField(
                    value, { value = it },
                    singleLine = true,
                    visualTransformation = if (echo) VisualTransformation.None else PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = if (echo) KeyboardType.Ascii else KeyboardType.Password),
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        },
        confirmButton = { TextButton(onClick = { onSubmit(value) }) { Text(stringResource(R.string.ok)) } },
        dismissButton = { TextButton(onClick = onCancel) { Text(stringResource(R.string.cancel)) } },
    )
}

/**
 * 主机密钥确认：显示密钥类型与 SHA256 指纹。
 * 首次连接是普通确认；[changed] 为 true（与加密库记录不一致）时用警告样式，默认动作是不信任，
 * 点空白处或返回键也按不信任处理。
 */
@Composable
fun HostKeyDialog(
    host: String,
    port: Int,
    keyType: String,
    fingerprint: String,
    changed: Boolean,
    onTrust: () -> Unit,
    onReject: () -> Unit,
) {
    val danger = MaterialTheme.colorScheme.error
    val address = if (port == 22) host else "$host:$port"
    AlertDialog(
        onDismissRequest = onReject,
        properties = DialogProperties(dismissOnClickOutside = !changed),
        icon = { Icon(if (changed) Icons.Rounded.Warning else Icons.Rounded.Key, null, tint = if (changed) danger else MaterialTheme.colorScheme.primary) },
        title = {
            Text(
                stringResource(if (changed) R.string.host_key_changed_title else R.string.host_key_new_title),
                color = if (changed) danger else MaterialTheme.colorScheme.onSurface,
            )
        },
        text = {
            Column {
                Text(
                    stringResource(if (changed) R.string.host_key_changed_message else R.string.host_key_new_message, address),
                    style = MaterialTheme.typography.bodyMedium,
                    color = if (changed) danger else MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(14.dp))
                Column(
                    Modifier
                        .fillMaxWidth()
                        .background(MaterialTheme.colorScheme.surfaceContainerHigh, RoundedCornerShape(10.dp))
                        .then(if (changed) Modifier.border(1.dp, danger.copy(alpha = 0.6f), RoundedCornerShape(10.dp)) else Modifier)
                        .padding(12.dp),
                ) {
                    FieldLine(stringResource(R.string.host_key_host), address)
                    Spacer(Modifier.height(8.dp))
                    FieldLine(stringResource(R.string.host_key_type), keyType)
                    Spacer(Modifier.height(8.dp))
                    FieldLine(stringResource(R.string.host_key_fingerprint), fingerprint.removePrefix("SHA256:"))
                }
                if (changed) {
                    Spacer(Modifier.height(12.dp))
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.Rounded.Warning, null, tint = danger, modifier = Modifier.size(16.dp))
                        Spacer(Modifier.width(6.dp))
                        Text(stringResource(R.string.host_key_changed_hint), style = MaterialTheme.typography.bodySmall, color = danger)
                    }
                }
            }
        },
        // 变化时「取消」放在确认位（主按钮），信任降为危险色的次要按钮
        confirmButton = {
            if (changed) {
                TextButton(onClick = onReject) { Text(stringResource(R.string.cancel), fontWeight = FontWeight.SemiBold) }
            } else {
                TextButton(onClick = onTrust) { Text(stringResource(R.string.host_key_trust)) }
            }
        },
        dismissButton = {
            if (changed) {
                TextButton(onClick = onTrust, colors = ButtonDefaults.textButtonColors(contentColor = danger)) {
                    Text(stringResource(R.string.host_key_trust_anyway))
                }
            } else {
                TextButton(onClick = onReject) { Text(stringResource(R.string.cancel)) }
            }
        },
    )
}

@Composable
private fun FieldLine(label: String, value: String) {
    Column {
        Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        SelectionContainer {
            Text(value, style = MaterialTheme.typography.bodyMedium, fontFamily = FontFamily.Monospace)
        }
    }
}
