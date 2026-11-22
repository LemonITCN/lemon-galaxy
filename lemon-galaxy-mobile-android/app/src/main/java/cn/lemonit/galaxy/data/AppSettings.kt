package cn.lemonit.galaxy.data

import android.content.Context
import android.content.SharedPreferences
import androidx.appcompat.app.AppCompatDelegate
import androidx.core.os.LocaleListCompat
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.ui.theme.Palette
import cn.lemonit.galaxy.ui.theme.TerminalThemes
import cn.lemonit.galaxy.ui.theme.ThemeMode
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** 远程画面画质档位：码率与帧率上限。 */
enum class VideoQuality(val key: String, val label: Int, val bitrateKbps: UInt, val maxFps: UInt) {
    AUTO("auto", R.string.quality_auto, 0u, 60u),
    SMOOTH("smooth", R.string.quality_smooth, 2_000u, 30u),
    BALANCED("balanced", R.string.quality_balanced, 6_000u, 60u),
    HD("hd", R.string.quality_hd, 12_000u, 60u),
    ULTRA("ultra", R.string.quality_ultra, 30_000u, 60u),
    ;

    companion object {
        fun of(key: String?) = entries.firstOrNull { it.key == key } ?: AUTO
    }
}

enum class AppLanguage(val tag: String, val label: Int) {
    SYSTEM("", R.string.lang_system),
    ZH("zh-CN", R.string.lang_zh),
    EN("en", R.string.lang_en),
}

data class Prefs(
    val themeMode: ThemeMode = ThemeMode.SYSTEM,
    val palette: Palette = Palette.GALAXY,
    val dynamicColor: Boolean = false,
    val terminalTheme: String = TerminalThemes.DEFAULT_DARK,
    val terminalFontSize: Int = 14,
    val appLock: Boolean = false,
    val defaultQuality: VideoQuality = VideoQuality.AUTO,
    /** AI 输入框上次选的档位（也是设置页里的默认档位）；用时不超过管理员上限。 */
    val defaultAiMode: AiMode = AiMode.CONFIRM_DANGEROUS,
    /** AI 输入框上次选的模型（供应商 ID 为 -1 表示没选过）。 */
    val aiProviderId: Long = -1L,
    val aiModelId: String = "",
    /** AI 输入框「先出计划」勾选。 */
    val aiPlanFirst: Boolean = false,
    /** 完全信任的时限（分钟），-1 表示本对话内一直有效。 */
    val aiTrustMinutes: Int = -1,
    /** 远程桌面操作方式：鼠标（触控板式，默认）或触控；菜单里切换后记住 */
    val remoteMouseMode: Boolean = true,
    /** 鼠标模式下画面左侧的悬浮鼠标按键（左键 / 右键），工具里开关，记住选择 */
    val remoteMouseKeys: Boolean = true,
    /** 悬浮鼠标按键的位置（在画面可用区域里的比例） */
    val mouseKeysX: Float = 0f,
    val mouseKeysY: Float = 0.3f,
    /** 远程桌面播放被控端声音（默认开，记住用户的选择） */
    val remoteAudio: Boolean = true,
    val terminalInputBox: Boolean = false,
    val lastServer: String = "",
    val lastUsername: String = "",
)

/**
 * 界面偏好（本机）。修改立即写盘并推给界面，主题、终端配色、字号即时生效。
 * 语言交给 AppCompat 的按应用语言（Android 13+ 与系统设置联动）。
 */
class AppSettings(private val sp: SharedPreferences?) {
    private val _prefs = MutableStateFlow(load())
    val prefs: StateFlow<Prefs> = _prefs.asStateFlow()

    private fun load(): Prefs {
        val p = sp ?: return Prefs()
        val d = Prefs()
        return Prefs(
            themeMode = ThemeMode.of(p.getString("theme_mode", null)),
            palette = Palette.of(p.getString("palette", null)),
            dynamicColor = p.getBoolean("dynamic_color", d.dynamicColor),
            terminalTheme = p.getString("terminal_theme", null) ?: d.terminalTheme,
            terminalFontSize = p.getInt("terminal_font_size", d.terminalFontSize),
            appLock = p.getBoolean("app_lock", d.appLock),
            defaultQuality = VideoQuality.of(p.getString("default_quality", null)),
            defaultAiMode = runCatching { AiMode.valueOf(p.getString("default_ai_mode", null) ?: "") }.getOrDefault(d.defaultAiMode),
            aiProviderId = p.getLong("ai_provider_id", d.aiProviderId),
            aiModelId = p.getString("ai_model_id", null) ?: d.aiModelId,
            aiPlanFirst = p.getBoolean("ai_plan_first", d.aiPlanFirst),
            aiTrustMinutes = p.getInt("ai_trust_minutes", d.aiTrustMinutes),
            // 新键：旧版 remote_mouse_mode 默认是触控且每次存设置都会写进去，沿用它就改不成默认鼠标
            remoteMouseMode = when (p.getString("remote_input_mode", null)) {
                "mouse" -> true
                "touch" -> false
                else -> d.remoteMouseMode
            },
            remoteMouseKeys = p.getBoolean("remote_mouse_keys", d.remoteMouseKeys),
            mouseKeysX = p.getFloat("mouse_keys_x", d.mouseKeysX),
            mouseKeysY = p.getFloat("mouse_keys_y", d.mouseKeysY),
            remoteAudio = p.getBoolean("remote_audio", d.remoteAudio),
            terminalInputBox = p.getBoolean("terminal_input_box", d.terminalInputBox),
            lastServer = p.getString("last_server", null) ?: "",
            lastUsername = p.getString("last_username", null) ?: "",
        )
    }

    fun update(block: (Prefs) -> Prefs) {
        val n = block(_prefs.value)
        _prefs.value = n
        sp?.edit()?.apply {
            putString("theme_mode", n.themeMode.key)
            putString("palette", n.palette.key)
            putBoolean("dynamic_color", n.dynamicColor)
            putString("terminal_theme", n.terminalTheme)
            putInt("terminal_font_size", n.terminalFontSize)
            putBoolean("app_lock", n.appLock)
            putString("default_quality", n.defaultQuality.key)
            putString("default_ai_mode", n.defaultAiMode.name)
            putLong("ai_provider_id", n.aiProviderId)
            putString("ai_model_id", n.aiModelId)
            putBoolean("ai_plan_first", n.aiPlanFirst)
            putInt("ai_trust_minutes", n.aiTrustMinutes)
            putString("remote_input_mode", if (n.remoteMouseMode) "mouse" else "touch")
            remove("remote_mouse_mode")
            putBoolean("remote_mouse_keys", n.remoteMouseKeys)
            putFloat("mouse_keys_x", n.mouseKeysX)
            putFloat("mouse_keys_y", n.mouseKeysY)
            putBoolean("remote_audio", n.remoteAudio)
            putBoolean("terminal_input_box", n.terminalInputBox)
            putString("last_server", n.lastServer)
            putString("last_username", n.lastUsername)
            apply()
        }
    }

    fun language(): AppLanguage {
        val tags = AppCompatDelegate.getApplicationLocales().toLanguageTags()
        return when {
            tags.isEmpty() -> AppLanguage.SYSTEM
            tags.startsWith("zh") -> AppLanguage.ZH
            else -> AppLanguage.EN
        }
    }

    fun setLanguage(lang: AppLanguage) {
        AppCompatDelegate.setApplicationLocales(
            if (lang == AppLanguage.SYSTEM) LocaleListCompat.getEmptyLocaleList() else LocaleListCompat.forLanguageTags(lang.tag),
        )
    }

    companion object {
        const val TERMINAL_FONT_MIN = 8
        const val TERMINAL_FONT_MAX = 28

        fun create(context: Context) = AppSettings(context.getSharedPreferences("settings", Context.MODE_PRIVATE))

        /** 测试与预览用，不落盘。 */
        fun inMemory(initial: Prefs = Prefs()) = AppSettings(null).also { s -> s.update { initial } }
    }
}
