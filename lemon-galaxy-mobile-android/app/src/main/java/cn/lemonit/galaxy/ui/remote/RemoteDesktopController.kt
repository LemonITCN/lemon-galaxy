package cn.lemonit.galaxy.ui.remote

import android.view.Surface
import cn.lemonit.galaxy.app.ConnectionManager
import cn.lemonit.galaxy.app.userMessage
import cn.lemonit.galaxy.core.bridge.AudioPlaybackInterface
import cn.lemonit.galaxy.core.bridge.ClipboardData
import cn.lemonit.galaxy.core.bridge.DisplayChannelInterface
import cn.lemonit.galaxy.core.bridge.DisplayInfo
import cn.lemonit.galaxy.core.bridge.DisplayOptions
import cn.lemonit.galaxy.core.bridge.InputEvent
import cn.lemonit.galaxy.core.bridge.PathInfo
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.RecordingInfo
import cn.lemonit.galaxy.core.bridge.VideoCodec
import cn.lemonit.galaxy.data.VideoQuality
import cn.lemonit.galaxy.ui.components.Clock
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

enum class RemoteStatus { CONNECTING, STREAMING, FAILED }

data class RemoteUi(
    val title: String,
    val platform: Platform,
    val status: RemoteStatus = RemoteStatus.CONNECTING,
    val error: String? = null,
    val displays: List<DisplayInfo> = emptyList(),
    val displayId: UInt? = null,
    val quality: VideoQuality = VideoQuality.AUTO,
    val mouseMode: Boolean = true,
    /** 对端能力里有 "client_cursor"：鼠标模式下被控端画面里不画光标，由本机画（跟手） */
    val clientCursor: Boolean = false,
    /**
     * 光标的初始位置（打开 / 换屏幕时为屏幕中心）与被控端最近一次回报（已换算成被控屏幕像素）。
     * 每次回报 [cursorSeq] 加一；本机移动不改这里，界面自己记（见 [LocalPointer]）。
     */
    val cursorX: Int = 0,
    val cursorY: Int = 0,
    val cursorVisible: Boolean = true,
    val cursorSeq: Int = 0,
    val videoW: Int = 0,
    val videoH: Int = 0,
    val path: PathInfo? = null,
    /** 对端能力里有 "privacy"：工具栏显示隐私屏开关 */
    val privacySupported: Boolean = false,
    /** 有控制权限才能切换隐私屏 */
    val privacyAllowed: Boolean = false,
    val privacyOn: Boolean = false,
    val privacyBusy: Boolean = false,
    /** 对端能力里有 "audio"（对方给了听声音的权限）：工具栏显示声音开关 */
    val audioSupported: Boolean = false,
    /** 正在本机播放被控端的声音 */
    val audioOn: Boolean = false,
    val audioBusy: Boolean = false,
    /** 录像开始的时刻（Clock.now()）；null 表示没在录 */
    val recordingSince: Long? = null,
    /** 正在开始录像（等内核建文件、等关键帧） */
    val recordingBusy: Boolean = false,
) {
    val recording: Boolean get() = recordingSince != null

    /** 本机画光标：鼠标模式且被控端支持。 */
    val localCursor: Boolean get() = mouseMode && clientCursor
    val display: DisplayInfo? get() = displays.firstOrNull { it.id == displayId }
    val remoteW: Int get() = display?.width?.toInt() ?: videoW
    val remoteH: Int get() = display?.height?.toInt() ?: videoH
}

/** 录像文件名：LemonGalaxy-<设备名>-<yyyyMMdd-HHmmss>.mp4（设备名里文件系统不允许的字符换成 -）。 */
fun recordingFileName(deviceName: String, at: Long): String {
    val safe = deviceName.replace(Regex("[\\\\/:*?\"<>|\\p{Cntrl}]"), "-").trim().trim('.').ifEmpty { "device" }
    val ts = SimpleDateFormat("yyyyMMdd-HHmmss", Locale.US).format(Date(at))
    return "LemonGalaxy-$safe-$ts.mp4"
}

/** 录像计时：不到 1 小时 mm:ss，否则 h:mm:ss。 */
fun formatElapsed(ms: Long): String {
    val t = (ms / 1000).coerceAtLeast(0)
    val h = t / 3600
    val m = t % 3600 / 60
    val sec = t % 60
    return if (h > 0) String.format(Locale.US, "%d:%02d:%02d", h, m, sec) else String.format(Locale.US, "%02d:%02d", m, sec)
}

class RemoteDesktopController(
    private val deviceId: String,
    title: String,
    platform: Platform,
    private val conn: ConnectionManager,
    private val scope: CoroutineScope,
    quality: VideoQuality,
    mouseMode: Boolean,
    /** 设置里的声音开关：画面打开后自动开始 */
    audioWanted: Boolean = false,
    /** 开始播放声音失败（参数是原因），界面用提示条显示 */
    private val onAudioError: (String) -> Unit = {},
    /** 录像存放目录（取不到时不能录） */
    private val recordingDir: () -> File? = { null },
    /** 录像停止并写好：[auto] 为真表示离开界面 / 切后台时自动停的（界面已不在，只提示保存位置） */
    private val onRecordingSaved: (info: RecordingInfo, auto: Boolean) -> Unit = { _, _ -> },
    /** 开始或停止录像失败（参数是原因） */
    private val onRecordingError: (String) -> Unit = {},
) {
    private val _ui = MutableStateFlow(RemoteUi(title, platform, quality = quality, mouseMode = mouseMode))
    val ui: StateFlow<RemoteUi> = _ui.asStateFlow()
    @Volatile private var channel: DisplayChannelInterface? = null

    // 声音：以下状态都只在主线程（scope 是 Main）读写
    private var audioWanted = audioWanted
    private var audio: AudioPlaybackInterface? = null
    /** 每次开始或停止加一：开到一半被关掉、切后台或离开时，迟到的结果直接停掉丢弃 */
    private var audioGen = 0
    private var foreground = true
    private var stopped = false
    /** 每次开始或停止录像加一：开始到一半被停掉时，迟到的结果直接收掉 */
    private var recGen = 0

    val decoder = VideoDecoder(
        requestKeyframe = { channel?.requestKeyframe() },
        onVideoSize = { w, h ->
            val before = _ui.value
            _ui.update { it.copy(videoW = w, videoH = h, status = RemoteStatus.STREAMING) }
            // 画面中途变了尺寸（对方改分辨率）：重新取屏幕信息，输入坐标和画面比例跟着更新
            if (before.status == RemoteStatus.STREAMING && (before.videoW != w || before.videoH != h)) refreshDisplays()
        },
        onCursorChanged = { x, y, v ->
            // 回报是画面像素，输入与界面用被控屏幕像素：画面被缩放过时换算一下（相等时原样）
            _ui.update {
                val sx = if (it.videoW > 0 && it.remoteW > 0) x.toLong() * it.remoteW / it.videoW else x.toLong()
                val sy = if (it.videoH > 0 && it.remoteH > 0) y.toLong() * it.remoteH / it.videoH else y.toLong()
                it.copy(cursorX = sx.toInt(), cursorY = sy.toInt(), cursorVisible = v, cursorSeq = it.cursorSeq + 1)
            }
        },
        onClosedByRemote = { reason ->
            _ui.update { it.copy(status = RemoteStatus.FAILED, error = reason) }
            // 画面断了：录像到此为止（界面还在，弹保存提示）
            scope.launch { stopRecording(auto = false) }
        },
    )

    fun start() {
        scope.launch {
            try {
                val peer = conn.peer(deviceId)
                _ui.update {
                    it.copy(
                        path = peer.path.value,
                        privacySupported = "privacy" in peer.info.capabilities,
                        privacyAllowed = peer.info.permissions.control,
                        audioSupported = "audio" in peer.info.capabilities,
                        clientCursor = "client_cursor" in peer.info.capabilities,
                    )
                }
                scope.launch { peer.path.collect { p -> _ui.update { it.copy(path = p) } } }
                // 会话结束：被控端已自动关掉隐私屏，开关复位；声音通道随会话失效，停掉本机播放
                scope.launch {
                    peer.closedReason.collect { r ->
                        if (r != null) {
                            _ui.update { it.copy(privacyOn = false) }
                            stopAudio()
                            stopRecording(auto = false)
                        }
                    }
                }
                val displays = peer.session.listDisplays()
                val first = displays.firstOrNull { it.primary } ?: displays.firstOrNull()
                    ?: throw IllegalStateException("no display")
                _ui.update { it.copy(displays = displays) }
                open(first.id)
            } catch (e: Exception) {
                _ui.update { it.copy(status = RemoteStatus.FAILED, error = e.userMessage()) }
            }
        }
    }

    private suspend fun open(displayId: UInt) {
        // 换屏幕 / 重开画面前先停录像拿到结果（关画面时内核也会结束录像，但不返回信息）
        stopRecording(auto = false)
        channel?.stop()
        channel = null
        _ui.update {
            // 光标先放屏幕中心（被控端有回报后以回报为准）
            val d = it.displays.firstOrNull { x -> x.id == displayId }
            it.copy(
                displayId = displayId, status = RemoteStatus.CONNECTING, error = null,
                cursorX = d?.let { x -> (x.width / 2u).toInt() } ?: it.cursorX,
                cursorY = d?.let { x -> (x.height / 2u).toInt() } ?: it.cursorY,
                cursorVisible = true,
            )
        }
        val q = _ui.value.quality
        val peer = conn.peer(deviceId)
        channel = peer.session.openDisplay(
            DisplayOptions(displayId, q.maxFps, q.bitrateKbps, VideoDecoder.supportedCodecs(), viewOnly = false),
            decoder,
        )
        // 每路画面各自记着要不要画光标（默认画）：鼠标模式由本机画，告诉被控端别画
        applyClientCursor()
        // 画面打开后按设置开始播放声音（已在播放时不重复开）
        if (audioWanted) startAudio()
    }

    /** 重新取屏幕列表（分辨率变化后）；失败不影响画面。 */
    private fun refreshDisplays() {
        scope.launch {
            runCatching { conn.peer(deviceId).session.listDisplays() }.getOrNull()?.takeIf { it.isNotEmpty() }?.let { list ->
                _ui.update { it.copy(displays = list) }
            }
        }
    }

    fun switchDisplay(id: UInt) {
        if (id == _ui.value.displayId) return
        scope.launch {
            try {
                open(id)
            } catch (e: Exception) {
                _ui.update { it.copy(status = RemoteStatus.FAILED, error = e.userMessage()) }
            }
        }
    }

    fun setQuality(q: VideoQuality) {
        _ui.update { it.copy(quality = q) }
        channel?.setBitrate(q.bitrateKbps)
    }

    fun setMouseMode(on: Boolean) {
        _ui.update { it.copy(mouseMode = on) }
        applyClientCursor()
    }

    /** 鼠标模式本机画光标、被控端画面里不画；触控模式画回画面里。只对支持 "client_cursor" 的被控端发（老版本收到会关流）。 */
    private fun applyClientCursor() {
        val s = _ui.value
        if (s.clientCursor) channel?.setClientCursor(s.mouseMode)
    }

    /** 切换隐私屏，失败时抛出（界面提示原因）。 */
    suspend fun setPrivacy(on: Boolean) {
        if (_ui.value.privacyBusy) return
        _ui.update { it.copy(privacyBusy = true) }
        try {
            conn.peer(deviceId).session.setPrivacy(on)
            _ui.update { it.copy(privacyOn = on) }
        } finally {
            _ui.update { it.copy(privacyBusy = false) }
        }
    }

    /** 用户点声音开关（调用方负责记住选择）。 */
    fun setAudio(on: Boolean) {
        audioWanted = on
        if (on) startAudio() else stopAudio()
    }

    /** 切到后台先停掉声音，回到前台按开关重新开始。 */
    fun setForeground(fg: Boolean) {
        if (fg == foreground) return
        foreground = fg
        if (!fg) {
            stopAudio()
            stopRecording(auto = true)
        } else if (audioWanted && channel != null) {
            startAudio()
        }
    }

    private fun startAudio() {
        val s = _ui.value
        if (!s.audioSupported || s.audioBusy || audio != null || !foreground || stopped) return
        val gen = ++audioGen
        _ui.update { it.copy(audioBusy = true) }
        scope.launch {
            val result = try {
                Result.success(conn.peer(deviceId).session.startAudio(1f))
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Result.failure(e)
            }
            if (gen != audioGen) {
                // 等待期间被关掉了：这一路不要了
                result.getOrNull()?.stop()
                return@launch
            }
            _ui.update { it.copy(audioBusy = false) }
            result
                .onSuccess { a ->
                    audio = a
                    _ui.update { it.copy(audioOn = true) }
                }
                .onFailure { e -> e.userMessage().takeIf { it.isNotEmpty() }?.let(onAudioError) }
        }
    }

    /** 停止本机播放（不改设置里的开关）。 */
    private fun stopAudio() {
        audioGen++
        val a = audio
        audio = null
        _ui.update { it.copy(audioOn = false, audioBusy = false) }
        a?.stop()
    }

    /** 工具栏录像按钮：没在录就开始，在录就停止。 */
    fun toggleRecording() {
        if (_ui.value.recording) stopRecording(auto = false) else startRecording()
    }

    fun startRecording() {
        val s = _ui.value
        val ch = channel
        if (ch == null || s.recording || s.recordingBusy || stopped || !foreground) return
        val dir = recordingDir() ?: return
        val file = File(dir, recordingFileName(s.title, Clock.now()))
        val gen = ++recGen
        _ui.update { it.copy(recordingBusy = true) }
        scope.launch {
            val error = try {
                withContext(Dispatchers.IO) { dir.mkdirs() }
                // 正在收听声音时一起录
                ch.startRecording(file.path, withAudio = _ui.value.audioOn)
                null
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                e
            }
            if (gen != recGen || ch !== channel) {
                // 等待期间被停掉、离开或换了画面：这一路不要了
                if (error == null) runCatching { ch.stopRecording() }.getOrNull()?.paths?.forEach { File(it).delete() }
                return@launch
            }
            _ui.update { it.copy(recordingBusy = false, recordingSince = if (error == null) Clock.now() else null) }
            error?.userMessage()?.takeIf { it.isNotEmpty() }?.let(onRecordingError)
        }
    }

    /** 停止录像并交出结果；[auto] 为真表示离开界面 / 切后台时自动停的。没在录时什么也不做。 */
    fun stopRecording(auto: Boolean) {
        recGen++
        val wasRecording = _ui.value.recording
        if (wasRecording || _ui.value.recordingBusy) _ui.update { it.copy(recordingSince = null, recordingBusy = false) }
        if (!wasRecording) return
        val ch = channel ?: return
        try {
            val info = ch.stopRecording()
            if (info.paths.isNotEmpty()) onRecordingSaved(info, auto)
        } catch (e: Exception) {
            e.userMessage().takeIf { it.isNotEmpty() }?.let(onRecordingError)
        }
    }

    fun setSurface(s: Surface?) = decoder.setSurface(s)

    /** 光标位置由界面本机记（[LocalPointer]），这里只管发出去。 */
    fun send(e: InputEvent) {
        channel?.sendInput(e)
    }

    fun sendAll(events: List<InputEvent>) = events.forEach(::send)

    suspend fun clipboardSend(text: String) = withContext(Dispatchers.Default) {
        conn.peer(deviceId).session.clipboardSet(ClipboardData.Text(text))
    }

    suspend fun clipboardGet(): String? = withContext(Dispatchers.Default) {
        (conn.peer(deviceId).session.clipboardGet() as? ClipboardData.Text)?.text
    }

    fun stop() {
        // 离开远程桌面：先停录像拿到结果
        stopRecording(auto = true)
        stopped = true
        stopAudio()
        // 离开远程桌面时关掉隐私屏（同一会话可能还被终端、文件用着，被控端不会自动关）
        if (_ui.value.privacyOn) {
            conn.existingPeer(deviceId)?.let { p -> scope.launch { runCatching { p.session.setPrivacy(false) } } }
            _ui.update { it.copy(privacyOn = false) }
        }
        channel?.stop()
        channel = null
        decoder.release()
    }
}
