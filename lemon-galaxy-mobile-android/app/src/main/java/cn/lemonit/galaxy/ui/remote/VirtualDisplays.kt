package cn.lemonit.galaxy.ui.remote

import kotlin.math.max
import kotlin.math.min

/** 分辨率（宽 × 高，像素）。 */
data class VdSize(val width: Int, val height: Int) {
    val label: String get() = "$width×$height"
}

/**
 * 加虚拟屏的档位与合法范围（docs/protocol.md 8.6，与内核 galaxy-proto `virtual_display_mode_ok` 一致）。
 * 界面不传 0 刷新率，默认 60。
 */
object VirtualDisplays {
    const val MIN_WIDTH = 640
    const val MAX_WIDTH = 7680
    const val MIN_HEIGHT = 480
    const val MAX_HEIGHT = 4320
    const val MIN_REFRESH = 24
    const val MAX_REFRESH = 240

    val DEFAULT_SIZE = VdSize(1920, 1080)
    const val DEFAULT_REFRESH = 60

    /** 常用分辨率档位（四端相同）。 */
    val PRESETS = listOf(
        VdSize(1280, 720),
        VdSize(1920, 1080),
        VdSize(2560, 1440),
        VdSize(3840, 2160),
        VdSize(1920, 1200),
        VdSize(2560, 1600),
    )

    /** 刷新率档位（Hz）。 */
    val REFRESH_RATES = listOf(30, 60, 120, 144)

    fun sizeOk(width: Int, height: Int): Boolean = width in MIN_WIDTH..MAX_WIDTH && height in MIN_HEIGHT..MAX_HEIGHT

    fun refreshOk(hz: Int): Boolean = hz in MIN_REFRESH..MAX_REFRESH

    /** 「本机屏幕」档位：本机主屏物理像素取横屏方向（长边为宽）；超出合法范围返回 null（不显示这一项）。 */
    fun localScreen(pxWidth: Int, pxHeight: Int): VdSize? {
        val w = max(pxWidth, pxHeight)
        val h = min(pxWidth, pxHeight)
        return if (sizeOk(w, h)) VdSize(w, h) else null
    }

    /** 数字输入框只留数字，最多 [maxLength] 位（宽高 4 位、刷新率 3 位）。 */
    fun digits(input: String, maxLength: Int): String = input.filter { it in '0'..'9' }.take(maxLength)
}

/** 分辨率选哪一项。 */
sealed interface VdResolution {
    data class Preset(val size: VdSize) : VdResolution

    /** 本机屏幕 */
    data object Local : VdResolution

    /** 自定义：宽、高两个输入框 */
    data object Custom : VdResolution
}

/**
 * 加虚拟屏表单（纯逻辑，界面与单元测试共用）。默认 1920×1080 @ 60 Hz。
 * 自定义的宽、高、刷新率保存输入框里的原文，提交时再解析校验。
 */
data class VirtualDisplayForm(
    val resolution: VdResolution = VdResolution.Preset(VirtualDisplays.DEFAULT_SIZE),
    val width: String = "",
    val height: String = "",
    /** 选中的刷新率档位；null 表示自定义（看 [customRefresh]）。 */
    val refresh: Int? = VirtualDisplays.DEFAULT_REFRESH,
    val customRefresh: String = "",
) {
    fun selectPreset(size: VdSize) = copy(resolution = VdResolution.Preset(size))

    fun selectLocal() = copy(resolution = VdResolution.Local)

    /** 选自定义：输入框还空着时先填上当前选中的分辨率，改一改就行。 */
    fun selectCustom(local: VdSize?): VirtualDisplayForm {
        if (resolution == VdResolution.Custom) return this
        val current = size(local)
        return if (width.isEmpty() && height.isEmpty() && current != null) {
            copy(resolution = VdResolution.Custom, width = current.width.toString(), height = current.height.toString())
        } else {
            copy(resolution = VdResolution.Custom)
        }
    }

    fun withWidth(input: String) = copy(width = VirtualDisplays.digits(input, 4))

    fun withHeight(input: String) = copy(height = VirtualDisplays.digits(input, 4))

    fun selectRefresh(hz: Int) = copy(refresh = hz)

    /** 选自定义刷新率：输入框还空着时先填上当前档位。 */
    fun selectCustomRefresh(): VirtualDisplayForm {
        val current = refresh ?: return this
        return copy(refresh = null, customRefresh = customRefresh.ifEmpty { current.toString() })
    }

    fun withCustomRefresh(input: String) = copy(customRefresh = VirtualDisplays.digits(input, 3))

    /** 要加的分辨率；自定义值不合法、或本机屏幕不可用时为 null。 */
    fun size(local: VdSize?): VdSize? = when (val r = resolution) {
        is VdResolution.Preset -> r.size
        VdResolution.Local -> local
        VdResolution.Custom -> {
            val w = width.toIntOrNull()
            val h = height.toIntOrNull()
            if (w != null && h != null && VirtualDisplays.sizeOk(w, h)) VdSize(w, h) else null
        }
    }

    /** 要加的刷新率；自定义值不合法时为 null。 */
    fun refreshHz(): Int? = refresh ?: customRefresh.toIntOrNull()?.takeIf(VirtualDisplays::refreshOk)

    /** 自定义分辨率不合法（含没填）：按钮置灰，输入框下方显示范围。 */
    val sizeInvalid: Boolean get() = resolution == VdResolution.Custom && size(null) == null

    /** 自定义刷新率不合法（含没填）。 */
    val refreshInvalid: Boolean get() = refresh == null && refreshHz() == null

    /** 「加虚拟屏」按钮能不能点：值合法、没加满、不在进行中。 */
    fun canSubmit(local: VdSize?, added: Int, max: Int, busy: Boolean): Boolean =
        !busy && max > 0 && added < max && size(local) != null && refreshHz() != null
}
