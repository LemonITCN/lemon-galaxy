package cn.lemonit.galaxy.ui.remote

import android.content.res.Configuration
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.KeyboardBackspace
import androidx.compose.material.icons.automirrored.rounded.KeyboardReturn
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.core.bridge.DisplayInfo
import cn.lemonit.galaxy.core.bridge.InputEvent
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.PointerButton
import cn.lemonit.galaxy.core.bridge.RecordingInfo
import cn.lemonit.galaxy.data.VideoQuality
import cn.lemonit.galaxy.ui.components.Clock
import cn.lemonit.galaxy.ui.components.formatBytes
import kotlinx.coroutines.delay
import kotlin.math.abs
import kotlin.math.roundToInt

interface RemoteActions {
    fun onInput(events: List<InputEvent>)
    fun onDisplay(id: UInt)
    fun onQuality(q: VideoQuality)
    fun onMouseMode(on: Boolean)

    /** 悬浮鼠标按键显示 / 隐藏（调用方记住选择）。 */
    fun onMouseKeys(on: Boolean) = Unit

    /** 拖把手挪了悬浮鼠标按键的位置（比例），调用方记住。 */
    fun onMouseKeysMoved(x: Float, y: Float) = Unit
    fun onClipboardSend()
    fun onClipboardGet()
    fun onDisconnect()

    /** 隐私屏开关（对端能力里有 "privacy" 时才显示）。 */
    fun onPrivacy(on: Boolean) = Unit

    /** 声音开关（对端能力里有 "audio" 时才显示）。 */
    fun onSound(on: Boolean) = Unit

    /** 录像按钮：没在录就开始，在录就停止。 */
    fun onRecord() = Unit

    /** 打开这台设备的文件管理（会离开远程画面，返回时重新打开）。 */
    fun onFiles() = Unit

    /** 横竖屏切换。 */
    fun onRotate() = Unit

    /** 在被控端加一块虚拟屏（会话的虚拟屏上限大于 0 时才有入口），加好后直接切过去。 */
    fun onAddVirtualDisplay(width: Int, height: Int, refresh: Int) = Unit

    /** 删掉自己加的虚拟屏。 */
    fun onRemoveVirtualDisplay(id: UInt) = Unit
}

private fun click(button: PointerButton, x: Int, y: Int) = listOf(
    InputEvent.MouseMove(x, y),
    InputEvent.MouseButton(button, true, x, y),
    InputEvent.MouseButton(button, false, x, y),
)

/** 屏幕上方的短暂提示。 */
private data class Notice(val title: String, val subtitle: String? = null)

/**
 * 远程桌面：画面（硬件解码 SurfaceView，测试时为占位图）+ 手势 + 右侧贴边菜单。
 * [video] 由调用方提供，传入的 Modifier 已带好位置与尺寸（始终按被控屏幕比例，放大后可以超出屏幕）。
 * 光标位置由本机记（[RemoteViewState.pointer]，手指一动立即更新）。鼠标模式且被控端支持 "client_cursor" 时，
 * 被控端画面里不画光标，箭头在这里本地画（跟手）；其余情况光标在画面里，这里不再叠加。
 */
@Composable
fun RemoteDesktopScreen(
    ui: RemoteUi,
    actions: RemoteActions,
    video: @Composable (Modifier) -> Unit,
    initialMenuOpen: Boolean = false,
    initialPanel: RemotePanel? = null,
    initialKeyboard: Boolean = false,
    /** 打开时显示器面板里已展开加虚拟屏表单（截图用） */
    initialAddDisplay: Boolean = false,
    state: RemoteViewState = remember { RemoteViewState() },
) {
    val density = LocalDensity.current
    var menuOpen by remember { mutableStateOf(initialMenuOpen || initialPanel != null) }
    var panel by remember { mutableStateOf(initialPanel) }
    // 显示器面板里展开的加虚拟屏表单
    var addDisplayOpen by remember { mutableStateOf(initialAddDisplay && initialPanel == RemotePanel.DISPLAY) }
    var keyboardOpen by remember { mutableStateOf(initialKeyboard) }
    var notice by remember { mutableStateOf<Notice?>(null) }
    val currentUi by rememberUpdatedState(ui)
    val currentActions by rememberUpdatedState(actions)
    val zoomText = stringResource(R.string.rd_zoom_level)
    val displayText = stringResource(R.string.rd_display_n)
    val touchText = stringResource(R.string.rd_mode_touch)
    val mouseText = stringResource(R.string.rd_mode_mouse)
    val touchHint = stringResource(R.string.rd_mode_touch_hint)
    val mouseHint = stringResource(R.string.rd_mode_mouse_hint)
    val primaryText = stringResource(R.string.rd_primary)
    val vdAddedText = stringResource(R.string.rd_vd_added)
    val vdRemovedText = stringResource(R.string.rd_vd_removed)
    val zoomNotice by rememberUpdatedState { v: Viewport -> notice = Notice(String.format(zoomText, (v.zoom * 100).roundToInt())) }

    fun showDisplay(d: DisplayInfo) {
        val list = currentUi.displays
        val i = list.indexOfFirst { it.id == d.id }
        notice = Notice(
            String.format(displayText, i + 1, list.size),
            d.name + " · ${d.width}×${d.height}" + if (d.primary) " · $primaryText" else "",
        )
    }

    BoxWithConstraints(Modifier.fillMaxSize().background(Color.Black).imePadding()) {
        val cw = with(density) { maxWidth.toPx() }
        val ch = with(density) { maxHeight.toPx() }
        // 视图或被控分辨率变化：旋转保持倍数与中心，换分辨率回到适应屏幕（不在组合期订阅缩放，缩放只触发重新摆放）
        val old = Snapshot.withoutReadObservation { state.viewport }
        val synced = old.resized(cw, ch, ui.remoteW, ui.remoteH)
        if (synced != old) state.viewport = synced
        // 换屏幕（含第一次打开）：光标放到控制器给的初始位置
        if (Snapshot.withoutReadObservation { state.pointerDisplay } != ui.displayId) {
            state.pointerDisplay = ui.displayId
            state.pointerSeq = ui.cursorSeq
            state.pointer.reset(ui.cursorX, ui.cursorY)
        }
        // 被控端回报了光标（对面有人动了鼠标，或自己移动的回声）：按本机停手规则决定采不采用
        LaunchedEffect(ui.cursorSeq) {
            if (ui.cursorSeq != state.pointerSeq) {
                state.pointerSeq = ui.cursorSeq
                state.pointer.hostReport(ui.cursorX, ui.cursorY, ui.cursorVisible, Clock.now())
            }
        }

        val keys = remember { MouseKeysState() }
        val keysShown = ui.mouseMode && ui.mouseKeys && ui.status == RemoteStatus.STREAMING
        fun sendKeys(presses: List<MouseKeysLogic.Press>) {
            if (presses.isEmpty()) return
            val x = state.pointer.x
            val y = state.pointer.y
            currentActions.onInput(presses.map { InputEvent.MouseButton(it.button, it.down, x, y) })
            state.pointer.moved(x, y, Clock.now())
        }
        // 按键条收起（切到触控、关掉按键、断开）：按着的键抬起
        LaunchedEffect(keysShown) { if (!keysShown) sendKeys(keys.run { cancel() }) }
        DisposableEffect(Unit) { onDispose { sendKeys(keys.run { cancel() }) } }

        val sink = remember {
            object : RemoteGestureSink {
                var dragButtonDown = false
                fun remote(o: Offset) = state.viewport.toRemote(o.x, o.y)
                fun cursor() = state.pointer.x to state.pointer.y
                fun send(e: List<InputEvent>) {
                    // 本机先记下光标位置（不等被控端回报），再发出去
                    val now = Clock.now()
                    e.forEach { ev ->
                        when (ev) {
                            is InputEvent.MouseMove -> state.pointer.moved(ev.x, ev.y, now)
                            is InputEvent.MouseButton -> state.pointer.moved(ev.x, ev.y, now)
                            else -> Unit
                        }
                    }
                    currentActions.onInput(e)
                }

                override fun tap(at: Offset) {
                    // 鼠标按键按着或锁定时（正在拖拽），画面轻点不再单击
                    if (currentUi.mouseMode && keys.logic.busy) return
                    val (x, y) = if (currentUi.mouseMode) cursor() else remote(at)
                    send(click(PointerButton.LEFT, x, y))
                }

                override fun twoFingerTap() {
                    val (x, y) = cursor()
                    send(click(PointerButton.RIGHT, x, y))
                }

                override fun twoFingerDoubleTap(at: Offset) {
                    state.viewport = state.viewport.toggled(at.x, at.y)
                    zoomNotice(state.viewport)
                }

                override fun longPress(at: Offset) {
                    if (!currentUi.mouseMode) {
                        val (x, y) = remote(at)
                        send(click(PointerButton.RIGHT, x, y))
                    }
                }

                override fun dragStart(at: Offset, afterLongPress: Boolean) {
                    if (currentUi.mouseMode) {
                        if (afterLongPress) {
                            val (x, y) = cursor()
                            send(listOf(InputEvent.MouseButton(PointerButton.LEFT, true, x, y)))
                            dragButtonDown = true
                        }
                    } else if (!afterLongPress) {
                        val (x, y) = remote(at)
                        send(listOf(InputEvent.MouseMove(x, y), InputEvent.MouseButton(PointerButton.LEFT, true, x, y)))
                        dragButtonDown = true
                    }
                }

                override fun dragMove(at: Offset, delta: Offset) {
                    if (currentUi.mouseMode) {
                        // 鼠标按键刚按下还没定性：先按下，拖拽从起点开始
                        sendKeys(keys.run { beforeMove() })
                        // 触控板：相对移动，带一点加速；放大后画面跟着光标走，光标始终看得见
                        val k = state.viewport.remotePerViewPx() * 1.6f
                        val (cx, cy) = cursor()
                        val nx = (cx + delta.x * k).roundToInt().coerceIn(0, (currentUi.remoteW - 1).coerceAtLeast(0))
                        val ny = (cy + delta.y * k).roundToInt().coerceIn(0, (currentUi.remoteH - 1).coerceAtLeast(0))
                        send(listOf(InputEvent.MouseMove(nx, ny)))
                        state.viewport = state.viewport.follow(nx, ny)
                    } else if (dragButtonDown) {
                        val (x, y) = remote(at)
                        send(listOf(InputEvent.MouseMove(x, y)))
                    }
                }

                override fun dragEnd() {
                    if (dragButtonDown) {
                        val (x, y) = cursor()
                        send(listOf(InputEvent.MouseButton(PointerButton.LEFT, false, x, y)))
                        dragButtonDown = false
                    }
                }

                override fun scroll(dx: Float, dy: Float) {
                    // 像触控板「自然滚动」：手指上推 = 内容跟着上移。协议里 dy 为正是滚轮往上推（内容下移），
                    // 所以手指位移直接作为滚动量；按视图像素换算成被控像素
                    val k = state.viewport.remotePerViewPx()
                    send(listOf(InputEvent.Wheel((dx * k).roundToInt(), (dy * k).roundToInt())))
                }

                override fun pinch(factor: Float, from: Offset, to: Offset) {
                    state.viewport = state.viewport.pinched(factor, from.x, from.y, to.x, to.y)
                    zoomNotice(state.viewport)
                }

                override fun twoFingerPan(delta: Offset) {
                    state.viewport = state.viewport.panned(delta.x, delta.y)
                }
            }
        }
        Box(
            Modifier
                .fillMaxSize()
                .clipToBounds()
                .testTag("remote-surface")
                .pointerInput(Unit) { detectRemoteGestures(sink) { !currentUi.mouseMode && state.viewport.isZoomed } },
        ) {
            if (ui.remoteW > 0) {
                // 自己摆放：按画面实际大小测量（不受父布局夹住，宽高同比例），放到左上角坐标处
                video(
                    Modifier
                        .layout { m, c ->
                            val v = state.viewport
                            val p = m.measure(Constraints.fixed(v.contentW.roundToInt().coerceAtLeast(1), v.contentH.roundToInt().coerceAtLeast(1)))
                            layout(c.maxWidth, c.maxHeight) { p.place(v.left.roundToInt(), v.top.roundToInt()) }
                        }
                        .testTag("remote-video"),
                )
            }
            if (ui.localCursor && ui.status == RemoteStatus.STREAMING) {
                LocalPointerArrow(state.pointer, { state.viewport }, Modifier.fillMaxSize())
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
        // 悬浮鼠标按键放在画面之外（同层兄弟），按键上的手指不会进画面手势
        if (keysShown) {
            // 左侧留出刘海 / 圆角，右侧留出菜单把手
            MouseKeysBar(
                keys, ui.mouseKeysX, ui.mouseKeysY,
                areaW = cw - with(density) { 72.dp.toPx() }, areaH = ch - with(density) { 24.dp.toPx() },
                onPress = ::sendKeys,
                onMoved = { x, y -> currentActions.onMouseKeysMoved(x, y) },
                modifier = Modifier.windowInsetsPadding(WindowInsets.displayCutout).padding(start = 12.dp, top = 12.dp),
            )
        }
        if (keyboardOpen) {
            RemoteKeyboard(ui.platform, actions, Modifier.align(Alignment.BottomCenter)) { keyboardOpen = false }
        }
        val elapsed = recordingElapsed(ui.recordingSince)
        // 菜单收起时左上角一直显示录像中（红点 + 计时）
        if (elapsed != null && !menuOpen) {
            RecordingBadge(elapsed, Modifier.align(Alignment.TopStart).windowInsetsPadding(WindowInsets.displayCutout).padding(12.dp))
        }
        val shown = notice
        AnimatedVisibility(
            shown != null,
            modifier = Modifier.align(Alignment.TopCenter).windowInsetsPadding(WindowInsets.displayCutout).padding(top = 16.dp),
            enter = fadeIn(),
            exit = fadeOut(),
        ) {
            var last by remember { mutableStateOf(Notice("")) }
            if (shown != null) last = shown
            RemoteNotice(last.title, last.subtitle, Modifier)
        }
        LaunchedEffect(shown) {
            if (shown != null) {
                delay(1600)
                notice = null
            }
        }
        // 加好虚拟屏：表单收起回到屏幕卡片（控制器已切到新屏），轻提示一下；删掉同样提示
        var seenAdded by remember { mutableIntStateOf(ui.virtualAddedSeq) }
        LaunchedEffect(ui.virtualAddedSeq) {
            if (ui.virtualAddedSeq != seenAdded) {
                seenAdded = ui.virtualAddedSeq
                addDisplayOpen = false
                notice = Notice(String.format(vdAddedText, ui.virtualAdded?.let { "${it.width}×${it.height}" } ?: ""))
            }
        }
        var seenRemoved by remember { mutableIntStateOf(ui.virtualRemovedSeq) }
        LaunchedEffect(ui.virtualRemovedSeq) {
            if (ui.virtualRemovedSeq != seenRemoved) {
                seenRemoved = ui.virtualRemovedSeq
                notice = Notice(vdRemovedText)
            }
        }
        val configuration = LocalConfiguration.current
        val portrait = configuration.orientation == Configuration.ORIENTATION_PORTRAIT
        // 只在“是否放大 / 是否原始像素”变化时重组，捏合过程中不跟着每帧重组
        val zoomFlags by remember(state) {
            derivedStateOf {
                val v = state.viewport
                val canActual = v.actualSizeZoom > 1.01f
                Triple(v.isZoomed, canActual, canActual && abs(v.zoom - v.actualSizeZoom) < 0.01f)
            }
        }
        BackHandler(enabled = menuOpen) {
            when {
                addDisplayOpen -> addDisplayOpen = false
                panel != null -> panel = null
                else -> menuOpen = false
            }
        }
        RemoteDock(
            ui = ui,
            elapsed = elapsed,
            expanded = menuOpen,
            panel = panel,
            addDisplayOpen = addDisplayOpen,
            view = DockView(
                zoomed = zoomFlags.first,
                canActualSize = zoomFlags.second,
                atActualSize = zoomFlags.third,
                portrait = portrait,
            ),
            actions = actions,
            callbacks = DockCallbacks(
                onSwitchDisplay = { step ->
                    val list = currentUi.displays
                    if (list.size > 1) {
                        val i = list.indexOfFirst { it.id == currentUi.displayId }.coerceAtLeast(0)
                        // 循环切换：两块屏时上滑下滑都能换过去
                        val next = list[((i + step) % list.size + list.size) % list.size]
                        showDisplay(next)
                        currentActions.onDisplay(next.id)
                    }
                },
                onPickDisplay = { id ->
                    currentUi.displays.firstOrNull { it.id == id }?.let(::showDisplay)
                    currentActions.onDisplay(id)
                },
                onKeyboard = {
                    keyboardOpen = !keyboardOpen
                    menuOpen = false
                    panel = null
                },
                onMouseMode = {
                    val on = !currentUi.mouseMode
                    notice = Notice(if (on) mouseText else touchText, if (on) mouseHint else touchHint)
                    currentActions.onMouseMode(on)
                },
                onFit = { state.viewport = state.viewport.fit() },
                onActualSize = {
                    val v = state.viewport
                    state.viewport = v.zoomedTo(v.actualSizeZoom, v.containerW / 2f, v.containerH / 2f)
                },
                onRotate = { currentActions.onRotate() },
            ),
            onExpanded = { open ->
                menuOpen = open
                if (!open) {
                    panel = null
                    addDisplayOpen = false
                }
            },
            onPanel = {
                panel = it
                addDisplayOpen = false
            },
            onAddDisplayOpen = { addDisplayOpen = it },
        )
    }
}

/** 录像计时（每秒刷新）；没在录返回 null。 */
@Composable
private fun recordingElapsed(since: Long?): String? {
    if (since == null) return null
    var now by remember(since) { mutableLongStateOf(Clock.now()) }
    LaunchedEffect(since) {
        while (true) {
            delay(1000 - (Clock.now() - since).coerceAtLeast(0) % 1000)
            now = Clock.now()
        }
    }
    return formatElapsed(now - since)
}

private val RecordRed = Color(0xFFFF4D4F)

@Composable
private fun RecordingBadge(elapsed: String, modifier: Modifier) {
    Row(
        modifier
            .clip(RoundedCornerShape(50))
            .background(Color(0xCC1B1E2B))
            .padding(horizontal = 10.dp, vertical = 5.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(8.dp).clip(CircleShape).background(RecordRed))
        Spacer(Modifier.width(6.dp))
        Text(elapsed, color = Color.White, style = MaterialTheme.typography.labelMedium, fontFamily = FontFamily.Monospace)
    }
}

/** 录像停止后：时长、大小、保存位置，可分享或存到相册。 */
@Composable
fun RecordingSavedDialog(
    info: RecordingInfo,
    location: String,
    canSaveToGallery: Boolean,
    onShare: () -> Unit,
    onSaveToGallery: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.rd_record_saved)) },
        text = {
            Column {
                Text(stringResource(R.string.rd_record_summary, formatElapsed(info.durationMs.toLong()), formatBytes(info.bytes.toLong())))
                if (info.paths.size > 1) {
                    Spacer(Modifier.height(6.dp))
                    Text(stringResource(R.string.rd_record_parts, info.paths.size), style = MaterialTheme.typography.bodySmall)
                }
                Spacer(Modifier.height(10.dp))
                Text(
                    location,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        confirmButton = { TextButton(onClick = onShare) { Text(stringResource(R.string.rd_record_share)) } },
        dismissButton = {
            Row {
                if (canSaveToGallery) TextButton(onClick = onSaveToGallery) { Text(stringResource(R.string.rd_record_save_gallery)) }
                TextButton(onClick = onDismiss) { Text(stringResource(R.string.done)) }
            }
        },
    )
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
