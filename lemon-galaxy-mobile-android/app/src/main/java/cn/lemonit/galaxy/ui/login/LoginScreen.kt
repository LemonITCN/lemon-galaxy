package cn.lemonit.galaxy.ui.login

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.QrCodeScanner
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material.icons.rounded.VisibilityOff
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.AccountController
import cn.lemonit.galaxy.app.LoginStep
import cn.lemonit.galaxy.app.LoginUiState
import cn.lemonit.galaxy.ui.components.InfoButton
import cn.lemonit.galaxy.ui.components.Pill
import cn.lemonit.galaxy.ui.icons.PlatformIcons
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions

/** 带扫码的登录页（连接到 AccountController）。 */
@Composable
fun LoginRoute(account: AccountController, isMock: Boolean, onToast: (String) -> Unit) {
    val state by account.login.collectAsState()
    val invalidQr = stringResource(R.string.login_scan_invalid)
    val prompt = stringResource(R.string.login_scan_prompt)
    val scan = rememberLauncherForActivityResult(ScanContract()) { r ->
        val text = r.contents ?: return@rememberLauncherForActivityResult
        val parsed = AccountController.parseScanned(text)
        if (parsed != null) account.edit { it.copy(server = parsed) } else onToast(invalidQr)
    }
    LoginScreen(
        state = state,
        isMock = isMock,
        onEdit = account::edit,
        onSubmitCredentials = account::submitCredentials,
        onSubmitTotp = account::submitTotp,
        onSubmitNewPassword = account::submitNewPassword,
        onBack = account::backToCredentials,
        onScan = {
            scan.launch(
                ScanOptions().setDesiredBarcodeFormats(ScanOptions.QR_CODE).setPrompt(prompt)
                    .setBeepEnabled(false).setOrientationLocked(false),
            )
        },
    )
}

@Composable
fun LoginScreen(
    state: LoginUiState,
    isMock: Boolean,
    onEdit: ((LoginUiState) -> LoginUiState) -> Unit,
    onSubmitCredentials: () -> Unit,
    onSubmitTotp: () -> Unit,
    onSubmitNewPassword: () -> Unit,
    onBack: () -> Unit,
    onScan: () -> Unit,
) {
    Surface(color = MaterialTheme.colorScheme.background, modifier = Modifier.fillMaxSize()) {
        Box(
            Modifier.fillMaxSize().systemBarsPadding().imePadding().verticalScroll(rememberScrollState()),
            contentAlignment = Alignment.TopCenter,
        ) {
            Column(
                Modifier.widthIn(max = 440.dp).fillMaxWidth().padding(horizontal = 24.dp, vertical = 32.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Spacer(Modifier.height(24.dp))
                Icon(PlatformIcons.Galaxy, null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(56.dp))
                Spacer(Modifier.height(12.dp))
                Text(
                    stringResource(R.string.app_name),
                    style = MaterialTheme.typography.headlineSmall,
                    fontWeight = FontWeight.SemiBold,
                )
                Spacer(Modifier.height(32.dp))
                when (state.step) {
                    LoginStep.CREDENTIALS -> CredentialsStep(state, onEdit, onSubmitCredentials, onScan)
                    LoginStep.TOTP -> TotpStep(state, onEdit, onSubmitTotp, onBack)
                    LoginStep.CHANGE_PASSWORD -> ChangePasswordStep(state, onEdit, onSubmitNewPassword, onBack)
                }
                state.error?.takeIf { it.isNotEmpty() }?.let {
                    Spacer(Modifier.height(12.dp))
                    Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium, textAlign = TextAlign.Center)
                }
                if (isMock) {
                    Spacer(Modifier.height(28.dp))
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Pill(stringResource(R.string.login_mock_badge), MaterialTheme.colorScheme.tertiary)
                        InfoButton(stringResource(R.string.login_mock_info))
                    }
                }
            }
        }
    }
}

@Composable
private fun StepTitle(text: String, info: String?) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(text, style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f, fill = false))
        if (info != null) InfoButton(info)
    }
    Spacer(Modifier.height(16.dp))
}

@Composable
private fun SubmitButton(text: String, busy: Boolean, enabled: Boolean, onClick: () -> Unit) {
    Button(
        onClick = onClick,
        enabled = enabled && !busy,
        modifier = Modifier.fillMaxWidth().height(50.dp),
    ) {
        if (busy) {
            CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp, color = MaterialTheme.colorScheme.onPrimary)
        } else {
            Text(text, fontSize = 16.sp)
        }
    }
}

@Composable
private fun PasswordField(value: String, label: String, onChange: (String) -> Unit, imeAction: ImeAction, onDone: () -> Unit, error: String? = null) {
    var visible by remember { mutableStateOf(false) }
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text(label) },
        singleLine = true,
        visualTransformation = if (visible) VisualTransformation.None else PasswordVisualTransformation(),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = imeAction),
        keyboardActions = KeyboardActions(onDone = { onDone() }, onGo = { onDone() }),
        isError = error != null,
        supportingText = error?.let { { Text(it) } },
        trailingIcon = {
            IconButton(onClick = { visible = !visible }) {
                Icon(
                    if (visible) Icons.Rounded.VisibilityOff else Icons.Rounded.Visibility,
                    stringResource(if (visible) R.string.login_hide_password else R.string.login_show_password),
                )
            }
        },
        modifier = Modifier.fillMaxWidth(),
    )
}

@Composable
private fun CredentialsStep(
    state: LoginUiState,
    onEdit: ((LoginUiState) -> LoginUiState) -> Unit,
    onSubmit: () -> Unit,
    onScan: () -> Unit,
) {
    val serverInvalid = state.server.isNotBlank() && !AccountController.isValidServer(state.server)
    val canSubmit = AccountController.isValidServer(state.server) && state.username.isNotBlank() && state.password.isNotEmpty()
    StepTitle(stringResource(R.string.login_title), null)
    OutlinedTextField(
        value = state.server,
        onValueChange = { v -> onEdit { it.copy(server = v.trim()) } },
        label = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.login_server))
            }
        },
        placeholder = { Text(stringResource(R.string.login_server_placeholder), fontFamily = FontFamily.Monospace) },
        textStyle = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
        singleLine = true,
        isError = serverInvalid,
        supportingText = if (serverInvalid) ({ Text(stringResource(R.string.login_server_invalid)) }) else null,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Next),
        trailingIcon = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                InfoButton(stringResource(R.string.login_server_info))
                IconButton(onClick = onScan) { Icon(Icons.Rounded.QrCodeScanner, stringResource(R.string.login_scan)) }
            }
        },
        modifier = Modifier.fillMaxWidth(),
    )
    Spacer(Modifier.height(8.dp))
    OutlinedTextField(
        value = state.username,
        onValueChange = { v -> onEdit { it.copy(username = v) } },
        label = { Text(stringResource(R.string.login_username)) },
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii, imeAction = ImeAction.Next),
        modifier = Modifier.fillMaxWidth(),
    )
    Spacer(Modifier.height(8.dp))
    PasswordField(state.password, stringResource(R.string.login_password), { v -> onEdit { it.copy(password = v) } }, ImeAction.Go, { if (canSubmit) onSubmit() })
    Spacer(Modifier.height(20.dp))
    SubmitButton(stringResource(R.string.login_sign_in), state.busy, canSubmit, onSubmit)
}

@Composable
private fun TotpStep(
    state: LoginUiState,
    onEdit: ((LoginUiState) -> LoginUiState) -> Unit,
    onSubmit: () -> Unit,
    onBack: () -> Unit,
) {
    StepTitle(stringResource(R.string.login_totp_title), stringResource(R.string.login_totp_info))
    OutlinedTextField(
        value = state.totp,
        onValueChange = { v -> onEdit { it.copy(totp = v.filter(Char::isDigit).take(6)) } },
        label = { Text(stringResource(R.string.login_totp_code)) },
        singleLine = true,
        textStyle = MaterialTheme.typography.headlineSmall.copy(fontFamily = FontFamily.Monospace, letterSpacing = 6.sp, textAlign = TextAlign.Center),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword, imeAction = ImeAction.Done),
        keyboardActions = KeyboardActions(onDone = { if (state.totp.length == 6) onSubmit() }),
        modifier = Modifier.fillMaxWidth(),
    )
    Spacer(Modifier.height(20.dp))
    SubmitButton(stringResource(R.string.login_totp_verify), state.busy, state.totp.length == 6, onSubmit)
    BackLink(onBack)
}

@Composable
private fun ChangePasswordStep(
    state: LoginUiState,
    onEdit: ((LoginUiState) -> LoginUiState) -> Unit,
    onSubmit: () -> Unit,
    onBack: () -> Unit,
) {
    val tooShort = state.newPassword.isNotEmpty() && state.newPassword.length < 8
    val mismatch = state.confirmPassword.isNotEmpty() && state.confirmPassword != state.newPassword
    val ok = state.newPassword.length >= 8 && state.newPassword == state.confirmPassword
    StepTitle(stringResource(R.string.login_change_title), stringResource(R.string.login_change_info))
    PasswordField(
        state.newPassword, stringResource(R.string.login_change_new), { v -> onEdit { it.copy(newPassword = v) } }, ImeAction.Next, {},
        if (tooShort) stringResource(R.string.password_too_short) else null,
    )
    Spacer(Modifier.height(8.dp))
    PasswordField(
        state.confirmPassword, stringResource(R.string.login_change_confirm), { v -> onEdit { it.copy(confirmPassword = v) } }, ImeAction.Done,
        { if (ok) onSubmit() },
        if (mismatch) stringResource(R.string.password_mismatch) else null,
    )
    Spacer(Modifier.height(20.dp))
    SubmitButton(stringResource(R.string.login_change_submit), state.busy, ok, onSubmit)
    BackLink(onBack)
}

@Composable
private fun BackLink(onBack: () -> Unit) {
    Spacer(Modifier.height(8.dp))
    TextButton(onClick = onBack) {
        Row(horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.AutoMirrored.Rounded.ArrowBack, null, Modifier.size(18.dp))
            Spacer(Modifier.width(6.dp))
            Text(stringResource(R.string.back))
        }
    }
}
