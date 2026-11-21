package cn.lemonit.galaxy.ui.hosts

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.ExposedDropdownMenuDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MenuAnchorType
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
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
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.EntryKind
import cn.lemonit.galaxy.app.HostsData
import cn.lemonit.galaxy.core.bridge.DeviceKind
import cn.lemonit.galaxy.core.bridge.SshAuth
import cn.lemonit.galaxy.core.bridge.VaultData
import cn.lemonit.galaxy.core.bridge.VaultEntry
import cn.lemonit.galaxy.ui.components.GalaxyTopBar
import cn.lemonit.galaxy.ui.components.InfoButton
import cn.lemonit.galaxy.ui.theme.TerminalTheme

private fun tagsOf(s: String) = s.split(',', '，').map { it.trim() }.filter { it.isNotEmpty() }.distinct()

/** SSH / RDP / 凭据 / 片段 / 设备别名的编辑页。 */
@Composable
fun EntryEditorScreen(
    kind: EntryKind,
    entry: VaultEntry?,
    deviceId: String?,
    initialFolder: String?,
    data: HostsData,
    themes: List<TerminalTheme>,
    onSave: (VaultData, String?) -> Unit,
    onDelete: (() -> Unit)?,
    onBack: () -> Unit,
) {
    val isNew = entry == null || entry.id.isEmpty()
    val title = stringResource(
        when (kind) {
            EntryKind.SSH -> if (isNew) R.string.editor_new_ssh else R.string.editor_edit_ssh
            EntryKind.RDP -> if (isNew) R.string.editor_new_rdp else R.string.editor_edit_rdp
            EntryKind.CREDENTIAL -> if (isNew) R.string.editor_new_credential else R.string.editor_edit_credential
            EntryKind.SNIPPET -> if (isNew) R.string.editor_new_snippet else R.string.editor_edit_snippet
            EntryKind.DEVICE -> R.string.editor_edit_device
        },
    )
    var folder by remember { mutableStateOf(entry?.folderId ?: initialFolder) }
    // 各类表单的字段集中放在一个可变状态里
    val f = remember { FormState.from(kind, entry?.data, deviceId) }
    val valid = f.valid(kind)
    Scaffold(
        topBar = {
            GalaxyTopBar(
                title = title,
                onBack = onBack,
                actions = {
                    if (onDelete != null && !isNew) {
                        IconButton(onClick = onDelete) { Icon(Icons.Rounded.Delete, stringResource(R.string.delete)) }
                    }
                    TextButton(enabled = valid, onClick = {
                        onSave(f.build(kind, entry?.data), folder)
                        onBack()
                    }) { Text(stringResource(R.string.save)) }
                },
            )
        },
    ) { pad ->
        Column(
            Modifier.padding(pad).fillMaxSize().imePadding().verticalScroll(rememberScrollState()),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Column(Modifier.widthIn(max = 640.dp).fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
                when (kind) {
                    EntryKind.SSH -> SshForm(f, data, themes)
                    EntryKind.RDP -> RdpForm(f, data)
                    EntryKind.CREDENTIAL -> CredentialForm(f)
                    EntryKind.SNIPPET -> SnippetForm(f)
                    EntryKind.DEVICE -> DeviceForm(f, data, deviceId)
                }
                if (kind != EntryKind.CREDENTIAL && kind != EntryKind.SNIPPET) {
                    Spacer(Modifier.height(8.dp))
                    FolderField(data, folder) { folder = it }
                }
                Spacer(Modifier.height(32.dp))
            }
        }
    }
}

/** 表单字段（各类条目共用）。 */
class FormState {
    var label by mutableStateOf("")
    var host by mutableStateOf("")
    var port by mutableStateOf("")
    var username by mutableStateOf("")
    var password by mutableStateOf("")
    var authKind by mutableStateOf(0)
    var credentialId by mutableStateOf<String?>(null)
    var jumpDevice by mutableStateOf<String?>(null)
    var jumpHost by mutableStateOf<String?>(null)
    var tags by mutableStateOf("")
    var note by mutableStateOf("")
    var theme by mutableStateOf<String?>(null)
    var domain by mutableStateOf("")
    var privateKey by mutableStateOf("")
    var passphrase by mutableStateOf("")
    var command by mutableStateOf("")
    var alias by mutableStateOf("")
    var favorite = false
    var deviceId: String? = null

    val portValid get() = port.toIntOrNull()?.let { it in 1..65535 } == true

    fun valid(kind: EntryKind) = when (kind) {
        EntryKind.SSH -> label.isNotBlank() && host.isNotBlank() && portValid && username.isNotBlank() && (authKind != 1 || credentialId != null)
        EntryKind.RDP -> label.isNotBlank() && host.isNotBlank() && portValid
        EntryKind.CREDENTIAL -> label.isNotBlank()
        EntryKind.SNIPPET -> label.isNotBlank() && command.isNotBlank()
        EntryKind.DEVICE -> true
    }

    fun build(kind: EntryKind, old: VaultData?): VaultData = when (kind) {
        EntryKind.SSH -> VaultData.SshHost(
            label.trim(), host.trim(), port.toInt().toUShort(), username.trim(),
            when (authKind) {
                1 -> SshAuth.PrivateKey(credentialId!!)
                2 -> SshAuth.Agent
                3 -> SshAuth.Interactive
                else -> SshAuth.Password(password.ifEmpty { null })
            },
            jumpDevice, jumpHost, tagsOf(tags), note, (old as? VaultData.SshHost)?.favorite ?: false, theme,
        )
        EntryKind.RDP -> VaultData.RdpHost(
            label.trim(), host.trim(), port.toInt().toUShort(), username.trim(), password.ifEmpty { null }, domain.trim().ifEmpty { null },
            jumpDevice, tagsOf(tags), note, (old as? VaultData.RdpHost)?.favorite ?: false,
        )
        EntryKind.CREDENTIAL -> VaultData.Credential(label.trim(), username.trim(), password.ifEmpty { null }, privateKey.ifBlank { null }, passphrase.ifEmpty { null })
        EntryKind.SNIPPET -> VaultData.Snippet(label.trim(), command, tagsOf(tags))
        EntryKind.DEVICE -> VaultData.DeviceMeta(deviceId!!, alias.trim().ifEmpty { null }, tagsOf(tags), note, favorite)
    }

    companion object {
        fun from(kind: EntryKind, d: VaultData?, deviceId: String?) = FormState().apply {
            this.deviceId = deviceId
            when (d) {
                is VaultData.SshHost -> {
                    label = d.label; host = d.host; port = d.port.toString(); username = d.username
                    when (val a = d.auth) {
                        is SshAuth.Password -> { authKind = 0; password = a.password.orEmpty() }
                        is SshAuth.PrivateKey -> { authKind = 1; credentialId = a.credentialId }
                        SshAuth.Agent -> authKind = 2
                        SshAuth.Interactive -> authKind = 3
                    }
                    jumpDevice = d.jumpDeviceId; jumpHost = d.jumpHostId; tags = d.tags.joinToString(", "); note = d.note; theme = d.terminalTheme
                }
                is VaultData.RdpHost -> {
                    label = d.label; host = d.host; port = d.port.toString(); username = d.username; password = d.password.orEmpty()
                    domain = d.domain.orEmpty(); jumpDevice = d.jumpDeviceId; tags = d.tags.joinToString(", "); note = d.note
                }
                is VaultData.Credential -> {
                    label = d.label; username = d.username; password = d.password.orEmpty(); privateKey = d.privateKey.orEmpty(); passphrase = d.passphrase.orEmpty()
                }
                is VaultData.Snippet -> { label = d.label; command = d.command; tags = d.tags.joinToString(", ") }
                is VaultData.DeviceMeta -> { alias = d.alias.orEmpty(); tags = d.tags.joinToString(", "); note = d.note; favorite = d.favorite }
                else -> {
                    port = when (kind) {
                        EntryKind.SSH -> "22"
                        EntryKind.RDP -> "3389"
                        else -> ""
                    }
                }
            }
        }
    }
}

@Composable
private fun Field(
    value: String,
    onChange: (String) -> Unit,
    label: String,
    mono: Boolean = false,
    secret: Boolean = false,
    number: Boolean = false,
    lines: Int = 1,
    error: String? = null,
    modifier: Modifier = Modifier.fillMaxWidth(),
) {
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text(label) },
        singleLine = lines == 1,
        minLines = lines,
        maxLines = if (lines == 1) 1 else 12,
        isError = error != null,
        supportingText = error?.let { { Text(it) } },
        textStyle = if (mono) MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace) else MaterialTheme.typography.bodyLarge,
        visualTransformation = if (secret) PasswordVisualTransformation() else androidx.compose.ui.text.input.VisualTransformation.None,
        keyboardOptions = KeyboardOptions(
            keyboardType = when {
                number -> KeyboardType.Number
                secret -> KeyboardType.Password
                mono -> KeyboardType.Ascii
                else -> KeyboardType.Text
            },
        ),
        modifier = modifier.padding(vertical = 4.dp),
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun <T> DropdownField(label: String, options: List<Pair<T, String>>, selected: T, onSelect: (T) -> Unit, modifier: Modifier = Modifier) {
    var open by remember { mutableStateOf(false) }
    ExposedDropdownMenuBox(open, { open = it }, modifier.padding(vertical = 4.dp)) {
        OutlinedTextField(
            value = options.firstOrNull { it.first == selected }?.second.orEmpty(),
            onValueChange = {},
            readOnly = true,
            label = { Text(label) },
            trailingIcon = { ExposedDropdownMenuDefaults.TrailingIcon(open) },
            modifier = Modifier.fillMaxWidth().menuAnchor(MenuAnchorType.PrimaryNotEditable),
        )
        ExposedDropdownMenu(open, { open = false }) {
            options.forEach { (v, text) ->
                DropdownMenuItem(text = { Text(text) }, onClick = {
                    onSelect(v)
                    open = false
                })
            }
        }
    }
}

@Composable
private fun FolderField(data: HostsData, folder: String?, onSelect: (String?) -> Unit) {
    val options = listOf<Pair<String?, String>>(null to stringResource(R.string.move_root)) +
        data.folders.map { it.id to data.folderPath(it.id) }.sortedBy { it.second }
    DropdownField(stringResource(R.string.field_folder), options, folder, onSelect, Modifier.fillMaxWidth())
}

@Composable
private fun TagsAndNote(f: FormState, withNote: Boolean = true) {
    Field(f.tags, { f.tags = it }, stringResource(R.string.field_tags))
    if (withNote) Field(f.note, { f.note = it }, stringResource(R.string.field_note), lines = 2)
}

@Composable
private fun HostPort(f: FormState) {
    Row {
        Field(f.host, { f.host = it.trim() }, stringResource(R.string.field_host), mono = true, modifier = Modifier.weight(1f))
        Spacer(Modifier.width(8.dp))
        Field(
            f.port, { v -> f.port = v.filter(Char::isDigit).take(5) }, stringResource(R.string.field_port), number = true,
            error = if (f.port.isNotEmpty() && !f.portValid) stringResource(R.string.port_invalid) else null,
            modifier = Modifier.width(110.dp),
        )
    }
}

@Composable
private fun JumpField(f: FormState, data: HostsData, allowSsh: Boolean) {
    val devices = data.devices.filter { it.kind != DeviceKind.MOBILE }
    val options = buildList<Pair<String, String>> {
        add("" to stringResource(R.string.jump_none))
        devices.forEach { add("d:${it.id}" to stringResource(R.string.jump_device, data.deviceName(it.id))) }
        if (allowSsh) data.sshHosts.forEach { add("s:${it.id}" to stringResource(R.string.jump_host, (it.data as VaultData.SshHost).label)) }
    }
    val sel = f.jumpDevice?.let { "d:$it" } ?: f.jumpHost?.let { "s:$it" } ?: ""
    Row(verticalAlignment = Alignment.CenterVertically) {
        DropdownField(stringResource(R.string.field_jump), options, sel, { v ->
            f.jumpDevice = v.removePrefix("d:").takeIf { v.startsWith("d:") }
            f.jumpHost = v.removePrefix("s:").takeIf { v.startsWith("s:") }
        }, Modifier.weight(1f))
        InfoButton(stringResource(R.string.jump_info))
    }
}

@Composable
private fun SshForm(f: FormState, data: HostsData, themes: List<TerminalTheme>) {
    Field(f.label, { f.label = it }, stringResource(R.string.field_label))
    HostPort(f)
    Field(f.username, { f.username = it.trim() }, stringResource(R.string.field_username), mono = true)
    Spacer(Modifier.height(8.dp))
    Text(stringResource(R.string.field_auth), style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
    Spacer(Modifier.height(6.dp))
    val authLabels = listOf(R.string.auth_password, R.string.auth_key, R.string.auth_agent, R.string.auth_interactive)
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
        authLabels.forEachIndexed { i, l ->
            SegmentedButton(
                selected = f.authKind == i,
                onClick = { f.authKind = i },
                shape = SegmentedButtonDefaults.itemShape(i, authLabels.size),
                label = { Text(stringResource(l), maxLines = 1) },
            )
        }
    }
    Spacer(Modifier.height(4.dp))
    when (f.authKind) {
        0 -> Field(f.password, { f.password = it }, stringResource(R.string.field_password_optional), secret = true)
        1 -> DropdownField(
            stringResource(R.string.field_credential),
            data.credentials.map { it.id to (it.data as VaultData.Credential).label },
            f.credentialId, { f.credentialId = it }, Modifier.fillMaxWidth(),
        )
    }
    JumpField(f, data, allowSsh = true)
    DropdownField(
        stringResource(R.string.field_terminal_theme),
        listOf<Pair<String?, String>>(null to stringResource(R.string.use_default)) + themes.map { it.id to it.name },
        f.theme, { f.theme = it }, Modifier.fillMaxWidth(),
    )
    TagsAndNote(f)
}

@Composable
private fun RdpForm(f: FormState, data: HostsData) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Field(f.label, { f.label = it }, stringResource(R.string.field_label), modifier = Modifier.weight(1f))
        InfoButton(stringResource(R.string.rdp_unsupported))
    }
    HostPort(f)
    Field(f.username, { f.username = it.trim() }, stringResource(R.string.field_username), mono = true)
    Field(f.password, { f.password = it }, stringResource(R.string.field_password), secret = true)
    Field(f.domain, { f.domain = it }, stringResource(R.string.field_domain), mono = true)
    JumpField(f, data, allowSsh = false)
    TagsAndNote(f)
}

@Composable
private fun CredentialForm(f: FormState) {
    Field(f.label, { f.label = it }, stringResource(R.string.field_label))
    Field(f.username, { f.username = it.trim() }, stringResource(R.string.field_username), mono = true)
    Field(f.password, { f.password = it }, stringResource(R.string.field_password), secret = true)
    Field(f.privateKey, { f.privateKey = it }, stringResource(R.string.field_private_key), mono = true, lines = 5)
    Field(f.passphrase, { f.passphrase = it }, stringResource(R.string.field_passphrase), secret = true)
}

@Composable
private fun SnippetForm(f: FormState) {
    Field(f.label, { f.label = it }, stringResource(R.string.field_label))
    Field(f.command, { f.command = it }, stringResource(R.string.field_command), mono = true, lines = 4)
    TagsAndNote(f, withNote = false)
}

@Composable
private fun DeviceForm(f: FormState, data: HostsData, deviceId: String?) {
    val d = deviceId?.let { data.device(it) }
    Field(f.alias, { f.alias = it }, stringResource(R.string.field_alias) + (d?.let { "  (${it.hostname})" } ?: ""))
    TagsAndNote(f)
}
