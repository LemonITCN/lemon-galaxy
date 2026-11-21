package cn.lemonit.galaxy.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.rounded.Android
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.DesktopWindows
import androidx.compose.material.icons.rounded.PhoneIphone
import androidx.compose.material.icons.rounded.Terminal
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupProperties
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.core.bridge.DeviceInfo
import cn.lemonit.galaxy.core.bridge.DeviceKind
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.ui.icons.PlatformIcons
import cn.lemonit.galaxy.ui.theme.Galaxy

/** 小 i 图标：点开显示说明弹层（界面上不铺说明文字）。 */
@Composable
fun InfoButton(text: String, modifier: Modifier = Modifier, size: Dp = 18.dp) {
    var open by remember { mutableStateOf(false) }
    Box(modifier) {
        Icon(
            Icons.Outlined.Info,
            contentDescription = stringResource(R.string.info),
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier
                .clip(CircleShape)
                .clickable { open = true }
                .padding(6.dp)
                .size(size),
        )
        if (open) {
            Popup(
                alignment = Alignment.TopStart,
                offset = IntOffset(0, 72),
                onDismissRequest = { open = false },
                properties = PopupProperties(focusable = true),
            ) {
                Surface(
                    shape = RoundedCornerShape(12.dp),
                    color = MaterialTheme.colorScheme.inverseSurface,
                    contentColor = MaterialTheme.colorScheme.inverseOnSurface,
                    shadowElevation = 6.dp,
                    modifier = Modifier.widthIn(max = 300.dp).padding(horizontal = 8.dp),
                ) {
                    Text(text, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(14.dp))
                }
            }
        }
    }
}

@Composable
fun StatusDot(color: Color, modifier: Modifier = Modifier, size: Dp = 8.dp) {
    Box(modifier.size(size).clip(CircleShape).background(color))
}

@Composable
fun deviceStatusColor(d: DeviceInfo): Color = when {
    d.online -> Galaxy.colors.online
    d.lanSeen -> Galaxy.colors.lan
    else -> Galaxy.colors.offline
}

fun platformIcon(platform: Platform): ImageVector = when (platform) {
    Platform.MACOS, Platform.IOS -> PlatformIcons.Apple
    Platform.WINDOWS -> PlatformIcons.Windows
    Platform.LINUX -> PlatformIcons.Linux
    Platform.ANDROID -> Icons.Rounded.Android
    Platform.HARMONYOS -> Icons.Rounded.PhoneIphone
}

/** 列表左侧的图标块：圆角方块底 + 平台/类型图标 + 右下角状态点。 */
@Composable
fun EntryAvatar(
    icon: ImageVector,
    tint: Color = MaterialTheme.colorScheme.onSurface,
    container: Color = MaterialTheme.colorScheme.surfaceContainerHighest,
    status: Color? = null,
    size: Dp = 40.dp,
) {
    Box(Modifier.size(size)) {
        Box(
            Modifier.fillMaxSize().clip(RoundedCornerShape(10.dp)).background(container),
            contentAlignment = Alignment.Center,
        ) {
            Icon(icon, null, tint = tint, modifier = Modifier.size(size * 0.5f))
        }
        if (status != null) {
            Box(
                Modifier
                    .align(Alignment.BottomEnd)
                    .padding(0.dp)
                    .size(12.dp)
                    .clip(CircleShape)
                    .background(MaterialTheme.colorScheme.surface)
                    .padding(2.dp)
                    .clip(CircleShape)
                    .background(status),
            )
        }
    }
}

fun deviceKindIcon(kind: DeviceKind, platform: Platform): ImageVector = when (kind) {
    DeviceKind.MOBILE -> Icons.Rounded.PhoneIphone
    else -> platformIcon(platform)
}

val SshIcon: ImageVector get() = Icons.Rounded.Terminal
val RdpIcon: ImageVector get() = Icons.Rounded.DesktopWindows
val ComputerIcon: ImageVector get() = Icons.Rounded.Computer

@Composable
fun SectionHeader(text: String, modifier: Modifier = Modifier, trailing: @Composable RowScope.() -> Unit = {}) {
    Row(
        modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, top = 16.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text.uppercase(),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            fontWeight = FontWeight.SemiBold,
            modifier = Modifier.weight(1f),
        )
        trailing()
    }
}

@Composable
fun Tag(text: String, modifier: Modifier = Modifier, color: Color = MaterialTheme.colorScheme.onSurfaceVariant) {
    Text(
        text,
        style = MaterialTheme.typography.labelSmall,
        color = color,
        maxLines = 1,
        modifier = modifier
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(4.dp))
            .padding(horizontal = 5.dp, vertical = 1.dp),
    )
}

/** 风险级别、状态等彩色小标签。 */
@Composable
fun Pill(text: String, color: Color, modifier: Modifier = Modifier, filled: Boolean = false) {
    Text(
        text,
        style = MaterialTheme.typography.labelSmall,
        fontWeight = FontWeight.SemiBold,
        color = if (filled) MaterialTheme.colorScheme.surface else color,
        maxLines = 1,
        modifier = modifier
            .clip(RoundedCornerShape(6.dp))
            .background(if (filled) color else color.copy(alpha = 0.14f))
            .padding(horizontal = 7.dp, vertical = 2.dp),
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun GalaxyTopBar(
    title: String,
    onBack: (() -> Unit)? = null,
    subtitle: (@Composable () -> Unit)? = null,
    actions: @Composable RowScope.() -> Unit = {},
    centered: Boolean = false,
) {
    val colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.surface)
    val titleContent: @Composable () -> Unit = {
        Column {
            Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.titleMedium)
            subtitle?.invoke()
        }
    }
    val nav: @Composable () -> Unit = {
        if (onBack != null) {
            IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, stringResource(R.string.back)) }
        }
    }
    if (centered) {
        CenterAlignedTopAppBar(title = titleContent, navigationIcon = nav, actions = actions, colors = colors)
    } else {
        TopAppBar(title = titleContent, navigationIcon = nav, actions = actions, colors = colors)
    }
}

@Composable
fun ConfirmDialog(
    title: String,
    message: String?,
    confirmText: String,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
    destructive: Boolean = false,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = message?.let { { Text(it) } },
        confirmButton = {
            TextButton(onClick = {
                onConfirm()
                onDismiss()
            }) {
                Text(confirmText, color = if (destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary)
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) } },
    )
}

@Composable
fun TextInputDialog(
    title: String,
    label: String,
    initial: String = "",
    confirmText: String = stringResource(R.string.ok),
    validate: (String) -> String? = { if (it.isBlank()) "" else null },
    onConfirm: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    var value by remember { mutableStateOf(initial) }
    val error = validate(value)
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            OutlinedTextField(
                value = value,
                onValueChange = { value = it },
                label = { Text(label) },
                singleLine = true,
                isError = error != null && error.isNotEmpty(),
                supportingText = error?.takeIf { it.isNotEmpty() }?.let { { Text(it) } },
                modifier = Modifier.fillMaxWidth(),
            )
        },
        confirmButton = {
            TextButton(enabled = error == null, onClick = {
                onConfirm(value.trim())
                onDismiss()
            }) { Text(confirmText) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) } },
    )
}

@Composable
fun EmptyState(icon: ImageVector, text: String, modifier: Modifier = Modifier, action: (@Composable () -> Unit)? = null) {
    Column(
        modifier.fillMaxWidth().padding(vertical = 48.dp, horizontal = 24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(icon, null, tint = MaterialTheme.colorScheme.outline, modifier = Modifier.size(40.dp))
        Spacer(Modifier.height(12.dp))
        Text(text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        if (action != null) {
            Spacer(Modifier.height(12.dp))
            action()
        }
    }
}

/** 相对时间（刚刚 / N 分钟前 / N 小时前 / N 天前）。 */
@Composable
fun relativeTime(at: Long, now: Long = Clock.now()): String {
    val d = (now - at).coerceAtLeast(0) / 60_000
    return when {
        d < 1 -> stringResource(R.string.time_just_now)
        d < 60 -> stringResource(R.string.time_minutes_ago, d.toInt())
        d < 24 * 60 -> stringResource(R.string.time_hours_ago, (d / 60).toInt())
        else -> stringResource(R.string.time_days_ago, (d / 60 / 24).toInt())
    }
}

fun formatBytes(n: Long): String {
    if (n < 1024) return "$n B"
    val units = listOf("KB", "MB", "GB", "TB")
    var v = n.toDouble() / 1024
    var i = 0
    while (v >= 1024 && i < units.lastIndex) {
        v /= 1024
        i++
    }
    return if (v >= 100) "%.0f %s".format(v, units[i]) else "%.1f %s".format(v, units[i])
}

fun formatCount(n: Long): String = when {
    n >= 1_000_000 -> "%.1fM".format(n / 1_000_000.0)
    n >= 1_000 -> "%.1fK".format(n / 1_000.0)
    else -> n.toString()
}

/** 当前时间来源（截图测试里固定为 MockData.NOW）。 */
object Clock {
    @Volatile var fixedNow: Long? = null
    fun now(): Long = fixedNow ?: System.currentTimeMillis()
}
