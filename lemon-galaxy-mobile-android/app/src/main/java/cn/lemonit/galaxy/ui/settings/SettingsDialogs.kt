package cn.lemonit.galaxy.ui.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.userMessage
import cn.lemonit.galaxy.core.bridge.TotpSetup
import cn.lemonit.galaxy.ui.components.InfoButton
import kotlinx.coroutines.launch

@Composable
fun ChangePasswordDialog(submit: suspend (String, String) -> Unit, onDone: () -> Unit, onDismiss: () -> Unit) {
    var old by remember { mutableStateOf("") }
    var new by remember { mutableStateOf("") }
    var confirm by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val ok = old.isNotEmpty() && new.length >= 8 && new == confirm
    AlertDialog(
        onDismissRequest = { if (!busy) onDismiss() },
        title = { Text(stringResource(R.string.settings_change_password)) },
        text = {
            Column {
                PwField(old, { old = it }, stringResource(R.string.cp_current))
                PwField(new, { new = it }, stringResource(R.string.cp_new), if (new.isNotEmpty() && new.length < 8) stringResource(R.string.password_too_short) else null)
                PwField(confirm, { confirm = it }, stringResource(R.string.cp_confirm), if (confirm.isNotEmpty() && confirm != new) stringResource(R.string.password_mismatch) else null)
                error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
            }
        },
        confirmButton = {
            TextButton(enabled = ok && !busy, onClick = {
                busy = true
                scope.launch {
                    try {
                        submit(old, new)
                        onDone()
                        onDismiss()
                    } catch (e: Exception) {
                        error = e.userMessage()
                    }
                    busy = false
                }
            }) { if (busy) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp) else Text(stringResource(R.string.save)) }
        },
        dismissButton = { TextButton(onClick = onDismiss, enabled = !busy) { Text(stringResource(R.string.cancel)) } },
    )
}

@Composable
private fun PwField(value: String, onChange: (String) -> Unit, label: String, error: String? = null) {
    OutlinedTextField(
        value, onChange,
        label = { Text(label) },
        singleLine = true,
        visualTransformation = PasswordVisualTransformation(),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
        isError = error != null,
        supportingText = error?.let { { Text(it) } },
        modifier = Modifier.fillMaxWidth(),
    )
}

/** 两步验证：未开启时走设置流程（显示密钥 + 填验证码），已开启时输入验证码关闭。 */
@Composable
fun TwoFactorDialog(
    enabled: Boolean,
    setup: suspend () -> TotpSetup,
    verify: suspend (String) -> Unit,
    disable: suspend (String) -> Unit,
    onDone: (Boolean) -> Unit,
    onDismiss: () -> Unit,
) {
    var info by remember { mutableStateOf<TotpSetup?>(null) }
    var code by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    LaunchedEffect(enabled) {
        if (!enabled) {
            try {
                info = setup()
            } catch (e: Exception) {
                error = e.userMessage()
            }
        }
    }
    AlertDialog(
        onDismissRequest = { if (!busy) onDismiss() },
        title = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(if (enabled) R.string.totp_disable_title else R.string.totp_setup_title))
                if (!enabled) InfoButton(stringResource(R.string.totp_setup_info))
            }
        },
        text = {
            Column {
                if (!enabled) {
                    val i = info
                    if (i == null) {
                        CircularProgressIndicator(Modifier.size(24.dp))
                    } else {
                        Text(stringResource(R.string.totp_secret), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            SelectionContainer(Modifier.weight(1f)) {
                                Text(i.secret.chunked(4).joinToString(" "), style = MaterialTheme.typography.titleMedium, fontFamily = FontFamily.Monospace)
                            }
                            IconButton(onClick = {
                                (context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager).setPrimaryClip(ClipData.newPlainText("otpauth", i.otpauthUri))
                            }) {
                                Icon(Icons.Rounded.ContentCopy, stringResource(R.string.copy))
                            }
                        }
                        Spacer(Modifier.height(12.dp))
                    }
                }
                OutlinedTextField(
                    code, { v -> code = v.filter(Char::isDigit).take(6) },
                    label = { Text(stringResource(R.string.login_totp_code)) },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword),
                    textStyle = MaterialTheme.typography.titleMedium.copy(fontFamily = FontFamily.Monospace),
                    modifier = Modifier.fillMaxWidth(),
                )
                error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
            }
        },
        confirmButton = {
            TextButton(enabled = code.length == 6 && !busy && (enabled || info != null), onClick = {
                busy = true
                scope.launch {
                    try {
                        if (enabled) disable(code) else verify(code)
                        onDone(!enabled)
                        onDismiss()
                    } catch (e: Exception) {
                        error = e.userMessage()
                    }
                    busy = false
                }
            }) { Text(stringResource(if (enabled) R.string.totp_disable else R.string.totp_enable), color = if (enabled) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary) }
        },
        dismissButton = { TextButton(onClick = onDismiss, enabled = !busy) { Text(stringResource(R.string.cancel)) } },
    )
}
