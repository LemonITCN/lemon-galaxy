package cn.lemonit.galaxy.ui.app

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.Settings
import androidx.biometric.BiometricManager
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.core.content.FileProvider
import cn.lemonit.galaxy.BuildConfig
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.AppContainer
import cn.lemonit.galaxy.app.EntryKind
import cn.lemonit.galaxy.app.HostItem
import cn.lemonit.galaxy.app.HostTree
import cn.lemonit.galaxy.app.Navigator
import cn.lemonit.galaxy.app.Route
import cn.lemonit.galaxy.app.Tab
import cn.lemonit.galaxy.app.TargetRef
import cn.lemonit.galaxy.app.UpdateState
import cn.lemonit.galaxy.app.userMessage
import cn.lemonit.galaxy.core.CoreProvider
import cn.lemonit.galaxy.core.bridge.VaultData
import cn.lemonit.galaxy.core.bridge.VaultEntry
import cn.lemonit.galaxy.core.platform.HoldMulticastLock
import cn.lemonit.galaxy.data.AppLanguage
import cn.lemonit.galaxy.data.Prefs
import cn.lemonit.galaxy.ui.ai.ConversationListScreen
import cn.lemonit.galaxy.ui.components.ConfirmDialog
import cn.lemonit.galaxy.ui.components.TextInputDialog
import cn.lemonit.galaxy.ui.files.FilesHomeScreen
import cn.lemonit.galaxy.ui.files.TransferActions
import cn.lemonit.galaxy.ui.hosts.AddKind
import cn.lemonit.galaxy.ui.hosts.AssistDialog
import cn.lemonit.galaxy.ui.hosts.FolderAction
import cn.lemonit.galaxy.ui.hosts.HostAction
import cn.lemonit.galaxy.ui.hosts.HostActionsContent
import cn.lemonit.galaxy.ui.hosts.HostsActions
import cn.lemonit.galaxy.ui.hosts.HostsFilter
import cn.lemonit.galaxy.ui.hosts.HostsScreen
import cn.lemonit.galaxy.ui.hosts.MoveDialog
import cn.lemonit.galaxy.ui.hosts.reachable
import cn.lemonit.galaxy.ui.hosts.sshViaText
import cn.lemonit.galaxy.ui.hosts.wakeSentMessage
import cn.lemonit.galaxy.ui.settings.AboutInfo
import cn.lemonit.galaxy.ui.settings.ChangePasswordDialog
import cn.lemonit.galaxy.ui.settings.SettingsActions
import cn.lemonit.galaxy.ui.settings.SettingsScreen
import cn.lemonit.galaxy.ui.settings.TwoFactorDialog
import cn.lemonit.galaxy.ui.settings.UnknownSourcesDialog
import cn.lemonit.galaxy.ui.settings.UpdateActions
import cn.lemonit.galaxy.ui.settings.UpdateDialog
import cn.lemonit.galaxy.ui.theme.TerminalThemes
import kotlinx.coroutines.launch
import java.io.File

@Composable
fun TabContent(tab: Tab, container: AppContainer, nav: Navigator, openUrl: (String) -> Unit) {
    when (tab) {
        Tab.HOSTS -> HostsTab(container, nav)
        Tab.AI -> AiTab(container, nav)
        Tab.FILES -> FilesTab(container, nav)
        Tab.SETTINGS -> SettingsTab(container, nav, openUrl)
    }
}

/** 打开某个条目的某项功能（设备与主机页、文件页、AI 页共用）。 */
fun openHostAction(container: AppContainer, nav: Navigator, item: HostItem, action: HostAction) {
    val target = when (item) {
        is HostItem.Device -> TargetRef.Device(item.device.id)
        is HostItem.Ssh -> TargetRef.Ssh(item.entry.id)
        is HostItem.Rdp -> null
    }
    when (action) {
        HostAction.REMOTE -> when (item) {
            is HostItem.Device -> nav.push(Route.Remote(item.device.id))
            // RDP 主机交给系统里的 Windows App（经设备时内核先开本机端口转发）
            is HostItem.Rdp -> container.rdp.open(item.entry.id)
            is HostItem.Ssh -> Unit
        }
        HostAction.TERMINAL -> target?.let {
            container.terminals.open(it)
            nav.push(Route.Terminal(it))
        }
        HostAction.FILES -> target?.let { nav.push(Route.Files(it)) }
        HostAction.AI -> nav.push(Route.NewConversation(target))
        HostAction.EDIT -> when (item) {
            is HostItem.Device -> nav.push(Route.EditEntry(EntryKind.DEVICE, item.device.id, item.folderId))
            is HostItem.Ssh -> nav.push(Route.EditEntry(EntryKind.SSH, item.entry.id, item.folderId))
            is HostItem.Rdp -> nav.push(Route.EditEntry(EntryKind.RDP, item.entry.id, item.folderId))
        }
        HostAction.FAVORITE -> container.hosts.toggleFavorite(item)
        HostAction.WAKE -> (item as? HostItem.Device)?.let { d -> container.appContext?.resources?.let { wakeDevice(container, d.device.id, it) } }
        HostAction.MOVE, HostAction.DELETE -> Unit
    }
}

/** 远程开机，结果用提示条告知（成功时说明经哪台设备代发）。[res] 用界面的（跟随应用内语言）。 */
fun wakeDevice(container: AppContainer, deviceId: String, res: android.content.res.Resources) {
    container.scope.launch {
        try {
            container.toast(wakeSentMessage(res, container.hosts.wake(deviceId)))
        } catch (e: Exception) {
            val msg = e.userMessage()
            if (msg.isNotEmpty()) container.toast(res.getString(R.string.wake_failed, msg))
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HostsTab(container: AppContainer, nav: Navigator) {
    HoldMulticastLock(discovery = true)
    val context = LocalContext.current
    val data by container.hosts.data.collectAsState()
    val refreshing by container.hosts.refreshing.collectAsState()
    var filter by remember { mutableStateOf(HostsFilter()) }
    var sheetItem by remember { mutableStateOf<HostItem?>(null) }
    var moveItem by remember { mutableStateOf<HostItem?>(null) }
    var moveFolder by remember { mutableStateOf<VaultEntry?>(null) }
    var deleteItem by remember { mutableStateOf<HostItem?>(null) }
    var deleteFolder by remember { mutableStateOf<VaultEntry?>(null) }
    var folderDialog by remember { mutableStateOf<Pair<VaultEntry?, String?>?>(null) } // (重命名的文件夹, 新建时的父文件夹)
    var assist by remember { mutableStateOf(false) }
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    val selectedKey = when (val t = nav.top) {
        is Route.Terminal -> when (val tg = t.target) {
            is TargetRef.Device -> "device:${tg.deviceId}"
            is TargetRef.Ssh -> "ssh:${tg.hostId}"
            null -> null
        }
        is Route.Files -> when (val tg = t.target) {
            is TargetRef.Device -> "device:${tg.deviceId}"
            is TargetRef.Ssh -> "ssh:${tg.hostId}"
        }
        is Route.Remote -> "device:${t.deviceId}"
        else -> null
    }

    HostsScreen(
        data = data,
        filter = filter,
        refreshing = refreshing,
        selectedKey = selectedKey,
        actions = object : HostsActions {
            override fun onFilter(f: HostsFilter) {
                filter = f
            }

            override fun onRefresh() = container.hosts.refresh()
            override fun onItem(item: HostItem) {
                sheetItem = item
            }

            override fun onFolderToggle(folder: VaultEntry, expanded: Boolean) = container.hosts.setFolderExpanded(folder, expanded)
            override fun onFolderMenu(folder: VaultEntry, action: FolderAction) {
                when (action) {
                    FolderAction.RENAME -> folderDialog = folder to null
                    FolderAction.NEW_SUBFOLDER -> folderDialog = null to folder.id
                    FolderAction.MOVE -> moveFolder = folder
                    FolderAction.DELETE -> deleteFolder = folder
                }
            }

            override fun onAdd(kind: AddKind) {
                when (kind) {
                    AddKind.FOLDER -> folderDialog = null to null
                    AddKind.SSH -> nav.push(Route.EditEntry(EntryKind.SSH, null, null))
                    AddKind.RDP -> nav.push(Route.EditEntry(EntryKind.RDP, null, null))
                    AddKind.CREDENTIAL -> nav.push(Route.EditEntry(EntryKind.CREDENTIAL, null, null))
                    AddKind.SNIPPET -> nav.push(Route.EditEntry(EntryKind.SNIPPET, null, null))
                    AddKind.ASSIST -> assist = true
                }
            }

            override fun onCredential(entry: VaultEntry) = nav.push(Route.EditEntry(EntryKind.CREDENTIAL, entry.id, null))
            override fun onSnippet(entry: VaultEntry) = nav.push(Route.EditEntry(EntryKind.SNIPPET, entry.id, null))
        },
    )

    sheetItem?.let { item ->
        val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
        val liveVia by container.connections.sshVia.collectAsState()
        val via = (item as? HostItem.Ssh)?.let { sshViaText(data, it.host, liveVia[it.entry.id]) }
        ModalBottomSheet(onDismissRequest = { sheetItem = null }, sheetState = sheetState) {
            val canWake = item is HostItem.Device && data.canWake(item.device.id)
            HostActionsContent(item, sshVia = via, sshConnected = item is HostItem.Ssh && item.entry.id in liveVia, canWake = canWake, onAction = { a ->
                sheetItem = null
                when (a) {
                    HostAction.MOVE -> moveItem = item
                    HostAction.DELETE -> deleteItem = item
                    HostAction.WAKE -> (item as? HostItem.Device)?.let { wakeDevice(container, it.device.id, context.resources) }
                    else -> openHostAction(container, nav, item, a)
                }
            })
        }
    }
    moveItem?.let { item ->
        MoveDialog(data, emptySet(), item.folderId, onPick = {
            container.hosts.move(item, it)
            moveItem = null
        }, onDismiss = { moveItem = null })
    }
    moveFolder?.let { f ->
        MoveDialog(data, HostTree.descendants(data, f.id), f.folderId, onPick = {
            container.hosts.moveFolder(f, it)
            moveFolder = null
        }, onDismiss = { moveFolder = null })
    }
    deleteItem?.let { item ->
        val msg = if (item is HostItem.Device) stringResource(R.string.remove_device_message, item.title)
        else stringResource(R.string.delete_item_message, item.title)
        ConfirmDialog(
            title = stringResource(if (item is HostItem.Device) R.string.action_remove_device else R.string.delete),
            message = msg,
            confirmText = stringResource(if (item is HostItem.Device) R.string.action_remove_device else R.string.delete),
            destructive = true,
            onConfirm = {
                when (item) {
                    is HostItem.Device -> container.hosts.removeDevice(item.device.id)
                    is HostItem.Ssh -> container.hosts.delete(item.entry.id)
                    is HostItem.Rdp -> container.hosts.delete(item.entry.id)
                }
            },
            onDismiss = { deleteItem = null },
        )
    }
    deleteFolder?.let { f ->
        ConfirmDialog(
            title = stringResource(R.string.delete),
            message = stringResource(R.string.folder_delete_message, (f.data as VaultData.Folder).name),
            confirmText = stringResource(R.string.delete),
            destructive = true,
            onConfirm = { container.hosts.delete(f.id) },
            onDismiss = { deleteFolder = null },
        )
    }
    folderDialog?.let { (renaming, parent) ->
        TextInputDialog(
            title = stringResource(if (renaming != null) R.string.folder_rename else R.string.folder_new),
            label = stringResource(R.string.folder_name),
            initial = (renaming?.data as? VaultData.Folder)?.name.orEmpty(),
            onConfirm = { name ->
                if (renaming != null) {
                    container.hosts.save(renaming.copy(data = (renaming.data as VaultData.Folder).copy(name = name)))
                } else {
                    container.hosts.saveData(null, parent, VaultData.Folder(name, null, true))
                }
            },
            onDismiss = { folderDialog = null },
        )
    }
    if (assist) {
        AssistDialog(onConnect = { code, verify ->
            scope.launch {
                try {
                    val session = container.client.connectAssist(code, verify)
                    container.toast(session.info().hostname)
                } catch (e: Exception) {
                    container.toast(e.userMessage())
                }
            }
        }, onDismiss = { assist = false })
    }
}

@Composable
fun AiTab(container: AppContainer, nav: Navigator) {
    val items by container.agents.items.collectAsState()
    val selected = (nav.top as? Route.Conversation)?.id
    val wide = isWideLayout()
    LaunchedEffect(Unit) {
        container.agents.loadModels()
        container.agents.refreshList()
    }
    ConversationListScreen(
        items = items,
        onOpen = { id ->
            if (nav.top is Route.Conversation || nav.top is Route.NewConversation) nav.replace(Route.Conversation(id)) else nav.push(Route.Conversation(id))
        },
        // 平板双栏：右侧没有打开的对话时就是新对话
        onNew = { if (wide) nav.selectTab(Tab.AI) else nav.push(Route.NewConversation(null)) },
        onDelete = { id ->
            container.agents.delete(id)
            if (selected == id) nav.pop()
        },
        selectedId = selected,
    )
}

@Composable
fun FilesTab(container: AppContainer, nav: Navigator) {
    val data by container.hosts.data.collectAsState()
    val transfers by container.transfers.items.collectAsState()
    val targets = HostTree.items(data).filter { (it is HostItem.Device && it.reachable()) || it is HostItem.Ssh }
        .sortedWith(compareBy({ it !is HostItem.Device }, { it.title.lowercase() }))
    val selected = (nav.top as? Route.Files)?.target?.let { if (it is TargetRef.Device) "device:${it.deviceId}" else "ssh:${(it as TargetRef.Ssh).hostId}" }
    FilesHomeScreen(
        targets = targets,
        transfers = transfers,
        selectedKey = selected,
        onBrowse = { item ->
            val r = Route.Files(if (item is HostItem.Device) TargetRef.Device(item.device.id) else TargetRef.Ssh((item as HostItem.Ssh).entry.id))
            if (nav.top is Route.Files) nav.replace(r) else nav.push(r)
        },
        actions = object : TransferActions {
            override fun onPause(id: Long) = container.transfers.pause(id)
            override fun onResume(id: Long) = container.transfers.resume(id)
            override fun onCancel(id: Long) = container.transfers.cancel(id)
            override fun onRetry(id: Long) = container.transfers.retry(id)
            override fun onClearFinished() = container.transfers.clearFinished()
        },
    )
}

@Composable
fun SettingsTab(container: AppContainer, nav: Navigator, openUrl: (String) -> Unit) {
    val context = LocalContext.current
    val prefs by container.settings.prefs.collectAsState()
    val user by container.account.user.collectAsState()
    val server by container.account.server.collectAsState()
    val totp by container.account.totpEnabled.collectAsState()
    var language by remember { mutableStateOf(container.settings.language()) }
    var changePw by remember { mutableStateOf(false) }
    var twoFactor by remember { mutableStateOf(false) }
    var logout by remember { mutableStateOf(false) }
    var unknownSources by remember { mutableStateOf(false) }
    val updateState by container.updates.state.collectAsState()
    val updateDialog by container.updates.dialog.collectAsState()
    val biometric = remember {
        BiometricManager.from(context).canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_WEAK or BiometricManager.Authenticators.DEVICE_CREDENTIAL) ==
            BiometricManager.BIOMETRIC_SUCCESS
    }
    val pwDone = stringResource(R.string.cp_done)
    val tfOn = stringResource(R.string.totp_enabled_toast)
    val tfOff = stringResource(R.string.totp_disabled_toast)
    SettingsScreen(
        prefs = prefs,
        language = language,
        user = user,
        server = server,
        totpEnabled = totp,
        themes = TerminalThemes.all(context),
        about = AboutInfo(
            appVersion = runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull() ?: "?",
            coreVersion = container.core.coreVersion(),
            mockCore = BuildConfig.USE_MOCK_CORE || CoreProvider.isMock,
        ),
        biometricAvailable = biometric,
        actions = object : SettingsActions {
            override fun onPrefs(block: (Prefs) -> Prefs) = container.settings.update(block)
            override fun onLanguage(lang: AppLanguage) {
                language = lang
                container.settings.setLanguage(lang)
            }

            override fun onChangePassword() {
                changePw = true
            }

            override fun onTwoFactor() {
                twoFactor = true
            }

            override fun onLogout() {
                logout = true
            }

            override fun onAppLock(enabled: Boolean) = container.settings.update { it.copy(appLock = enabled) }
            override fun onLicenses() = nav.push(Route.Licenses)
            override fun onSource() = openUrl("https://github.com/LemonITCN/lemon-galaxy")
            override fun onCheckUpdate() {
                when (container.updates.state.value) {
                    is UpdateState.Available, is UpdateState.Downloading, is UpdateState.Downloaded -> container.updates.openDialog()
                    else -> container.updates.check()
                }
            }
        },
        update = updateState,
    )
    if (updateDialog) {
        UpdateDialog(
            updateState,
            current = runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull() ?: "?",
            actions = object : UpdateActions {
                override fun onDownload() = container.updates.download()
                override fun onInstall() {
                    (updateState as? UpdateState.Downloaded)?.let { installApk(context, it.file, onNeedPermission = { unknownSources = true }, onError = container::toast) }
                }

                override fun onDismiss() = container.updates.dismissDialog()
            },
        )
    }
    // 下载完成后直接交给系统安装器（只自动弹一次，之后点「安装」）
    LaunchedEffect(updateState) {
        val s = updateState
        if (s is UpdateState.Downloaded) installApk(context, s.file, onNeedPermission = { unknownSources = true }, onError = container::toast)
    }
    if (unknownSources) {
        UnknownSourcesDialog(
            onOpenSettings = {
                runCatching {
                    context.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${context.packageName}")))
                }
            },
            onDismiss = { unknownSources = false },
        )
    }
    if (changePw) {
        ChangePasswordDialog(container.account::changePassword, requireTotp = totp == true, onDone = { container.toast(pwDone) }, onDismiss = { changePw = false })
    }
    if (twoFactor && totp != null) {
        TwoFactorDialog(
            enabled = totp == true,
            setup = container.account::totpSetup,
            verify = container.account::totpVerify,
            disable = container.account::totpDisable,
            onDone = { on -> container.toast(if (on) tfOn else tfOff) },
            onDismiss = { twoFactor = false },
        )
    }
    if (logout) {
        ConfirmDialog(
            title = stringResource(R.string.settings_logout),
            message = stringResource(R.string.logout_message),
            confirmText = stringResource(R.string.settings_logout),
            destructive = true,
            onConfirm = {
                nav.reset()
                container.signOut()
            },
            onDismiss = { logout = false },
        )
    }
}

/**
 * 把下载好的 APK 交给系统安装器（FileProvider 授权读取）。
 * 系统还没允许本应用「安装未知应用」时先引导用户去设置里打开。
 */
fun installApk(context: Context, file: File, onNeedPermission: () -> Unit, onError: (String) -> Unit) {
    if (!context.packageManager.canRequestPackageInstalls()) {
        onNeedPermission()
        return
    }
    try {
        val uri = FileProvider.getUriForFile(context, context.packageName + ".updates", file)
        val intent = Intent(Intent.ACTION_VIEW)
            .setDataAndType(uri, "application/vnd.android.package-archive")
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        context.startActivity(intent)
    } catch (e: Exception) {
        onError(context.getString(R.string.update_install_failed, e.message ?: e.javaClass.simpleName))
    }
}
