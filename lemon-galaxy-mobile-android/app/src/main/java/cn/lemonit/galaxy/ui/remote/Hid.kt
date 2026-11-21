package cn.lemonit.galaxy.ui.remote

import cn.lemonit.galaxy.core.bridge.InputEvent

/** USB HID 用法码（Keyboard/Keypad Page 0x07），远程键盘与快捷组合键用。 */
object Hid {
    const val ENTER = 0x28u
    const val ESC = 0x29u
    const val BACKSPACE = 0x2Au
    const val TAB = 0x2Bu
    const val SPACE = 0x2Cu
    const val F1 = 0x3Au
    const val F4 = 0x3Du
    const val INSERT = 0x49u
    const val HOME = 0x4Au
    const val PAGE_UP = 0x4Bu
    const val DELETE = 0x4Cu
    const val END = 0x4Du
    const val PAGE_DOWN = 0x4Eu
    const val RIGHT = 0x4Fu
    const val LEFT = 0x50u
    const val DOWN = 0x51u
    const val UP = 0x52u
    const val CTRL = 0xE0u
    const val SHIFT = 0xE1u
    const val ALT = 0xE2u
    const val GUI = 0xE3u

    /** 字母 a–z → 0x04–0x1D。 */
    fun letter(c: Char): UInt = (0x04 + (c.lowercaseChar() - 'a')).toUInt()

    fun f(n: Int): UInt = (0x3A + (n - 1)).toUInt()

    /** 按下全部修饰键 → 按下并抬起主键 → 逆序抬起修饰键。 */
    fun combo(vararg keys: UInt): List<InputEvent> {
        if (keys.isEmpty()) return emptyList()
        val mods = keys.dropLast(1)
        val main = keys.last()
        return mods.map { InputEvent.Key(it, true) } +
            InputEvent.Key(main, true) + InputEvent.Key(main, false) +
            mods.reversed().map { InputEvent.Key(it, false) }
    }

    fun tap(key: UInt): List<InputEvent> = listOf(InputEvent.Key(key, true), InputEvent.Key(key, false))
}
