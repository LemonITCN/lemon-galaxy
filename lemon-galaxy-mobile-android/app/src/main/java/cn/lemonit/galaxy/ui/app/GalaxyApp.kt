package cn.lemonit.galaxy.ui.app

import android.content.Intent
import android.content.res.Configuration
import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.AutoAwesome
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material.icons.outlined.FolderOpen
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.rounded.AutoAwesome
import androidx.compose.material.icons.rounded.Dns
import androidx.compose.material.icons.rounded.FolderOpen
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.AppContainer
import cn.lemonit.galaxy.app.Navigator
import cn.lemonit.galaxy.app.RdpLinks
import cn.lemonit.galaxy.app.Route
import cn.lemonit.galaxy.app.SecretPromptBroker
import cn.lemonit.galaxy.app.Tab
import cn.lemonit.galaxy.core.CoreProvider
import cn.lemonit.galaxy.ui.hosts.HostKeyDialog
import cn.lemonit.galaxy.ui.hosts.RdpForwardBanner
import cn.lemonit.galaxy.ui.hosts.RdpMissingClientDialog
import cn.lemonit.galaxy.ui.hosts.SecretAskDialog
import cn.lemonit.galaxy.ui.icons.PlatformIcons
import cn.lemonit.galaxy.ui.login.LoginRoute
import cn.lemonit.galaxy.ui.theme.GalaxyTheme

/** 应用根：主题、应用锁、登录 / 主界面切换、SSH 交互提示框、一次性提示。 */
@Composable
fun GalaxyApp(container: AppContainer, locked: Boolean, onUnlock: () -> Unit) {
    val prefs by container.settings.prefs.collectAsState()
    GalaxyTheme(prefs.themeMode, prefs.palette, prefs.dynamicColor) {
        val signedIn by container.account.signedIn.collectAsState()
        var restored by remember { mutableStateOf(false) }
        val snackbar = remember { SnackbarHostState() }
        LaunchedEffect(Unit) {
            // 内核读本地会话并解密，放到后台线程
            kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) { container.account.restore() }
            restored = true
        }
        LaunchedEffect(signedIn) { if (signedIn) container.onSignedIn() }
        val locale = LocalConfiguration.current.locales[0]
        LaunchedEffect(locale) { container.setCoreLocale(locale.language == "zh") }
        LaunchedEffect(Unit) { container.messages.collect { snackbar.showSnackbar(it) } }
        Surface(color = MaterialTheme.colorScheme.background, modifier = Modifier.fillMaxSize()) {
            Box(Modifier.fillMaxSize()) {
                when {
                    !restored -> Unit
                    !signedIn -> LoginRoute(container.account, CoreProvider.isMock, container::toast)
                    else -> MainRoute(container)
                }
                SnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter).padding(bottom = 88.dp))
                if (locked) LockScreen(onUnlock)
            }
            SecretPromptHost(container.prompts)
        }
    }
}

@Composable
fun LockScreen(onUnlock: () -> Unit) {
    Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Icon(PlatformIcons.Galaxy, null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(56.dp))
            Spacer(Modifier.height(16.dp))
            Text(stringResource(R.string.lock_title), style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.height(24.dp))
            Button(onClick = onUnlock) {
                Icon(Icons.Rounded.Lock, null, Modifier.size(18.dp))
                Spacer(Modifier.width(8.dp))
                Text(stringResource(R.string.lock_unlock))
            }
        }
    }
    LaunchedEffect(Unit) { onUnlock() }
}

/** 内核 SecretPrompt 弹框：密码 / 私钥口令 / 键盘交互问题，以及主机密钥确认。 */
@Composable
fun SecretPromptHost(broker: SecretPromptBroker) {
    val req by broker.request.collectAsState()
    when (val r = req) {
        null -> Unit
        is SecretPromptBroker.Request.Ask -> SecretAskDialog(
            key = r.id, title = r.title, prompt = r.prompt, echo = r.echo,
            onSubmit = { broker.respond(it) }, onCancel = { broker.respond(null) },
        )
        is SecretPromptBroker.Request.HostKey -> HostKeyDialog(
            host = r.host, port = r.port, keyType = r.keyType, fingerprint = r.fingerprint, changed = r.changed,
            onTrust = { broker.respondHostKey(true) }, onReject = { broker.respondHostKey(false) },
        )
    }
}

private data class TabSpec(val tab: Tab, val label: Int, val icon: ImageVector, val selectedIcon: ImageVector)

private val TABS = listOf(
    TabSpec(Tab.HOSTS, R.string.nav_hosts, Icons.Outlined.Dns, Icons.Rounded.Dns),
    TabSpec(Tab.AI, R.string.nav_ai, Icons.Outlined.AutoAwesome, Icons.Rounded.AutoAwesome),
    TabSpec(Tab.FILES, R.string.nav_files, Icons.Outlined.FolderOpen, Icons.Rounded.FolderOpen),
    TabSpec(Tab.SETTINGS, R.string.nav_settings, Icons.Outlined.Settings, Icons.Rounded.Settings),
)

/** 宽屏判断：平板横屏或可折叠展开时用双栏。 */
@Composable
fun isWideLayout(): Boolean {
    val c = LocalConfiguration.current
    return c.screenWidthDp >= 840 || (c.screenWidthDp >= 600 && c.orientation == Configuration.ORIENTATION_LANDSCAPE)
}

@Composable
fun MainRoute(container: AppContainer, navigator: Navigator = remember { Navigator() }) {
    val nav = navigator
    BackHandler(enabled = nav.stack.isNotEmpty()) { nav.pop() }
    val context = LocalContext.current
    val openUrl: (String) -> Unit = { url -> runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) } }
    val wide = isWideLayout()
    val top = nav.top
    val rdpForwards by container.rdp.forwards.collectAsState()
    val rdpMissing by container.rdp.missingClient.collectAsState()
    if (rdpMissing) RdpMissingClientDialog(onInstall = { openUrl(RdpLinks.PLAY_STORE) }, onDismiss = container.rdp::dismissMissingClient)
    val banner = @Composable { RdpForwardBanner(rdpForwards, onStop = container.rdp::close, onStopAll = container.rdp::closeAll) }
    when {
        top != null && top.fullscreen -> RouteContent(top, container, nav, showBack = true, openUrl = openUrl)
        wide -> Row(Modifier.fillMaxSize()) {
            NavigationRail(containerColor = MaterialTheme.colorScheme.surfaceContainer, header = {
                Icon(PlatformIcons.Galaxy, null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.padding(vertical = 12.dp).size(28.dp))
            }) {
                TABS.forEach { t ->
                    NavigationRailItem(
                        selected = nav.tab == t.tab,
                        onClick = { nav.selectTab(t.tab) },
                        icon = { Icon(if (nav.tab == t.tab) t.selectedIcon else t.icon, null) },
                        label = { Text(stringResource(t.label)) },
                    )
                }
            }
            if (nav.tab == Tab.SETTINGS && top == null) {
                Column(Modifier.weight(1f).fillMaxHeight()) {
                    Box(Modifier.weight(1f)) { TabContent(Tab.SETTINGS, container, nav, openUrl) }
                    banner()
                }
            } else {
                Column(Modifier.width(380.dp).fillMaxHeight()) {
                    Box(Modifier.weight(1f)) { TabContent(nav.tab, container, nav, openUrl) }
                    banner()
                }
                VerticalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                Box(Modifier.weight(1f).fillMaxHeight()) {
                    if (top != null) {
                        RouteContent(top, container, nav, showBack = nav.stack.size > 1 || top is Route.EditEntry || top is Route.FileEditor || top is Route.Licenses, openUrl = openUrl)
                    } else if (nav.tab == Tab.AI) {
                        // AI 页右侧没有打开的对话时就是一个新对话
                        ChatRoute(null, null, container, nav, showBack = false)
                    } else {
                        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                            Text(stringResource(R.string.select_item), color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    }
                }
            }
        }
        top != null -> RouteContent(top, container, nav, showBack = true, openUrl = openUrl)
        else -> Scaffold(
            bottomBar = {
                Column {
                    banner()
                    NavigationBar(containerColor = MaterialTheme.colorScheme.surfaceContainer) {
                        TABS.forEach { t ->
                            NavigationBarItem(
                                selected = nav.tab == t.tab,
                                onClick = { nav.selectTab(t.tab) },
                                icon = { Icon(if (nav.tab == t.tab) t.selectedIcon else t.icon, null) },
                                label = { Text(stringResource(t.label)) },
                            )
                        }
                    }
                }
            },
        ) { pad ->
            Box(Modifier.padding(bottom = pad.calculateBottomPadding()).fillMaxSize()) { TabContent(nav.tab, container, nav, openUrl) }
        }
    }
}
