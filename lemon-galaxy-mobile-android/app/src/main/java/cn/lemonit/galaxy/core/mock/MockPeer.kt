package cn.lemonit.galaxy.core.mock

import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.AudioPlaybackInterface
import cn.lemonit.galaxy.core.bridge.ClipboardData
import cn.lemonit.galaxy.core.bridge.DisplayChannelInterface
import cn.lemonit.galaxy.core.bridge.DisplayInfo
import cn.lemonit.galaxy.core.bridge.DisplayOptions
import cn.lemonit.galaxy.core.bridge.ExecRequest
import cn.lemonit.galaxy.core.bridge.ExecResult
import cn.lemonit.galaxy.core.bridge.FileClientInterface
import cn.lemonit.galaxy.core.bridge.GalaxyException
import cn.lemonit.galaxy.core.bridge.ImageData
import cn.lemonit.galaxy.core.bridge.PathInfo
import cn.lemonit.galaxy.core.bridge.PathKind
import cn.lemonit.galaxy.core.bridge.PeerInfo
import cn.lemonit.galaxy.core.bridge.PeerListener
import cn.lemonit.galaxy.core.bridge.PeerSessionInterface
import cn.lemonit.galaxy.core.bridge.Permissions
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.PortForwardInterface
import cn.lemonit.galaxy.core.bridge.SshInfo
import cn.lemonit.galaxy.core.bridge.SshSessionInterface
import cn.lemonit.galaxy.core.bridge.TerminalChannelInterface
import cn.lemonit.galaxy.core.bridge.TerminalOptions
import cn.lemonit.galaxy.core.bridge.TerminalSessionInfo
import cn.lemonit.galaxy.core.bridge.TerminalSink
import cn.lemonit.galaxy.core.bridge.VideoSink
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import kotlin.random.Random

/** 一台模拟被控端的常驻状态（终端会话、文件树、剪贴板），跨多次连接保留。 */
internal class MockHost(
    val id: String,
    val hostname: String,
    val platform: Platform,
    val user: String,
    val scope: CoroutineScope,
    val tr: (String, String) -> String,
) {
    val fs = MockFs(if (platform == Platform.MACOS || platform == Platform.WINDOWS) "/Users/$user" else "/home/$user", user, tr)
    val shells = ConcurrentHashMap<String, MockShell>()
    private val seq = AtomicInteger(1)
    var clipboard: ClipboardData = ClipboardData.Text(tr("来自远端的剪贴板文字", "Clipboard text from the remote"))

    fun newShell(): MockShell {
        val id = "${hostname.lowercase()}-${seq.getAndIncrement()}"
        val s = MockShell(id, hostname, user, platform, fs, scope, tr) { shells.remove(it.id) }
        shells[id] = s
        return s
    }
}

internal class MockPeerSession(
    private val host: MockHost,
    private val core: MockCore,
    private val osVersion: String,
    private val arch: String,
) : PeerSessionInterface {
    private val scope = host.scope
    private val basePath = MockData.pathFor(host.id)
    @Volatile private var currentPath = PathInfo(basePath.first, basePath.second.toUInt(), remoteAddr(basePath.first))
    @Volatile private var listener: PeerListener? = null
    @Volatile private var closed = false
    private val ticker: Job

    /** 本会话加的虚拟屏：属于会话，断开时被控端自动删掉（这里随会话对象一起清空）。 */
    private val virtualDisplays = mutableListOf<DisplayInfo>()
    private val nextVirtualId = AtomicInteger(VIRTUAL_ID_BASE + 1)
    @Volatile private var lastAddedId: UInt? = null
    private val channels = java.util.concurrent.CopyOnWriteArrayList<MockDisplayChannel>()

    /** 测试用：列屏结果暂时还没有刚加的那块（被控端列表滞后）。 */
    @Volatile var listLagsBehindAdd = false

    /** 测试用：被控端那边虚拟屏都没了（如 Windows 换控制台会话），开着的画面随之被关。 */
    fun dropVirtualDisplays() {
        val gone = synchronized(virtualDisplays) { virtualDisplays.map { it.id }.also { virtualDisplays.clear() } }
        channels.filter { it.displayId in gone }.forEach { it.closeByRemote("display removed") }
    }

    /** 测试用：开着的画面都被被控端结束（或画面没有响应），会话还在。 */
    fun endDisplayStreams(reason: String) = channels.forEach { it.closeByRemote(reason) }

    init {
        // 每 2 秒抖动一下延迟，模拟后台持续探测
        ticker = scope.launch {
            val r = Random(host.id.hashCode())
            while (isActive && !closed) {
                delay(2000)
                val rtt = (basePath.second + r.nextInt(-2, 6)).coerceAtLeast(1)
                currentPath = currentPath.copy(rttMs = rtt.toUInt())
                listener?.onPathChanged(currentPath)
            }
        }
    }

    private fun remoteAddr(kind: PathKind) = when (kind) {
        PathKind.LAN -> "192.168.1.${20 + (host.id.hashCode() and 0x3F)}:51820"
        PathKind.DIRECT -> "[240e:3b0:1c::${(host.id.hashCode() and 0xFFF).toString(16)}]:51820"
        PathKind.RELAY_UDP -> "galaxy.example.com:7443 (UDP)"
        PathKind.RELAY_TCP -> "galaxy.example.com:7443 (TCP)"
        PathKind.CONNECTING, PathKind.CLOSED -> ""
    }

    private val hasGui get() = host.platform == Platform.MACOS || host.platform == Platform.WINDOWS

    override fun info(): PeerInfo = PeerInfo(
        deviceId = host.id,
        hostname = host.hostname,
        platform = host.platform,
        osVersion = osVersion,
        arch = arch,
        systemUser = host.user,
        shell = if (host.platform == Platform.WINDOWS) "powershell" else "/bin/bash",
        permissions = Permissions(view = hasGui, control = hasGui, files = true, terminal = true, clipboard = true, audio = hasGui, aiCap = AiMode.FULL_TRUST),
        capabilities = buildList {
            addAll(listOf("terminal", "files", "exec", "clipboard", "port_forward"))
            if (hasGui) addAll(listOf("display", "input", "screenshot", "ui_tree", "privacy", "audio"))
            if (hasGui) addAll(listOf("virtual_display", "virtual_display_max=$VIRTUAL_DISPLAY_MAX"))
        },
        path = currentPath,
    )

    override fun path(): PathInfo = currentPath

    override fun setListener(listener: PeerListener) {
        this.listener = listener
    }

    override fun disconnect() {
        if (closed) return
        closed = true
        ticker.cancel()
        host.shells.values.forEach { if (it.attached) it.sink = null }
        synchronized(virtualDisplays) { virtualDisplays.clear() }
        // 与真内核一样：开着的画面流随会话断掉，会话关闭时通知监听器
        channels.forEach { it.closeByRemote("disconnected") }
        listener?.onClosed("disconnected")
    }

    override fun isClosed(): Boolean = closed

    override suspend fun openTerminal(options: TerminalOptions, sink: TerminalSink): TerminalChannelInterface {
        delay(250)
        val attachId = options.attachSession
        val shell = if (attachId != null) {
            host.shells[attachId] ?: throw GalaxyException.NotFound(host.tr("会话已结束", "Session has ended"))
        } else {
            host.newShell()
        }
        shell.resize(options.cols.toInt(), options.rows.toInt())
        if (attachId != null) {
            shell.attach(sink)
        } else {
            shell.sink = sink
            shell.start()
        }
        return MockTerminalChannel(shell)
    }

    override suspend fun listTerminals(): List<TerminalSessionInfo> {
        delay(150)
        return host.shells.values.sortedBy { it.createdAt }.map { TerminalSessionInfo(it.id, it.title, it.createdAt, it.attached) }
    }

    override suspend fun killTerminal(sessionId: String) {
        host.shells[sessionId]?.kill()
    }

    override fun files(): FileClientInterface = MockFileClient(host.fs, scope, host.tr, bytesPerSecond(currentPath.kind))

    override suspend fun exec(request: ExecRequest): ExecResult {
        delay(300)
        val out = when (request.command.substringBefore(' ')) {
            "uname" -> "Linux ${host.hostname} 6.8.0 x86_64\n"
            "whoami" -> "${host.user}\n"
            else -> ""
        }
        return ExecResult(0, out, "", false, false, 300u)
    }

    /** 被控端的屏幕：固定的几块 + 本会话加的虚拟屏。 */
    private fun displays(): List<DisplayInfo> = MockData.displaysFor(host.platform) + synchronized(virtualDisplays) { virtualDisplays.toList() }

    override suspend fun listDisplays(): List<DisplayInfo> {
        delay(150)
        if (!hasGui) throw GalaxyException.Permission(host.tr("该设备没有图形界面", "This device has no graphical display"))
        val all = displays()
        return if (listLagsBehindAdd) all.filter { it.id != lastAddedId } else all
    }

    override suspend fun openDisplay(options: DisplayOptions, sink: VideoSink): DisplayChannelInterface {
        delay(300)
        val display = displays().firstOrNull { it.id == options.displayId }
            ?: throw GalaxyException.NotFound(host.tr("找不到屏幕", "Display not found"))
        return MockDisplayChannel(display, core.desktopFrames(), sink, scope, options.maxFps.toInt().coerceAtMost(30)).also { channels += it }
    }

    override suspend fun screenshot(displayId: UInt, maxWidth: UInt): ImageData {
        delay(200)
        return core.desktopPreview() ?: throw GalaxyException.Other("no preview")
    }

    override fun virtualDisplayMax(): UInt = if (hasGui) VIRTUAL_DISPLAY_MAX.toUInt() else 0u

    override suspend fun addVirtualDisplay(width: UInt, height: UInt, refresh: UInt): DisplayInfo {
        delay(400)
        if (!hasGui) throw GalaxyException.Permission(host.tr("被控端不支持虚拟屏", "The remote device does not support virtual displays"))
        if (width !in 640u..7680u || height !in 480u..4320u || (refresh != 0u && refresh !in 24u..240u)) {
            throw GalaxyException.Protocol(host.tr("分辨率或刷新率不合法", "Invalid resolution or refresh rate"))
        }
        synchronized(virtualDisplays) {
            if (virtualDisplays.size >= VIRTUAL_DISPLAY_MAX) {
                throw GalaxyException.Other(host.tr("最多加 $VIRTUAL_DISPLAY_MAX 块虚拟屏", "At most $VIRTUAL_DISPLAY_MAX virtual displays"))
            }
            val id = nextVirtualId.getAndIncrement()
            val d = DisplayInfo(id.toUInt(), "LemonGalaxy Display ${id - VIRTUAL_ID_BASE}", width, height, 1.0f, false, true)
            virtualDisplays.add(d)
            lastAddedId = d.id
            return d
        }
    }

    override suspend fun removeVirtualDisplay(displayId: UInt) {
        delay(250)
        val removed = synchronized(virtualDisplays) { virtualDisplays.removeAll { it.id == displayId } }
        if (removed) {
            // 被控端删屏时关掉还开着的这一路画面
            channels.filter { it.displayId == displayId }.forEach { it.closeByRemote("display removed") }
            return
        }
        throw if (MockData.displaysFor(host.platform).any { it.id == displayId }) {
            GalaxyException.Permission(host.tr("只能删本会话加的虚拟屏", "Only virtual displays added in this session can be removed"))
        } else {
            GalaxyException.NotFound(host.tr("找不到屏幕", "Display not found"))
        }
    }

    override suspend fun clipboardGet(): ClipboardData? = host.clipboard

    override suspend fun clipboardSet(data: ClipboardData) {
        host.clipboard = data
    }

    override suspend fun forwardPort(localPort: UShort, remoteHost: String, remotePort: UShort): PortForwardInterface = MockPortForward(localPort)

    override suspend fun wakeOnLan(mac: String) {
        delay(200)
    }

    override suspend fun setPrivacy(on: Boolean) {
        delay(200)
        if (!hasGui) throw GalaxyException.Permission(host.tr("被控端不支持隐私屏", "The remote device does not support the privacy screen"))
    }

    override suspend fun startAudio(volume: Float): AudioPlaybackInterface {
        delay(150)
        if (!hasGui) throw GalaxyException.Permission(host.tr("被控端不能听声音", "Sound is not available on the remote device"))
        return MockAudioPlayback(volume)
    }

    companion object {
        /** 模拟被控端（Windows / macOS）最多能加的虚拟屏块数，与真被控端一致。 */
        const val VIRTUAL_DISPLAY_MAX = 3

        /** 虚拟屏 id 从 101 开始，不和固定的几块屏撞。 */
        private const val VIRTUAL_ID_BASE = 100

        fun bytesPerSecond(kind: PathKind): Long = when (kind) {
            PathKind.LAN -> 40L * 1024 * 1024
            PathKind.DIRECT -> 8L * 1024 * 1024
            PathKind.RELAY_UDP -> 3L * 1024 * 1024
            PathKind.RELAY_TCP, PathKind.CONNECTING, PathKind.CLOSED -> 1L * 1024 * 1024
        }
    }
}

/** 模拟远端声音：不出声，只记下音量、静音与是否在播。 */
internal class MockAudioPlayback(volume: Float) : AudioPlaybackInterface {
    @Volatile var volume: Float = volume.coerceIn(0f, 1f)
        private set
    @Volatile var muted = false
        private set
    private val active = AtomicBoolean(true)

    init {
        playing.incrementAndGet()
    }

    override fun setVolume(volume: Float) {
        this.volume = volume.coerceIn(0f, 1f)
    }

    override fun setMuted(muted: Boolean) {
        this.muted = muted
    }

    override fun isActive(): Boolean = active.get()

    override fun stop() {
        if (active.getAndSet(false)) playing.decrementAndGet()
    }

    companion object {
        /** 还没停的模拟声音路数（测试核对界面有没有漏停）。 */
        val playing = AtomicInteger()
    }
}

internal class MockPortForward(private val port: UShort) : PortForwardInterface {
    override fun localPort(): UShort = port
    override fun stop() = Unit
}

internal class MockSshSession(private val host: MockHost, private val info: SshInfo) : SshSessionInterface {
    @Volatile private var closed = false

    override fun info(): SshInfo = info
    override fun isClosed(): Boolean = closed

    override suspend fun openTerminal(options: TerminalOptions, sink: TerminalSink): TerminalChannelInterface {
        delay(200)
        val shell = host.newShell()
        shell.resize(options.cols.toInt(), options.rows.toInt())
        shell.sink = sink
        shell.start()
        return MockTerminalChannel(shell)
    }

    override fun files(): FileClientInterface = MockFileClient(host.fs, host.scope, host.tr, 6L * 1024 * 1024)

    override suspend fun exec(request: ExecRequest): ExecResult {
        delay(300)
        return ExecResult(0, "", "", false, false, 300u)
    }

    override suspend fun forwardPort(localPort: UShort, remoteHost: String, remotePort: UShort): PortForwardInterface = MockPortForward(localPort)

    override fun disconnect() {
        // SSH 直连没有被控端常驻会话：断开即结束
        closed = true
        host.shells.values.toList().forEach { it.kill() }
    }
}
