package cn.lemonit.galaxy.ui.app

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.content.pm.ActivityInfo
import android.os.Build
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.lifecycle.compose.LifecycleStartEffect
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.AiModelsState
import cn.lemonit.galaxy.app.AppContainer
import cn.lemonit.galaxy.app.MentionCandidate
import cn.lemonit.galaxy.app.Refs
import cn.lemonit.galaxy.app.Tab
import cn.lemonit.galaxy.app.aiAvailability
import cn.lemonit.galaxy.app.canSend
import cn.lemonit.galaxy.app.capMode
import cn.lemonit.galaxy.app.mentionCandidates
import cn.lemonit.galaxy.app.pickDefaultModel
import cn.lemonit.galaxy.core.bridge.AiModel
import cn.lemonit.galaxy.ui.ai.ChatActions
import cn.lemonit.galaxy.ui.ai.ChatScreen
import cn.lemonit.galaxy.ui.ai.ChatSettings
import cn.lemonit.galaxy.ui.ai.ComposerState
import cn.lemonit.galaxy.ui.ai.RefSources
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
import cn.lemonit.galaxy.core.bridge.RecordingInfo
import cn.lemonit.galaxy.core.bridge.TerminalSessionInfo
import cn.lemonit.galaxy.core.bridge.VaultData
import cn.lemonit.galaxy.core.platform.HoldMulticastLock
import cn.lemonit.galaxy.data.AppSettings
import cn.lemonit.galaxy.data.VideoQuality
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
import cn.lemonit.galaxy.ui.hosts.sshViaText
import cn.lemonit.galaxy.ui.remote.RemoteActions
import cn.lemonit.galaxy.ui.remote.RemoteDesktopController
import cn.lemonit.galaxy.ui.remote.RemoteDesktopScreen
import cn.lemonit.galaxy.ui.remote.Recordings
import cn.lemonit.galaxy.ui.remote.RecordingSavedDialog
import cn.lemonit.galaxy.ui.settings.LicensesScreen
import cn.lemonit.galaxy.ui.terminal.SessionsPanel
import cn.lemonit.galaxy.ui.terminal.TerminalActions
import cn.lemonit.galaxy.ui.terminal.TerminalScreen
import cn.lemonit.galaxy.ui.terminal.TerminalTabUi
import cn.lemonit.galaxy.ui.theme.TerminalThemes
import com.termux.view.TerminalView
import kotlinx.coroutines.launch
import java.io.File

@Composable
fun RouteContent(route: Route, container: AppContainer, nav: Navigator, showBack: Boolean, openUrl: (String) -> Unit) {
    when (route) {
        is Route.Terminal -> TerminalRoute(container, nav, showBack)
        is Route.Files -> FilesRoute(route.target, container, nav, showBack)
        is Route.FileEditor -> FileEditorRoute(route, container, nav)
        is Route.Remote -> RemoteRoute(route.deviceId, container, nav)
        is Route.Conversation -> ChatRoute(route.id, null, container, nav, showBack)
        is Route.NewConversation -> ChatRoute(null, route.target, container, nav, showBack)
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
    val liveVia by container.connections.sshVia.collectAsState()
    val uis = tabs.map { t ->
        val status by t.status.collectAsState()
        val title by t.title.collectAsState()
        val via = (t.target as? TargetRef.Ssh)?.let { tg -> (data.entry(tg.hostId)?.data as? VaultData.SshHost)?.let { sshViaText(data, it, liveVia[tg.hostId]) } }
        TerminalTabUi(t.id, title, t.targetName, status, t.session, t.target is TargetRef.Ssh, via)
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
    // 刚停下的录像（手动停止或画面断开时弹保存提示）
    var savedRecording by remember { mutableStateOf<RecordingInfo?>(null) }
    val controller = remember(deviceId) {
        RemoteDesktopController(
            deviceId, data.deviceName(deviceId), device?.platform ?: Platform.WINDOWS,
            container.connections, container.scope, prefs.defaultQuality, prefs.remoteMouseMode,
            audioWanted = prefs.remoteAudio,
            onAudioError = { msg -> container.toast(context.getString(R.string.rd_sound_failed, msg)) },
            recordingDir = { Recordings.dir(context) },
            onRecordingSaved = { info, auto ->
                if (auto) {
                    // 离开界面或切后台时自动停的：界面已不在，只提示保存位置
                    val where = Recordings.displayPath(File(info.paths.first()).parent ?: info.paths.first())
                    container.toast(context.getString(R.string.rd_record_auto_saved, where))
                } else {
                    savedRecording = info
                }
            },
            onRecordingError = { msg -> container.toast(context.getString(R.string.rd_record_failed, msg)) },
            onVirtualAddFailed = { msg -> container.toast(context.getString(R.string.rd_vd_add_failed, msg)) },
            onVirtualRemoveFailed = { msg -> container.toast(context.getString(R.string.rd_vd_remove_failed, msg)) },
        ).also {
            it.setMouseKeys(prefs.remoteMouseKeys)
            it.setMouseKeysPos(prefs.mouseKeysX, prefs.mouseKeysY)
            it.start()
        }
    }
    val ui by controller.ui.collectAsState()
    val scope = rememberCoroutineScope()
    val sentText = stringResource(R.string.rd_clipboard_sent)
    val gotText = stringResource(R.string.rd_clipboard_received)
    val emptyText = stringResource(R.string.rd_clipboard_empty)
    val privacyOnText = stringResource(R.string.rd_privacy_on_toast)
    val privacyOffText = stringResource(R.string.rd_privacy_off_toast)

    // 默认横屏，菜单里可切到竖屏（跟随重力在两个横向 / 两个竖向之间转）
    var portrait by rememberSaveable { mutableStateOf(false) }
    // 全屏沉浸，离开时恢复
    DisposableEffect(Unit) {
        val activity = context.findActivity()
        val oldOrientation = activity?.requestedOrientation
        if (!portrait) activity?.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
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
    // 在记下原方向之后再改
    LaunchedEffect(portrait) {
        context.findActivity()?.requestedOrientation =
            if (portrait) ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT else ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
    }
    // 声音只在前台播放：切后台停掉，回来按开关重新开始
    LifecycleStartEffect(controller) {
        controller.setForeground(true)
        onStopOrDispose { controller.setForeground(false) }
    }

    RemoteDesktopScreen(
        ui = ui,
        actions = object : RemoteActions {
            override fun onInput(events: List<InputEvent>) = controller.sendAll(events)
            override fun onDisplay(id: UInt) = controller.switchDisplay(id)
            override fun onQuality(q: VideoQuality) = controller.setQuality(q)
            override fun onMouseKeys(on: Boolean) {
                container.settings.update { it.copy(remoteMouseKeys = on) }
                controller.setMouseKeys(on)
            }
            override fun onMouseKeysMoved(x: Float, y: Float) {
                container.settings.update { it.copy(mouseKeysX = x, mouseKeysY = y) }
                controller.setMouseKeysPos(x, y)
            }
            override fun onMouseMode(on: Boolean) {
                // 记住选择，下次打开远程桌面沿用
                container.settings.update { it.copy(remoteMouseMode = on) }
                controller.setMouseMode(on)
            }
            override fun onPrivacy(on: Boolean) {
                scope.launch {
                    try {
                        controller.setPrivacy(on)
                        container.toast(if (on) privacyOnText else privacyOffText)
                    } catch (e: Exception) {
                        container.toast(context.getString(R.string.rd_privacy_failed, e.userMessage()))
                    }
                }
            }
            override fun onSound(on: Boolean) {
                container.settings.update { it.copy(remoteAudio = on) }
                controller.setAudio(on)
            }
            override fun onRecord() = controller.toggleRecording()
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

            override fun onFiles() {
                nav.push(Route.Files(TargetRef.Device(deviceId)))
            }

            override fun onRotate() {
                portrait = !portrait
            }

            override fun onAddVirtualDisplay(width: Int, height: Int, refresh: Int) = controller.addVirtualDisplay(width, height, refresh)

            override fun onRemoveVirtualDisplay(id: UInt) = controller.removeVirtualDisplay(id)
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
                // 缓冲区固定为画面分辨率：放大时只缩放显示，不跟着视图尺寸重新分配（放大 5 倍时视图远大于屏幕）
                update = { v -> if (ui.videoW > 0 && ui.videoH > 0) v.holder.setFixedSize(ui.videoW, ui.videoH) },
                modifier = modifier,
            )
        },
    )
    savedRecording?.let { info ->
        val shareTitle = stringResource(R.string.rd_record_share)
        val galleryDone = stringResource(R.string.rd_record_saved_gallery)
        RecordingSavedDialog(
            info = info,
            location = info.paths.joinToString("\n") { Recordings.displayPath(it) },
            canSaveToGallery = Recordings.canSaveToGallery,
            onShare = {
                runCatching { Recordings.share(context, info.paths, shareTitle) }.onFailure { e -> container.toast(e.userMessage()) }
            },
            onSaveToGallery = {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    scope.launch {
                        try {
                            Recordings.saveToGallery(context, info.paths)
                            container.toast(galleryDone)
                            savedRecording = null
                        } catch (e: Exception) {
                            container.toast(context.getString(R.string.rd_record_gallery_failed, e.userMessage()))
                        }
                    }
                }
            },
            onDismiss = { savedRecording = null },
        )
    }
}

// ---------------- AI ----------------

/**
 * AI 对话页（docs/ai-chat-ui.md）。[conversationId] 为空是还没发消息的新对话：发第一条消息时才创建对话。
 * [initialTarget] 从设备与主机页点「AI」进来时预先放入的引用。
 */
@Composable
fun ChatRoute(conversationId: String?, initialTarget: TargetRef?, container: AppContainer, nav: Navigator, showBack: Boolean) {
    val agents = container.agents
    val modelsState by agents.state.collectAsState()
    val user by container.account.user.collectAsState()
    val prefs by container.settings.prefs.collectAsState()
    val data by container.hosts.data.collectAsState()
    val scope = rememberCoroutineScope()
    val wide = isWideLayout()
    // 进入 AI 页面就调用 ai_models()
    LaunchedEffect(Unit) { agents.loadModels() }
    val availability = aiAvailability(modelsState, user?.aiEnabled)
    val models = (modelsState as? AiModelsState.Loaded)?.models
    val modelItems = models?.items.orEmpty()
    val maxMode = models?.maxMode ?: AiMode.FULL_TRUST

    // 已打开的直接用；历史对话要向内核重新打开（异步）
    val controller by androidx.compose.runtime.produceState(conversationId?.let { agents.opened(it) }, conversationId) {
        if (conversationId != null && value == null) {
            value = try {
                agents.open(conversationId)
            } catch (e: Exception) {
                container.toast(e.userMessage())
                null
            }
            if (value == null) nav.pop()
        }
    }
    if (conversationId != null && controller == null) {
        androidx.compose.foundation.layout.Box(androidx.compose.ui.Modifier.fillMaxSize(), contentAlignment = androidx.compose.ui.Alignment.Center) {
            androidx.compose.material3.CircularProgressIndicator()
        }
        return
    }
    val c = controller
    val ui = c?.ui?.collectAsState()?.value
    val composer = remember(conversationId) {
        ComposerState(refs = initialTarget?.let { listOf(targetRef(data, it)) }.orEmpty())
    }
    val settings = if (ui == null) {
        ChatSettings(
            modelItems,
            pickDefaultModel(modelItems, prefs.aiProviderId, prefs.aiModelId),
            capMode(prefs.defaultAiMode, maxMode),
            maxMode,
            prefs.aiPlanFirst,
        )
    } else {
        ChatSettings(
            modelItems,
            modelItems.firstOrNull { it.providerId == ui.providerId && it.id == ui.modelId }
                ?: cn.lemonit.galaxy.core.bridge.AiModel(ui.providerId, "", ui.modelId, ui.modelId, false, false, 0u),
            ui.mode,
            maxMode,
            ui.planFirst,
        )
    }
    val sources = remember(data) {
        RefSources(
            targets = mentionCandidates(data),
            snippets = data.snippets.mapNotNull { e ->
                (e.data as? VaultData.Snippet)?.let { MentionCandidate(Refs.snippet(e.id, it.label), null, true, it.command) }
            },
            forwards = data.vault.mapNotNull { e ->
                (e.data as? VaultData.PortForward)?.let { f ->
                    val via = f.viaDeviceId?.let { data.deviceName(it) } ?: f.viaSshHostId?.let { (data.entry(it)?.data as? VaultData.SshHost)?.label }.orEmpty()
                    MentionCandidate(Refs.portForward(e.id, f.label), null, true, "${f.localPort} → ${f.remoteHost}:${f.remotePort}" + if (via.isNotEmpty()) "  ·  $via" else "")
                }
            },
            placeName = { Refs.placeName(data, it) },
            browser = { t -> FileBrowserController(t, container.connections, scope, container::toast) },
        )
    }
    var deleting by remember { mutableStateOf(false) }
    ChatScreen(
        conversationId = conversationId,
        title = ui?.title.orEmpty(),
        availability = availability,
        loadError = (modelsState as? AiModelsState.Failed)?.message,
        settings = settings,
        items = ui?.items.orEmpty(),
        running = ui?.running == true,
        composer = composer,
        sources = sources,
        showBack = showBack || !wide,
        actions = object : ChatActions {
            override fun onBack() {
                nav.pop()
            }

            override fun onNewChat() {
                if (wide && nav.tab == Tab.AI) nav.selectTab(Tab.AI) else nav.replace(Route.NewConversation(null))
            }

            override fun onDelete() {
                deleting = true
            }

            override fun onRetry() = agents.loadModels()

            override fun onSend() {
                val text = composer.value.text
                val refs = composer.refs
                if (!canSend(text, refs)) return
                if (c != null) {
                    c.send(text, refs)
                    composer.clear()
                    return
                }
                val m = settings.model ?: return
                if (composer.busy) return
                composer.busy = true
                composer.error = null
                scope.launch {
                    try {
                        val created = agents.create(
                            ConversationConfig(
                                m.providerId, m.id, emptyList(), settings.mode, settings.planFirst,
                                trustMinutes = if (settings.mode == AiMode.FULL_TRUST) prefs.aiTrustMinutes.takeIf { it > 0 }?.toUInt() else null,
                                title = null,
                            ),
                        )
                        created.send(text, refs)
                        composer.clear()
                        val route = Route.Conversation(created.ui.value.id)
                        if (nav.top is Route.NewConversation) nav.replace(route) else nav.push(route)
                    } catch (e: Exception) {
                        // 创建失败：消息留在输入框里，错误显示在输入框上方
                        composer.error = e.userMessage()
                    }
                    composer.busy = false
                }
            }

            override fun onQuickReply(text: String) {
                c?.send(text)
            }

            override fun onStop() {
                c?.stop()
            }

            override fun onModel(model: AiModel) {
                container.settings.update { it.copy(aiProviderId = model.providerId.toLong(), aiModelId = model.id) }
                c?.setModel(model.providerId, model.id)?.let(container::toast)
            }

            override fun onMode(mode: AiMode, trustMinutes: UInt?) {
                container.settings.update { it.copy(defaultAiMode = mode, aiTrustMinutes = trustMinutes?.toInt() ?: -1) }
                c?.setMode(mode, trustMinutes)
            }

            override fun onPlanFirst(on: Boolean) {
                if (c == null) container.settings.update { it.copy(aiPlanFirst = on) }
            }

            override fun onDecide(callId: String, decision: ApprovalDecision) {
                c?.decide(callId, decision)
            }

            override fun onAnswer(questionId: String, answer: String) {
                c?.answer(questionId, answer)
            }

            override fun onSecret(requestId: String, secret: CharArray) {
                c?.provideSecret(requestId, secret)
            }
        },
    )
    if (deleting && conversationId != null) {
        ConfirmDialog(
            title = stringResource(R.string.ai_delete),
            message = stringResource(R.string.ai_delete_message),
            confirmText = stringResource(R.string.delete),
            destructive = true,
            onConfirm = {
                agents.delete(conversationId)
                nav.pop()
            },
            onDismiss = { deleting = false },
        )
    }
}

/** 设备或主机 → 引用标签。 */
private fun targetRef(data: cn.lemonit.galaxy.app.HostsData, t: TargetRef) = when (t) {
    is TargetRef.Device -> Refs.device(t.deviceId, data.deviceName(t.deviceId))
    is TargetRef.Ssh -> Refs.sshHost(t.hostId, data.targetName(t))
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
