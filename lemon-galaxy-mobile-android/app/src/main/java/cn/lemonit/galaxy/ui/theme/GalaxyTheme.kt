package cn.lemonit.galaxy.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import cn.lemonit.galaxy.R

enum class ThemeMode(val key: String, val label: Int) {
    SYSTEM("system", R.string.theme_system),
    LIGHT("light", R.string.theme_light),
    DARK("dark", R.string.theme_dark),
    ;

    companion object {
        fun of(key: String?) = entries.firstOrNull { it.key == key } ?: SYSTEM
    }
}

/** 界面配色：每套给出深浅两种强调色，表面色共用一套偏冷的中性色（石墨为纯中性）。 */
enum class Palette(val key: String, val label: Int, val darkAccent: Color, val lightAccent: Color, val neutralTint: Boolean = true) {
    GALAXY("galaxy", R.string.palette_galaxy, Color(0xFF8AA4FF), Color(0xFF3A5BD9)),
    LEMON("lemon", R.string.palette_lemon, Color(0xFFC6E35A), Color(0xFF4F6B00)),
    OCEAN("ocean", R.string.palette_ocean, Color(0xFF4FD1C5), Color(0xFF00796B)),
    GRAPHITE("graphite", R.string.palette_graphite, Color(0xFFD0D4DE), Color(0xFF3D4350), neutralTint = false),
    ROSE("rose", R.string.palette_rose, Color(0xFFFF8FA8), Color(0xFFC2185B)),
    ;

    companion object {
        fun of(key: String?) = entries.firstOrNull { it.key == key } ?: GALAXY
    }
}

/** Material 色板之外的语义色：在线状态、风险级别等。 */
@Immutable
data class GalaxyColors(
    val online: Color,
    val lan: Color,
    val offline: Color,
    val riskRead: Color,
    val riskNormal: Color,
    val riskDangerous: Color,
    val riskUnknown: Color,
    val riskForbidden: Color,
    val success: Color,
    val warning: Color,
    val codeBackground: Color,
    val isDark: Boolean,
)

private val DarkGalaxyColors = GalaxyColors(
    online = Color(0xFF3DD68C), lan = Color(0xFF5AA9FF), offline = Color(0xFF6B7085),
    riskRead = Color(0xFF3DD68C), riskNormal = Color(0xFF5AA9FF), riskDangerous = Color(0xFFFF8A4C),
    riskUnknown = Color(0xFFE5B94E), riskForbidden = Color(0xFFFF5C6C),
    success = Color(0xFF3DD68C), warning = Color(0xFFE5B94E), codeBackground = Color(0xFF0E1018), isDark = true,
)

private val LightGalaxyColors = GalaxyColors(
    online = Color(0xFF16A34A), lan = Color(0xFF2563EB), offline = Color(0xFF9AA0B0),
    riskRead = Color(0xFF15803D), riskNormal = Color(0xFF2563EB), riskDangerous = Color(0xFFD9480F),
    riskUnknown = Color(0xFFB7791F), riskForbidden = Color(0xFFDC2626),
    success = Color(0xFF16A34A), warning = Color(0xFFB7791F), codeBackground = Color(0xFFF1F3F8), isDark = false,
)

val LocalGalaxyColors = staticCompositionLocalOf { DarkGalaxyColors }

object Galaxy {
    val colors: GalaxyColors @Composable get() = LocalGalaxyColors.current
}

private fun mix(a: Color, b: Color, t: Float) = Color(
    red = a.red + (b.red - a.red) * t,
    green = a.green + (b.green - a.green) * t,
    blue = a.blue + (b.blue - a.blue) * t,
    alpha = 1f,
)

fun schemeFor(palette: Palette, dark: Boolean): ColorScheme {
    if (dark) {
        val accent = palette.darkAccent
        // 深色表面：Termius 式偏蓝的近黑
        val base = if (palette.neutralTint) Color(0xFF11131D) else Color(0xFF121212)
        val step = if (palette.neutralTint) Color(0xFF3A3F58) else Color(0xFF444444)
        fun s(t: Float) = mix(base, step, t)
        return darkColorScheme(
            primary = accent,
            onPrimary = mix(accent, Color.Black, 0.82f),
            primaryContainer = mix(base, accent, 0.28f),
            onPrimaryContainer = mix(accent, Color.White, 0.6f),
            secondary = mix(accent, Color(0xFFB8BCCB), 0.55f),
            onSecondary = Color(0xFF15171F),
            secondaryContainer = mix(base, accent, 0.16f),
            onSecondaryContainer = Color(0xFFE2E5F0),
            tertiary = Color(0xFFE5B94E),
            onTertiary = Color(0xFF2A2000),
            background = base,
            onBackground = Color(0xFFE6E8F0),
            surface = base,
            onSurface = Color(0xFFE6E8F0),
            surfaceVariant = s(0.30f),
            onSurfaceVariant = Color(0xFFA3A8BC),
            surfaceTint = accent,
            inverseSurface = Color(0xFFE6E8F0),
            inverseOnSurface = Color(0xFF1A1C24),
            outline = s(0.55f),
            outlineVariant = s(0.30f),
            error = Color(0xFFFF6B78),
            onError = Color(0xFF3A0008),
            errorContainer = Color(0xFF5C1520),
            onErrorContainer = Color(0xFFFFD9DC),
            scrim = Color.Black,
            surfaceBright = s(0.32f),
            surfaceDim = base,
            surfaceContainerLowest = mix(base, Color.Black, 0.3f),
            surfaceContainerLow = s(0.06f),
            surfaceContainer = s(0.11f),
            surfaceContainerHigh = s(0.17f),
            surfaceContainerHighest = s(0.24f),
        )
    }
    val accent = palette.lightAccent
    val tint = if (palette.neutralTint) Color(0xFFE9ECF5) else Color(0xFFEDEDED)
    return lightColorScheme(
        primary = accent,
        onPrimary = Color.White,
        primaryContainer = mix(Color.White, accent, 0.14f),
        onPrimaryContainer = mix(accent, Color.Black, 0.45f),
        secondary = mix(accent, Color(0xFF5A6072), 0.6f),
        onSecondary = Color.White,
        secondaryContainer = mix(Color.White, accent, 0.09f),
        onSecondaryContainer = Color(0xFF1A1C24),
        tertiary = Color(0xFF8A5A00),
        onTertiary = Color.White,
        background = if (palette.neutralTint) Color(0xFFF6F7FB) else Color(0xFFF6F6F6),
        onBackground = Color(0xFF1A1C24),
        surface = if (palette.neutralTint) Color(0xFFF6F7FB) else Color(0xFFF6F6F6),
        onSurface = Color(0xFF1A1C24),
        surfaceVariant = tint,
        onSurfaceVariant = Color(0xFF5A6072),
        surfaceTint = accent,
        inverseSurface = Color(0xFF2B2E3A),
        inverseOnSurface = Color(0xFFF2F3F8),
        outline = Color(0xFFBFC4D2),
        outlineVariant = Color(0xFFDDE0E8),
        error = Color(0xFFD32F2F),
        onError = Color.White,
        errorContainer = Color(0xFFFDE2E2),
        onErrorContainer = Color(0xFF5C0A0A),
        scrim = Color.Black,
        surfaceBright = Color.White,
        surfaceDim = Color(0xFFE2E4EC),
        surfaceContainerLowest = Color.White,
        surfaceContainerLow = Color(0xFFFFFFFF),
        surfaceContainer = mix(Color.White, tint, 0.5f),
        surfaceContainerHigh = mix(Color.White, tint, 0.8f),
        surfaceContainerHighest = tint,
    )
}

private val GalaxyTypography = Typography().let { t ->
    t.copy(
        titleLarge = t.titleLarge.copy(fontWeight = FontWeight.SemiBold, fontSize = 22.sp),
        titleMedium = t.titleMedium.copy(fontWeight = FontWeight.SemiBold),
        titleSmall = t.titleSmall.copy(fontWeight = FontWeight.SemiBold),
        labelLarge = t.labelLarge.copy(fontWeight = FontWeight.Medium),
    )
}

val MonoStyle = TextStyle(fontFamily = FontFamily.Monospace, fontSize = 13.sp, lineHeight = 18.sp)

private val GalaxyShapes = Shapes(
    extraSmall = RoundedCornerShape(6.dp),
    small = RoundedCornerShape(8.dp),
    medium = RoundedCornerShape(12.dp),
    large = RoundedCornerShape(16.dp),
    extraLarge = RoundedCornerShape(24.dp),
)

@Composable
fun isDarkTheme(mode: ThemeMode): Boolean = when (mode) {
    ThemeMode.SYSTEM -> isSystemInDarkTheme()
    ThemeMode.LIGHT -> false
    ThemeMode.DARK -> true
}

@Composable
fun GalaxyTheme(
    mode: ThemeMode = ThemeMode.SYSTEM,
    palette: Palette = Palette.GALAXY,
    dynamicColor: Boolean = false,
    content: @Composable () -> Unit,
) {
    val dark = isDarkTheme(mode)
    val scheme = if (dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        val ctx = LocalContext.current
        if (dark) dynamicDarkColorScheme(ctx) else dynamicLightColorScheme(ctx)
    } else {
        schemeFor(palette, dark)
    }
    CompositionLocalProvider(LocalGalaxyColors provides if (dark) DarkGalaxyColors else LightGalaxyColors) {
        MaterialTheme(colorScheme = scheme, typography = GalaxyTypography, shapes = GalaxyShapes, content = content)
    }
}
