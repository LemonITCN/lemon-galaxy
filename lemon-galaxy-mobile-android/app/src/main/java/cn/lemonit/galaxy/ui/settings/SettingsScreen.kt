package cn.lemonit.galaxy.ui.settings

import cn.lemonit.galaxy.app.UpdateState
import androidx.compose.material3.CircularProgressIndicator
import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Logout
import androidx.compose.material.icons.automirrored.rounded.OpenInNew
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.Dns
import androidx.compose.material.icons.rounded.Fingerprint
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material.icons.rounded.Password
import androidx.compose.material.icons.rounded.Person
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.ServerInfo
import cn.lemonit.galaxy.core.bridge.UserInfo
import cn.lemonit.galaxy.data.AppLanguage
import cn.lemonit.galaxy.data.AppSettings
import cn.lemonit.galaxy.data.Prefs
import cn.lemonit.galaxy.data.VideoQuality
import cn.lemonit.galaxy.app.SELECTABLE_MODES
import cn.lemonit.galaxy.ui.ai.modeLabel
import cn.lemonit.galaxy.ui.components.GalaxyTopBar
import cn.lemonit.galaxy.ui.components.InfoButton
import cn.lemonit.galaxy.ui.components.SectionHeader
import cn.lemonit.galaxy.ui.hosts.DropdownField
import cn.lemonit.galaxy.ui.theme.Palette
import cn.lemonit.galaxy.ui.theme.TerminalTheme
import cn.lemonit.galaxy.ui.theme.ThemeMode

interface SettingsActions {
    fun onPrefs(block: (Prefs) -> Prefs)
    fun onLanguage(lang: AppLanguage)
    fun onChangePassword()
    fun onTwoFactor()
    fun onLogout()
    fun onAppLock(enabled: Boolean)
    fun onLicenses()
    fun onSource()

    /** 关于 - 检查更新；有新版本时点它重新打开更新弹框。 */
    fun onCheckUpdate() = Unit
}

data class AboutInfo(val appVersion: String, val coreVersion: String, val mockCore: Boolean)

@Composable
fun SettingsScreen(
    prefs: Prefs,
    language: AppLanguage,
    user: UserInfo?,
    server: ServerInfo?,
    totpEnabled: Boolean?,
    themes: List<TerminalTheme>,
    about: AboutInfo,
    biometricAvailable: Boolean,
    actions: SettingsActions,
    update: UpdateState = UpdateState.Idle,
) {
    Scaffold(topBar = { GalaxyTopBar(stringResource(R.string.settings_title)) }) { pad ->
        LazyColumn(Modifier.padding(pad).fillMaxSize(), contentPadding = PaddingValues(bottom = 32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            item("account") {
                Section(stringResource(R.string.settings_account)) {
                    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                        Box(
                            Modifier.size(44.dp).clip(CircleShape).background(MaterialTheme.colorScheme.primaryContainer),
                            contentAlignment = Alignment.Center,
                        ) {
                            Text(
                                (user?.displayName ?: "?").take(1).uppercase(),
                                style = MaterialTheme.typography.titleMedium,
                                color = MaterialTheme.colorScheme.onPrimaryContainer,
                            )
                        }
                        Spacer(Modifier.width(14.dp))
                        Column(Modifier.weight(1f)) {
                            Text(user?.displayName ?: "", style = MaterialTheme.typography.titleMedium)
                            Text(user?.username ?: "", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    }
                    Row2(Icons.Rounded.Dns, stringResource(R.string.settings_server), server?.address, monoSubtitle = true) {
                        Text(
                            server?.let { "v" + it.version } ?: "",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Row2(Icons.Rounded.Password, stringResource(R.string.settings_change_password), onClick = actions::onChangePassword) { Chevron() }
                    if (totpEnabled != null) {
                        Row2(Icons.Rounded.Key, stringResource(R.string.settings_two_factor), onClick = actions::onTwoFactor) {
                            Text(
                                stringResource(if (totpEnabled) R.string.tf_on else R.string.tf_off),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            Chevron()
                        }
                    }
                    Row2(Icons.AutoMirrored.Rounded.Logout, stringResource(R.string.settings_logout), color = MaterialTheme.colorScheme.error, onClick = actions::onLogout)
                }
            }
            item("appearance") {
                Section(stringResource(R.string.settings_appearance)) {
                    Label(stringResource(R.string.settings_theme))
                    Segmented(ThemeMode.entries.map { it to stringResource(it.label) }, prefs.themeMode) { m -> actions.onPrefs { it.copy(themeMode = m) } }
                    Label(stringResource(R.string.settings_palette))
                    Row(
                        Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 4.dp),
                        horizontalArrangement = Arrangement.spacedBy(14.dp),
                    ) {
                        Palette.entries.forEach { p ->
                            Swatch(p, selected = prefs.palette == p && !prefs.dynamicColor) { actions.onPrefs { it.copy(palette = p, dynamicColor = false) } }
                        }
                    }
                    val dynamicSupported = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
                    SwitchRow(stringResource(R.string.settings_dynamic_color), prefs.dynamicColor && dynamicSupported, dynamicSupported, stringResource(R.string.settings_dynamic_color_info)) { on ->
                        actions.onPrefs { it.copy(dynamicColor = on) }
                    }
                    Label(stringResource(R.string.settings_terminal_theme))
                    Row(
                        Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 4.dp),
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        themes.forEach { t ->
                            TerminalThemeCard(t, prefs.terminalTheme == t.id, prefs.terminalFontSize) { actions.onPrefs { it.copy(terminalTheme = t.id) } }
                        }
                    }
                    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(stringResource(R.string.settings_font_size), style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
                        Text("${prefs.terminalFontSize} sp", style = MaterialTheme.typography.bodyMedium, fontFamily = FontFamily.Monospace, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    Slider(
                        value = prefs.terminalFontSize.toFloat(),
                        onValueChange = { v -> actions.onPrefs { it.copy(terminalFontSize = kotlin.math.round(v).toInt()) } },
                        valueRange = AppSettings.TERMINAL_FONT_MIN.toFloat()..AppSettings.TERMINAL_FONT_MAX.toFloat(),
                        modifier = Modifier.padding(horizontal = 16.dp),
                    )
                    Label(stringResource(R.string.settings_language))
                    Segmented(AppLanguage.entries.map { it to stringResource(it.label) }, language, actions::onLanguage)
                    Spacer(Modifier.height(8.dp))
                }
            }
            item("security") {
                Section(stringResource(R.string.settings_security)) {
                    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.Rounded.Fingerprint, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                        Spacer(Modifier.width(16.dp))
                        Text(stringResource(R.string.settings_app_lock), style = MaterialTheme.typography.bodyLarge)
                        InfoButton(stringResource(if (biometricAvailable) R.string.app_lock_info else R.string.app_lock_unavailable))
                        Spacer(Modifier.weight(1f))
                        Switch(checked = prefs.appLock, onCheckedChange = actions::onAppLock, enabled = biometricAvailable || prefs.appLock)
                    }
                }
            }
            item("defaults") {
                Section(stringResource(R.string.settings_defaults)) {
                    Column(Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
                        DropdownField(
                            stringResource(R.string.settings_default_quality),
                            VideoQuality.entries.map { it to stringResource(it.label) },
                            prefs.defaultQuality,
                            { q -> actions.onPrefs { it.copy(defaultQuality = q) } },
                            Modifier.fillMaxWidth(),
                        )
                        DropdownField(
                            stringResource(R.string.settings_default_input),
                            listOf(false to stringResource(R.string.rd_mode_touch), true to stringResource(R.string.rd_mode_mouse)),
                            prefs.remoteMouseMode,
                            { m -> actions.onPrefs { it.copy(remoteMouseMode = m) } },
                            Modifier.fillMaxWidth(),
                        )
                        DropdownField(
                            stringResource(R.string.settings_default_ai_mode),
                            SELECTABLE_MODES.map { it to stringResource(modeLabel(it)) },
                            prefs.defaultAiMode,
                            { m: AiMode -> actions.onPrefs { it.copy(defaultAiMode = m) } },
                            Modifier.fillMaxWidth(),
                        )
                    }
                }
            }
            item("about") {
                Section(stringResource(R.string.settings_about)) {
                    Row2(null, stringResource(R.string.about_version)) {
                        Text(about.appVersion, style = MaterialTheme.typography.bodyMedium, fontFamily = FontFamily.Monospace, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    Row2(null, stringResource(R.string.about_core)) {
                        Text(
                            if (about.mockCore) stringResource(R.string.about_core_mock, about.coreVersion) else about.coreVersion,
                            style = MaterialTheme.typography.bodyMedium,
                            fontFamily = FontFamily.Monospace,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    UpdateRow(update, about.appVersion, actions::onCheckUpdate)
                    Row2(null, stringResource(R.string.about_licenses), onClick = actions::onLicenses) { Chevron() }
                    Row2(null, stringResource(R.string.about_source), onClick = actions::onSource) {
                        Icon(Icons.AutoMirrored.Rounded.OpenInNew, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(18.dp))
                    }
                }
            }
        }
    }
}

/** 关于里的「检查更新」行：状态写在副标题里，检查、下载时显示进度圈。 */
@Composable
private fun UpdateRow(state: UpdateState, current: String, onClick: () -> Unit) {
    val subtitle = when (state) {
        UpdateState.Idle -> stringResource(R.string.update_idle, current)
        UpdateState.Checking -> stringResource(R.string.update_checking)
        is UpdateState.UpToDate -> stringResource(R.string.update_up_to_date, state.current)
        is UpdateState.Available -> stringResource(R.string.update_available_short, state.info.version)
        is UpdateState.Downloading -> stringResource(
            R.string.update_downloading_short, state.info.version,
            if (state.total <= 0) 0 else (state.done * 100 / state.total).toInt(),
        )
        is UpdateState.Downloaded -> stringResource(R.string.update_downloaded_short, state.info.version)
        is UpdateState.Failed -> state.message
    }
    val busy = state is UpdateState.Checking || state is UpdateState.Downloading
    Row2(
        null, stringResource(R.string.update_check), subtitle = subtitle,
        color = MaterialTheme.colorScheme.onSurface,
        onClick = if (state is UpdateState.Checking) null else onClick,
    ) {
        when {
            busy -> CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
            state is UpdateState.Available || state is UpdateState.Downloaded ->
                Box(Modifier.size(8.dp).clip(CircleShape).background(MaterialTheme.colorScheme.error))
            else -> Chevron()
        }
    }
}

@Composable
private fun Section(title: String, content: @Composable () -> Unit) {
    Column(Modifier.widthIn(max = 720.dp).fillMaxWidth()) {
        SectionHeader(title)
        Column(
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp)
                .clip(RoundedCornerShape(16.dp))
                .background(MaterialTheme.colorScheme.surfaceContainerLow)
                .border(1.dp, MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f), RoundedCornerShape(16.dp)),
        ) { content() }
    }
}

@Composable
private fun Label(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 14.dp, bottom = 6.dp),
    )
}

@Composable
private fun Chevron() = Icon(Icons.Rounded.ChevronRight, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)

@Composable
private fun Row2(
    icon: ImageVector?,
    title: String,
    subtitle: String? = null,
    monoSubtitle: Boolean = false,
    color: Color = MaterialTheme.colorScheme.onSurface,
    onClick: (() -> Unit)? = null,
    trailing: @Composable () -> Unit = {},
) {
    ListItem(
        headlineContent = { Text(title, color = color) },
        supportingContent = subtitle?.let {
            {
                Text(
                    it,
                    maxLines = 1,
                    overflow = if (monoSubtitle) TextOverflow.MiddleEllipsis else TextOverflow.Ellipsis,
                    fontFamily = if (monoSubtitle) FontFamily.Monospace else null,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        },
        leadingContent = icon?.let { { Icon(it, null, tint = if (color == MaterialTheme.colorScheme.onSurface) MaterialTheme.colorScheme.onSurfaceVariant else color) } },
        trailingContent = { Row(verticalAlignment = Alignment.CenterVertically) { trailing() } },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier,
    )
}

@Composable
private fun <T> Segmented(options: List<Pair<T, String>>, selected: T, onSelect: (T) -> Unit) {
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
        options.forEachIndexed { i, (v, label) ->
            SegmentedButton(
                selected = v == selected,
                onClick = { onSelect(v) },
                shape = SegmentedButtonDefaults.itemShape(i, options.size),
                label = { Text(label, maxLines = 1) },
            )
        }
    }
}

@Composable
private fun SwitchRow(title: String, checked: Boolean, enabled: Boolean, info: String, onChange: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(title, style = MaterialTheme.typography.bodyLarge, color = if (enabled) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.outline)
        InfoButton(info)
        Spacer(Modifier.weight(1f))
        Switch(checked = checked, onCheckedChange = onChange, enabled = enabled)
    }
}

@Composable
private fun Swatch(p: Palette, selected: Boolean, onClick: () -> Unit) {
    val dark = cn.lemonit.galaxy.ui.theme.Galaxy.colors.isDark
    val c = if (dark) p.darkAccent else p.lightAccent
    Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.clickable(onClick = onClick).padding(4.dp)) {
        Box(
            Modifier
                .size(40.dp)
                .clip(CircleShape)
                .background(c)
                .border(if (selected) 3.dp else 0.dp, MaterialTheme.colorScheme.onSurface, CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            if (selected) Icon(Icons.Rounded.Check, null, tint = MaterialTheme.colorScheme.surface, modifier = Modifier.size(20.dp))
        }
        Spacer(Modifier.height(4.dp))
        Text(stringResource(p.label), style = MaterialTheme.typography.labelSmall)
    }
}

/** 终端配色预览卡：背景、提示符、文字与 8 个常用色。 */
@Composable
fun TerminalThemeCard(t: TerminalTheme, selected: Boolean, fontSize: Int, onClick: () -> Unit) {
    Column(
        Modifier
            .width(148.dp)
            .clip(RoundedCornerShape(12.dp))
            .border(if (selected) 2.dp else 1.dp, if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(12.dp))
            .clickable(onClick = onClick),
    ) {
        Column(Modifier.fillMaxWidth().background(t.backgroundColor).padding(10.dp)) {
            val mono = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace, fontSize = (fontSize.coerceIn(10, 13) - 1).sp)
            Row {
                Text("demo@web", style = mono, color = t.ansi(2), fontWeight = FontWeight.Bold)
                Text(":~$ ", style = mono, color = t.ansi(4))
            }
            Text("ls -la", style = mono, color = t.foregroundColor)
            Row {
                Text("docs ", style = mono, color = t.ansi(12))
                Text("run.sh", style = mono, color = t.ansi(10))
            }
            Spacer(Modifier.height(6.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(3.dp)) {
                (1..6).forEach { Box(Modifier.size(10.dp).clip(RoundedCornerShape(2.dp)).background(t.ansi(it))) }
            }
        }
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        Row(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainer).padding(horizontal = 10.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(t.name, style = MaterialTheme.typography.labelMedium, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
            if (selected) Icon(Icons.Rounded.Check, null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(16.dp))
        }
    }
}

/** 开源许可页。 */
data class LicenseEntry(val name: String, val license: String, val url: String)

val LICENSES = listOf(
    LicenseEntry("Termux terminal-emulator / terminal-view (modified)", "Apache License 2.0", "https://github.com/termux/termux-app"),
    LicenseEntry("AndroidX, Jetpack Compose, Material 3", "Apache License 2.0", "https://developer.android.com/jetpack/androidx"),
    LicenseEntry("Material Icons", "Apache License 2.0", "https://fonts.google.com/icons"),
    LicenseEntry("Kotlin, kotlinx.coroutines, kotlinx.serialization", "Apache License 2.0", "https://kotlinlang.org"),
    LicenseEntry("Java Native Access (JNA)", "Apache License 2.0 / LGPL 2.1", "https://github.com/java-native-access/jna"),
    LicenseEntry("ZXing, zxing-android-embedded", "Apache License 2.0", "https://github.com/journeyapps/zxing-android-embedded"),
    LicenseEntry("Simple Icons (Apple, Linux marks)", "CC0 1.0", "https://simpleicons.org"),
    LicenseEntry("LemonGalaxy", "MIT License", "https://github.com/LemonITCN/lemon-galaxy"),
)

@Composable
fun LicensesScreen(onBack: () -> Unit, onOpen: (String) -> Unit) {
    Scaffold(topBar = { GalaxyTopBar(stringResource(R.string.licenses_title), onBack = onBack) }) { pad ->
        LazyColumn(Modifier.padding(pad).fillMaxSize()) {
            LICENSES.forEach { l ->
                item(l.name) {
                    ListItem(
                        headlineContent = { Text(l.name) },
                        supportingContent = { Text(l.license) },
                        trailingContent = { Icon(Icons.AutoMirrored.Rounded.OpenInNew, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(18.dp)) },
                        modifier = Modifier.clickable { onOpen(l.url) },
                    )
                }
            }
        }
    }
}
