package cn.lemonit.galaxy.screenshots

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import cn.lemonit.galaxy.ui.settings.UpdateDialog
import cn.lemonit.galaxy.ui.settings.UpdateActions
import cn.lemonit.galaxy.ui.hosts.RdpForwardBanner
import cn.lemonit.galaxy.app.UpdateState
import cn.lemonit.galaxy.app.RdpForward
import android.content.Context
import android.graphics.BitmapFactory
import android.os.Looper
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import cn.lemonit.galaxy.app.AppContainer
import cn.lemonit.galaxy.app.ChatItem
import cn.lemonit.galaxy.app.EntryKind
import cn.lemonit.galaxy.app.HostItem
import cn.lemonit.galaxy.app.HostTree
import cn.lemonit.galaxy.app.LoginStep
import cn.lemonit.galaxy.app.Navigator
import cn.lemonit.galaxy.app.Route
import cn.lemonit.galaxy.app.Tab
import cn.lemonit.galaxy.app.TargetRef
import cn.lemonit.galaxy.core.CoreProvider
import cn.lemonit.galaxy.core.bridge.AgentTarget
import cn.lemonit.galaxy.core.bridge.AgentTargetKind
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.ApprovalDecision
import cn.lemonit.galaxy.core.bridge.ConversationConfig
import cn.lemonit.galaxy.core.bridge.FileEntry
import cn.lemonit.galaxy.core.bridge.InputEvent
import cn.lemonit.galaxy.core.bridge.VaultData
import cn.lemonit.galaxy.core.mock.MockCore
import cn.lemonit.galaxy.core.mock.MockData
import cn.lemonit.galaxy.core.platform.MemorySecureStore
import cn.lemonit.galaxy.data.AppLanguage
import cn.lemonit.galaxy.data.AppSettings
import cn.lemonit.galaxy.data.Prefs
import cn.lemonit.galaxy.data.VideoQuality
import cn.lemonit.galaxy.ui.ai.ConversationActions
import cn.lemonit.galaxy.ui.ai.ConversationListScreen
import cn.lemonit.galaxy.ui.ai.ConversationScreen
import cn.lemonit.galaxy.ui.ai.NewConversationDraft
import cn.lemonit.galaxy.ui.ai.NewConversationScreen
import cn.lemonit.galaxy.ui.ai.key
import cn.lemonit.galaxy.ui.app.MainRoute
import cn.lemonit.galaxy.ui.components.Clock
import cn.lemonit.galaxy.ui.files.FileBrowserActions
import cn.lemonit.galaxy.ui.files.FileBrowserScreen
import cn.lemonit.galaxy.ui.files.FileMenuContent
import cn.lemonit.galaxy.ui.files.FilesHomeScreen
import cn.lemonit.galaxy.ui.files.SortKey
import cn.lemonit.galaxy.ui.files.TransferActions
import cn.lemonit.galaxy.ui.hosts.AddKind
import cn.lemonit.galaxy.ui.hosts.EntryEditorScreen
import cn.lemonit.galaxy.ui.hosts.FolderAction
import cn.lemonit.galaxy.ui.hosts.HostActionsContent
import cn.lemonit.galaxy.ui.hosts.HostKeyDialog
import cn.lemonit.galaxy.ui.hosts.HostsActions
import cn.lemonit.galaxy.ui.hosts.HostsFilter
import cn.lemonit.galaxy.ui.hosts.HostsScreen
import cn.lemonit.galaxy.ui.hosts.HostsSubTab
import cn.lemonit.galaxy.ui.hosts.sshViaText
import cn.lemonit.galaxy.ui.login.LoginScreen
import cn.lemonit.galaxy.ui.remote.RemoteActions
import cn.lemonit.galaxy.ui.remote.RemoteDesktopScreen
import cn.lemonit.galaxy.ui.remote.RemotePanel
import cn.lemonit.galaxy.ui.remote.RemoteViewState
import cn.lemonit.galaxy.core.bridge.DisplayInfo
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.pinch
import cn.lemonit.galaxy.ui.remote.RecordingSavedDialog
import cn.lemonit.galaxy.core.bridge.RecordingInfo
import cn.lemonit.galaxy.ui.settings.AboutInfo
import cn.lemonit.galaxy.ui.settings.SettingsActions
import cn.lemonit.galaxy.ui.settings.SettingsScreen
import cn.lemonit.galaxy.ui.terminal.TerminalActions
import cn.lemonit.galaxy.ui.terminal.TerminalScreen
import cn.lemonit.galaxy.ui.terminal.TerminalTabUi
import cn.lemonit.galaxy.ui.theme.GalaxyTheme
import cn.lemonit.galaxy.ui.theme.TerminalThemes
import cn.lemonit.galaxy.ui.theme.ThemeMode
import com.github.takahirom.roborazzi.ExperimentalRoborazziApi
import com.github.takahirom.roborazzi.captureRoboImage
import com.github.takahirom.roborazzi.captureScreenRoboImage
import com.termux.view.TerminalView
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

private const val PHONE_ZH = "zh-rCN-w400dp-h860dp-port-xhdpi"
private const val PHONE_EN = "en-rUS-w400dp-h860dp-port-xhdpi"
private const val PHONE_LONG_ZH = "zh-rCN-w400dp-h1900dp-port-xhdpi"
private const val PHONE_LONG_EN = "en-rUS-w400dp-h1900dp-port-xhdpi"
private const val PHONE_LAND_ZH = "zh-rCN-w860dp-h400dp-land-xhdpi"
private const val PHONE_LAND_EN = "en-rUS-w860dp-h400dp-land-xhdpi"
private const val TABLET_ZH = "zh-rCN-w1280dp-h800dp-land-mdpi"
private const val TABLET_EN = "en-rUS-w1280dp-h800dp-land-mdpi"

/**
 * 主要页面截图（JVM 上用 Robolectric 渲染 Compose，Roborazzi 存 PNG，只生成不比对）。
 * 运行：./gradlew :app:testDebugUnitTest --tests '*ScreenshotTest*'
 * 产物：app/build/outputs/roborazzi/<页面>_<phone|land|tablet>_<zh|en>_<light|dark>.png
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = PHONE_ZH)
class ScreenshotTest {
    @get:Rule
    val compose = createComposeRule()

    private val context: Context get() = ApplicationProvider.getApplicationContext()
    private val zh: Boolean get() = context.resources.configuration.locales[0].language == "zh"
    private var container: AppContainer? = null

    @Before
    fun setUp() {
        Clock.fixedNow = MockData.NOW
    }

    @After
    fun tearDown() {
        container?.connections?.disconnectAll()
        Clock.fixedNow = null
    }

    private fun size(): String {
        val c = context.resources.configuration
        return when {
            c.screenWidthDp >= 1000 -> "tablet"
            c.screenWidthDp > c.screenHeightDp -> "land"
            else -> "phone"
        }
    }

    private fun shot(name: String, dark: Boolean, content: @Composable () -> Unit) {
        compose.mainClock.autoAdvance = false
        compose.setContent {
            GalaxyTheme(mode = if (dark) ThemeMode.DARK else ThemeMode.LIGHT) {
                Surface(color = MaterialTheme.colorScheme.background, modifier = Modifier.fillMaxSize()) { content() }
            }
        }
        compose.mainClock.advanceTimeBy(800)
        capture(name, dark)
    }

    private fun fileFor(name: String, dark: Boolean) = "build/outputs/roborazzi/${name}_${size()}_${if (zh) "zh" else "en"}_${if (dark) "dark" else "light"}.png"

    private fun capture(name: String, dark: Boolean) {
        compose.onRoot().captureRoboImage(fileFor(name, dark))
    }

    /** 弹框在单独的窗口里：截整个屏幕（含所有窗口）。 */
    @OptIn(ExperimentalRoborazziApi::class)
    private fun shotScreen(name: String, dark: Boolean, content: @Composable () -> Unit) {
        compose.mainClock.autoAdvance = false
        compose.setContent {
            GalaxyTheme(mode = if (dark) ThemeMode.DARK else ThemeMode.LIGHT) {
                Surface(color = MaterialTheme.colorScheme.background, modifier = Modifier.fillMaxSize()) { content() }
            }
        }
        compose.mainClock.advanceTimeBy(800)
        captureScreenRoboImage(fileFor(name, dark))
    }

    /** 处理主线程消息并等真实时间（模拟内核在后台线程按真实时间推进）。 */
    private fun pump(ms: Long, until: () -> Boolean = { false }) {
        val end = System.currentTimeMillis() + ms
        while (System.currentTimeMillis() < end) {
            // 主线程上的 delay() 走 Robolectric 的时钟：跟着真实时间推进
            shadowOf(Looper.getMainLooper()).idleFor(java.time.Duration.ofMillis(30))
            if (until()) break
            Thread.sleep(30)
        }
        shadowOf(Looper.getMainLooper()).idle()
    }

    // ---------------- 登录 ----------------

    private fun login(step: LoginStep, dark: Boolean, error: String? = null) = shot("login_${step.name.lowercase()}", dark) {
        LoginScreen(Fixtures.login(step, error), isMock = true, onEdit = {}, onSubmitCredentials = {}, onSubmitTotp = {}, onSubmitNewPassword = {}, onBack = {}, onScan = {})
    }

    @Test fun loginZhLight() = login(LoginStep.CREDENTIALS, false)

    @Test @Config(qualifiers = PHONE_EN)
    fun loginEnDark() = login(LoginStep.CREDENTIALS, true)

    @Test fun loginTotpZhDark() = login(LoginStep.TOTP, true)

    @Test @Config(qualifiers = PHONE_EN)
    fun loginChangePasswordEnLight() = login(LoginStep.CHANGE_PASSWORD, false, "Incorrect username or password")

    @Test @Config(qualifiers = TABLET_EN)
    fun loginTabletEnLight() = login(LoginStep.CREDENTIALS, false)

    // ---------------- 设备与主机 ----------------

    private val noHostsActions = object : HostsActions {
        override fun onFilter(f: HostsFilter) = Unit
        override fun onRefresh() = Unit
        override fun onItem(item: HostItem) = Unit
        override fun onFolderToggle(folder: cn.lemonit.galaxy.core.bridge.VaultEntry, expanded: Boolean) = Unit
        override fun onFolderMenu(folder: cn.lemonit.galaxy.core.bridge.VaultEntry, action: FolderAction) = Unit
        override fun onAdd(kind: AddKind) = Unit
        override fun onCredential(entry: cn.lemonit.galaxy.core.bridge.VaultEntry) = Unit
        override fun onSnippet(entry: cn.lemonit.galaxy.core.bridge.VaultEntry) = Unit
    }

    private fun hosts(dark: Boolean, filter: HostsFilter = HostsFilter(), name: String = "hosts") = shot(name, dark) {
        HostsScreen(Fixtures.hosts, filter, refreshing = false, actions = noHostsActions)
    }

    @Test fun hostsZhDark() = hosts(true)

    @Test @Config(qualifiers = PHONE_EN)
    fun hostsEnLight() = hosts(false)

    @Test fun hostsSearchZhLight() = hosts(false, HostsFilter(query = "prod"), "hosts_search")

    @Test @Config(qualifiers = PHONE_EN)
    fun hostsSnippetsEnDark() = hosts(true, HostsFilter(subTab = HostsSubTab.SNIPPETS), "hosts_snippets")

    private fun hostSheet(dark: Boolean, deviceId: String, name: String) = shot(name, dark) {
        val item = HostTree.items(Fixtures.hosts).first { it is HostItem.Device && it.device.id == deviceId }
        Surface(color = MaterialTheme.colorScheme.surfaceContainerLow) {
            HostActionsContent(item, onAction = {}, canWake = Fixtures.hosts.canWake(deviceId))
        }
    }

    /** 离线、能远程开机的设备：面板里有「远程开机」与 i 说明 */
    @Test fun hostSheetWakeZhLight() = hostSheet(false, MockData.DEV_RENDER, "host_sheet_wake")

    @Test @Config(qualifiers = PHONE_EN)
    fun hostSheetWakeEnDark() = hostSheet(true, MockData.DEV_RENDER, "host_sheet_wake")

    @Test fun hostSheetZhLight() = hostSheet(false, MockData.DEV_OFFICE_PC, "host_sheet")

    @Test @Config(qualifiers = PHONE_EN)
    fun hostSheetEnDark() = hostSheet(true, MockData.DEV_WEB_01, "host_sheet")

    private fun sshSheet(dark: Boolean) = shot("host_sheet_ssh", dark) {
        val item = HostTree.items(Fixtures.hosts).first { it is HostItem.Ssh && it.entry.id == MockData.SSH_GITLAB } as HostItem.Ssh
        Surface(color = MaterialTheme.colorScheme.surfaceContainerLow) {
            HostActionsContent(item, onAction = {}, sshVia = sshViaText(Fixtures.hosts, item.host, null))
        }
    }

    @Test fun hostSheetSshZhLight() = sshSheet(false)

    @Test @Config(qualifiers = PHONE_EN)
    fun hostSheetSshEnDark() = sshSheet(true)

    private fun hostKey(dark: Boolean, changed: Boolean) = shotScreen(if (changed) "ssh_host_key_changed" else "ssh_host_key", dark) {
        HostKeyDialog("10.0.2.15", 22, "ssh-ed25519", "SHA256:3EDwKv5fv67WpFIHRKfVmkbbTSnNckJjoNmrY5SyKAM", changed, onTrust = {}, onReject = {})
    }

    /** 经设备转发中的 RDP：主界面底部提示条 */
    private fun rdpBanner(dark: Boolean) = shot("rdp_forward", dark) {
        Column(Modifier.fillMaxSize()) {
            Box(Modifier.weight(1f)) { HostsScreen(Fixtures.hosts, HostsFilter(), refreshing = false, actions = noHostsActions) }
            RdpForwardBanner(
                listOf(RdpForward("f1", MockData.RDP_FINANCE, "Finance PC", 53389), RdpForward("f2", "r-x", "WIN-SRV-2022", 53390)),
                onStop = {}, onStopAll = {},
            )
        }
    }

    @Test fun rdpForwardZhLight() = rdpBanner(false)

    @Test @Config(qualifiers = PHONE_EN)
    fun rdpForwardEnDark() = rdpBanner(true)

    private val noUpdateActions = object : UpdateActions {
        override fun onDownload() = Unit
        override fun onInstall() = Unit
        override fun onDismiss() = Unit
    }

    private fun update(dark: Boolean, state: (cn.lemonit.galaxy.core.bridge.UpdateInfo) -> UpdateState, name: String) = shotScreen(name, dark) {
        UpdateDialog(state(MockData.update(zh)), "0.1.0", noUpdateActions)
    }

    @Test fun updateZhLight() = update(false, { UpdateState.Available(it) }, "update")

    @Test @Config(qualifiers = PHONE_EN)
    fun updateDownloadingEnDark() = update(true, { UpdateState.Downloading(it, it.size.toLong() * 37 / 100, it.size.toLong()) }, "update_downloading")

    @Test fun hostKeyZhLight() = hostKey(false, false)

    @Test fun hostKeyChangedZhDark() = hostKey(true, true)

    @Test @Config(qualifiers = PHONE_EN)
    fun hostKeyChangedEnLight() = hostKey(false, true)

    private fun editor(dark: Boolean) = shot("editor_ssh", dark) {
        val e = Fixtures.hosts.entry(MockData.SSH_GITLAB)
        EntryEditorScreen(EntryKind.SSH, e, null, e?.folderId, Fixtures.hosts, TerminalThemes.all(context), onSave = { _, _ -> }, onDelete = {}, onBack = {})
    }

    @Test fun editorZhDark() = editor(true)

    @Test @Config(qualifiers = PHONE_EN)
    fun editorEnLight() = editor(false)

    // ---------------- 终端 ----------------

    private val noTerminalActions = object : TerminalActions {
        override fun onBack() = Unit
        override fun onSelect(id: Long) = Unit
        override fun onCloseTab(id: Long, kill: Boolean) = Unit
        override fun onNewSession() = Unit
        override fun onShowSessions() = Unit
        override fun onReconnect(id: Long) = Unit
        override fun onToggleInputBox() = Unit
        override fun onSendLine(text: String) = Unit
        override fun onSnippet(snippet: VaultData.Snippet) = Unit
        override fun onFontScale(bigger: Boolean) = Unit
        override fun onVoice() = Unit
        override fun onViewBound(id: Long, view: TerminalView?) = Unit
    }

    private fun terminal(dark: Boolean, inputBox: Boolean, themeId: String) {
        val session = Fixtures.terminalSession(Fixtures.terminalSample(zh))
        val second = Fixtures.terminalSession("")
        val tabs = listOf(
            TerminalTabUi(1, "demo@web-01: ~", "web-01 (nginx)", Fixtures.tabStatus(), session, false),
            TerminalTabUi(2, "gitlab", "gitlab", cn.lemonit.galaxy.app.TabStatus.Detached, second, true),
        )
        shot(if (inputBox) "terminal_inputbox" else "terminal", dark) {
            TerminalScreen(
                tabs = tabs, activeId = 1, path = Fixtures.path, theme = TerminalThemes.find(context, themeId), fontSizeSp = 12,
                inputBox = inputBox, snippets = Fixtures.hosts.snippets.map { it.data as VaultData.Snippet }, voiceAvailable = true,
                actions = noTerminalActions,
            )
        }
    }

    @Test fun terminalZhDark() = terminal(true, false, "lemon-dark")

    @Test @Config(qualifiers = PHONE_EN)
    fun terminalEnLight() = terminal(false, true, "github-light")

    @Test @Config(qualifiers = PHONE_EN)
    fun terminalDraculaEnDark() = terminal(true, false, "dracula")

    // ---------------- 文件 ----------------

    private val noFileActions = object : FileBrowserActions {
        override fun onBack() = Unit
        override fun onOpen(entry: FileEntry) = Unit
        override fun onMenu(entry: FileEntry) = Unit
        override fun onUp() = Unit
        override fun onHome() = Unit
        override fun onRefresh() = Unit
        override fun onUpload() = Unit
        override fun onNewFolder() = Unit
        override fun onSort(key: SortKey) = Unit
        override fun onToggleHidden() = Unit
    }

    private val noTransferActions = object : TransferActions {
        override fun onPause(id: Long) = Unit
        override fun onResume(id: Long) = Unit
        override fun onCancel(id: Long) = Unit
        override fun onRetry(id: Long) = Unit
        override fun onClearFinished() = Unit
    }

    private fun files(dark: Boolean) = shot("files", dark) { FileBrowserScreen("web-01 (nginx)", Fixtures.files(zh), noFileActions) }

    @Test fun filesZhLight() = files(false)

    @Test @Config(qualifiers = PHONE_EN)
    fun filesEnDark() = files(true)

    private fun fileMenu(dark: Boolean) = shot("file_menu", dark) {
        val e = Fixtures.files(zh).entries.first { it.name == "deploy.sh" }
        Surface(color = MaterialTheme.colorScheme.surfaceContainerLow) { FileMenuContent(e, {}, {}, {}, {}, {}) }
    }

    @Test fun fileMenuZhDark() = fileMenu(true)

    private fun filesHome(dark: Boolean) = shot("files_home", dark) {
        val targets = HostTree.items(Fixtures.hosts).filter { (it is HostItem.Device && (it.device.online || it.device.lanSeen)) || it is HostItem.Ssh }
        FilesHomeScreen(targets, Fixtures.transfers(zh), onBrowse = {}, actions = noTransferActions)
    }

    @Test fun filesHomeZhDark() = filesHome(true)

    @Test @Config(qualifiers = PHONE_EN)
    fun filesHomeEnLight() = filesHome(false)

    // ---------------- 远程桌面 ----------------

    private val noRemoteActions = object : RemoteActions {
        override fun onInput(events: List<InputEvent>) = Unit
        override fun onDisplay(id: UInt) = Unit
        override fun onQuality(q: VideoQuality) = Unit
        override fun onMouseMode(on: Boolean) = Unit
        override fun onClipboardSend() = Unit
        override fun onClipboardGet() = Unit
        override fun onDisconnect() = Unit
    }

    private fun remote(
        dark: Boolean,
        name: String,
        menu: Boolean = false,
        panel: RemotePanel? = null,
        keyboard: Boolean = false,
        privacy: Boolean = false,
        soundOn: Boolean = true,
        recording: Boolean = false,
        displays: List<DisplayInfo>? = null,
        then: (RemoteViewState) -> Unit = {},
    ) {
        val bytes = context.assets.open("mock/desktop_preview.jpg").use { it.readBytes() }
        val bmp = BitmapFactory.decodeByteArray(bytes, 0, bytes.size).asImageBitmap()
        val view = RemoteViewState()
        val base = Fixtures.remote(zh)
            .let { if (privacy) it.copy(privacySupported = true, privacyAllowed = true, privacyOn = true) else it }
            .let { if (displays != null) it.copy(displays = displays) else it }
            .copy(audioOn = soundOn, recordingSince = if (recording) MockData.NOW - 83_000 else null)
        val ui = mutableStateOf(base)
        val actions = object : RemoteActions by noRemoteActions {
            override fun onDisplay(id: UInt) {
                ui.value = ui.value.copy(displayId = id)
            }
        }
        compose.mainClock.autoAdvance = false
        compose.setContent {
            GalaxyTheme(mode = if (dark) ThemeMode.DARK else ThemeMode.LIGHT) {
                Surface(color = MaterialTheme.colorScheme.background, modifier = Modifier.fillMaxSize()) {
                    RemoteDesktopScreen(
                        ui = ui.value,
                        actions = actions,
                        video = { m -> Image(bmp, null, contentScale = ContentScale.FillBounds, modifier = m) },
                        initialMenuOpen = menu,
                        initialPanel = panel,
                        initialKeyboard = keyboard,
                        state = view,
                    )
                }
            }
        }
        compose.mainClock.advanceTimeBy(800)
        // 手势要跟着时钟走：做手势时打开自动推进，做完再停住截图
        compose.mainClock.autoAdvance = true
        then(view)
        compose.waitForIdle()
        compose.mainClock.autoAdvance = false
        compose.mainClock.advanceTimeBy(400)
        capture(name, dark)
    }

    private val threeDisplays = listOf(
        DisplayInfo(1u, "Built-in Retina Display", 2560u, 1440u, 2.0f, true, false),
        DisplayInfo(2u, "DELL U2723QE", 2560u, 1440u, 1.0f, false, false),
        DisplayInfo(3u, "LG HDR 4K", 3840u, 2160u, 1.5f, false, false),
    )

    /** 收起：右边缘只有一个贴边把手（绿点 = 直连，黄点 = 中转；圆点 = 几块屏） */
    @Test @Config(qualifiers = PHONE_LAND_ZH)
    fun remoteHandleZhDark() = remote(true, "remote_handle", displays = threeDisplays)

    @Test @Config(qualifiers = PHONE_ZH)
    fun remoteHandlePortraitZhLight() = remote(false, "remote_handle", displays = threeDisplays)

    /** 展开：右侧竖排菜单（不滚动） */
    @Test @Config(qualifiers = PHONE_LAND_ZH)
    fun remoteMenuZhDark() = remote(true, "remote_menu", menu = true, displays = threeDisplays)

    @Test @Config(qualifiers = PHONE_LAND_EN)
    fun remoteMenuEnLight() = remote(false, "remote_menu", menu = true)

    @Test @Config(qualifiers = PHONE_ZH)
    fun remoteMenuPortraitZhDark() = remote(true, "remote_menu", menu = true, displays = threeDisplays)

    /** 二级面板：显示器 / 画质 / 视图 / 连接 */
    @Test @Config(qualifiers = PHONE_LAND_ZH)
    fun remotePanelDisplayZhDark() = remote(true, "remote_panel_display", panel = RemotePanel.DISPLAY, displays = threeDisplays)

    @Test @Config(qualifiers = PHONE_LAND_EN)
    fun remotePanelDisplayEnLight() = remote(false, "remote_panel_display", panel = RemotePanel.DISPLAY, displays = threeDisplays)

    @Test @Config(qualifiers = PHONE_ZH)
    fun remotePanelDisplayPortraitZhDark() = remote(true, "remote_panel_display", panel = RemotePanel.DISPLAY, displays = threeDisplays)

    /** 二级面板：快捷键 */
    @Test @Config(qualifiers = PHONE_LAND_ZH)
    fun remotePanelShortcutsZhDark() = remote(true, "remote_panel_shortcuts", panel = RemotePanel.SHORTCUTS)

    @Test @Config(qualifiers = PHONE_LAND_EN)
    fun remotePanelShortcutsEnDark() = remote(true, "remote_panel_shortcuts", panel = RemotePanel.SHORTCUTS)

    /** 二级面板：工具（声音开、隐私屏关） */
    @Test @Config(qualifiers = PHONE_LAND_ZH)
    fun remotePanelToolsZhDark() = remote(true, "remote_panel_tools", panel = RemotePanel.TOOLS)

    @Test @Config(qualifiers = PHONE_LAND_EN)
    fun remotePanelToolsEnLight() = remote(false, "remote_panel_tools", panel = RemotePanel.TOOLS)

    /** 隐私屏已开：方块高亮 */
    @Test @Config(qualifiers = PHONE_LAND_ZH)
    fun remotePrivacyZhDark() = remote(true, "remote_privacy", panel = RemotePanel.TOOLS, privacy = true)

    /** 声音已关：方块不高亮、图标为静音 */
    @Test @Config(qualifiers = PHONE_LAND_EN)
    fun remoteSoundOffEnLight() = remote(false, "remote_sound_off", panel = RemotePanel.TOOLS, soundOn = false)

    /** 录像中：工具里红色停止方块 + 计时，菜单“工具”项带红点 */
    @Test @Config(qualifiers = PHONE_LAND_ZH)
    fun remoteRecordingZhDark() = remote(true, "remote_recording", panel = RemotePanel.TOOLS, recording = true)

    /** 录像中、菜单收起：左上角红点 + 计时，把手上的点变红 */
    @Test @Config(qualifiers = PHONE_LAND_EN)
    fun remoteRecordingBadgeEnLight() = remote(false, "remote_recording_badge", recording = true)

    /** 在把手上上滑：切到下一块显示器，上方提示“显示器 2 / 3” */
    private fun displaySwitch(dark: Boolean) = remote(dark, "remote_display_switch", displays = threeDisplays) {
        compose.onNodeWithTag("remote-dock-handle").performTouchInput {
            down(center)
            repeat(8) { moveBy(Offset(0f, -12f)) }
            up()
        }
    }

    @Test @Config(qualifiers = PHONE_LAND_ZH)
    fun remoteDisplaySwitchZhDark() = displaySwitch(true)

    @Test @Config(qualifiers = PHONE_LAND_EN)
    fun remoteDisplaySwitchEnLight() = displaySwitch(false)

    /** 双指捏合放大：画面保持比例，超出部分裁掉，上方显示缩放倍数 */
    private fun zoomed(dark: Boolean) = remote(dark, "remote_zoomed") {
        compose.onNodeWithTag("remote-surface").performTouchInput {
            val c = Offset(width * 0.3f, height * 0.4f)
            pinch(c - Offset(40f, 0f), c - Offset(200f, 0f), c + Offset(40f, 0f), c + Offset(200f, 0f), 400)
        }
    }

    @Test @Config(qualifiers = PHONE_LAND_ZH)
    fun remoteZoomedZhDark() = zoomed(true)

    @Test @Config(qualifiers = PHONE_ZH)
    fun remoteZoomedPortraitZhLight() = zoomed(false)

    private fun recordingSaved(dark: Boolean) = shotScreen("remote_recording_saved", dark) {
        val path = "Android/data/cn.lemonit.galaxy/files/Movies/LemonGalaxy/LemonGalaxy-Office PC-20261121-213000.mp4"
        RecordingSavedDialog(
            RecordingInfo(listOf("/storage/emulated/0/$path"), 83_400u, 12_874_342u),
            location = path,
            canSaveToGallery = true,
            onShare = {}, onSaveToGallery = {}, onDismiss = {},
        )
    }

    @Test @Config(qualifiers = PHONE_LAND_ZH)
    fun remoteRecordingSavedZhLight() = recordingSaved(false)

    @Test @Config(qualifiers = PHONE_LAND_EN)
    fun remoteRecordingSavedEnDark() = recordingSaved(true)

    @Test @Config(qualifiers = PHONE_LAND_EN)
    fun remoteKeyboardEnLight() = remote(false, "remote_keyboard", keyboard = true)

    @Test @Config(qualifiers = TABLET_ZH)
    fun remoteTabletZhDark() = remote(true, "remote_panel_display", panel = RemotePanel.DISPLAY, displays = threeDisplays)

    // ---------------- AI ----------------

    private val noConversationActions = object : ConversationActions {
        override fun onBack() = Unit
        override fun onSend(text: String) = Unit
        override fun onStop() = Unit
        override fun onDecide(callId: String, decision: ApprovalDecision) = Unit
        override fun onAnswer(questionId: String, answer: String) = Unit
        override fun onSecret(requestId: String, secret: CharArray) = Unit
        override fun onMode(mode: AiMode, trustMinutes: UInt?) = Unit
        override fun onDelete() = Unit
    }

    private fun conversation(dark: Boolean, later: Boolean) {
        val ui = if (later) Fixtures.conversationLater(zh) else Fixtures.conversation(zh, Fixtures.preview(context))
        shot(if (later) "ai_conversation_later" else "ai_conversation", dark) {
            ConversationScreen(ui, noConversationActions)
        }
    }

    @Test fun aiConversationZhDark() = conversation(true, false)

    @Test @Config(qualifiers = PHONE_LONG_EN)
    fun aiConversationEnLight() = conversation(false, false)

    @Test @Config(qualifiers = PHONE_LONG_ZH)
    fun aiConversationLaterZhLight() = conversation(false, true)

    @Test @Config(qualifiers = PHONE_EN)
    fun aiConversationLaterEnDark() = conversation(true, true)

    private fun aiList(dark: Boolean) = shot("ai_list", dark) {
        ConversationListScreen(Fixtures.summaries(zh), enabled = true, onOpen = {}, onNew = {}, onDelete = {})
    }

    @Test fun aiListZhDark() = aiList(true)

    @Test @Config(qualifiers = PHONE_EN)
    fun aiListEnLight() = aiList(false)

    private fun aiNew(dark: Boolean) = shot("ai_new", dark) {
        val targets = HostTree.items(Fixtures.hosts).filter { (it is HostItem.Device && (it.device.online || it.device.lanSeen)) || it is HostItem.Ssh }
        val draft = NewConversationDraft(
            modelKey = Fixtures.models.items[1].key(),
            targets = setOf("device:${MockData.DEV_WEB_01}", "ssh:${MockData.SSH_GITLAB}"),
            mode = AiMode.FULL_TRUST,
            trustMinutes = 60u,
            planFirst = true,
        )
        NewConversationScreen(Fixtures.models, null, targets, draft, onDraft = {}, onStart = {}, onBack = {})
    }

    @Test @Config(qualifiers = PHONE_LONG_ZH)
    fun aiNewZhLight() = aiNew(false)

    @Test @Config(qualifiers = PHONE_EN)
    fun aiNewEnDark() = aiNew(true)

    // ---------------- 设置 ----------------

    private val noSettingsActions = object : SettingsActions {
        override fun onPrefs(block: (Prefs) -> Prefs) = Unit
        override fun onLanguage(lang: AppLanguage) = Unit
        override fun onChangePassword() = Unit
        override fun onTwoFactor() = Unit
        override fun onLogout() = Unit
        override fun onAppLock(enabled: Boolean) = Unit
        override fun onLicenses() = Unit
        override fun onSource() = Unit
    }

    private fun settings(dark: Boolean) = shot("settings", dark) {
        SettingsScreen(
            prefs = Prefs(themeMode = if (dark) ThemeMode.DARK else ThemeMode.LIGHT, terminalTheme = "nord", terminalFontSize = 14, appLock = true),
            language = if (zh) AppLanguage.ZH else AppLanguage.EN,
            user = Fixtures.user,
            server = Fixtures.server,
            totpEnabled = true,
            themes = TerminalThemes.all(context),
            about = AboutInfo("0.1.0", "0.1.0-mock", true),
            biometricAvailable = true,
            actions = noSettingsActions,
            update = UpdateState.Available(MockData.update(zh)),
        )
    }

    @Test @Config(qualifiers = PHONE_LONG_ZH)
    fun settingsZhLight() = settings(false)

    @Test @Config(qualifiers = PHONE_LONG_EN)
    fun settingsEnDark() = settings(true)

    // ---------------- 平板双栏（真实 AppContainer + 模拟内核） ----------------

    private fun appContainer(dark: Boolean): AppContainer {
        val core = MockCore { name -> context.assets.open(name).use { it.readBytes() } }
        core.coreInit(CoreProvider.configFor(context))
        CoreProvider.install(core)
        val c = AppContainer(context, core, MemorySecureStore(), AppSettings.inMemory(Prefs(themeMode = if (dark) ThemeMode.DARK else ThemeMode.LIGHT)))
        runBlocking { c.client.login(Fixtures.server.address, "alice", "password", null) }
        c.account.restore()
        c.hosts.reload()
        c.agents.loadModels()
        pump(500) { c.agents.models.value != null }
        container = c
        return c
    }

    private fun app(name: String, dark: Boolean, c: AppContainer, nav: Navigator, settle: Long = 600, until: () -> Boolean = { false }) {
        compose.mainClock.autoAdvance = false
        compose.setContent {
            GalaxyTheme(mode = if (dark) ThemeMode.DARK else ThemeMode.LIGHT) {
                Surface(color = MaterialTheme.colorScheme.background, modifier = Modifier.fillMaxSize()) {
                    MainRoute(c, remember { nav })
                }
            }
        }
        compose.mainClock.advanceTimeBy(300)
        val end = System.currentTimeMillis() + settle
        while (System.currentTimeMillis() < end) {
            pump(60)
            compose.mainClock.advanceTimeBy(100)
            if (until()) break
        }
        compose.mainClock.advanceTimeBy(800)
        capture(name, dark)
    }

    @Test @Config(qualifiers = TABLET_ZH)
    fun tabletHostsTerminalZhDark() {
        val c = appContainer(true)
        val target = TargetRef.Device(MockData.DEV_WEB_01)
        val tab = c.terminals.open(target)
        val nav = Navigator(Tab.HOSTS).apply { push(Route.Terminal(target)) }
        app("app_hosts_terminal", true, c, nav, settle = 2500) { tab.status.value == cn.lemonit.galaxy.app.TabStatus.Connected }
        tab.sendText("ls -la\r")
        pump(800)
        tab.sendText("df -h\r")
        pump(800)
        compose.mainClock.advanceTimeBy(500)
        capture("app_hosts_terminal", true)
    }

    @Test @Config(qualifiers = TABLET_EN)
    fun tabletAiEnLight() {
        val c = appContainer(false)
        val m = MockData.models().first()
        val conv = runBlocking {
            c.agents.create(
                ConversationConfig(m.providerId, m.id, listOf(AgentTarget(AgentTargetKind.DEVICE, MockData.DEV_WEB_01)), AiMode.CONFIRM_DANGEROUS, false, null, null),
                m.name, m.vision, listOf("web-01 (nginx)"),
            )
        }
        conv.send("web-01 is almost out of disk, please clean up")
        pump(15_000) { conv.ui.value.items.any { it is ChatItem.Tool && it.phase == cn.lemonit.galaxy.app.ToolPhase.AWAITING } }
        c.agents.refreshList()
        val nav = Navigator(Tab.AI).apply { push(Route.Conversation(conv.ui.value.id)) }
        app("app_ai", false, c, nav)
    }

    @Test @Config(qualifiers = TABLET_ZH)
    fun tabletFilesZhLight() {
        val c = appContainer(false)
        c.transfers.seed(Fixtures.transfers(true))
        val nav = Navigator(Tab.FILES).apply { push(Route.Files(TargetRef.Device(MockData.DEV_BUILD_MAC))) }
        app("app_files", false, c, nav, settle = 1500)
    }

    @Test @Config(qualifiers = TABLET_EN)
    fun tabletSettingsEnDark() {
        val c = appContainer(true)
        val nav = Navigator(Tab.SETTINGS)
        app("app_settings", true, c, nav, settle = 600)
    }

    @Test @Config(qualifiers = TABLET_EN)
    fun tabletHostsEmptyDetailEnLight() {
        val c = appContainer(false)
        val nav = Navigator(Tab.HOSTS)
        app("app_hosts", false, c, nav, settle = 600)
    }
}
