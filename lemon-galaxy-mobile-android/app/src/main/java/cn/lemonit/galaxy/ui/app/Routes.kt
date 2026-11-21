package cn.lemonit.galaxy.ui.app

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.content.pm.ActivityInfo
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.AgentTargetOf
import cn.lemonit.galaxy.app.AppContainer
import cn.lemonit.galaxy.app.ConversationController
import cn.lemonit.galaxy.app.EntryKind
import cn.lemonit.galaxy.app.HostItem
import cn.lemonit.galaxy.app.HostTree
import cn.lemonit.galaxy.app.Navigator
import cn.lemonit.galaxy.app.Route
import cn.lemonit.galaxy.app.TabStatus
import cn.lemonit.galaxy.app.TargetRef
import cn.lemonit.galaxy.app.userMessage
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.ApprovalDecision
import cn.lemonit.galaxy.core.bridge.ConversationConfig
import cn.lemonit.galaxy.core.bridge.FileEntry
import cn.lemonit.galaxy.core.bridge.FileKind
import cn.lemonit.galaxy.core.bridge.InputEvent
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.TerminalSessionInfo
import cn.lemonit.galaxy.core.bridge.VaultData
import cn.lemonit.galaxy.core.platform.HoldMulticastLock
import cn.lemonit.galaxy.data.AppSettings
import cn.lemonit.galaxy.data.VideoQuality
import cn.lemonit.galaxy.ui.ai.ConversationActions
import cn.lemonit.galaxy.ui.ai.ConversationScreen
import cn.lemonit.galaxy.ui.ai.NewConversationDraft
import cn.lemonit.galaxy.ui.ai.NewConversationScreen
import cn.lemonit.galaxy.ui.ai.key
import cn.lemonit.galaxy.ui.components.ConfirmDialog
import cn.lemonit.galaxy.ui.components.TextInputDialog
import cn.lemonit.galaxy.ui.files.ChmodDialog
import cn.lemonit.galaxy.ui.files.FileBrowserActions
import cn.lemonit.galaxy.ui.files.FileBrowserController
import cn.lemonit.galaxy.ui.files.FileBrowserScreen
import cn.lemonit.galaxy.ui.files.FileEditorScreen
import cn.lemonit.galaxy.ui.files.FileMenuContent
import cn.lemonit.galaxy.ui.files.SortKey
import cn.lemonit.galaxy.ui.hosts.EntryEditorScreen
import cn.lemonit.galaxy.ui.hosts.reachable
import cn.lemonit.galaxy.ui.remote.RemoteActions
import cn.lemonit.galaxy.ui.remote.RemoteDesktopController
import cn.lemonit.galaxy.ui.remote.RemoteDesktopScreen
import cn.lemonit.galaxy.ui.settings.LicensesScreen
import cn.lemonit.galaxy.ui.terminal.SessionsPanel
import cn.lemonit.galaxy.ui.terminal.TerminalActions
import cn.lemonit.galaxy.ui.terminal.TerminalScreen
import cn.lemonit.galaxy.ui.terminal.TerminalTabUi
import cn.lemonit.galaxy.ui.theme.TerminalThemes
import com.termux.view.TerminalView
import kotlinx.coroutines.launch

@Composable
fun RouteContent(route: Route, container: AppContainer, nav: Navigator, showBack: Boolean, openUrl: (String) -> Unit) {
    when (route) {
        is Route.Terminal -> TerminalRoute(container, nav, showBack)
        is Route.Files -> FilesRoute(route.target, container, nav, showBack)
        is Route.FileEditor -> FileEditorRoute(route, container, nav)
        is Route.Remote -> RemoteRoute(route.deviceId, container, nav)
        is Route.Conversation -> ConversationRoute(route.id, container, nav, showBack)
        is Route.NewConversation -> NewConversationRoute(route.target, container, nav, showBack)
        is Route.EditEntry -> EditorRoute(route, container, nav)
        Route.Licenses -> LicensesScreen(onBack = { nav.pop() }, onOpen = openUrl)
    }
}

fun Context.findActivity(): Activity? {
    var c: Context? = this
    while (c is ContextWrapper) {
        if (c is Activity) return c
        c = c.baseContext
    }
    return null
}

// ---------------- 终端 ----------------

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TerminalRoute(container: AppContainer, nav: Navigator, showBack: Boolean) {
    // 连接期间持有组播锁，内核的局域网发现才收得到回应
    HoldMulticastLock()
    val context = LocalContext.current
    val tabs by container.terminals.tabs.collectAsState()
    val activeId by container.terminals.active.collectAsState()
    val prefs by container.settings.prefs.collectAsState()
    val data by container.hosts.data.collectAsState()
    val paths by container.connections.paths.collectAsState()
    val scope = rememberCoroutineScope()
    var sessionsOpen by remember { mutableStateOf(false) }
    var remote by remember { mutableStateOf<List<TerminalSessionInfo>?>(null) }

    // 每个标签的状态与标题
    val uis = tabs.map { t ->
        val status by t.status.collectAsState()
        val title by t.title.collectAsState()
        TerminalTabUi(t.id, title, t.targetName, status, t.session, t.target is TargetRef.Ssh)
    }
    LaunchedEffect(tabs.isEmpty()) { if (tabs.isEmpty()) nav.pop() }
    val active = tabs.firstOrNull { it.id == activeId } ?: tabs.firstOrNull() ?: return
    val path = (active.target as? TargetRef.Device)?.let { paths[it.deviceId] }
    // SSH 主机可以单独指定终端配色
    val hostTheme = (active.target as? TargetRef.Ssh)?.let { (data.entry(it.hostId)?.data as? VaultData.SshHost)?.terminalTheme }
    val theme = TerminalThemes.find(context, hostTheme ?: prefs.terminalTheme)
    val voiceAvailable = remember { SpeechRecognizer.isRecognitionAvailable(context) }
    val voice = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { r ->
        val text = r.data?.getStringArrayListExtra(RecognizerIntent.EXTRA_RESULTS)?.firstOrNull() ?: return@rememberLauncherForActivityResult
        active.sendText(text)
    }

    TerminalScreen(
        tabs = uis,
        activeId = active.id,
        path = path,
        theme = theme,
        fontSizeSp = prefs.terminalFontSize,
        inputBox = prefs.terminalInputBox,
        snippets = data.snippets.mapNotNull { it.data as? VaultData.Snippet },
        voiceAvailable = voiceAvailable,
        showBack = showBack || !isWideLayout(),
        actions = object : TerminalActions {
            override fun onBack() {
                nav.pop()
            }

            override fun onSelect(id: Long) = container.terminals.select(id)
            override fun onCloseTab(id: Long, kill: Boolean) {
                tabs.firstOrNull { it.id == id }?.let { container.terminals.close(it, kill) }
            }

            override fun onNewSession() {
                container.terminals.open(active.target, forceNew = true)
            }

            override fun onShowSessions() {
                sessionsOpen = true
                remote = null
                val t = active.target
                if (t is TargetRef.Device) {
                    scope.launch {
                        remote = try {
                            container.terminals.remoteSessions(t)
                        } catch (e: Exception) {
                            container.toast(e.userMessage())
                            emptyList()
                        }
                    }
                }
            }

            override fun onReconnect(id: Long) {
                tabs.firstOrNull { it.id == id }?.reconnect()
            }

            override fun onToggleInputBox() = container.settings.update { it.copy(terminalInputBox = !it.terminalInputBox) }
            override fun onSendLine(text: String) = active.sendText(text + "\r")
            override fun onSnippet(snippet: VaultData.Snippet) = active.sendText(snippet.command + "\r")
            override fun onFontScale(bigger: Boolean) = container.settings.update {
                it.copy(terminalFontSize = (it.terminalFontSize + if (bigger) 1 else -1).coerceIn(AppSettings.TERMINAL_FONT_MIN, AppSettings.TERMINAL_FONT_MAX))
            }

            override fun onVoice() {
                runCatching {
                    voice.launch(Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH).putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM))
                }
            }

            override fun onViewBound(id: Long, view: TerminalView?) {
                tabs.firstOrNull { it.id == id }?.bindView(view)
            }
        },
    )

    if (sessionsOpen) {
        val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
        val t = active.target
        ModalBottomSheet(onDismissRequest = { sessionsOpen = false }, sheetState = sheet) {
            SessionsPanel(
                tabs = uis,
                activeId = active.id,
                remoteTitle = if (t is TargetRef.Device) stringResource(R.string.terminal_remote_sessions, active.targetName) else null,
                remote = remote,
                openRemoteIds = tabs.filter { it.status.value == TabStatus.Connected }.mapNotNull { it.remoteSessionId }.toSet(),
                onSelect = {
                    container.terminals.select(it)
                    sessionsOpen = false
                },
                onAttach = { sid ->
                    container.terminals.open(t, attachId = sid)
                    sessionsOpen = false
                },
                onKillRemote = { sid ->
                    if (t is TargetRef.Device) {
                        scope.launch {
                            runCatching { container.terminals.killRemote(t, sid) }
                            remote = runCatching { container.terminals.remoteSessions(t) }.getOrDefault(emptyList())
                        }
                    }
                },
                onNew = {
                    container.terminals.open(t, forceNew = true)
                    sessionsOpen = false
                },
            )
        }
    }
}

// ---------------- 文件 ----------------

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FilesRoute(target: TargetRef, container: AppContainer, nav: Navigator, showBack: Boolean) {
    // 连接期间持有组播锁，内核的局域网发现才收得到回应
    HoldMulticastLock()
    val data by container.hosts.data.collectAsState()
    val title = data.targetName(target)
    val controller = remember(target) { FileBrowserController(target, container.connections, container.scope, container::toast).also { it.start() } }
    val state by controller.state.collectAsState()
    var menuFor by remember { mutableStateOf<FileEntry?>(null) }
    var renameFor by remember { mutableStateOf<FileEntry?>(null) }
    var chmodFor by remember { mutableStateOf<FileEntry?>(null) }
    var deleteFor by remember { mutableStateOf<FileEntry?>(null) }
    var newFolder by remember { mutableStateOf(false) }
    var pendingDownload by remember { mutableStateOf<FileEntry?>(null) }
    val started = stringResource(R.string.files_download_started)
    val upStarted = stringResource(R.string.files_upload_started)

    val createDoc = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/octet-stream")) { uri ->
        val e = pendingDownload
        pendingDownload = null
        if (uri != null && e != null) {
            container.transfers.download(target, title, e.path, e.size.toLong(), uri)
            container.toast(started)
        }
    }
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.GetMultipleContents()) { uris ->
        uris.forEach { container.transfers.upload(target, title, it, state.path) { controller.refresh() } }
        if (uris.isNotEmpty()) container.toast(upStarted)
    }
    val download = { e: FileEntry ->
        pendingDownload = e
        createDoc.launch(e.name)
    }

    FileBrowserScreen(
        title = title,
        state = state,
        showBack = showBack || !isWideLayout(),
        actions = object : FileBrowserActions {
            override fun onBack() {
                nav.pop()
            }

            override fun onOpen(entry: FileEntry) {
                when {
                    entry.kind == FileKind.DIR -> controller.open(entry.path)
                    entry.kind == FileKind.SYMLINK -> controller.open(
                        entry.linkTarget?.let { if (it.startsWith("/")) it else entry.path.substringBeforeLast('/') + "/" + it } ?: entry.path,
                    )
                    FileBrowserController.looksText(entry) -> nav.push(Route.FileEditor(target, entry.path))
                    else -> menuFor = entry
                }
            }

            override fun onMenu(entry: FileEntry) {
                menuFor = entry
            }

            override fun onUp() = controller.up()
            override fun onHome() = controller.open(state.home)
            override fun onRefresh() = controller.refresh()
            override fun onUpload() = pick.launch("*/*")
            override fun onNewFolder() {
                newFolder = true
            }

            override fun onSort(key: SortKey) = controller.setSort(key)
            override fun onToggleHidden() = controller.toggleHidden()
        },
    )
    menuFor?.let { e ->
        val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
        ModalBottomSheet(onDismissRequest = { menuFor = null }, sheetState = sheet) {
            FileMenuContent(
                e,
                onEdit = if (e.kind == FileKind.FILE) ({
                    menuFor = null
                    nav.push(Route.FileEditor(target, e.path))
                }) else null,
                onDownload = if (e.kind == FileKind.FILE) ({
                    menuFor = null
                    download(e)
                }) else null,
                onRename = {
                    menuFor = null
                    renameFor = e
                },
                onChmod = {
                    menuFor = null
                    chmodFor = e
                },
                onDelete = {
                    menuFor = null
                    deleteFor = e
                },
            )
        }
    }
    renameFor?.let { e ->
        TextInputDialog(
            title = stringResource(R.string.rename),
            label = stringResource(R.string.files_new_name),
            initial = e.name,
            validate = { if (it.isBlank() || it.contains('/')) "" else null },
            onConfirm = { controller.rename(e, it) },
            onDismiss = { renameFor = null },
        )
    }
    chmodFor?.let { e -> ChmodDialog(e, { controller.chmod(e, it) }) { chmodFor = null } }
    deleteFor?.let { e ->
        ConfirmDialog(
            title = stringResource(R.string.files_delete_title, e.name),
            message = stringResource(if (e.kind == FileKind.DIR) R.string.files_delete_dir_message else R.string.files_delete_file_message),
            confirmText = stringResource(R.string.delete),
            destructive = true,
            onConfirm = { controller.delete(e) },
            onDismiss = { deleteFor = null },
        )
    }
    if (newFolder) {
        TextInputDialog(
            title = stringResource(R.string.files_new_folder),
            label = stringResource(R.string.files_new_name),
            validate = { if (it.isBlank() || it.contains('/')) "" else null },
            onConfirm = { controller.mkdir(it) },
            onDismiss = { newFolder = false },
        )
    }
}

@Composable
fun FileEditorRoute(route: Route.FileEditor, container: AppContainer, nav: Navigator) {
    val controller = remember(route) { FileBrowserController(route.target, container.connections, container.scope, container::toast) }
    var original by remember { mutableStateOf<String?>(null) }
    var text by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var saving by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val saved = stringResource(R.string.editor_saved)
    LaunchedEffect(route) {
        try {
            val t = controller.readText(route.path)
            original = t
            text = t
        } catch (e: Exception) {
            error = e.userMessage()
        }
    }
    FileEditorScreen(
        name = route.path.substringAfterLast('/'),
        path = route.path,
        text = text,
        error = error,
        saving = saving,
        onChange = { text = it },
        dirty = text != null && text != original,
        onSave = {
            val t = text ?: return@FileEditorScreen
            saving = true
            scope.launch {
                try {
                    controller.writeText(route.path, t)
                    original = t
                    container.toast(saved)
                } catch (e: Exception) {
                    container.toast(e.userMessage())
                }
                saving = false
            }
        },
        onBack = { nav.pop() },
    )
}

// ---------------- 远程桌面 ----------------

@Composable
fun RemoteRoute(deviceId: String, container: AppContainer, nav: Navigator) {
    // 连接期间持有组播锁，内核的局域网发现才收得到回应
    HoldMulticastLock()
    val context = LocalContext.current
    val data by container.hosts.data.collectAsState()
    val prefs = container.settings.prefs.value
    val device = data.device(deviceId)
    val controller = remember(deviceId) {
        RemoteDesktopController(
            deviceId, data.deviceName(deviceId), device?.platform ?: Platform.WINDOWS,
            container.connections, container.scope, prefs.defaultQuality, prefs.remoteMouseMode,
        ).also { it.start() }
    }
    val ui by controller.ui.collectAsState()
    val scope = rememberCoroutineScope()
    val sentText = stringResource(R.string.rd_clipboard_sent)
    val gotText = stringResource(R.string.rd_clipboard_received)
    val emptyText = stringResource(R.string.rd_clipboard_empty)

    // 横屏全屏沉浸，离开时恢复
    DisposableEffect(Unit) {
        val activity = context.findActivity()
        val oldOrientation = activity?.requestedOrientation
        activity?.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
        val window = activity?.window
        val insets = window?.let { WindowCompat.getInsetsController(it, it.decorView) }
        insets?.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        insets?.hide(WindowInsetsCompat.Type.systemBars())
        onDispose {
            insets?.show(WindowInsetsCompat.Type.systemBars())
            if (oldOrientation != null) activity.requestedOrientation = oldOrientation
            controller.stop()
        }
    }

    RemoteDesktopScreen(
        ui = ui,
        actions = object : RemoteActions {
            override fun onInput(events: List<InputEvent>) = controller.sendAll(events)
            override fun onDisplay(id: UInt) = controller.switchDisplay(id)
            override fun onQuality(q: VideoQuality) = controller.setQuality(q)
            override fun onMouseMode(on: Boolean) = controller.setMouseMode(on)
            override fun onClipboardSend() {
                val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                val text = cm.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(context)?.toString()
                if (text.isNullOrEmpty()) {
                    container.toast(emptyText)
                    return
                }
                scope.launch {
                    try {
                        controller.clipboardSend(text)
                        container.toast(sentText)
                    } catch (e: Exception) {
                        container.toast(e.userMessage())
                    }
                }
            }

            override fun onClipboardGet() {
                scope.launch {
                    try {
                        val text = controller.clipboardGet()
                        if (text.isNullOrEmpty()) {
                            container.toast(emptyText)
                        } else {
                            val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                            cm.setPrimaryClip(ClipData.newPlainText("remote", text))
                            container.toast(gotText)
                        }
                    } catch (e: Exception) {
                        container.toast(e.userMessage())
                    }
                }
            }

            override fun onDisconnect() {
                nav.pop()
            }
        },
        video = { modifier ->
            AndroidView(
                factory = { ctx ->
                    SurfaceView(ctx).apply {
                        holder.addCallback(object : SurfaceHolder.Callback {
                            override fun surfaceCreated(h: SurfaceHolder) = controller.setSurface(h.surface)
                            override fun surfaceChanged(h: SurfaceHolder, format: Int, width: Int, height: Int) = Unit
                            override fun surfaceDestroyed(h: SurfaceHolder) = controller.setSurface(null)
                        })
                    }
                },
                modifier = modifier,
            )
        },
    )
}

// ---------------- AI ----------------

@Composable
fun ConversationRoute(id: String, container: AppContainer, nav: Navigator, showBack: Boolean) {
    val controller: ConversationController? = remember(id) { container.agents.open(id) }
    val models by container.agents.models.collectAsState()
    if (controller == null) {
        LaunchedEffect(id) { nav.pop() }
        return
    }
    val ui by controller.ui.collectAsState()
    var deleting by remember { mutableStateOf(false) }
    LaunchedEffect(ui.running, ui.items.size) { container.agents.refreshList() }
    ConversationScreen(
        ui = ui,
        maxMode = models?.maxMode ?: AiMode.FULL_TRUST,
        showBack = showBack || !isWideLayout(),
        actions = object : ConversationActions {
            override fun onBack() {
                nav.pop()
            }

            override fun onSend(text: String) = controller.send(text)
            override fun onStop() = controller.stop()
            override fun onDecide(callId: String, decision: ApprovalDecision) = controller.decide(callId, decision)
            override fun onAnswer(questionId: String, answer: String) = controller.answer(questionId, answer)
            override fun onSecret(requestId: String, secret: CharArray) = controller.provideSecret(requestId, secret)
            override fun onMode(mode: AiMode, trustMinutes: UInt?) = controller.setMode(mode, trustMinutes)
            override fun onDelete() {
                deleting = true
            }
        },
    )
    if (deleting) {
        ConfirmDialog(
            title = stringResource(R.string.ai_delete),
            message = stringResource(R.string.ai_delete_message),
            confirmText = stringResource(R.string.delete),
            destructive = true,
            onConfirm = {
                container.agents.delete(id)
                nav.pop()
            },
            onDismiss = { deleting = false },
        )
    }
}

@Composable
fun NewConversationRoute(target: TargetRef?, container: AppContainer, nav: Navigator, showBack: Boolean) {
    val data by container.hosts.data.collectAsState()
    val models by container.agents.models.collectAsState()
    val modelsError by container.agents.modelsError.collectAsState()
    val prefs = container.settings.prefs.value
    val targets = HostTree.items(data).filter { (it is HostItem.Device && it.reachable()) || it is HostItem.Ssh }
    val initialKey = when (target) {
        is TargetRef.Device -> "device:${target.deviceId}"
        is TargetRef.Ssh -> "ssh:${target.hostId}"
        null -> null
    }
    var draft by remember {
        mutableStateOf(NewConversationDraft(targets = setOfNotNull(initialKey), mode = prefs.defaultAiMode))
    }
    LaunchedEffect(models) {
        if (models == null) container.agents.loadModels()
        if (draft.modelKey == null) models?.items?.firstOrNull()?.let { draft = draft.copy(modelKey = it.key()) }
    }
    NewConversationScreen(
        models = models,
        modelsError = modelsError,
        targets = targets,
        draft = draft,
        onDraft = { draft = it },
        showBack = showBack || !isWideLayout(),
        onBack = { nav.pop() },
        onStart = {
            val m = models?.items?.firstOrNull { it.key() == draft.modelKey } ?: return@NewConversationScreen
            val chosen = targets.filter { it.key in draft.targets }
            val config = ConversationConfig(m.providerId, m.id, chosen.mapNotNull(AgentTargetOf::of), draft.mode, draft.planFirst)
            val c = container.agents.create(config, m.name, m.vision, chosen.map { it.title }, if (draft.mode == AiMode.FULL_TRUST) draft.trustMinutes else null)
            nav.replace(Route.Conversation(c.ui.value.id))
        },
    )
}

// ---------------- 编辑条目 ----------------

@Composable
fun EditorRoute(route: Route.EditEntry, container: AppContainer, nav: Navigator) {
    val context = LocalContext.current
    val data by container.hosts.data.collectAsState()
    val entry = when (route.kind) {
        EntryKind.DEVICE -> route.id?.let { data.metaFor(it) }
        else -> route.id?.let { data.entry(it) }
    }
    var confirmDelete by remember { mutableStateOf(false) }
    EntryEditorScreen(
        kind = route.kind,
        entry = entry,
        deviceId = if (route.kind == EntryKind.DEVICE) route.id else null,
        initialFolder = route.folderId,
        data = data,
        themes = TerminalThemes.all(context),
        onSave = { d, folder ->
            if (route.kind == EntryKind.DEVICE) {
                val meta = d as VaultData.DeviceMeta
                container.hosts.updateDeviceMeta(meta.deviceId, folder, moveFolder = true) { meta.copy(favorite = it.favorite) }
            } else {
                container.hosts.saveData(entry?.id, folder, d)
            }
        },
        onDelete = if (route.kind != EntryKind.DEVICE && entry != null) ({ confirmDelete = true }) else null,
        onBack = { nav.pop() },
    )
    if (confirmDelete && entry != null) {
        ConfirmDialog(
            title = stringResource(R.string.delete),
            message = null,
            confirmText = stringResource(R.string.delete),
            destructive = true,
            onConfirm = {
                container.hosts.delete(entry.id)
                nav.pop()
            },
            onDismiss = { confirmDelete = false },
        )
    }
}
