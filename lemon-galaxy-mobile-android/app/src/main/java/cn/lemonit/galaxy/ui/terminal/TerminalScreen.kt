package cn.lemonit.galaxy.ui.terminal

import android.view.KeyEvent
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.rounded.Send
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Code
import androidx.compose.material.icons.rounded.Keyboard
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.KeyboardArrowUp
import androidx.compose.material.icons.rounded.Mic
import androidx.compose.material.icons.rounded.MoreVert
import androidx.compose.material.icons.automirrored.rounded.ShortText
import androidx.compose.material.icons.automirrored.rounded.ViewList
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.TabStatus
import cn.lemonit.galaxy.core.bridge.PathInfo
import cn.lemonit.galaxy.core.bridge.PathKind
import cn.lemonit.galaxy.core.bridge.VaultData
import cn.lemonit.galaxy.ui.components.GalaxyTopBar
import cn.lemonit.galaxy.ui.components.InfoButton
import cn.lemonit.galaxy.ui.components.StatusDot
import cn.lemonit.galaxy.ui.theme.Galaxy
import cn.lemonit.galaxy.ui.theme.TerminalTheme
import com.termux.terminal.TerminalSession
import com.termux.view.TerminalView

/** 一个终端标签在界面上的快照。 */
data class TerminalTabUi(
    val id: Long,
    val title: String,
    val targetName: String,
    val status: TabStatus,
    val session: TerminalSession,
    val isSsh: Boolean,
)

interface TerminalActions {
    fun onBack()
    fun onSelect(id: Long)
    fun onCloseTab(id: Long, kill: Boolean)
    fun onNewSession()
    fun onShowSessions()
    fun onReconnect(id: Long)
    fun onToggleInputBox()
    fun onSendLine(text: String)
    fun onSnippet(snippet: VaultData.Snippet)
    fun onFontScale(bigger: Boolean)
    fun onVoice()
    fun onViewBound(id: Long, view: TerminalView?)
}

@Composable
fun TerminalScreen(
    tabs: List<TerminalTabUi>,
    activeId: Long?,
    path: PathInfo?,
    theme: TerminalTheme,
    fontSizeSp: Int,
    inputBox: Boolean,
    snippets: List<VaultData.Snippet>,
    voiceAvailable: Boolean,
    actions: TerminalActions,
    mods: ModifierKeys = remember { ModifierKeys() },
    viewRef: TerminalViewRef = remember { TerminalViewRef() },
    showBack: Boolean = true,
) {
    val active = tabs.firstOrNull { it.id == activeId } ?: tabs.firstOrNull()
    Column(Modifier.fillMaxSize().background(theme.backgroundColor).imePadding()) {
        Surface(color = MaterialTheme.colorScheme.surface) {
            Column {
                GalaxyTopBar(
                    title = active?.title ?: stringResource(R.string.terminal_title),
                    onBack = if (showBack) actions::onBack else null,
                    subtitle = { PathLine(path, active) },
                    actions = {
                        SnippetMenu(snippets, actions::onSnippet)
                        IconButton(onClick = actions::onToggleInputBox) {
                            Icon(
                                Icons.AutoMirrored.Rounded.ShortText, stringResource(R.string.terminal_input_box),
                                tint = if (inputBox) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        IconButton(onClick = actions::onShowSessions) { Icon(Icons.AutoMirrored.Rounded.ViewList, stringResource(R.string.terminal_sessions)) }
                        if (active != null) TabMenu(active, actions)
                    },
                )
                if (tabs.size > 1) TabStrip(tabs, active?.id, actions)
            }
        }
        Box(Modifier.weight(1f).fillMaxWidth()) {
            if (active != null) {
                TerminalViewHost(
                    session = active.session,
                    theme = theme,
                    fontSizeSp = fontSizeSp,
                    mods = mods,
                    ref = viewRef,
                    onFontScale = actions::onFontScale,
                    onViewBound = { actions.onViewBound(active.id, it) },
                    modifier = Modifier.fillMaxSize().padding(start = 4.dp, end = 2.dp, top = 2.dp),
                )
                StatusOverlay(active, actions)
            }
        }
        Surface(color = MaterialTheme.colorScheme.surfaceContainer, modifier = Modifier.navigationBarsPadding()) {
            Column {
                if (inputBox) InputBoxRow(actions, voiceAvailable)
                ExtraKeysBar(mods, viewRef)
            }
        }
    }
}

@Composable
private fun PathLine(path: PathInfo?, tab: TerminalTabUi?) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        val (label, color) = when {
            tab?.isSsh == true -> stringResource(R.string.path_ssh) to Galaxy.colors.online
            path == null -> "—" to Galaxy.colors.offline
            path.kind == PathKind.LAN -> stringResource(R.string.path_lan) to Galaxy.colors.online
            path.kind == PathKind.DIRECT -> stringResource(R.string.path_direct) to Galaxy.colors.online
            path.kind == PathKind.RELAY_UDP -> stringResource(R.string.path_relay_udp) to Galaxy.colors.warning
            else -> stringResource(R.string.path_relay_tcp) to Galaxy.colors.warning
        }
        StatusDot(color, size = 7.dp)
        Spacer(Modifier.width(6.dp))
        Text(
            label + (path?.takeIf { tab?.isSsh != true }?.let { "  ·  " + stringResource(R.string.path_rtt, it.rttMs.toInt()) } ?: ""),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        InfoButton(stringResource(R.string.path_info), size = 14.dp)
        if (tab != null && tab.targetName != tab.title) {
            Text(
                tab.targetName,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
private fun TabStrip(tabs: List<TerminalTabUi>, activeId: Long?, actions: TerminalActions) {
    Row(
        Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 8.dp, vertical = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        tabs.forEach { t ->
            val selected = t.id == activeId
            Row(
                Modifier
                    .clip(RoundedCornerShape(8.dp))
                    .background(if (selected) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surfaceContainerHigh)
                    .clickable { actions.onSelect(t.id) }
                    .padding(start = 10.dp, end = 2.dp, top = 2.dp, bottom = 2.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                StatusDot(statusColor(t.status), size = 6.dp)
                Spacer(Modifier.width(6.dp))
                Text(t.title, style = MaterialTheme.typography.labelLarge, maxLines = 1, modifier = Modifier.widthIn(max = 160.dp), overflow = TextOverflow.Ellipsis)
                IconButton(onClick = { actions.onCloseTab(t.id, false) }, modifier = Modifier.size(28.dp)) {
                    Icon(Icons.Rounded.Close, stringResource(R.string.close), Modifier.size(14.dp))
                }
            }
        }
        IconButton(onClick = actions::onNewSession, modifier = Modifier.size(32.dp)) {
            Icon(Icons.Rounded.Add, stringResource(R.string.terminal_new_session), Modifier.size(18.dp))
        }
    }
}

@Composable
private fun statusColor(s: TabStatus): Color = when (s) {
    TabStatus.Connected -> Galaxy.colors.online
    TabStatus.Connecting -> Galaxy.colors.warning
    else -> Galaxy.colors.offline
}

@Composable
private fun TabMenu(tab: TerminalTabUi, actions: TerminalActions) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) { Icon(Icons.Rounded.MoreVert, stringResource(R.string.more)) }
        DropdownMenu(open, { open = false }) {
            DropdownMenuItem(text = { Text(stringResource(R.string.terminal_new_session)) }, onClick = {
                open = false
                actions.onNewSession()
            })
            if (!tab.isSsh) {
                DropdownMenuItem(text = { Text(stringResource(R.string.terminal_detach)) }, onClick = {
                    open = false
                    actions.onCloseTab(tab.id, false)
                })
            }
            DropdownMenuItem(text = { Text(stringResource(R.string.terminal_kill), color = MaterialTheme.colorScheme.error) }, onClick = {
                open = false
                actions.onCloseTab(tab.id, true)
            })
        }
    }
}

@Composable
private fun SnippetMenu(snippets: List<VaultData.Snippet>, onPick: (VaultData.Snippet) -> Unit) {
    if (snippets.isEmpty()) return
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) { Icon(Icons.Rounded.Code, stringResource(R.string.terminal_snippets)) }
        DropdownMenu(open, { open = false }) {
            snippets.forEach { s ->
                DropdownMenuItem(
                    text = {
                        Column {
                            Text(s.label)
                            Text(s.command, style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1)
                        }
                    },
                    onClick = {
                        open = false
                        onPick(s)
                    },
                )
            }
        }
    }
}

@Composable
private fun StatusOverlay(tab: TerminalTabUi, actions: TerminalActions) {
    val s = tab.status
    if (s == TabStatus.Connected) return
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.BottomCenter) {
        Surface(
            shape = RoundedCornerShape(14.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHighest,
            shadowElevation = 4.dp,
            modifier = Modifier.padding(16.dp),
        ) {
            Row(Modifier.padding(horizontal = 16.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                when (s) {
                    TabStatus.Connecting -> {
                        CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                        Spacer(Modifier.width(12.dp))
                        Text(stringResource(R.string.terminal_connecting))
                    }
                    is TabStatus.Failed, TabStatus.Detached, is TabStatus.Exited -> {
                        val text = when (s) {
                            is TabStatus.Failed -> s.message.ifEmpty { stringResource(R.string.terminal_disconnected) }
                            is TabStatus.Exited -> stringResource(R.string.terminal_exited)
                            else -> stringResource(R.string.terminal_disconnected)
                        }
                        Text(text, modifier = Modifier.widthIn(max = 220.dp), maxLines = 2, overflow = TextOverflow.Ellipsis)
                        Spacer(Modifier.width(12.dp))
                        if (s is TabStatus.Exited) {
                            OutlinedButton(onClick = actions::onNewSession) { Text(stringResource(R.string.terminal_new_session)) }
                        } else {
                            Button(onClick = { actions.onReconnect(tab.id) }) { Text(stringResource(R.string.terminal_reconnect)) }
                        }
                    }
                    else -> Unit
                }
            }
        }
    }
}

@Composable
private fun InputBoxRow(actions: TerminalActions, voiceAvailable: Boolean) {
    var text by remember { mutableStateOf("") }
    val send = {
        actions.onSendLine(text)
        text = ""
    }
    Row(Modifier.fillMaxWidth().padding(start = 8.dp, end = 4.dp, top = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        TextField(
            value = text,
            onValueChange = { text = it },
            placeholder = { Text(stringResource(R.string.terminal_input_hint)) },
            textStyle = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
            singleLine = false,
            maxLines = 4,
            shape = RoundedCornerShape(12.dp),
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
            keyboardActions = KeyboardActions(onSend = { send() }),
            colors = TextFieldDefaults.colors(
                focusedIndicatorColor = Color.Transparent,
                unfocusedIndicatorColor = Color.Transparent,
                focusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
            ),
            modifier = Modifier.weight(1f).heightIn(min = 48.dp),
        )
        if (voiceAvailable) {
            IconButton(onClick = actions::onVoice) { Icon(Icons.Rounded.Mic, stringResource(R.string.terminal_voice)) }
        }
        IconButton(onClick = send) {
            Icon(Icons.AutoMirrored.Rounded.Send, stringResource(R.string.terminal_send), tint = MaterialTheme.colorScheme.primary)
        }
    }
}

private sealed class XKey(val label: String) {
    class Code(label: String, val keyCode: Int, val icon: ImageVector? = null) : XKey(label)
    class Char(label: String, val text: String = label) : XKey(label)
    class Mod(label: String, val ctrl: Boolean) : XKey(label)
}

private val EXTRA_KEYS: List<XKey> = listOf(
    XKey.Code("ESC", KeyEvent.KEYCODE_ESCAPE),
    XKey.Code("TAB", KeyEvent.KEYCODE_TAB),
    XKey.Mod("CTRL", ctrl = true),
    XKey.Mod("ALT", ctrl = false),
    XKey.Code("←", KeyEvent.KEYCODE_DPAD_LEFT, Icons.AutoMirrored.Rounded.KeyboardArrowLeft),
    XKey.Code("↑", KeyEvent.KEYCODE_DPAD_UP, Icons.Rounded.KeyboardArrowUp),
    XKey.Code("↓", KeyEvent.KEYCODE_DPAD_DOWN, Icons.Rounded.KeyboardArrowDown),
    XKey.Code("→", KeyEvent.KEYCODE_DPAD_RIGHT, Icons.AutoMirrored.Rounded.KeyboardArrowRight),
    XKey.Char("|"), XKey.Char("/"), XKey.Char("-"), XKey.Char("~"), XKey.Char(":"), XKey.Char("_"),
    XKey.Code("HOME", KeyEvent.KEYCODE_MOVE_HOME),
    XKey.Code("END", KeyEvent.KEYCODE_MOVE_END),
    XKey.Code("PGUP", KeyEvent.KEYCODE_PAGE_UP),
    XKey.Code("PGDN", KeyEvent.KEYCODE_PAGE_DOWN),
    XKey.Char("*"), XKey.Char("&"), XKey.Char("$"), XKey.Char(">"), XKey.Char("<"), XKey.Char("\\"), XKey.Char("'"), XKey.Char("\""),
    XKey.Code("F1", KeyEvent.KEYCODE_F1), XKey.Code("F2", KeyEvent.KEYCODE_F2), XKey.Code("F3", KeyEvent.KEYCODE_F3),
    XKey.Code("F4", KeyEvent.KEYCODE_F4), XKey.Code("F5", KeyEvent.KEYCODE_F5), XKey.Code("F10", KeyEvent.KEYCODE_F10),
)

/** 底部快捷键栏：横向滚动；Ctrl / Alt 为一次性修饰键（高亮表示已按下）。 */
@Composable
fun ExtraKeysBar(mods: ModifierKeys, ref: TerminalViewRef) {
    Row(
        Modifier.fillMaxWidth().height(48.dp).padding(vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconButton(onClick = ref::showKeyboard, modifier = Modifier.size(40.dp)) {
            Icon(Icons.Rounded.Keyboard, stringResource(R.string.terminal_keyboard), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Row(
            Modifier.weight(1f).horizontalScroll(rememberScrollState()).padding(end = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            EXTRA_KEYS.forEach { k ->
                val active = k is XKey.Mod && (if (k.ctrl) mods.ctrl.value else mods.alt.value)
                val onClick: () -> Unit = when (k) {
                    is XKey.Code -> ({ ref.sendKey(k.keyCode, mods) })
                    is XKey.Char -> ({ ref.sendChar(k.text, mods) })
                    is XKey.Mod -> ({ if (k.ctrl) mods.ctrl.value = !mods.ctrl.value else mods.alt.value = !mods.alt.value })
                }
                KeyCap(k, active, onClick)
            }
        }
    }
}

@Composable
private fun KeyCap(k: XKey, active: Boolean, onClick: () -> Unit) {
    val bg = if (active) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceContainerHighest
    val fg = if (active) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface
    Box(
        Modifier
            .height(36.dp)
            .widthIn(min = 40.dp)
            .clip(RoundedCornerShape(8.dp))
            .background(bg)
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.6f), RoundedCornerShape(8.dp))
            .clickable(onClick = onClick)
            .padding(horizontal = 10.dp),
        contentAlignment = Alignment.Center,
    ) {
        val icon = (k as? XKey.Code)?.icon
        if (icon != null) {
            Icon(icon, k.label, tint = fg, modifier = Modifier.size(20.dp))
        } else {
            Text(
                k.label,
                color = fg,
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.Medium,
                fontSize = if (k.label.length > 2) 12.sp else 15.sp,
            )
        }
    }
}

/** 会话面板：本机打开的标签 + 被控端上的会话（可接续）。 */
@Composable
fun SessionsPanel(
    tabs: List<TerminalTabUi>,
    activeId: Long?,
    remoteTitle: String?,
    remote: List<cn.lemonit.galaxy.core.bridge.TerminalSessionInfo>?,
    openRemoteIds: Set<String>,
    onSelect: (Long) -> Unit,
    onAttach: (String) -> Unit,
    onKillRemote: (String) -> Unit,
    onNew: () -> Unit,
) {
    Column(Modifier.fillMaxWidth().padding(bottom = 16.dp)) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(R.string.terminal_sessions), style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
            InfoButton(stringResource(R.string.terminal_sessions_info))
            IconButton(onClick = onNew) { Icon(Icons.Rounded.Add, stringResource(R.string.terminal_new_session)) }
        }
        tabs.forEach { t ->
            SessionRow(t.title, t.targetName, statusColor(t.status), t.id == activeId, null) { onSelect(t.id) }
        }
        if (remoteTitle != null) {
            HorizontalDivider(Modifier.padding(vertical = 8.dp), color = MaterialTheme.colorScheme.outlineVariant)
            Text(
                remoteTitle,
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp),
            )
            when {
                remote == null -> Box(Modifier.fillMaxWidth().padding(16.dp), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator(Modifier.size(22.dp), strokeWidth = 2.dp)
                }
                remote.none { it.id !in openRemoteIds } -> Text(
                    stringResource(R.string.terminal_no_remote_sessions),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 20.dp, vertical = 8.dp),
                )
                else -> remote.filter { it.id !in openRemoteIds }.forEach { r ->
                    SessionRow(
                        r.title,
                        "${r.id}  ·  " + stringResource(if (r.attached) R.string.terminal_attached_elsewhere else R.string.terminal_detached_state),
                        if (r.attached) Galaxy.colors.warning else Galaxy.colors.offline,
                        false,
                        trailing = {
                            Row {
                                IconButton(onClick = { onKillRemote(r.id) }) { Icon(Icons.Rounded.Close, stringResource(R.string.terminal_kill)) }
                                OutlinedButton(onClick = { onAttach(r.id) }) { Text(stringResource(R.string.terminal_attach)) }
                            }
                        },
                    ) { onAttach(r.id) }
                }
            }
        }
    }
}

@Composable
private fun SessionRow(title: String, subtitle: String, dot: Color, selected: Boolean, trailing: (@Composable () -> Unit)?, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 8.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(if (selected) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent)
            .clickable(onClick = onClick)
            .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(32.dp).clip(CircleShape).background(MaterialTheme.colorScheme.surfaceContainerHighest), contentAlignment = Alignment.Center) {
            StatusDot(dot, size = 8.dp)
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, fontFamily = FontFamily.Monospace)
        }
        trailing?.invoke()
    }
}
