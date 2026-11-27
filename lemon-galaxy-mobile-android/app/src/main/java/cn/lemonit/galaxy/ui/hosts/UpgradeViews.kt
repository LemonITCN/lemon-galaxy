package cn.lemonit.galaxy.ui.hosts

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.SystemUpdateAlt
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.DeviceUpgrade
import cn.lemonit.galaxy.app.RemoteUpgrade
import cn.lemonit.galaxy.core.bridge.DeviceInfo
import cn.lemonit.galaxy.ui.components.Pill
import cn.lemonit.galaxy.ui.theme.Galaxy

/** 设备列表里标题旁的小标记：可升级 / 升级中；都不是时不画。 */
@Composable
fun UpgradeBadge(device: DeviceInfo, state: DeviceUpgrade?, modifier: Modifier = Modifier) {
    val text = when {
        state?.running == true -> stringResource(R.string.upgrade_badge_running)
        RemoteUpgrade.available(device) -> stringResource(R.string.upgrade_badge)
        else -> return
    }
    Pill(text, MaterialTheme.colorScheme.primary, modifier)
}

/** 详情里要不要显示升级这一块：能被控，且有新版本或有这台设备的升级记录。 */
fun showUpgradeCard(device: DeviceInfo, state: DeviceUpgrade?): Boolean =
    RemoteUpgrade.controllable(device) && (device.updateVersion.isNotEmpty() || state != null)

/**
 * 设备详情里的升级块：左侧状态图标，中间两行状态，右侧「升级被控端」。
 * 点了直接开始（不再确认）；进度就地更新：正在请求 → 正在升级到 x → 对方已断开、正在替换重启 → 已升级到 x / 失败原因。
 */
@Composable
fun UpgradeCard(device: DeviceInfo, upgrade: DeviceUpgrade?, onUpgrade: () -> Unit, modifier: Modifier = Modifier) {
    val state = upgrade?.shownFor(device)
    val (title, detail) = upgradeTexts(device, state)
    Row(
        modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp)
            .clip(RoundedCornerShape(14.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .padding(start = 14.dp, end = 10.dp, top = 10.dp, bottom = 10.dp)
            .testTag("upgrade_card"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(24.dp), contentAlignment = Alignment.Center) {
            when {
                state?.running == true -> CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                state is DeviceUpgrade.Done -> Icon(Icons.Rounded.CheckCircle, null, tint = Galaxy.colors.success, modifier = Modifier.size(22.dp))
                state is DeviceUpgrade.Failed -> Icon(Icons.Rounded.ErrorOutline, null, tint = MaterialTheme.colorScheme.error, modifier = Modifier.size(22.dp))
                else -> Icon(Icons.Rounded.SystemUpdateAlt, null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(22.dp))
            }
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f).padding(vertical = 4.dp)) {
            Text(
                title,
                style = MaterialTheme.typography.bodyMedium,
                color = if (state is DeviceUpgrade.Failed) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (detail.isNotEmpty()) {
                Text(
                    detail,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        if (state?.running != true && (device.updateVersion.isNotEmpty() || state is DeviceUpgrade.Failed)) {
            Spacer(Modifier.width(8.dp))
            FilledTonalButton(
                onClick = onUpgrade,
                enabled = RemoteUpgrade.canStart(device, state),
                contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp),
                modifier = Modifier.testTag("upgrade_button"),
            ) {
                Text(stringResource(if (state is DeviceUpgrade.Failed) R.string.retry else R.string.upgrade_action), maxLines = 1)
            }
        }
    }
}

/** 「正在升级到 x」；不知道目标版本时只说「升级中」。 */
@Composable
private fun upgradingTo(version: String): String =
    if (version.isEmpty()) stringResource(R.string.upgrade_badge_running) else stringResource(R.string.upgrade_installing, version)

/** 升级块两行文字（标题、补充）。 */
@Composable
private fun upgradeTexts(device: DeviceInfo, state: DeviceUpgrade?): Pair<String, String> = when (state) {
    null -> stringResource(R.string.upgrade_new_version, device.updateVersion) to stringResource(R.string.update_idle, device.appVersion)
    is DeviceUpgrade.Requesting -> stringResource(R.string.upgrade_requesting) to
        if (state.version.isNotEmpty()) stringResource(R.string.update_versions, device.appVersion, state.version) else stringResource(R.string.update_idle, device.appVersion)
    is DeviceUpgrade.Installing -> upgradingTo(state.version) to stringResource(R.string.upgrade_installing_detail)
    is DeviceUpgrade.Restarting -> upgradingTo(state.version) to stringResource(R.string.upgrade_restarting_detail)
    is DeviceUpgrade.Done -> when {
        state.alreadyLatest -> stringResource(R.string.upgrade_latest) to stringResource(R.string.update_idle, state.version.ifEmpty { device.appVersion })
        state.version.isEmpty() -> stringResource(R.string.upgrade_done_plain) to stringResource(R.string.upgrade_done_detail)
        else -> stringResource(R.string.upgrade_done, state.version) to stringResource(R.string.upgrade_done_detail)
    }
    is DeviceUpgrade.Failed -> stringResource(R.string.upgrade_failed) to state.reason
}
