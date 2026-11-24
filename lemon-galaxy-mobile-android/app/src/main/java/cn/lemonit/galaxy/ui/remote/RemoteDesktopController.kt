package cn.lemonit.galaxy.ui.remote

import android.view.Surface
import cn.lemonit.galaxy.app.ConnectionManager
import cn.lemonit.galaxy.app.PeerHandle
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
import cn.lemonit.galaxy.core.bridge.VideoSink
import cn.lemonit.galaxy.data.VideoQuality
import cn.lemonit.galaxy.ui.components.Clock
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
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

enum class RemoteStatus {
    CONNECTING,
    STREAMING,
    /** 画面或会话断了，正在自动重连：最后一帧留着变暗，输入丢掉 */
    RECONNECTING,
    FAILED,
}

/**
 * 自动重连的等待间隔：断开后 1、2、4、8、15 秒，之后每 15 秒，共 10 次（约 2 分钟）都不成就显示失败。
 * 四端一致（Mac、Windows、iOS 同一规则）。
 */
val RECONNECT_DELAYS_MS: List<Long> = listOf(1_000, 2_000, 4_000, 8_000, 15_000, 15_000, 15_000, 15_000, 15_000, 15_000)

/**
 * 重连打开画面后最多等这么久出画面（状态变 STREAMING），没有就算这次失败、接着重试。
 * 被控端不发空闲保活帧（如 Mac，没有 video_keepalive）时内核看门狗管不到，靠这里兜底。
 */
const val RECONNECT_PICTURE_TIMEOUT_MS = 20_000L

/** 等出画面时多久看一次状态（StateFlow 相同值不会再发射，只能轮询） */
private const val PICTURE_POLL_MS = 250L

data class RemoteUi(
    val title: String,
    val platform: Platform,
    val status: RemoteStatus = RemoteStatus.CONNECTING,
    val error: String? = null,
    val displays: List<DisplayInfo> = emptyList(),
    val displayId: UInt? = null,
    val quality: VideoQuality = VideoQuality.AUTO,
    val mouseMode: Boolean = true,
    /** 画面左侧的悬浮鼠标按键（只在鼠标模式下显示） */
    val mouseKeys: Boolean = true,
    val mouseKeysX: Float = 0f,
    val mouseKeysY: Float = 0.3f,
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
    /**
     * 最多能在被控端加几块虚拟屏（docs/protocol.md 8.6）：0 表示不支持或没有控制权限，不显示入口。
     * 每次拿到新会话重新取；会话断开时归零。
     */
    val virtualMax: Int = 0,
    /** 本会话里自己加的虚拟屏 id：只有这些能删。会话换了或断开时清空。 */
    val myVirtualIds: Set<UInt> = emptySet(),
    /** 正在加虚拟屏（按钮转圈并禁用） */
    val virtualAdding: Boolean = false,
    /** 正在删的虚拟屏 */
    val virtualRemoving: Set<UInt> = emptySet(),
    /** 每加好一块加一：界面据此收起加屏表单并提示 [virtualAdded] */
    val virtualAddedSeq: Int = 0,
    val virtualAdded: DisplayInfo? = null,
    /** 每删掉一块加一：界面提示已删掉 */
    val virtualRemovedSeq: Int = 0,
) {
    val recording: Boolean get() = recordingSince != null

    /** 已加满：自己加的块数到了上限。 */
    val virtualFull: Boolean get() = myVirtualIds.size >= virtualMax

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
    /** 加虚拟屏失败（参数是原因），界面用提示条显示，加屏表单不关 */
    private val onVirtualAddFailed: (String) -> Unit = {},
    /** 删虚拟屏失败（参数是原因） */
    private val onVirtualRemoveFailed: (String) -> Unit = {},
    /** 自动重连的等待间隔（测试用短的） */
    private val reconnectDelaysMs: List<Long> = RECONNECT_DELAYS_MS,
    /** 重连打开画面后等出画面的上限（测试用短的） */
    private val pictureTimeoutMs: Long = RECONNECT_PICTURE_TIMEOUT_MS,
) {
    private val _ui = MutableStateFlow(RemoteUi(title, platform, quality = quality, mouseMode = mouseMode))
    val ui: StateFlow<RemoteUi> = _ui.asStateFlow()
    @Volatile private var channel: DisplayChannelInterface? = null
    /** 当前这一路画面的回调；换屏幕、离开时换掉，旧画面迟到的帧与关闭通知直接丢掉 */
    @Volatile private var activeSink: VideoSink? = null

    // 声音：以下状态都只在主线程（scope 是 Main）读写
    private var audioWanted = audioWanted
    private var audio: AudioPlaybackInterface? = null
    /** 每次开始或停止加一：开到一半被关掉、切后台或离开时，迟到的结果直接停掉丢弃 */
    private var audioGen = 0
    private var foreground = true
    private var stopped = false
    /** 每次开始或停止录像加一：开始到一半被停掉时，迟到的结果直接收掉 */
    private var recGen = 0
    /** 当前绑定的会话与跟着它的监听（路径、断开、自己加的虚拟屏）；换会话或离开时取消 */
    private var boundPeer: PeerHandle? = null
    private var peerJob: Job? = null

    // 自动重连：以下状态都只在主线程读写
    private var reconnectJob: Job? = null
    /**
     * 下一次重连用第几个间隔（也就是已用掉几次）。新画面真正显示出第一帧后才清零：
     * 重连打开了画面、但一帧没来就又被关掉（内核判定没有响应）的，接着用剩下的次数，不从头来。点重试、回到前台也从头来。
     */
    private var reconnectStep = 0
    /** 断在后台：不重连（网络可能被冻结），回到前台再开始 */
    private var reconnectPending = false
    /** 下一次尝试先在原会话上重开画面（只有画面断了、会话还活着时） */
    private var reopenFirst = false
    /** 每次断开加一：一次尝试期间又断了，这次不算成功 */
    private var lossSeq = 0
    /** 断开的原因（重连都失败、又没有更具体的原因时显示） */
    private var lossReason = ""
    /** 出过画面：之后断了才自动重连（第一次就没打开的直接显示失败） */
    @Volatile private var everStreamed = false

    val decoder = VideoDecoder(
        requestKeyframe = { channel?.requestKeyframe() },
        onVideoSize = { w, h ->
            everStreamed = true
            val before = _ui.value
            _ui.update { it.copy(videoW = w, videoH = h, status = RemoteStatus.STREAMING) }
            // 画面中途变了尺寸（对方改分辨率）：重新取屏幕信息，输入坐标和画面比例跟着更新
            if (before.status == RemoteStatus.STREAMING && (before.videoW != w || before.videoH != h)) refreshDisplays()
        },
        onFrameShown = {
            // 新画面显示出第一帧：重连次数清零（期间又断了的不算）
            scope.launch { if (_ui.value.status == RemoteStatus.STREAMING) reconnectStep = 0 }
        },
        onCursorChanged = { x, y, v ->
            // 回报是画面像素，输入与界面用被控屏幕像素：画面被缩放过时换算一下（相等时原样）
            _ui.update {
                val sx = if (it.videoW > 0 && it.remoteW > 0) x.toLong() * it.remoteW / it.videoW else x.toLong()
                val sy = if (it.videoH > 0 && it.remoteH > 0) y.toLong() * it.remoteH / it.videoH else y.toLong()
                it.copy(cursorX = sx.toInt(), cursorY = sy.toInt(), cursorVisible = v, cursorSeq = it.cursorSeq + 1)
            }
        },
    )

    /** 当前会话；连接断过、重新连上换了会话时重新绑定（能力、虚拟屏上限、自己加的虚拟屏）。只在 [scope] 线程上调用。 */
    private suspend fun peer(): PeerHandle {
        val p = conn.peer(deviceId)
        if (p !== boundPeer) bind(p)
        return p
    }

    private fun bind(peer: PeerHandle) {
        val previous = boundPeer
        boundPeer = peer
        peerJob?.cancel()
        val virtualMax = runCatching { peer.session.virtualDisplayMax().toInt() }.getOrDefault(0)
        _ui.update {
            it.copy(
                // 换了会话：上一个会话加的虚拟屏已被被控端删掉
                displays = if (previous != null) it.displays.filter { d -> d.id !in it.myVirtualIds } else it.displays,
                path = peer.path.value,
                privacySupported = "privacy" in peer.info.capabilities,
                privacyAllowed = peer.info.permissions.control,
                audioSupported = "audio" in peer.info.capabilities,
                clientCursor = "client_cursor" in peer.info.capabilities,
                virtualMax = if (peer.closed) 0 else virtualMax,
                myVirtualIds = peer.myVirtualDisplays.value,
                virtualRemoving = emptySet(),
            )
        }
        if (previous != null) refreshDisplays()
        peerJob = scope.launch {
            launch { peer.path.collect { p -> _ui.update { it.copy(path = p) } } }
            // 自己加的少了（删掉或会话结束被控端自动删）：列表里也去掉
            launch {
                peer.myVirtualDisplays.collect { ids ->
                    _ui.update { u ->
                        val gone = u.myVirtualIds - ids
                        u.copy(myVirtualIds = ids, displays = if (gone.isEmpty()) u.displays else u.displays.filter { it.id !in gone })
                    }
                }
            }
            // 会话结束：被控端已自动关掉隐私屏、删掉本会话加的虚拟屏，开关复位、加屏入口收起；声音通道随会话失效，停掉本机播放
            launch {
                peer.closedReason.collect { r ->
                    if (r != null) {
                        _ui.update {
                            it.copy(
                                privacyOn = false,
                                virtualMax = 0,
                                displays = it.displays.filter { d -> d.id !in it.myVirtualIds },
                                myVirtualIds = emptySet(),
                                virtualRemoving = emptySet(),
                            )
                        }
                        stopAudio()
                        stopRecording(auto = false)
                        // 远程桌面还开着、不是自己离开的：自动重连（会话已断，走重新连接）
                        if (!stopped && reconnectable()) connectionLost(r, sessionDead = true)
                    }
                }
            }
        }
    }

    /**
     * 每路画面一个回调，只有当前这一路的才交给解码器：切屏或删掉正在看的屏后，
     * 旧画面迟到的帧、光标、关闭通知（"stopped" 或被控端关流）不会把新画面标成失败。
     */
    private inner class StreamSink : VideoSink {
        private val current get() = activeSink === this

        override fun onConfig(codec: VideoCodec, width: UInt, height: UInt) {
            if (current) decoder.onConfig(codec, width, height)
        }

        override fun onFrame(data: ByteArray, ptsUs: ULong, keyframe: Boolean) {
            if (current) decoder.onFrame(data, ptsUs, keyframe)
        }

        override fun onCursor(x: Int, y: Int, visible: Boolean) {
            if (current) decoder.onCursor(x, y, visible)
        }

        /** 不是自己停的（自己停时先换掉了回调）：被控端结束、流出错、画面没有响应。回主线程再核对一次是不是当前这一路。 */
        override fun onClosed(reason: String) {
            if (current) scope.launch { if (current) streamClosed(reason) }
        }
    }

    private fun peerAlive(p: PeerHandle?) = p != null && !p.closed && !p.session.isClosed()

    /** 出过画面、现在正看着（或正在换屏 / 重连）：断了要自动重连。 */
    private fun reconnectable(): Boolean = when (_ui.value.status) {
        RemoteStatus.STREAMING, RemoteStatus.RECONNECTING -> true
        RemoteStatus.CONNECTING -> everStreamed
        RemoteStatus.FAILED -> false
    }

    /** 当前这一路画面断了。 */
    private fun streamClosed(reason: String) {
        if (stopped) return
        if (reconnectable()) {
            connectionLost(reason, sessionDead = !peerAlive(boundPeer))
            return
        }
        // 第一次打开就断了：显示失败（可重试）；录像到此为止（界面还在，弹保存提示）
        stopRecording(auto = false)
        dropChannel()
        _ui.update { it.copy(status = RemoteStatus.FAILED, error = reason) }
    }

    /** 停掉当前这一路画面：之后的输入丢掉，迟到的回调不再算。 */
    private fun dropChannel() {
        activeSink = null
        channel?.stop()
        channel = null
    }

    /**
     * 画面或会话断了（不是自己停的）：最后一帧留着变暗，按 [reconnectDelaysMs] 自动重连。
     * [fresh] 为真（点了重试、回到前台）时次数从头算，否则接着用剩下的（见 [reconnectStep]）；
     * 之前不在重连时（新一次断开）[sessionDead] 为假的，第一次先在原会话上只重开画面。
     * 在后台时只记下，回到前台再开始。
     */
    private fun connectionLost(reason: String, sessionDead: Boolean, fresh: Boolean = false) {
        if (stopped) return
        if (reason.isNotEmpty()) lossReason = reason
        lossSeq++
        if (fresh) reconnectStep = 0
        if (fresh || _ui.value.status != RemoteStatus.RECONNECTING) {
            reopenFirst = !sessionDead
        } else if (sessionDead) {
            reopenFirst = false
        }
        stopRecording(auto = false)
        dropChannel()
        _ui.update { it.copy(status = RemoteStatus.RECONNECTING, error = null) }
        if (!foreground) {
            reconnectPending = true
            return
        }
        if (reconnectJob?.isActive != true) reconnectJob = scope.launch { reconnectLoop() }
    }

    private suspend fun reconnectLoop() {
        var lastError: String? = null
        while (reconnectStep < reconnectDelaysMs.size) {
            delay(reconnectDelaysMs[reconnectStep++])
            val seq = lossSeq
            try {
                reconnectOnce()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                e.userMessage().takeIf { it.isNotEmpty() }?.let { lastError = it }
                activeSink = null
                continue
            }
            // 画面打开了：等真正出画面（状态变 STREAMING）才算连上，最多等 [pictureTimeoutMs]
            var waited = 0L
            while (seq == lossSeq && _ui.value.status != RemoteStatus.STREAMING && waited < pictureTimeoutMs) {
                delay(PICTURE_POLL_MS)
                waited += PICTURE_POLL_MS
            }
            // 尝试期间又断了：这次不算，接着按间隔重试
            if (seq != lossSeq) continue
            if (_ui.value.status == RemoteStatus.STREAMING) return
            // 一直没出画面：这次算失败，停掉这一路，接着用剩下的次数
            dropChannel()
        }
        // 都不成：显示失败（原因 + 重试）
        _ui.update { it.copy(status = RemoteStatus.FAILED, error = lastError ?: lossReason) }
    }

    /**
     * 重连一次：只有画面断了、会话还活着时先在原会话上重开同一块屏；不行（或会话已断）就经 [peer] 重新连接
     * （会话已关会建新会话并重新绑定），原来那块屏还在就开它，否则开主屏。
     */
    private suspend fun reconnectOnce() {
        val target = _ui.value.displayId
        if (reopenFirst) {
            reopenFirst = false
            if (target != null && peerAlive(boundPeer)) {
                try {
                    open(target, reconnecting = true)
                    return
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    // 落到下面整条重新连接
                }
            }
        }
        val peer = peer()
        val asked = peer.myVirtualDisplays.value
        val displays = peer.session.listDisplays()
        val next = displays.firstOrNull { it.id == target } ?: displays.firstOrNull { it.primary } ?: displays.firstOrNull()
            ?: throw IllegalStateException("no display")
        applyDisplays(peer, displays, asked)
        open(next.id, reconnecting = true)
    }

    /** 失败界面的「重试」：出过画面的按自动重连从头再来（取消还没到的重试），第一次就没打开的重新打开。 */
    fun retry() {
        if (stopped) return
        reconnectJob?.cancel()
        reconnectJob = null
        reconnectPending = false
        if (everStreamed) {
            connectionLost(lossReason, sessionDead = !peerAlive(boundPeer), fresh = true)
        } else {
            dropChannel()
            _ui.update { it.copy(status = RemoteStatus.CONNECTING, error = null) }
            start()
        }
    }

    /**
     * 重新取到的屏幕列表。[asked] 是发请求时「自己加的」集合：其中列表里已经没有的，被控端那边没了
     * （如 Windows 换控制台会话后屏消失），从集合去掉、计数腾出来。[added] 是刚加好的屏：被控端列表里暂时还没有就先补上。
     * 请求发出之后才加好的屏，这份列表里可能还没有，保留不动。
     */
    private fun applyDisplays(peer: PeerHandle, fetched: List<DisplayInfo>, asked: Set<UInt>, added: DisplayInfo? = null) {
        val list = fetched.toMutableList()
        if (added != null && list.none { it.id == added.id }) list += added
        val mine = peer.myVirtualDisplays.value
        _ui.value.displays.filter { d -> d.id in mine && d.id !in asked && list.none { it.id == d.id } }.forEach { list += it }
        val ids = list.mapTo(HashSet()) { it.id }
        peer.retainVirtualDisplays { it in ids || it !in asked }
        _ui.update { u ->
            u.copy(displays = list, myVirtualIds = if (boundPeer === peer) peer.myVirtualDisplays.value else u.myVirtualIds)
        }
    }

    fun start() {
        scope.launch {
            try {
                val peer = peer()
                val asked = peer.myVirtualDisplays.value
                val displays = peer.session.listDisplays()
                val first = displays.firstOrNull { it.primary } ?: displays.firstOrNull()
                    ?: throw IllegalStateException("no display")
                applyDisplays(peer, displays, asked)
                open(first.id)
            } catch (e: Exception) {
                _ui.update { it.copy(status = RemoteStatus.FAILED, error = e.userMessage()) }
            }
        }
    }

    /** 打开一路画面；[reconnecting] 为真时（自动重连）状态保持 RECONNECTING，出画面后变 STREAMING。 */
    private suspend fun open(displayId: UInt, reconnecting: Boolean = false) {
        // 换屏幕 / 重开画面前先停录像拿到结果（关画面时内核也会结束录像，但不返回信息）
        stopRecording(auto = false)
        // 先换掉回调：旧画面停下时的 "stopped" 不算新画面失败
        dropChannel()
        _ui.update {
            // 光标先放屏幕中心（被控端有回报后以回报为准）
            val d = it.displays.firstOrNull { x -> x.id == displayId }
            it.copy(
                displayId = displayId, status = if (reconnecting) RemoteStatus.RECONNECTING else RemoteStatus.CONNECTING, error = null,
                cursorX = d?.let { x -> (x.width / 2u).toInt() } ?: it.cursorX,
                cursorY = d?.let { x -> (x.height / 2u).toInt() } ?: it.cursorY,
                cursorVisible = true,
            )
        }
        val q = _ui.value.quality
        val peer = peer()
        val sink = StreamSink()
        activeSink = sink
        val ch = peer.session.openDisplay(
            DisplayOptions(displayId, q.maxFps, q.bitrateKbps, VideoDecoder.supportedCodecs(), viewOnly = false),
            sink,
        )
        if (activeSink !== sink) {
            // 打开期间断了、又换了画面或离开了：这一路不要了
            ch.stop()
            return
        }
        channel = ch
        // 每路画面各自记着要不要画光标（默认画）：鼠标模式由本机画，告诉被控端别画
        applyClientCursor()
        // 画面打开后按设置开始播放声音（已在播放时不重复开）
        if (audioWanted) startAudio()
    }

    /** 重新取屏幕列表（分辨率变化、换了会话后）；失败不影响画面。 */
    private fun refreshDisplays() {
        scope.launch {
            try {
                val peer = peer()
                val asked = peer.myVirtualDisplays.value
                val list = peer.session.listDisplays()
                if (list.isNotEmpty() && boundPeer === peer) applyDisplays(peer, list, asked)
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
            }
        }
    }

    fun switchDisplay(id: UInt) {
        if (id == _ui.value.displayId) return
        scope.launch { switchTo(id) }
    }

    private suspend fun switchTo(id: UInt) {
        // 重连期间不换屏（重连好了再换）
        if (_ui.value.status == RemoteStatus.RECONNECTING) return
        try {
            open(id)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            when {
                // 换屏期间断了：重连接手
                _ui.value.status == RemoteStatus.RECONNECTING -> Unit
                !peerAlive(boundPeer) && reconnectable() -> connectionLost(e.userMessage(), sessionDead = true)
                else -> _ui.update { it.copy(status = RemoteStatus.FAILED, error = e.userMessage()) }
            }
        }
    }

    /**
     * 加一块虚拟屏（docs/protocol.md 8.6）：记入自己加的、重新取屏幕列表、直接切过去看。
     * 失败时经 [onVirtualAddFailed] 提示原因（界面的加屏表单不关）。
     */
    fun addVirtualDisplay(width: Int, height: Int, refresh: Int) {
        val s = _ui.value
        if (stopped || s.virtualAdding || s.virtualMax <= 0 || s.virtualFull) return
        if (!VirtualDisplays.sizeOk(width, height) || !VirtualDisplays.refreshOk(refresh)) return
        _ui.update { it.copy(virtualAdding = true) }
        scope.launch {
            val added = try {
                val peer = peer()
                val d = peer.session.addVirtualDisplay(width.toUInt(), height.toUInt(), refresh.toUInt())
                peer.virtualDisplayAdded(d.id)
                val asked = peer.myVirtualDisplays.value
                val fetched = runCatching { peer.session.listDisplays() }.getOrNull()?.takeIf { it.isNotEmpty() } ?: _ui.value.displays
                applyDisplays(peer, fetched, asked, added = d)
                _ui.update { u -> u.copy(virtualAdding = false, virtualAddedSeq = u.virtualAddedSeq + 1, virtualAdded = d) }
                d
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _ui.update { it.copy(virtualAdding = false) }
                e.userMessage().takeIf { it.isNotEmpty() }?.let(onVirtualAddFailed)
                null
            }
            // 直接切过去看（已离开界面就不开画面了：屏留在会话里，回来还能删）
            if (added != null && !stopped) switchTo(added.id)
        }
    }

    /**
     * 删掉自己加的虚拟屏。正在看的就是它时先切回主屏（没有主屏就第一块）再删，
     * 免得画面流被被控端关掉、迟到的关闭通知把新画面标成失败。失败时经 [onVirtualRemoveFailed] 提示原因。
     */
    fun removeVirtualDisplay(id: UInt) {
        val s = _ui.value
        if (stopped || id !in s.myVirtualIds || id in s.virtualRemoving) return
        _ui.update { it.copy(virtualRemoving = it.virtualRemoving + id) }
        scope.launch {
            try {
                val peer = peer()
                if (_ui.value.displayId == id) {
                    val rest = _ui.value.displays.filter { it.id != id }
                    (rest.firstOrNull { it.primary } ?: rest.firstOrNull())?.let { switchTo(it.id) }
                }
                peer.session.removeVirtualDisplay(id)
                peer.virtualDisplayRemoved(id)
                val asked = peer.myVirtualDisplays.value
                val fetched = runCatching { peer.session.listDisplays() }.getOrNull()?.takeIf { it.isNotEmpty() } ?: _ui.value.displays
                applyDisplays(peer, fetched.filter { it.id != id }, asked)
                _ui.update { u -> u.copy(myVirtualIds = u.myVirtualIds - id, virtualRemoving = u.virtualRemoving - id, virtualRemovedSeq = u.virtualRemovedSeq + 1) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _ui.update { it.copy(virtualRemoving = it.virtualRemoving - id) }
                e.userMessage().takeIf { it.isNotEmpty() }?.let(onVirtualRemoveFailed)
            }
        }
    }

    fun setQuality(q: VideoQuality) {
        _ui.update { it.copy(quality = q) }
        channel?.setBitrate(q.bitrateKbps)
    }

    fun setMouseKeys(on: Boolean) = _ui.update { it.copy(mouseKeys = on) }

    fun setMouseKeysPos(x: Float, y: Float) = _ui.update { it.copy(mouseKeysX = x, mouseKeysY = y) }

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

    /**
     * 切到后台先停掉声音，回到前台按开关重新开始。
     * 后台不重连（网络可能被冻结）：正在重连的停下，回到前台从头再来；在后台时会话断了（或回来时发现已断）同样回前台再连。
     */
    fun setForeground(fg: Boolean) {
        if (fg == foreground) return
        foreground = fg
        if (!fg) {
            stopAudio()
            stopRecording(auto = true)
            if (reconnectJob?.isActive == true) {
                reconnectJob?.cancel()
                reconnectJob = null
                activeSink = null
                reconnectPending = true
            }
            return
        }
        if (stopped) return
        val bound = boundPeer
        when {
            reconnectPending -> {
                reconnectPending = false
                if (_ui.value.status == RemoteStatus.RECONNECTING) connectionLost("", sessionDead = !peerAlive(bound), fresh = true)
            }
            bound != null && !peerAlive(bound) && reconnectable() -> connectionLost(bound.closedReason.value ?: "", sessionDead = true)
            audioWanted && channel != null -> startAudio()
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
        reconnectJob?.cancel()
        reconnectJob = null
        reconnectPending = false
        dropChannel()
        decoder.release()
        peerJob?.cancel()
        peerJob = null
    }
}
