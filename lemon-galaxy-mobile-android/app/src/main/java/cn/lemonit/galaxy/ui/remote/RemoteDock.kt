package cn.lemonit.galaxy.ui.remote

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.mandatorySystemGestures
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.systemGestureExclusion
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.VolumeOff
import androidx.compose.material.icons.automirrored.rounded.VolumeUp
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material.icons.rounded.Apps
import androidx.compose.material.icons.rounded.ChevronLeft
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.FiberManualRecord
import androidx.compose.material.icons.rounded.FitScreen
import androidx.compose.material.icons.rounded.FolderOpen
import androidx.compose.material.icons.rounded.Keyboard
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.KeyboardArrowUp
import androidx.compose.material.icons.rounded.KeyboardCommandKey
import androidx.compose.material.icons.rounded.LinkOff
import androidx.compose.material.icons.rounded.Monitor
import androidx.compose.material.icons.rounded.Mouse
import androidx.compose.material.icons.rounded.ScreenRotation
import androidx.compose.material.icons.rounded.Stop
import androidx.compose.material.icons.rounded.TouchApp
import androidx.compose.material.icons.rounded.VisibilityOff
import androidx.compose.material.icons.rounded.ZoomIn
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.zIndex
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.core.bridge.ComboKey
import cn.lemonit.galaxy.core.bridge.InputEvent
import cn.lemonit.galaxy.core.bridge.PathInfo
import cn.lemonit.galaxy.core.bridge.PathKind
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.data.VideoQuality
import cn.lemonit.galaxy.ui.components.InfoButton
import cn.lemonit.galaxy.ui.theme.Galaxy
import kotlin.math.abs
import kotlin.math.roundToInt

/** 右侧菜单的二级面板。 */
enum class RemotePanel { DISPLAY, SHORTCUTS, TOOLS }

internal object DockColors {
    val Bg = Color(0xF21B1E2B)
    val Handle = Color(0xE61B1E2B)
    val Line = Color.White.copy(alpha = 0.14f)
    val Chip = Color.White.copy(alpha = 0.08f)
    val Accent = Color(0xFFC6E35A)
    val OnAccent = Color(0xFF1B1E2B)
    val Danger = Color(0xFFFF7A85)
    val Record = Color(0xFFFF4D4F)
    val Dim = Color.White.copy(alpha = 0.62f)
}

/** 连接路径的简称与颜色（局域网 / 直连为绿，中转为黄）。 */
@Composable
internal fun pathLabel(p: PathInfo): Pair<String, Color> = when (p.kind) {
    PathKind.LAN -> stringResource(R.string.path_lan) to Galaxy.colors.online
    PathKind.DIRECT -> stringResource(R.string.path_direct) to Galaxy.colors.online
    PathKind.RELAY_UDP -> stringResource(R.string.path_relay_udp) to Galaxy.colors.warning
    PathKind.RELAY_TCP -> stringResource(R.string.path_relay_tcp) to Galaxy.colors.warning
    PathKind.CONNECTING, PathKind.CLOSED -> "—" to Galaxy.colors.offline
}

/** 右侧菜单里调用方要处理的操作（缩放 / 旋转在界面层，其余转给 [RemoteActions]）。 */
internal class DockCallbacks(
    val onSwitchDisplay: (step: Int) -> Unit,
    val onPickDisplay: (id: UInt) -> Unit,
    val onKeyboard: () -> Unit,
    val onMouseMode: () -> Unit,
    val onFit: () -> Unit,
    val onActualSize: () -> Unit,
    val onRotate: () -> Unit,
)

/** 画面状态里右侧菜单要用到的部分。 */
internal class DockView(
    val zoomed: Boolean,
    val canActualSize: Boolean,
    val atActualSize: Boolean,
    val portrait: Boolean,
)

/**
 * 右侧贴边菜单（参照网易 UU 远程手机端的侧边工具栏）：
 * - 收起：贴右边缘的把手，轻点展开，上下滑动切换显示器，长按后拖动改变位置；
 * - 展开：右侧竖排菜单（不滚动，高度自适应），菜单上上下滑动同样切换显示器；
 * - 二级面板从菜单左侧弹出，放显示器 / 画质 / 快捷键 / 工具等详细功能。
 */
@Composable
internal fun RemoteDock(
    ui: RemoteUi,
    elapsed: String?,
    expanded: Boolean,
    panel: RemotePanel?,
    view: DockView,
    actions: RemoteActions,
    callbacks: DockCallbacks,
    onExpanded: (Boolean) -> Unit,
    onPanel: (RemotePanel?) -> Unit,
) {
    val density = LocalDensity.current
    val gestures = WindowInsets.mandatorySystemGestures
    BoxWithConstraints(
        Modifier
            .fillMaxSize()
            // 刘海在右侧时贴着刘海边，不压在挖孔下面
            .windowInsetsPadding(WindowInsets.displayCutout.only(WindowInsetsSides.End)),
    ) {
        val maxH = with(density) { maxHeight.toPx() }
        // 把手可放的竖直范围：避开顶部 / 底部系统手势区
        val topLimit = gestures.getTop(density) + with(density) { 8.dp.toPx() }
        val handleH = with(density) { HandleTouchH.toPx() }
        val bottomLimit = maxH - gestures.getBottom(density) - with(density) { 8.dp.toPx() } - handleH
        var handleFrac by remember { mutableFloatStateOf(0.4f) }
        val handleY = (topLimit + (bottomLimit - topLimit).coerceAtLeast(0f) * handleFrac).coerceIn(0f, (maxH - handleH).coerceAtLeast(0f))

        AnimatedVisibility(expanded, enter = fadeIn(), exit = fadeOut()) {
            // 点菜单外任意处收起（这层吃掉点按，不会误点到远端）
            Box(
                Modifier
                    .fillMaxSize()
                    .background(Color.Black.copy(alpha = 0.18f))
                    .pointerInput(Unit) { detectTapGestures { onExpanded(false) } },
            )
        }
        if (!expanded) {
            DockHandle(
                ui = ui,
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .offset { IntOffset(0, handleY.roundToInt()) },
                canSwitch = ui.displays.size > 1,
                onTap = { onExpanded(true) },
                onSwipe = callbacks.onSwitchDisplay,
                onMove = { dy ->
                    val range = (bottomLimit - topLimit).coerceAtLeast(1f)
                    handleFrac = (handleFrac + dy / range).coerceIn(0f, 1f)
                },
            )
        }
        AnimatedVisibility(
            expanded,
            // 菜单压在收起层上面
            modifier = Modifier.align(Alignment.CenterEnd).zIndex(1f),
            enter = slideInHorizontally { it } + fadeIn(),
            exit = slideOutHorizontally { it } + fadeOut(),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (panel != null) {
                    val panelMaxW = (this@BoxWithConstraints.maxWidth - RailW - 24.dp).coerceIn(200.dp, 384.dp)
                    DockPanel(
                        panel, ui, elapsed, view, actions, callbacks,
                        onClose = { onPanel(null) },
                        modifier = Modifier
                            .widthIn(max = panelMaxW)
                            .heightIn(max = this@BoxWithConstraints.maxHeight - 16.dp)
                            .padding(end = 8.dp),
                    )
                }
                DockRail(
                    ui, elapsed, panel, this@BoxWithConstraints.maxHeight - 12.dp, actions, callbacks,
                    onPanel = { p -> onPanel(if (p == panel) null else p) },
                    onCollapse = { onExpanded(false) },
                )
            }
        }
    }
}

private val RailW = 64.dp
private val HandleW = 22.dp
private val HandleH = 64.dp
private val HandleTouchH = 84.dp

/** 收起状态的把手：贴右边缘，左侧圆角。 */
@Composable
private fun DockHandle(
    ui: RemoteUi,
    modifier: Modifier,
    canSwitch: Boolean,
    onTap: () -> Unit,
    onSwipe: (Int) -> Unit,
    onMove: (Float) -> Unit,
) {
    val density = LocalDensity.current
    val haptic = LocalHapticFeedback.current
    val swipePx = with(density) { 28.dp.toPx() }
    val tap by rememberUpdatedState(onTap)
    val swipe by rememberUpdatedState(onSwipe)
    val move by rememberUpdatedState(onMove)
    val expandText = stringResource(R.string.rd_menu_expand)
    Box(
        modifier
            .size(HandleW + 14.dp, HandleTouchH)
            .testTag("remote-dock-handle")
            .semantics { contentDescription = expandText }
            // 右边缘是系统返回手势区：把手这一段让给应用
            .systemGestureExclusion()
            .pointerInput(canSwitch) {
                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false)
                    down.consume()
                    val slop = viewConfiguration.touchSlop
                    var dy = 0f
                    var up = false
                    // 0 未定 1 滑动切换显示器 2 拖动改位置
                    var mode = 0
                    withTimeoutOrNull(viewConfiguration.longPressTimeoutMillis) {
                        while (true) {
                            val c = awaitPointerEvent().changes.firstOrNull { it.id == down.id } ?: return@withTimeoutOrNull
                            if (!c.pressed) {
                                c.consume()
                                up = true
                                return@withTimeoutOrNull
                            }
                            dy += c.positionChange().y
                            c.consume()
                            if (abs(dy) > slop) {
                                mode = if (canSwitch) 1 else 2
                                if (mode == 2) move(dy)
                                return@withTimeoutOrNull
                            }
                        }
                    }
                    if (up) {
                        tap()
                        return@awaitEachGesture
                    }
                    if (mode == 0) {
                        // 长按：进入拖动改位置
                        mode = 2
                        haptic.performHapticFeedback(HapticFeedbackType.LongPress)
                    }
                    while (true) {
                        val c = awaitPointerEvent().changes.firstOrNull { it.id == down.id } ?: break
                        if (!c.pressed) {
                            c.consume()
                            break
                        }
                        val d = c.positionChange().y
                        dy += d
                        c.consume()
                        if (mode == 2) move(d)
                    }
                    // 手指上滑 = 下一个显示器
                    if (mode == 1 && abs(dy) > swipePx) swipe(if (dy < 0) 1 else -1)
                }
            },
        contentAlignment = Alignment.CenterEnd,
    ) {
        val shape = RoundedCornerShape(topStart = 12.dp, bottomStart = 12.dp)
        Column(
            Modifier
                .size(HandleW, HandleH)
                .clip(shape)
                .background(DockColors.Handle)
                .border(1.dp, DockColors.Line, shape),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            // 连接路径：绿 = 局域网 / 直连，黄 = 中转；录像中为红
            val dot = when {
                ui.recording -> DockColors.Record
                else -> ui.path?.let { pathLabel(it).second }
            }
            if (dot != null) {
                Box(Modifier.size(6.dp).clip(CircleShape).background(dot))
                Spacer(Modifier.height(4.dp))
            }
            Icon(Icons.Rounded.ChevronLeft, null, tint = Color.White, modifier = Modifier.size(20.dp))
            if (ui.displays.size > 1) {
                Spacer(Modifier.height(4.dp))
                DisplayDots(ui)
            }
        }
    }
}

/** 竖排小圆点：几块屏、当前第几块。 */
@Composable
private fun DisplayDots(ui: RemoteUi) {
    Column(verticalArrangement = Arrangement.spacedBy(3.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        ui.displays.take(5).forEach { d ->
            val on = d.id == ui.displayId
            Box(Modifier.size(if (on) 5.dp else 4.dp).clip(CircleShape).background(if (on) DockColors.Accent else Color.White.copy(alpha = 0.4f)))
        }
    }
}

/** 展开后的右侧竖排菜单：固定几项，按可用高度自适应，不需要滚动。 */
@Composable
private fun DockRail(
    ui: RemoteUi,
    elapsed: String?,
    panel: RemotePanel?,
    maxHeight: Dp,
    actions: RemoteActions,
    callbacks: DockCallbacks,
    onPanel: (RemotePanel) -> Unit,
    onCollapse: () -> Unit,
) {
    val density = LocalDensity.current
    val swipePx = with(density) { 28.dp.toPx() }
    val switch by rememberUpdatedState(callbacks.onSwitchDisplay)
    val multi = ui.displays.size > 1
    // 固定部分：收起键 + 连接状态 + 显示器切换
    val fixed = 24.dp + 30.dp + (if (multi) 54.dp else 0.dp) + 21.dp
    val itemH = ((maxHeight - fixed) / 6).coerceIn(36.dp, 54.dp)
    val shape = RoundedCornerShape(topStart = 16.dp, bottomStart = 16.dp)
    Column(
        Modifier
            .width(RailW)
            .testTag("remote-dock-rail")
            .systemGestureExclusion()
            .pointerInput(Unit) {
                // 菜单上上下滑动：切换显示器。在 Initial 阶段先于子项看到事件，
                // 越过阈值后吃掉移动事件，子项的点按随之取消
                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
                    val slop = viewConfiguration.touchSlop
                    var dx = 0f
                    var dy = 0f
                    var dragging = false
                    while (true) {
                        val c = awaitPointerEvent(PointerEventPass.Initial).changes.firstOrNull { it.id == down.id } ?: break
                        if (!c.pressed) {
                            if (dragging) c.consume()
                            break
                        }
                        val d = c.positionChange()
                        dx += d.x
                        dy += d.y
                        if (!dragging && abs(dy) > slop && abs(dy) > abs(dx)) dragging = true
                        if (dragging) c.consume()
                    }
                    if (dragging && abs(dy) > swipePx) switch(if (dy < 0) 1 else -1)
                }
            }
            .clip(shape)
            .background(DockColors.Bg)
            .border(1.dp, DockColors.Line, shape)
            .padding(vertical = 6.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(
            Modifier.fillMaxWidth().height(24.dp).clickable(onClick = onCollapse),
            contentAlignment = Alignment.Center,
        ) { Icon(Icons.Rounded.ChevronRight, stringResource(R.string.rd_menu_collapse), tint = DockColors.Dim, modifier = Modifier.size(20.dp)) }
        RailStatus(ui)
        if (multi) {
            val idx = ui.displays.indexOfFirst { it.id == ui.displayId }.coerceAtLeast(0)
            Box(Modifier.fillMaxWidth().height(17.dp).clickable { callbacks.onSwitchDisplay(-1) }, contentAlignment = Alignment.Center) {
                Icon(Icons.Rounded.KeyboardArrowUp, stringResource(R.string.rd_prev_display), tint = Color.White, modifier = Modifier.size(18.dp))
            }
            Text(
                "${idx + 1} / ${ui.displays.size}",
                color = DockColors.Accent,
                style = MaterialTheme.typography.labelMedium,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.height(20.dp).padding(top = 2.dp),
            )
            Box(Modifier.fillMaxWidth().height(17.dp).clickable { callbacks.onSwitchDisplay(1) }, contentAlignment = Alignment.Center) {
                Icon(Icons.Rounded.KeyboardArrowDown, stringResource(R.string.rd_next_display), tint = Color.White, modifier = Modifier.size(18.dp))
            }
        }
        Box(Modifier.padding(vertical = 4.dp).width(36.dp).height(1.dp).background(DockColors.Line))
        RailItem(Icons.Rounded.Monitor, stringResource(R.string.rd_display), itemH, selected = panel == RemotePanel.DISPLAY) { onPanel(RemotePanel.DISPLAY) }
        RailItem(Icons.Rounded.Keyboard, stringResource(R.string.rd_keyboard), itemH) { callbacks.onKeyboard() }
        RailItem(Icons.Rounded.KeyboardCommandKey, stringResource(R.string.rd_shortcuts), itemH, selected = panel == RemotePanel.SHORTCUTS) { onPanel(RemotePanel.SHORTCUTS) }
        RailItem(
            if (ui.mouseMode) Icons.Rounded.Mouse else Icons.Rounded.TouchApp,
            stringResource(if (ui.mouseMode) R.string.rd_mode_mouse else R.string.rd_mode_touch),
            itemH,
            active = ui.mouseMode,
        ) { callbacks.onMouseMode() }
        RailItem(
            Icons.Rounded.Apps,
            elapsed ?: stringResource(R.string.rd_tools),
            itemH,
            selected = panel == RemotePanel.TOOLS,
            badge = if (ui.recording) DockColors.Record else null,
        ) { onPanel(RemotePanel.TOOLS) }
        RailItem(Icons.Rounded.LinkOff, stringResource(R.string.rd_disconnect), itemH, danger = true) { actions.onDisconnect() }
    }
}

/** 菜单顶部的连接状态：路径（局域网 / 直连 / 中转）+ 往返延迟。 */
@Composable
private fun RailStatus(ui: RemoteUi) {
    val p = ui.path ?: return Spacer(Modifier.height(30.dp))
    val (label, color) = pathLabel(p)
    Column(Modifier.height(30.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(6.dp).clip(CircleShape).background(color))
            Spacer(Modifier.width(4.dp))
            Text(label, color = Color.White, fontSize = 10.sp, lineHeight = 12.sp, maxLines = 1)
        }
        if (p.kind != PathKind.CONNECTING && p.kind != PathKind.CLOSED) {
            Text(stringResource(R.string.path_rtt, p.rttMs.toInt()), color = DockColors.Dim, fontSize = 10.sp, lineHeight = 12.sp, maxLines = 1)
        }
    }
}

@Composable
private fun RailItem(
    icon: ImageVector,
    label: String,
    height: Dp,
    selected: Boolean = false,
    active: Boolean = false,
    danger: Boolean = false,
    badge: Color? = null,
    onClick: () -> Unit,
) {
    val tint = when {
        danger -> DockColors.Danger
        active || selected -> DockColors.Accent
        else -> Color.White
    }
    Column(
        Modifier
            .padding(horizontal = 4.dp)
            .fillMaxWidth()
            .height(height)
            .clip(RoundedCornerShape(10.dp))
            .background(if (selected) Color.White.copy(alpha = 0.1f) else Color.Transparent)
            .clickable(onClick = onClick),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Box {
            Icon(icon, null, tint = tint, modifier = Modifier.size(if (height >= 46.dp) 22.dp else 20.dp))
            if (badge != null) Box(Modifier.align(Alignment.TopEnd).offset(3.dp, (-2).dp).size(7.dp).clip(CircleShape).background(badge))
        }
        // 极矮的屏幕（低于 40dp 一格）只留图标
        if (height >= 40.dp) {
            Spacer(Modifier.height(if (height >= 46.dp) 3.dp else 1.dp))
            Text(
                label,
                color = tint.copy(alpha = 0.92f),
                fontSize = 10.sp,
                lineHeight = 12.sp,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                textAlign = TextAlign.Center,
            )
        }
    }
}

// ---------------- 二级面板 ----------------

@Composable
private fun DockPanel(
    panel: RemotePanel,
    ui: RemoteUi,
    elapsed: String?,
    view: DockView,
    actions: RemoteActions,
    callbacks: DockCallbacks,
    onClose: () -> Unit,
    modifier: Modifier,
) {
    val shape = RoundedCornerShape(16.dp)
    Column(
        modifier
            .clip(shape)
            .background(DockColors.Bg)
            .border(1.dp, DockColors.Line, shape)
            .testTag("remote-dock-panel")
            // 吃掉面板内空白处的点按，不穿透到下面的收起层
            .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {},
    ) {
        Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp, top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(
                stringResource(
                    when (panel) {
                        RemotePanel.DISPLAY -> R.string.rd_display
                        RemotePanel.SHORTCUTS -> R.string.rd_shortcuts
                        RemotePanel.TOOLS -> R.string.rd_tools
                    },
                ),
                color = Color.White,
                style = MaterialTheme.typography.titleSmall,
                modifier = Modifier.weight(1f),
            )
            if (panel == RemotePanel.TOOLS) DarkInfo(stringResource(R.string.rd_mode_info))
            Box(
                Modifier.size(40.dp).clip(CircleShape).clickable(onClick = onClose),
                contentAlignment = Alignment.Center,
            ) { Icon(Icons.Rounded.Close, stringResource(R.string.close), tint = DockColors.Dim, modifier = Modifier.size(20.dp)) }
        }
        // 面板内容按网格排，常规屏幕上一屏放得下；极矮的屏幕才需要滚动（面板不承担切换显示器手势）
        Column(Modifier.verticalScroll(rememberScrollState()).padding(start = 14.dp, end = 14.dp, bottom = 14.dp)) {
            when (panel) {
                RemotePanel.DISPLAY -> DisplayPanel(ui, view, actions, callbacks)
                RemotePanel.SHORTCUTS -> ShortcutsPanel(ui.platform, actions)
                RemotePanel.TOOLS -> ToolsPanel(ui, elapsed, actions)
            }
        }
    }
}

@Composable
private fun DarkInfo(text: String) {
    // 深色面板上的说明按钮：图标改成浅色
    MaterialTheme(colorScheme = MaterialTheme.colorScheme.copy(onSurfaceVariant = DockColors.Dim)) { InfoButton(text) }
}

@Composable
private fun SectionTitle(text: String) {
    Text(text, color = DockColors.Dim, style = MaterialTheme.typography.labelMedium, modifier = Modifier.padding(top = 10.dp, bottom = 6.dp))
}

@Composable
private fun DisplayPanel(ui: RemoteUi, view: DockView, actions: RemoteActions, callbacks: DockCallbacks) {
    if (ui.displays.isNotEmpty()) {
        Row(Modifier.horizontalScroll(rememberScrollState()).padding(top = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            ui.displays.forEachIndexed { i, d ->
                val on = d.id == ui.displayId
                val cardShape = RoundedCornerShape(10.dp)
                Row(
                    Modifier
                        .widthIn(min = 128.dp, max = 180.dp)
                        .clip(cardShape)
                        .background(if (on) DockColors.Accent.copy(alpha = 0.14f) else DockColors.Chip)
                        .border(1.dp, if (on) DockColors.Accent else Color.Transparent, cardShape)
                        .clickable { callbacks.onPickDisplay(d.id) }
                        .padding(horizontal = 10.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Box(
                        Modifier.size(24.dp).clip(RoundedCornerShape(6.dp)).background(if (on) DockColors.Accent else Color.White.copy(alpha = 0.14f)),
                        contentAlignment = Alignment.Center,
                    ) { Text("${i + 1}", color = if (on) DockColors.OnAccent else Color.White, fontSize = 12.sp, fontWeight = FontWeight.SemiBold) }
                    Spacer(Modifier.width(8.dp))
                    Column {
                        Text(d.name, color = Color.White, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(
                            "${d.width}×${d.height}" +
                                (if (d.primary) " · " + stringResource(R.string.rd_primary) else "") +
                                (if (d.isVirtual) " · " + stringResource(R.string.rd_virtual) else ""),
                            color = DockColors.Dim,
                            fontSize = 11.sp,
                            maxLines = 1,
                        )
                    }
                }
            }
        }
        if (ui.displays.size > 1) {
            Text(stringResource(R.string.rd_swipe_hint), color = DockColors.Dim, fontSize = 11.sp, modifier = Modifier.padding(top = 6.dp))
        }
    }
    SectionTitle(stringResource(R.string.rd_quality))
    Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        VideoQuality.entries.forEach { q ->
            Chip(stringResource(q.label), selected = q == ui.quality, modifier = Modifier.weight(1f)) { actions.onQuality(q) }
        }
    }
    SectionTitle(stringResource(R.string.rd_view))
    Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        Chip(stringResource(R.string.rd_fit), Icons.Rounded.FitScreen, selected = !view.zoomed, modifier = Modifier.weight(1f)) { callbacks.onFit() }
        Chip(stringResource(R.string.rd_actual_size), Icons.Rounded.ZoomIn, selected = view.atActualSize, enabled = view.canActualSize, modifier = Modifier.weight(1f)) { callbacks.onActualSize() }
        Chip(
            stringResource(if (view.portrait) R.string.rd_to_landscape else R.string.rd_to_portrait),
            Icons.Rounded.ScreenRotation,
            modifier = Modifier.weight(1f),
        ) { callbacks.onRotate() }
    }
    ui.path?.let { p ->
        val (label, color) = pathLabel(p)
        Row(Modifier.padding(top = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(7.dp).clip(CircleShape).background(color))
            Spacer(Modifier.width(6.dp))
            val rtt = if (p.kind == PathKind.CONNECTING || p.kind == PathKind.CLOSED) "" else " · " + stringResource(R.string.path_rtt, p.rttMs.toInt())
            val res = ui.display?.let { " · ${it.width}×${it.height}" } ?: ""
            Text(label + rtt + res, color = DockColors.Dim, style = MaterialTheme.typography.labelMedium, modifier = Modifier.weight(1f, fill = false))
            DarkInfo(stringResource(R.string.path_info))
        }
    }
}

@Composable
private fun Chip(
    label: String,
    icon: ImageVector? = null,
    selected: Boolean = false,
    enabled: Boolean = true,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    val fg = when {
        !enabled -> Color.White.copy(alpha = 0.35f)
        selected -> DockColors.OnAccent
        else -> Color.White
    }
    Row(
        modifier
            .height(36.dp)
            .clip(RoundedCornerShape(8.dp))
            .background(if (selected && enabled) DockColors.Accent else DockColors.Chip)
            .clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = 4.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            Icon(icon, null, tint = fg, modifier = Modifier.size(16.dp))
            Spacer(Modifier.width(4.dp))
        }
        Text(label, color = fg, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
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
            Shortcut(R.string.combo_lock) { listOf(InputEvent.SpecialCombo(ComboKey.LOCK_SCREEN)) },
        ) + common
    } else {
        listOf(
            Shortcut(R.string.combo_ctrl_alt_del) { listOf(InputEvent.SpecialCombo(ComboKey.CTRL_ALT_DEL)) },
            Shortcut(R.string.combo_win) { listOf(InputEvent.SpecialCombo(ComboKey.WIN_KEY)) },
            Shortcut(R.string.combo_alt_tab) { listOf(InputEvent.SpecialCombo(ComboKey.ALT_TAB)) },
            Shortcut(R.string.combo_alt_f4) { Hid.combo(Hid.ALT, Hid.F4) },
            Shortcut(R.string.combo_task_manager) { listOf(InputEvent.SpecialCombo(ComboKey.TASK_MANAGER)) },
            Shortcut(R.string.combo_show_desktop) { listOf(InputEvent.SpecialCombo(ComboKey.SHOW_DESKTOP)) },
            Shortcut(R.string.combo_lock) { listOf(InputEvent.SpecialCombo(ComboKey.LOCK_SCREEN)) },
        ) + common
    }
}

/** 快捷键：系统组合键一组、编辑常用键一组，三列网格。 */
@Composable
private fun ShortcutsPanel(platform: Platform, actions: RemoteActions) {
    val all = shortcutsFor(platform)
    val system = all.dropLast(4)
    val edit = all.takeLast(4)
    Grid(system, 3, Modifier.padding(top = 4.dp)) { s, m -> Chip(stringResource(s.label), modifier = m) { actions.onInput(s.events()) } }
    Box(Modifier.padding(vertical = 10.dp).fillMaxWidth().height(1.dp).background(DockColors.Line))
    Grid(edit, 4, Modifier) { s, m -> Chip(stringResource(s.label), modifier = m) { actions.onInput(s.events()) } }
}

/** 固定列数的网格（不足一行的格子留空，保证每格等宽）。 */
@Composable
private fun <T> Grid(items: List<T>, columns: Int, modifier: Modifier, cell: @Composable (T, Modifier) -> Unit) {
    Column(modifier, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        items.chunked(columns).forEach { row ->
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                row.forEach { cell(it, Modifier.weight(1f)) }
                repeat(columns - row.size) { Spacer(Modifier.weight(1f)) }
            }
        }
    }
}

private class Tool(
    val icon: ImageVector,
    val label: String,
    val active: Boolean = false,
    val danger: Boolean = false,
    val enabled: Boolean = true,
    val description: String? = null,
    val onClick: () -> Unit,
)

/** 工具：声音、隐私屏、录像、剪贴板、文件，三列方块（开启时高亮）。 */
@Composable
private fun ToolsPanel(ui: RemoteUi, elapsed: String?, actions: RemoteActions) {
    val tools = buildList {
        if (ui.audioSupported) {
            add(
                Tool(
                    if (ui.audioOn) Icons.AutoMirrored.Rounded.VolumeUp else Icons.AutoMirrored.Rounded.VolumeOff,
                    stringResource(R.string.rd_sound),
                    active = ui.audioOn,
                    enabled = !ui.audioBusy,
                ) { actions.onSound(!ui.audioOn) },
            )
        }
        if (ui.privacySupported) {
            add(
                Tool(
                    if (ui.privacyOn) Icons.Rounded.VisibilityOff else Icons.Outlined.VisibilityOff,
                    stringResource(if (ui.privacyOn) R.string.rd_privacy_on else R.string.rd_privacy),
                    active = ui.privacyOn,
                    enabled = ui.privacyAllowed && !ui.privacyBusy,
                ) { actions.onPrivacy(!ui.privacyOn) },
            )
        }
        add(
            Tool(
                if (ui.recording) Icons.Rounded.Stop else Icons.Rounded.FiberManualRecord,
                elapsed ?: stringResource(R.string.rd_record),
                danger = ui.recording,
                enabled = ui.status == RemoteStatus.STREAMING && !ui.recordingBusy,
                description = stringResource(if (ui.recording) R.string.rd_record_stop else R.string.rd_record),
            ) { actions.onRecord() },
        )
        add(Tool(Icons.Rounded.ContentPaste, stringResource(R.string.rd_send_clipboard)) { actions.onClipboardSend() })
        add(Tool(Icons.Rounded.ContentCopy, stringResource(R.string.rd_get_clipboard)) { actions.onClipboardGet() })
        add(Tool(Icons.Rounded.FolderOpen, stringResource(R.string.rd_files)) { actions.onFiles() })
    }
    Grid(tools, 3, Modifier.padding(top = 4.dp)) { t, m -> ToolTile(t, m) }
}

@Composable
private fun ToolTile(t: Tool, modifier: Modifier) {
    val fg = when {
        !t.enabled -> Color.White.copy(alpha = 0.35f)
        t.danger -> Color.White
        t.active -> DockColors.OnAccent
        else -> Color.White
    }
    val bg = when {
        t.danger -> DockColors.Record.copy(alpha = 0.85f)
        t.active && t.enabled -> DockColors.Accent
        else -> DockColors.Chip
    }
    Column(
        modifier
            .height(68.dp)
            .clip(RoundedCornerShape(10.dp))
            .background(bg)
            .clickable(enabled = t.enabled, onClick = t.onClick)
            .semantics { t.description?.let { contentDescription = it } }
            .padding(horizontal = 4.dp, vertical = 6.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(t.icon, null, tint = fg, modifier = Modifier.size(22.dp))
        Spacer(Modifier.height(4.dp))
        Text(
            t.label,
            color = fg,
            fontSize = 11.sp,
            lineHeight = 13.sp,
            maxLines = 2,
            textAlign = TextAlign.Center,
            overflow = TextOverflow.Ellipsis,
            fontFamily = if (t.danger) FontFamily.Monospace else null,
        )
    }
}

/** 屏幕上方居中的短暂提示（切换显示器、缩放倍数、操作方式）。 */
@Composable
internal fun RemoteNotice(title: String, subtitle: String?, modifier: Modifier) {
    Column(
        modifier
            .clip(RoundedCornerShape(14.dp))
            .background(Color(0xE61B1E2B))
            .border(1.dp, DockColors.Line, RoundedCornerShape(14.dp))
            .testTag("remote-notice")
            .padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(title, color = Color.White, style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
        if (!subtitle.isNullOrEmpty()) {
            Text(subtitle, color = DockColors.Dim, style = MaterialTheme.typography.labelMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}
