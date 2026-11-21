package cn.lemonit.galaxy.ui.theme

import android.content.Context
import androidx.compose.ui.graphics.Color
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.util.Properties

/**
 * 终端配色。每套一个 JSON 文件放在 assets/terminal-themes/，加新配色只需加文件：
 * { "id", "name", "dark", "order", "foreground", "background", "cursor", "colors": [16 个 #RRGGBB] }
 */
@Serializable
data class TerminalTheme(
    val id: String,
    val name: String,
    val dark: Boolean,
    val order: Int = 100,
    val foreground: String,
    val background: String,
    val cursor: String,
    val colors: List<String>,
) {
    val backgroundColor: Color get() = parse(background)
    val foregroundColor: Color get() = parse(foreground)
    val cursorColor: Color get() = parse(cursor)
    fun ansi(i: Int): Color = parse(colors[i])

    /** Termux TerminalColorScheme.updateWith() 需要的属性表。 */
    fun toTermuxProperties(): Properties = Properties().apply {
        setProperty("foreground", foreground)
        setProperty("background", background)
        setProperty("cursor", cursor)
        colors.forEachIndexed { i, c -> setProperty("color$i", c) }
    }

    companion object {
        fun parse(hex: String): Color = Color(android.graphics.Color.parseColor(hex))
    }
}

object TerminalThemes {
    const val DEFAULT_DARK = "lemon-dark"
    const val DEFAULT_LIGHT = "lemon-light"
    private const val DIR = "terminal-themes"
    private val json = Json { ignoreUnknownKeys = true }

    @Volatile
    private var cache: List<TerminalTheme>? = null

    fun all(context: Context): List<TerminalTheme> {
        cache?.let { return it }
        val list = (context.assets.list(DIR) ?: emptyArray())
            .filter { it.endsWith(".json") }
            .mapNotNull { name ->
                runCatching {
                    json.decodeFromString(TerminalTheme.serializer(), context.assets.open("$DIR/$name").use { it.readBytes().decodeToString() })
                }.getOrNull()
            }
            .filter { it.colors.size >= 16 }
            .sortedWith(compareBy({ it.order }, { it.name }))
        cache = list
        return list
    }

    fun find(context: Context, id: String?): TerminalTheme {
        val all = all(context)
        return all.firstOrNull { it.id == id } ?: all.firstOrNull { it.id == DEFAULT_DARK } ?: FALLBACK
    }

    val FALLBACK = TerminalTheme(
        DEFAULT_DARK, "LemonGalaxy Dark", true, 0, "#D7DAE5", "#141622", "#C6E35A",
        listOf(
            "#1E2130", "#F2777A", "#99CC99", "#FFCC66", "#6699CC", "#CC99CC", "#66CCCC", "#D3D0C8",
            "#747369", "#F99157", "#B5E08C", "#FFD98A", "#8AB4E8", "#E2B5E2", "#8AE0E0", "#F2F0EC",
        ),
    )
}
