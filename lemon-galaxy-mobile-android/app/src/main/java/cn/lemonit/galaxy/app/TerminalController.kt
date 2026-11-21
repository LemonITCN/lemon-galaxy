package cn.lemonit.galaxy.app

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.util.Log
import cn.lemonit.galaxy.core.bridge.EnvVar
import cn.lemonit.galaxy.core.bridge.TerminalChannelInterface
import cn.lemonit.galaxy.core.bridge.TerminalOptions
import cn.lemonit.galaxy.core.bridge.TerminalSessionInfo
import cn.lemonit.galaxy.core.bridge.TerminalSink
import com.termux.terminal.TerminalSession
import com.termux.terminal.TerminalSessionClient
import com.termux.view.TerminalView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.lang.ref.WeakReference
import java.util.concurrent.atomic.AtomicLong

sealed class TabStatus {
    data object Connecting : TabStatus()
    data object Connected : TabStatus()
    data object Detached : TabStatus()
    data class Exited(val code: Int?) : TabStatus()
    data class Failed(val message: String) : TabStatus()
}

/**
 * 一个终端标签：Termux 的仿真器（TerminalSession）与内核 TerminalChannel 之间的桥。
 * 远端输出 → TerminalSink.onOutput → session.appendRemoteOutput；用户输入 → Transport.write → channel.write。
 */
class TerminalTab internal constructor(
    val id: Long,
    val target: TargetRef,
    val targetName: String,
    private val scope: CoroutineScope,
    private val conn: ConnectionManager,
    private val appContext: Context?,
) : TerminalSessionClient {
    private val _status = MutableStateFlow<TabStatus>(TabStatus.Connecting)
    val status: StateFlow<TabStatus> = _status.asStateFlow()
    private val _title = MutableStateFlow(targetName)
    val title: StateFlow<String> = _title.asStateFlow()

    @Volatile var remoteSessionId: String? = null
        private set
    @Volatile private var channel: TerminalChannelInterface? = null
    @Volatile private var cols = 80
    @Volatile private var rows = 24
    private var viewRef = WeakReference<TerminalView>(null)

    private val transport = object : TerminalSession.Transport {
        override fun write(data: ByteArray, offset: Int, count: Int) {
            channel?.write(data.copyOfRange(offset, offset + count))
        }

        override fun resize(columns: Int, rows: Int) {
            cols = columns
            this@TerminalTab.rows = rows
            channel?.resize(columns.toUShort(), rows.toUShort())
        }

        override fun close() {
            channel?.kill()
        }
    }

    val session = TerminalSession(transport, 3000, this)

    private inner class Sink : TerminalSink {
        @Volatile var active = true
        override fun onOutput(data: ByteArray) {
            if (active) session.appendRemoteOutput(data, 0, data.size)
        }

        override fun onExit(code: Int?) {
            if (!active) return
            _status.value = TabStatus.Exited(code)
            session.onRemoteExit(code)
        }
    }

    private var sink: Sink? = null

    fun bindView(view: TerminalView?) {
        viewRef = WeakReference(view)
    }

    fun connect(attachId: String?) {
        _status.value = TabStatus.Connecting
        sink?.active = false
        val s = Sink().also { sink = it }
        scope.launch {
            try {
                val ch = when (target) {
                    is TargetRef.Device -> conn.peer(target.deviceId).session.openTerminal(
                        TerminalOptions(attachId, null, cols.toUShort(), rows.toUShort(), TERMINAL_ENV), s,
                    )
                    is TargetRef.Ssh -> conn.ssh(target.hostId, targetName).session.openTerminal(cols.toUShort(), rows.toUShort(), s)
                }
                channel = ch
                remoteSessionId = ch.sessionId()
                // 被控端给的会话标题（shell 名），程序用 OSC 设置标题后会被覆盖
                runCatching { ch.title() }.getOrNull()?.takeIf { it.isNotBlank() && session.title.isNullOrBlank() }?.let { _title.value = it }
                _status.value = TabStatus.Connected
            } catch (e: Exception) {
                _status.value = TabStatus.Failed(e.userMessage())
            }
        }
    }

    /** 断线后重连：设备会话接续原会话（先清屏，被控端会回放最近输出），SSH 新开一个。 */
    fun reconnect() {
        val attach = if (target is TargetRef.Device) remoteSessionId else null
        if (attach != null) {
            val clear = "\u001b[2J\u001b[3J\u001b[H".toByteArray()
            session.appendRemoteOutput(clear, 0, clear.size)
        }
        connect(attach)
    }

    fun detach() {
        sink?.active = false
        channel?.detach()
        channel = null
        _status.value = TabStatus.Detached
    }

    fun kill() {
        channel?.kill()
        channel = null
    }

    /** 片段、输入框整段发送。 */
    fun sendText(text: String) {
        val bytes = text.toByteArray()
        session.write(bytes, 0, bytes.size)
    }

    // ---------- TerminalSessionClient ----------

    override fun onTextChanged(changedSession: TerminalSession) {
        viewRef.get()?.onScreenUpdated()
    }

    override fun onTitleChanged(changedSession: TerminalSession) {
        _title.value = changedSession.title?.takeIf { it.isNotBlank() } ?: targetName
    }

    override fun onSessionFinished(finishedSession: TerminalSession) {
        if (_status.value !is TabStatus.Exited) _status.value = TabStatus.Exited(finishedSession.exitStatus)
        viewRef.get()?.invalidate()
    }

    override fun onCopyTextToClipboard(session: TerminalSession, text: String) {
        val cm = appContext?.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager ?: return
        cm.setPrimaryClip(ClipData.newPlainText("terminal", text))
    }

    override fun onPasteTextFromClipboard(session: TerminalSession?) {
        val cm = appContext?.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager ?: return
        val text = cm.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(appContext)?.toString() ?: return
        this.session.emulator?.paste(text)
    }

    override fun onBell(session: TerminalSession) = Unit
    override fun onColorsChanged(session: TerminalSession) {
        viewRef.get()?.invalidate()
    }

    override fun onTerminalCursorStateChange(state: Boolean) {
        viewRef.get()?.invalidate()
    }

    override fun getTerminalCursorStyle(): Int? = null
    override fun logError(tag: String?, message: String?) {
        Log.e(tag ?: TAG, message.orEmpty())
    }

    override fun logWarn(tag: String?, message: String?) {
        Log.w(tag ?: TAG, message.orEmpty())
    }

    override fun logInfo(tag: String?, message: String?) = Unit
    override fun logDebug(tag: String?, message: String?) = Unit
    override fun logVerbose(tag: String?, message: String?) = Unit
    override fun logStackTraceWithMessage(tag: String?, message: String?, e: Exception?) {
        Log.e(tag ?: TAG, message.orEmpty(), e)
    }

    override fun logStackTrace(tag: String?, e: Exception?) {
        Log.e(tag ?: TAG, "error", e)
    }

    private companion object {
        const val TAG = "Terminal"

        /** Termux 仿真器支持 256 色与真彩色。 */
        val TERMINAL_ENV = listOf(EnvVar("TERM", "xterm-256color"), EnvVar("COLORTERM", "truecolor"))
    }
}

/** 所有终端标签（可跨设备）。 */
class TerminalController(
    private val scope: CoroutineScope,
    private val conn: ConnectionManager,
    private val appContext: Context?,
    private val nameOf: (TargetRef) -> String,
) {
    private val ids = AtomicLong(1)
    private val _tabs = MutableStateFlow<List<TerminalTab>>(emptyList())
    val tabs: StateFlow<List<TerminalTab>> = _tabs.asStateFlow()
    private val _active = MutableStateFlow<Long?>(null)
    val active: StateFlow<Long?> = _active.asStateFlow()

    /** 打开终端：已有同目标的标签且不要求新建时直接切过去。 */
    fun open(target: TargetRef, attachId: String? = null, forceNew: Boolean = false): TerminalTab {
        if (attachId != null) {
            _tabs.value.firstOrNull { it.remoteSessionId == attachId }?.let {
                _active.value = it.id
                if (it.status.value !is TabStatus.Connected) it.connect(attachId)
                return it
            }
        } else if (!forceNew) {
            _tabs.value.firstOrNull { it.target == target }?.let {
                _active.value = it.id
                return it
            }
        }
        val tab = TerminalTab(ids.getAndIncrement(), target, nameOf(target), scope, conn, appContext)
        _tabs.update { it + tab }
        _active.value = tab.id
        tab.connect(attachId)
        return tab
    }

    fun select(id: Long) {
        _active.value = id
    }

    /** 关闭标签：kill=false 时只断开，被控端保留会话。 */
    fun close(tab: TerminalTab, kill: Boolean) {
        if (kill) tab.kill() else tab.detach()
        val remaining = _tabs.value.filter { it.id != tab.id }
        _tabs.value = remaining
        if (_active.value == tab.id) _active.value = remaining.lastOrNull()?.id
    }

    fun closeAll() {
        _tabs.value.forEach { it.detach() }
        _tabs.value = emptyList()
        _active.value = null
    }

    suspend fun remoteSessions(target: TargetRef.Device): List<TerminalSessionInfo> =
        conn.peer(target.deviceId).session.listTerminals()

    suspend fun killRemote(target: TargetRef.Device, sessionId: String) =
        conn.peer(target.deviceId).session.killTerminal(sessionId)
}
