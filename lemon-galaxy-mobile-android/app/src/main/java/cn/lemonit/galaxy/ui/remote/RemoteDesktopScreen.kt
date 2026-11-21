package cn.lemonit.galaxy.ui.remote

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.KeyboardBackspace
import androidx.compose.material.icons.automirrored.rounded.KeyboardReturn
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.HighQuality
import androidx.compose.material.icons.rounded.Keyboard
import androidx.compose.material.icons.rounded.KeyboardCommandKey
import androidx.compose.material.icons.rounded.LinkOff
import androidx.compose.material.icons.rounded.Monitor
import androidx.compose.material.icons.rounded.Mouse
import androidx.compose.material.icons.rounded.TouchApp
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.core.bridge.InputEvent
import cn.lemonit.galaxy.core.bridge.MouseButton
import cn.lemonit.galaxy.core.bridge.PathKind
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.SpecialCombo
import cn.lemonit.galaxy.data.VideoQuality
import cn.lemonit.galaxy.ui.components.InfoButton
import cn.lemonit.galaxy.ui.components.StatusDot
import cn.lemonit.galaxy.ui.icons.PlatformIcons
import cn.lemonit.galaxy.ui.theme.Galaxy
import kotlin.math.roundToInt

interface RemoteActions {
    fun onInput(events: List<InputEvent>)
    fun onDisplay(id: UInt)
    fun onQuality(q: VideoQuality)
    fun onMouseMode(on: Boolean)
    fun onClipboardSend()
    fun onClipboardGet()
    fun onDisconnect()
}

private fun click(button: MouseButton, x: Int, y: Int) = listOf(
    InputEvent.MouseMove(x, y),
    InputEvent.MouseButton(button, true, x, y),
    InputEvent.MouseButton(button, false, x, y),
)

/**
 * 远程桌面：画面（硬件解码 SurfaceView，测试时为占位图）+ 手势 + 悬浮工具球。
 * [video] 由调用方提供，Modifier 已带好位置与尺寸。
 */
@Composable
fun RemoteDesktopScreen(
    ui: RemoteUi,
    actions: RemoteActions,
    video: @Composable (Modifier) -> Unit,
    initialToolbarOpen: Boolean = false,
    initialKeyboard: Boolean = false,
) {
    val density = LocalDensity.current
    var viewport by remember { mutableStateOf(Viewport(1f, 1f, 1, 1)) }
    var toolbarOpen by remember { mutableStateOf(initialToolbarOpen) }
    var keyboardOpen by remember { mutableStateOf(initialKeyboard) }
    val currentUi by rememberUpdatedState(ui)
    val currentActions by rememberUpdatedState(actions)

    BoxWithConstraints(Modifier.fillMaxSize().background(Color.Black).imePadding()) {
        val cw = with(density) { maxWidth.toPx() }
        val ch = with(density) { maxHeight.toPx() }
        LaunchedEffect(cw, ch, ui.remoteW, ui.remoteH) {
            viewport = Viewport(cw, ch, ui.remoteW.coerceAtLeast(1), ui.remoteH.coerceAtLeast(1), viewport.zoom, viewport.panX, viewport.panY).clamped()
        }
        val vp = viewport
        val sink = remember {
            object : RemoteGestureSink {
                var dragButtonDown = false
                fun remote(o: Offset) = viewport.toRemote(o.x, o.y)
                fun cursor() = currentUi.cursorX to currentUi.cursorY
                fun send(e: List<InputEvent>) = currentActions.onInput(e)

                override fun tap(at: Offset) {
                    if (currentUi.mouseMode) {
                        val (x, y) = cursor()
                        send(click(MouseButton.LEFT, x, y))
                    } else {
                        val (x, y) = remote(at)
                        send(click(MouseButton.LEFT, x, y))
                    }
                }

                override fun twoFingerTap() {
                    val (x, y) = cursor()
                    send(click(MouseButton.RIGHT, x, y))
                }

                override fun longPress(at: Offset) {
                    if (!currentUi.mouseMode) {
                        val (x, y) = remote(at)
                        send(click(MouseButton.RIGHT, x, y))
                    }
                }

                override fun dragStart(at: Offset, afterLongPress: Boolean) {
                    if (currentUi.mouseMode) {
                        if (afterLongPress) {
                            val (x, y) = cursor()
                            send(listOf(InputEvent.MouseButton(MouseButton.LEFT, true, x, y)))
                            dragButtonDown = true
                        }
                    } else if (!afterLongPress) {
                        val (x, y) = remote(at)
                        send(listOf(InputEvent.MouseMove(x, y), InputEvent.MouseButton(MouseButton.LEFT, true, x, y)))
                        dragButtonDown = true
                    }
                }

                override fun dragMove(at: Offset, delta: Offset) {
                    if (currentUi.mouseMode) {
                        // 触控板：相对移动，带一点加速
                        val k = viewport.remotePerViewPx() * 1.6f
                        val (cx, cy) = cursor()
                        val nx = (cx + delta.x * k).roundToInt().coerceIn(0, currentUi.remoteW - 1)
                        val ny = (cy + delta.y * k).roundToInt().coerceIn(0, currentUi.remoteH - 1)
                        send(listOf(InputEvent.MouseMove(nx, ny)))
                        viewport = viewport.follow(nx, ny)
                    } else if (dragButtonDown) {
                        val (x, y) = remote(at)
                        send(listOf(InputEvent.MouseMove(x, y)))
                    }
                }

                override fun dragEnd() {
                    if (dragButtonDown) {
                        val (x, y) = cursor()
                        send(listOf(InputEvent.MouseButton(MouseButton.LEFT, false, x, y)))
                        dragButtonDown = false
                    }
                }

                override fun scroll(dx: Float, dy: Float) {
                    // 手指向上推 = 内容向上 = 滚轮向下；按视图像素换算成被控像素
                    val k = viewport.remotePerViewPx()
                    send(listOf(InputEvent.Wheel((-dx * k).roundToInt(), (-dy * k).roundToInt())))
                }

                override fun pinch(factor: Float, centroid: Offset, pan: Offset) {
                    viewport = viewport.zoomed(factor, centroid.x, centroid.y, pan.x, pan.y)
                }

                override fun twoFingerPan(delta: Offset) {
                    viewport = viewport.panned(delta.x, delta.y)
                }
            }
        }
        Box(
            Modifier
                .fillMaxSize()
                .clipToBounds()
                .pointerInput(Unit) { detectRemoteGestures(sink) { !currentUi.mouseMode && viewport.zoom > 1.01f } },
        ) {
            if (ui.remoteW > 0) {
                val wDp = with(density) { vp.contentW.toDp() }
                val hDp = with(density) { vp.contentH.toDp() }
                video(Modifier.offset { IntOffset(vp.left.roundToInt(), vp.top.roundToInt()) }.size(wDp, hDp))
                if (ui.mouseMode || ui.cursorVisible) {
                    val (px, py) = vp.toView(ui.cursorX, ui.cursorY)
                    Pointer(Modifier.offset { IntOffset(px.roundToInt(), py.roundToInt()) })
                }
            }
            if (ui.status != RemoteStatus.STREAMING) {
                Column(Modifier.align(Alignment.Center), horizontalAlignment = Alignment.CenterHorizontally) {
                    if (ui.status == RemoteStatus.CONNECTING) {
                        CircularProgressIndicator(color = Color.White, strokeWidth = 2.dp)
                        Spacer(Modifier.height(12.dp))
                        Text(stringResource(R.string.rd_connecting), color = Color.White.copy(alpha = 0.8f))
                    } else {
                        Text(ui.error?.takeIf { it.isNotEmpty() } ?: stringResource(R.string.rd_failed), color = Color.White)
                        Spacer(Modifier.height(12.dp))
                        TextButton(onClick = actions::onDisconnect) { Text(stringResource(R.string.close)) }
                    }
                }
            }
        }
        if (keyboardOpen) {
            RemoteKeyboard(ui.platform, actions, Modifier.align(Alignment.BottomCenter)) { keyboardOpen = false }
        }
        FloatingToolbar(
            ui = ui,
            open = toolbarOpen,
            onToggle = { toolbarOpen = !toolbarOpen },
            actions = actions,
            onKeyboard = {
                keyboardOpen = !keyboardOpen
                toolbarOpen = false
            },
            maxW = cw,
            maxH = ch,
        )
    }
}

/** 鼠标指针（白底黑边箭头）。 */
@Composable
private fun Pointer(modifier: Modifier) {
    Canvas(modifier.size(18.dp, 24.dp)) {
        val s = size.height / 24f
        val p = Path().apply {
            moveTo(0f, 0f)
            lineTo(0f, 19f * s)
            lineTo(4.5f * s, 14.8f * s)
            lineTo(7.6f * s, 21.6f * s)
            lineTo(10.6f * s, 20.3f * s)
            lineTo(7.6f * s, 13.6f * s)
            lineTo(13.6f * s, 13.6f * s)
            close()
        }
        drawPath(p, Color.White)
        drawPath(p, Color.Black, style = Stroke(width = 1.2f * s))
    }
}

@Composable
private fun FloatingToolbar(
    ui: RemoteUi,
    open: Boolean,
    onToggle: () -> Unit,
    actions: RemoteActions,
    onKeyboard: () -> Unit,
    maxW: Float,
    maxH: Float,
) {
    val density = LocalDensity.current
    val ballPx = with(density) { 48.dp.toPx() }
    var pos by remember { mutableStateOf(Offset(Float.NaN, Float.NaN)) }
    if (pos.x.isNaN() && maxW > 1f) pos = Offset(maxW - ballPx - with(density) { 16.dp.toPx() }, maxH * 0.35f)
    Box(Modifier.fillMaxSize()) {
        if (open) {
            ToolbarPanel(ui, actions, onKeyboard, Modifier.align(Alignment.TopCenter).padding(top = 12.dp, start = 12.dp, end = 12.dp))
        }
        Box(
            Modifier
                .offset { IntOffset(pos.x.roundToInt(), pos.y.roundToInt()) }
                .size(48.dp)
                .clip(CircleShape)
                .background(Color(0xCC1B1E2B))
                .border(1.dp, Color.White.copy(alpha = 0.25f), CircleShape)
                .pointerInput(maxW, maxH) {
                    detectDragGestures { change, drag ->
                        change.consume()
                        pos = Offset(
                            (pos.x + drag.x).coerceIn(0f, maxW - ballPx),
                            (pos.y + drag.y).coerceIn(0f, maxH - ballPx),
                        )
                    }
                }
                .clickable(onClick = onToggle),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                if (open) Icons.Rounded.Close else PlatformIcons.Galaxy,
                stringResource(R.string.rd_toolbar),
                tint = if (open) Color.White else Color(0xFFC6E35A),
                modifier = Modifier.size(24.dp),
            )
        }
    }
}

@Composable
private fun ToolbarPanel(ui: RemoteUi, actions: RemoteActions, onKeyboard: () -> Unit, modifier: Modifier) {
    Surface(
        shape = RoundedCornerShape(16.dp),
        color = Color(0xF01B1E2B),
        contentColor = Color.White,
        shadowElevation = 8.dp,
        modifier = modifier.widthIn(max = 720.dp),
    ) {
        Column(Modifier.padding(horizontal = 8.dp, vertical = 8.dp)) {
            Row(Modifier.padding(horizontal = 8.dp, vertical = 2.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(ui.title, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                Spacer(Modifier.width(10.dp))
                ui.path?.let { p ->
                    val (label, color) = when (p.kind) {
                        PathKind.LAN -> stringResource(R.string.path_lan) to Galaxy.colors.online
                        PathKind.DIRECT -> stringResource(R.string.path_direct) to Galaxy.colors.online
                        PathKind.RELAY_UDP -> stringResource(R.string.path_relay_udp) to Galaxy.colors.warning
                        PathKind.RELAY_TCP -> stringResource(R.string.path_relay_tcp) to Galaxy.colors.warning
                    }
                    StatusDot(color, size = 7.dp)
                    Spacer(Modifier.width(6.dp))
                    Text("$label · ${stringResource(R.string.path_rtt, p.rttMs.toInt())}", style = MaterialTheme.typography.labelMedium, color = Color.White.copy(alpha = 0.75f))
                }
                ui.display?.let { d ->
                    Text("  ·  ${d.width}×${d.height}", style = MaterialTheme.typography.labelMedium, color = Color.White.copy(alpha = 0.75f))
                }
            }
            Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(2.dp)) {
                DisplayButton(ui, actions)
                QualityButton(ui, actions)
                ToolButton(
                    if (ui.mouseMode) Icons.Rounded.Mouse else Icons.Rounded.TouchApp,
                    stringResource(if (ui.mouseMode) R.string.rd_mode_mouse else R.string.rd_mode_touch),
                    active = ui.mouseMode,
                ) { actions.onMouseMode(!ui.mouseMode) }
                ToolButton(Icons.Rounded.Keyboard, stringResource(R.string.rd_keyboard)) { onKeyboard() }
                ShortcutsButton(ui.platform, actions)
                ClipboardButton(actions)
                ToolButton(Icons.Rounded.LinkOff, stringResource(R.string.rd_disconnect), danger = true) { actions.onDisconnect() }
                Box(Modifier.align(Alignment.CenterVertically)) { InfoButton(stringResource(R.string.rd_mode_info)) }
            }
        }
    }
}

@Composable
private fun ToolButton(icon: ImageVector, label: String, active: Boolean = false, danger: Boolean = false, onClick: () -> Unit) {
    val tint = when {
        danger -> Color(0xFFFF7A85)
        active -> Color(0xFFC6E35A)
        else -> Color.White
    }
    Column(
        Modifier
            .clip(RoundedCornerShape(10.dp))
            .clickable(onClick = onClick)
            .padding(horizontal = 10.dp, vertical = 8.dp)
            .widthIn(min = 56.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(icon, null, tint = tint, modifier = Modifier.size(22.dp))
        Spacer(Modifier.height(4.dp))
        Text(label, style = MaterialTheme.typography.labelSmall, color = tint.copy(alpha = 0.9f), maxLines = 1)
    }
}

@Composable
private fun DisplayButton(ui: RemoteUi, actions: RemoteActions) {
    var open by remember { mutableStateOf(false) }
    Box {
        ToolButton(Icons.Rounded.Monitor, ui.display?.let { stringResource(R.string.rd_display) + " " + it.id } ?: stringResource(R.string.rd_display)) { open = true }
        DropdownMenu(open, { open = false }) {
            ui.displays.forEach { d ->
                DropdownMenuItem(
                    text = {
                        Column {
                            Text(d.name)
                            Text(
                                "${d.width}×${d.height}" + (if (d.primary) "  ·  " + stringResource(R.string.rd_primary) else "") +
                                    (if (d.isVirtual) "  ·  " + stringResource(R.string.rd_virtual) else ""),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    },
                    leadingIcon = { Icon(Icons.Rounded.Monitor, null, tint = if (d.id == ui.displayId) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant) },
                    onClick = {
                        open = false
                        actions.onDisplay(d.id)
                    },
                )
            }
        }
    }
}

@Composable
private fun QualityButton(ui: RemoteUi, actions: RemoteActions) {
    var open by remember { mutableStateOf(false) }
    Box {
        ToolButton(Icons.Rounded.HighQuality, stringResource(ui.quality.label)) { open = true }
        DropdownMenu(open, { open = false }) {
            VideoQuality.entries.forEach { q ->
                DropdownMenuItem(
                    text = { Text(stringResource(q.label) + if (q.bitrateKbps > 0u) "  ·  ${q.bitrateKbps / 1000u} Mbps" else "") },
                    onClick = {
                        open = false
                        actions.onQuality(q)
                    },
                    trailingIcon = { if (q == ui.quality) StatusDot(MaterialTheme.colorScheme.primary) },
                )
            }
        }
    }
}

private data class Shortcut(val label: Int, val events: () -> List<InputEvent>)

private fun shortcutsFor(platform: Platform): List<Shortcut> {
    val mac = platform == Platform.MACOS
    val mod = if (mac) Hid.GUI else Hid.CTRL
    val common = listOf(
        Shortcut(R.string.combo_copy) { Hid.combo(mod, Hid.letter('c')) },
        Shortcut(R.string.combo_paste) { Hid.combo(mod, Hid.letter('v')) },
        Shortcut(R.string.combo_undo) { Hid.combo(mod, Hid.letter('z')) },
        Shortcut(R.string.combo_select_all) { Hid.combo(mod, Hid.letter('a')) },
    )
    return if (mac) {
        listOf(
            Shortcut(R.string.combo_cmd) { Hid.tap(Hid.GUI) },
            Shortcut(R.string.combo_cmd_tab) { Hid.combo(Hid.GUI, Hid.TAB) },
            Shortcut(R.string.combo_spotlight) { Hid.combo(Hid.GUI, Hid.SPACE) },
            Shortcut(R.string.combo_lock) { listOf(InputEvent.SpecialCombo(SpecialCombo.LOCK_SCREEN)) },
        ) + common
    } else {
        listOf(
            Shortcut(R.string.combo_ctrl_alt_del) { listOf(InputEvent.SpecialCombo(SpecialCombo.CTRL_ALT_DEL)) },
            Shortcut(R.string.combo_win) { Hid.tap(Hid.GUI) },
            Shortcut(R.string.combo_alt_tab) { Hid.combo(Hid.ALT, Hid.TAB) },
            Shortcut(R.string.combo_alt_f4) { Hid.combo(Hid.ALT, Hid.F4) },
            Shortcut(R.string.combo_task_manager) { listOf(InputEvent.SpecialCombo(SpecialCombo.TASK_MANAGER)) },
            Shortcut(R.string.combo_show_desktop) { listOf(InputEvent.SpecialCombo(SpecialCombo.SHOW_DESKTOP)) },
            Shortcut(R.string.combo_lock) { listOf(InputEvent.SpecialCombo(SpecialCombo.LOCK_SCREEN)) },
        ) + common
    }
}

@Composable
private fun ShortcutsButton(platform: Platform, actions: RemoteActions) {
    var open by remember { mutableStateOf(false) }
    Box {
        ToolButton(Icons.Rounded.KeyboardCommandKey, stringResource(R.string.rd_shortcuts)) { open = true }
        DropdownMenu(open, { open = false }) {
            shortcutsFor(platform).forEachIndexed { i, s ->
                if (i == shortcutsFor(platform).size - 4) HorizontalDivider()
                DropdownMenuItem(text = { Text(stringResource(s.label)) }, onClick = {
                    open = false
                    actions.onInput(s.events())
                })
            }
        }
    }
}

@Composable
private fun ClipboardButton(actions: RemoteActions) {
    var open by remember { mutableStateOf(false) }
    Box {
        ToolButton(Icons.Rounded.ContentPaste, stringResource(R.string.rd_clipboard)) { open = true }
        DropdownMenu(open, { open = false }) {
            DropdownMenuItem(text = { Text(stringResource(R.string.rd_send_clipboard)) }, onClick = {
                open = false
                actions.onClipboardSend()
            })
            DropdownMenuItem(text = { Text(stringResource(R.string.rd_get_clipboard)) }, onClick = {
                open = false
                actions.onClipboardGet()
            })
        }
    }
}

/**
 * 远程键盘：隐藏的输入框接系统输入法（提交的文字走 Text 事件），
 * 上方一排特殊键走 HID（Esc、Tab、方向键、退格、回车、修饰键）。
 */
@Composable
private fun RemoteKeyboard(platform: Platform, actions: RemoteActions, modifier: Modifier, onClose: () -> Unit) {
    val focus = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    // 输入框里常驻一个占位字符，用来识别退格
    val sentinel = "​"
    var value by remember { mutableStateOf(TextFieldValue(sentinel, TextRange(1))) }
    var ctrl by remember { mutableStateOf(false) }
    var alt by remember { mutableStateOf(false) }
    var gui by remember { mutableStateOf(false) }
    fun withMods(key: UInt): List<InputEvent> {
        val mods = buildList {
            if (ctrl) add(Hid.CTRL)
            if (alt) add(Hid.ALT)
            if (gui) add(Hid.GUI)
        }
        ctrl = false
        alt = false
        gui = false
        return Hid.combo(*(mods + key).toUIntArray())
    }
    LaunchedEffect(Unit) {
        runCatching { focus.requestFocus() }
        keyboard?.show()
    }
    Surface(color = Color(0xF01B1E2B), contentColor = Color.White, modifier = modifier.fillMaxWidth()) {
        Column {
            Row(
                Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(6.dp),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RKey("Esc") { actions.onInput(withMods(Hid.ESC)) }
                RKey("Tab") { actions.onInput(withMods(Hid.TAB)) }
                RKey("Ctrl", ctrl) { ctrl = !ctrl }
                RKey("Alt", alt) { alt = !alt }
                RKey(if (platform == Platform.MACOS) "Cmd" else "Win", gui) { gui = !gui }
                RKey("←") { actions.onInput(withMods(Hid.LEFT)) }
                RKey("↑") { actions.onInput(withMods(Hid.UP)) }
                RKey("↓") { actions.onInput(withMods(Hid.DOWN)) }
                RKey("→") { actions.onInput(withMods(Hid.RIGHT)) }
                RKey("Del") { actions.onInput(withMods(Hid.DELETE)) }
                RKey("Home") { actions.onInput(withMods(Hid.HOME)) }
                RKey("End") { actions.onInput(withMods(Hid.END)) }
                (1..12).forEach { n -> RKey("F$n") { actions.onInput(withMods(Hid.f(n))) } }
                RIcon(Icons.AutoMirrored.Rounded.KeyboardBackspace) { actions.onInput(withMods(Hid.BACKSPACE)) }
                RIcon(Icons.AutoMirrored.Rounded.KeyboardReturn) { actions.onInput(withMods(Hid.ENTER)) }
                RIcon(Icons.Rounded.Close) {
                    keyboard?.hide()
                    onClose()
                }
            }
            BasicTextField(
                value = value,
                onValueChange = { nv ->
                    val t = nv.text
                    when {
                        !t.startsWith(sentinel) -> actions.onInput(Hid.tap(Hid.BACKSPACE))
                        t.length > 1 -> {
                            val typed = t.removePrefix(sentinel)
                            if (ctrl || alt || gui) {
                                typed.filter { it.isLetter() }.forEach { c -> actions.onInput(withMods(Hid.letter(c))) }
                            } else {
                                actions.onInput(listOf(InputEvent.Text(typed)))
                            }
                        }
                    }
                    value = TextFieldValue(sentinel, TextRange(1))
                },
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Text, imeAction = ImeAction.Send, autoCorrectEnabled = false),
                keyboardActions = KeyboardActions(onSend = { actions.onInput(Hid.tap(Hid.ENTER)) }),
                textStyle = MaterialTheme.typography.bodySmall.copy(color = Color.Transparent),
                modifier = Modifier.size(1.dp).focusRequester(focus),
            )
        }
    }
}

@Composable
private fun RKey(label: String, active: Boolean = false, onClick: () -> Unit) {
    Box(
        Modifier
            .height(36.dp)
            .widthIn(min = 40.dp)
            .clip(RoundedCornerShape(8.dp))
            .background(if (active) Color(0xFFC6E35A) else Color.White.copy(alpha = 0.1f))
            .clickable(onClick = onClick)
            .padding(horizontal = 10.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(label, color = if (active) Color(0xFF1B1E2B) else Color.White, fontFamily = FontFamily.Monospace, fontSize = 13.sp)
    }
}

@Composable
private fun RIcon(icon: ImageVector, onClick: () -> Unit) {
    Box(
        Modifier.height(36.dp).width(44.dp).clip(RoundedCornerShape(8.dp)).background(Color.White.copy(alpha = 0.1f)).clickable(onClick = onClick),
        contentAlignment = Alignment.Center,
    ) { Icon(icon, null, tint = Color.White, modifier = Modifier.size(20.dp)) }
}
