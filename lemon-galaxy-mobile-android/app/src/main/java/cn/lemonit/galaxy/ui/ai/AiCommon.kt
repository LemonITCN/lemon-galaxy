package cn.lemonit.galaxy.ui.ai

import android.graphics.BitmapFactory
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.HelpOutline
import androidx.compose.material.icons.rounded.AccountTree
import androidx.compose.material.icons.rounded.AdsClick
import androidx.compose.material.icons.rounded.Checklist
import androidx.compose.material.icons.rounded.Code
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.Dns
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.FolderOpen
import androidx.compose.material.icons.rounded.History
import androidx.compose.material.icons.rounded.Keyboard
import androidx.compose.material.icons.rounded.Language
import androidx.compose.material.icons.rounded.Lan
import androidx.compose.material.icons.rounded.Link
import androidx.compose.material.icons.rounded.LockOpen
import androidx.compose.material.icons.rounded.Memory
import androidx.compose.material.icons.rounded.Screenshot
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Shield
import androidx.compose.material.icons.rounded.SwapHoriz
import androidx.compose.material.icons.rounded.Terminal
import androidx.compose.material.icons.rounded.Timer
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material.icons.rounded.WebAsset
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.vector.ImageVector
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.DateGroup
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.ContextRef
import cn.lemonit.galaxy.core.bridge.ContextRefKind
import cn.lemonit.galaxy.core.bridge.ImageData
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.RiskLevel
import cn.lemonit.galaxy.ui.components.platformIcon
import cn.lemonit.galaxy.ui.components.SshIcon
import cn.lemonit.galaxy.ui.theme.Galaxy

fun modeLabel(m: AiMode): Int = when (m) {
    AiMode.OFF -> R.string.mode_off
    AiMode.READ_ONLY -> R.string.mode_read_only
    AiMode.CONFIRM_ALL -> R.string.mode_confirm_all
    AiMode.CONFIRM_DANGEROUS -> R.string.mode_confirm_dangerous
    AiMode.FULL_TRUST -> R.string.mode_full_trust
}

fun modeIcon(m: AiMode): ImageVector = when (m) {
    AiMode.OFF, AiMode.READ_ONLY -> Icons.Rounded.Visibility
    AiMode.CONFIRM_ALL -> Icons.Rounded.Checklist
    AiMode.CONFIRM_DANGEROUS -> Icons.Rounded.Shield
    AiMode.FULL_TRUST -> Icons.Rounded.LockOpen
}

@Composable
fun riskColor(r: RiskLevel): Color = when (r) {
    RiskLevel.READ -> Galaxy.colors.riskRead
    RiskLevel.NORMAL -> Galaxy.colors.riskNormal
    RiskLevel.DANGEROUS -> Galaxy.colors.riskDangerous
    RiskLevel.UNKNOWN -> Galaxy.colors.riskUnknown
    RiskLevel.FORBIDDEN -> Galaxy.colors.riskForbidden
}

fun riskLabel(r: RiskLevel): Int = when (r) {
    RiskLevel.READ -> R.string.risk_read
    RiskLevel.NORMAL -> R.string.risk_normal
    RiskLevel.DANGEROUS -> R.string.risk_dangerous
    RiskLevel.UNKNOWN -> R.string.risk_unknown
    RiskLevel.FORBIDDEN -> R.string.risk_forbidden
}

/** 工具的显示名与图标。 */
fun toolInfo(tool: String): Pair<Int?, ImageVector> = when (tool) {
    "run_command" -> R.string.tool_run_command to Icons.Rounded.Terminal
    "terminal_open", "terminal_send", "terminal_read", "terminal_close" -> R.string.tool_terminal to Icons.Rounded.Terminal
    "read_file" -> R.string.tool_read_file to Icons.Rounded.Description
    "write_file" -> R.string.tool_write_file to Icons.Rounded.Description
    "edit_file" -> R.string.tool_edit_file to Icons.Rounded.Description
    "list_dir" -> R.string.tool_list_dir to Icons.Rounded.FolderOpen
    "search_files" -> R.string.tool_search_files to Icons.Rounded.Search
    "upload_file", "download_file" -> R.string.tool_transfer to Icons.Rounded.SwapHoriz
    "screenshot" -> R.string.tool_screenshot to Icons.Rounded.Screenshot
    "ui_tree" -> R.string.tool_ui_tree to Icons.Rounded.AccountTree
    "mouse" -> R.string.tool_mouse to Icons.Rounded.AdsClick
    "keyboard" -> R.string.tool_keyboard to Icons.Rounded.Keyboard
    "wait" -> R.string.tool_wait to Icons.Rounded.Timer
    "use_target" -> R.string.tool_use_target to Icons.Rounded.Link
    "list_devices" -> R.string.tool_list_devices to Icons.Rounded.Dns
    "list_snippets" -> R.string.tool_list_snippets to Icons.Rounded.Code
    "list_port_forwards" -> R.string.tool_list_port_forwards to Icons.Rounded.Lan
    "recent_activity" -> R.string.tool_recent_activity to Icons.Rounded.History
    "use_app" -> R.string.tool_use_app to Icons.Rounded.WebAsset
    "app_list" -> R.string.tool_app_list to Icons.Rounded.WebAsset
    "app_state" -> R.string.tool_app_state to Icons.Rounded.WebAsset
    "app_click" -> R.string.tool_app_click to Icons.Rounded.WebAsset
    "app_set_value" -> R.string.tool_app_set_value to Icons.Rounded.WebAsset
    "app_type" -> R.string.tool_app_type to Icons.Rounded.WebAsset
    "app_key" -> R.string.tool_app_key to Icons.Rounded.WebAsset
    "app_scroll" -> R.string.tool_app_scroll to Icons.Rounded.WebAsset
    "app_action" -> R.string.tool_app_action to Icons.Rounded.WebAsset
    "app_open" -> R.string.tool_app_open to Icons.Rounded.WebAsset
    "web_fetch" -> R.string.tool_web_fetch to Icons.Rounded.Language
    "system_info" -> R.string.tool_system_info to Icons.Rounded.Memory
    "update_plan" -> R.string.tool_update_plan to Icons.Rounded.Checklist
    else -> null to Icons.AutoMirrored.Rounded.HelpOutline
}

/** 引用标签的图标。[platform] 为设备的平台（查不到时用通用电脑图标）。 */
fun refIcon(ref: ContextRef, platform: Platform?): ImageVector = when (ref.kind) {
    ContextRefKind.DEVICE -> platform?.let(::platformIcon) ?: Icons.Rounded.Computer
    ContextRefKind.SSH_HOST -> SshIcon
    ContextRefKind.FILE -> Icons.Rounded.Description
    ContextRefKind.FOLDER -> Icons.Rounded.Folder
    ContextRefKind.SNIPPET -> Icons.Rounded.Code
    ContextRefKind.PORT_FORWARD -> Icons.Rounded.Lan
    ContextRefKind.ACTIVITY -> Icons.Rounded.History
}

fun dateGroupLabel(g: DateGroup): Int = when (g) {
    DateGroup.TODAY -> R.string.ai_group_today
    DateGroup.YESTERDAY -> R.string.ai_group_yesterday
    DateGroup.LAST_7_DAYS -> R.string.ai_group_week
    DateGroup.EARLIER -> R.string.ai_group_earlier
}

@Composable
fun rememberBitmap(img: ImageData?): ImageBitmap? = remember(img) {
    img?.let { runCatching { BitmapFactory.decodeByteArray(it.data, 0, it.data.size)?.asImageBitmap() }.getOrNull() }
}
