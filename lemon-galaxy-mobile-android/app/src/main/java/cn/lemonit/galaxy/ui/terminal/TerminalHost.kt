package cn.lemonit.galaxy.ui.terminal

import android.content.Context
import android.util.Log
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.inputmethod.InputMethodManager
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.viewinterop.AndroidView
import cn.lemonit.galaxy.ui.theme.TerminalTheme
import com.termux.terminal.KeyHandler
import com.termux.terminal.TerminalColors
import com.termux.terminal.TerminalSession
import com.termux.view.TerminalView
import com.termux.view.TerminalViewClient

/** 快捷键栏上的 Ctrl / Alt 一次性修饰键（按一下对下一个键生效）。 */
class ModifierKeys {
    var ctrl = androidx.compose.runtime.mutableStateOf(false)
    var alt = androidx.compose.runtime.mutableStateOf(false)

    fun consumeCtrl(): Boolean = ctrl.value.also { ctrl.value = false }
    fun consumeAlt(): Boolean = alt.value.also { alt.value = false }

    fun keyMod(): Int {
        var m = 0
        if (consumeCtrl()) m = m or KeyHandler.KEYMOD_CTRL
        if (consumeAlt()) m = m or KeyHandler.KEYMOD_ALT
        return m
    }
}

/** 把终端配色写进 Termux 的全局配色表，并让已有会话立即换色。 */
object TerminalColorsApplier {
    @Volatile private var appliedId: String? = null

    fun apply(theme: TerminalTheme, sessions: List<TerminalSession>) {
        if (appliedId != theme.id) {
            TerminalColors.COLOR_SCHEME.updateWith(theme.toTermuxProperties())
            appliedId = theme.id
        }
        sessions.forEach { s -> s.emulator?.mColors?.reset() }
    }
}

private class GalaxyTerminalViewClient(
    private val view: () -> TerminalView?,
    private val mods: ModifierKeys,
    private val onFontScale: (Boolean) -> Unit,
) : TerminalViewClient {
    override fun onScale(scale: Float): Float {
        if (scale < 0.9f || scale > 1.1f) {
            onFontScale(scale > 1f)
            return 1.0f
        }
        return scale
    }

    override fun onSingleTapUp(e: MotionEvent?) {
        val v = view() ?: return
        v.requestFocus()
        (v.context.getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager)?.showSoftInput(v, 0)
    }

    override fun shouldBackButtonBeMappedToEscape() = false
    override fun shouldEnforceCharBasedInput() = true
    override fun shouldUseCtrlSpaceWorkaround() = false
    override fun isTerminalViewSelected() = true
    override fun copyModeChanged(copyMode: Boolean) = Unit
    override fun onKeyDown(keyCode: Int, e: KeyEvent?, session: TerminalSession?) = false
    override fun onKeyUp(keyCode: Int, e: KeyEvent?) = false
    override fun onLongPress(event: MotionEvent?) = false
    override fun readControlKey() = mods.consumeCtrl()
    override fun readAltKey() = mods.consumeAlt()
    override fun readShiftKey() = false
    override fun readFnKey() = false
    override fun onCodePoint(codePoint: Int, ctrlDown: Boolean, session: TerminalSession?) = false
    override fun onEmulatorSet() = Unit

    override fun logError(tag: String?, message: String?) {
        Log.e(tag ?: "TerminalView", message.orEmpty())
    }

    override fun logWarn(tag: String?, message: String?) = Unit
    override fun logInfo(tag: String?, message: String?) = Unit
    override fun logDebug(tag: String?, message: String?) = Unit
    override fun logVerbose(tag: String?, message: String?) = Unit
    override fun logStackTraceWithMessage(tag: String?, message: String?, e: Exception?) {
        Log.e(tag ?: "TerminalView", message.orEmpty(), e)
    }

    override fun logStackTrace(tag: String?, e: Exception?) {
        Log.e(tag ?: "TerminalView", "error", e)
    }
}

/** 引用当前终端控件，供快捷键栏发键。 */
class TerminalViewRef {
    var view: TerminalView? = null

    fun sendKey(keyCode: Int, mods: ModifierKeys) {
        val v = view ?: return
        v.handleKeyCode(keyCode, mods.keyMod())
    }

    fun sendChar(c: String, mods: ModifierKeys) {
        val v = view ?: return
        val ctrl = mods.consumeCtrl()
        val alt = mods.consumeAlt()
        c.codePoints().forEach { v.inputCodePoint(it, ctrl, alt) }
    }

    fun showKeyboard() {
        val v = view ?: return
        v.requestFocus()
        (v.context.getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager)?.showSoftInput(v, 0)
    }
}

/** Compose 里托管 Termux TerminalView。 */
@Composable
fun TerminalViewHost(
    session: TerminalSession,
    theme: TerminalTheme,
    fontSizeSp: Int,
    mods: ModifierKeys,
    ref: TerminalViewRef,
    onFontScale: (Boolean) -> Unit,
    onViewBound: (TerminalView?) -> Unit,
    modifier: Modifier = Modifier,
) {
    val density = LocalDensity.current
    val px = with(density) { fontSizeSp.toFloat() * fontScale * this.density }.toInt().coerceAtLeast(8)
    val client = remember(mods) { GalaxyTerminalViewClient({ ref.view }, mods, onFontScale) }
    AndroidView(
        factory = { ctx ->
            TerminalView(ctx, null).apply {
                setTerminalViewClient(client)
                isFocusable = true
                isFocusableInTouchMode = true
                setTextSize(px)
                keepScreenOn = true
            }
        },
        update = { v ->
            ref.view = v
            if (v.currentSession !== session) {
                v.attachSession(session)
                onViewBound(v)
                v.tag = null
            }
            // 只在配色或字号变化时重设，避免冲掉程序用 OSC 设置的颜色
            val key = "${theme.id}:$px"
            if (v.tag != key) {
                v.setBackgroundColor(theme.backgroundColor.toArgb())
                TerminalColorsApplier.apply(theme, listOf(session))
                v.setTextSize(px)
                v.tag = key
            }
            v.invalidate()
        },
        onRelease = {
            if (ref.view === it) ref.view = null
            onViewBound(null)
        },
        modifier = modifier,
    )
}
