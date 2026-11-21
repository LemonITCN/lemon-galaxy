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
import androidx.compose.foundation.text.KeyboardOptions
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
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.AppContainer
import cn.lemonit.galaxy.app.Navigator
import cn.lemonit.galaxy.app.Route
import cn.lemonit.galaxy.app.SecretPromptBroker
import cn.lemonit.galaxy.app.Tab
import cn.lemonit.galaxy.core.CoreProvider
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
            container.account.restore()
            restored = true
        }
        LaunchedEffect(signedIn) { if (signedIn) container.onSignedIn() }
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

/** 内核 SecretPrompt（键盘交互认证、私钥口令、首次连接确认指纹）弹框。 */
@Composable
fun SecretPromptHost(broker: SecretPromptBroker) {
    val req by broker.request.collectAsState()
    val r = req ?: return
    var value by remember(r.id) { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = { broker.respond(null) },
        title = { Text(r.title ?: stringResource(if (r.echo) R.string.host_key_title else R.string.secret_prompt_title)) },
        text = {
            Column {
                Text(r.prompt, style = MaterialTheme.typography.bodyMedium, fontFamily = if (r.echo) FontFamily.Monospace else null)
                Spacer(Modifier.height(12.dp))
                OutlinedTextField(
                    value, { value = it },
                    singleLine = true,
                    visualTransformation = if (r.echo) androidx.compose.ui.text.input.VisualTransformation.None else PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = if (r.echo) KeyboardType.Ascii else KeyboardType.Password),
                )
            }
        },
        confirmButton = { TextButton(onClick = { broker.respond(value) }) { Text(stringResource(R.string.ok)) } },
        dismissButton = { TextButton(onClick = { broker.respond(null) }) { Text(stringResource(R.string.cancel)) } },
    )
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
                Box(Modifier.weight(1f).fillMaxHeight()) { TabContent(Tab.SETTINGS, container, nav, openUrl) }
            } else {
                Box(Modifier.width(380.dp).fillMaxHeight()) { TabContent(nav.tab, container, nav, openUrl) }
                VerticalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                Box(Modifier.weight(1f).fillMaxHeight()) {
                    if (top != null) {
                        RouteContent(top, container, nav, showBack = nav.stack.size > 1 || top is Route.EditEntry || top is Route.FileEditor || top is Route.Licenses, openUrl = openUrl)
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
            },
        ) { pad ->
            Box(Modifier.padding(bottom = pad.calculateBottomPadding()).fillMaxSize()) { TabContent(nav.tab, container, nav, openUrl) }
        }
    }
}
