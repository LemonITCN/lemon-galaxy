import Foundation

/// USB HID 键盘用法码（Usage Page 0x07）。远程桌面的键盘、特殊键与快捷组合键都按它发（protocol.md 8.4）。
/// iOS 外接键盘的 UIKey.keyCode 本身就是这套用法码，可以直接转发。
public enum Hid {
    public static let enter: UInt32 = 0x28
    public static let esc: UInt32 = 0x29
    public static let backspace: UInt32 = 0x2A
    public static let tab: UInt32 = 0x2B
    public static let space: UInt32 = 0x2C
    public static let insert: UInt32 = 0x49
    public static let home: UInt32 = 0x4A
    public static let pageUp: UInt32 = 0x4B
    public static let delete: UInt32 = 0x4C
    public static let end: UInt32 = 0x4D
    public static let pageDown: UInt32 = 0x4E
    public static let right: UInt32 = 0x4F
    public static let left: UInt32 = 0x50
    public static let down: UInt32 = 0x51
    public static let up: UInt32 = 0x52
    public static let printScreen: UInt32 = 0x46
    public static let ctrl: UInt32 = 0xE0
    public static let shift: UInt32 = 0xE1
    public static let alt: UInt32 = 0xE2
    public static let gui: UInt32 = 0xE3

    /// 字母 a–z → 0x04–0x1D
    public static func letter(_ c: Character) -> UInt32 {
        let v = c.lowercased().unicodeScalars.first!.value
        return 0x04 + (v - 0x61)
    }

    /// F1–F12 → 0x3A–0x45
    public static func f(_ n: Int) -> UInt32 { UInt32(0x3A + (n - 1)) }

    /// 修饰键（用来判断外接键盘的按键是不是修饰键）
    public static func isModifier(_ usage: UInt32) -> Bool { (0xE0...0xE7).contains(usage) }

    /// 按下全部修饰键 → 按下并抬起主键 → 逆序抬起修饰键
    public static func combo(_ keys: [UInt32]) -> [InputEvent] {
        guard let main = keys.last else { return [] }
        let mods = keys.dropLast()
        return mods.map { InputEvent.key(hidUsage: $0, down: true) }
            + [.key(hidUsage: main, down: true), .key(hidUsage: main, down: false)]
            + mods.reversed().map { InputEvent.key(hidUsage: $0, down: false) }
    }

    public static func combo(_ keys: UInt32...) -> [InputEvent] { combo(keys) }

    public static func tap(_ key: UInt32) -> [InputEvent] { [.key(hidUsage: key, down: true), .key(hidUsage: key, down: false)] }

    /// 美式键盘上一个 ASCII 字符对应的（用法码, 是否要 Shift）。远程键盘按了 Ctrl/Alt 等修饰键时，字符要按键码发。
    public static func usage(for ch: Character) -> (UInt32, Bool)? {
        guard let s = ch.unicodeScalars.first, ch.unicodeScalars.count == 1, s.isASCII else { return nil }
        let v = s.value
        switch v {
        case 0x61...0x7A: return (0x04 + v - 0x61, false)
        case 0x41...0x5A: return (0x04 + v - 0x41, true)
        case 0x31...0x39: return (0x1E + v - 0x31, false)
        case 0x30: return (0x27, false)
        case 0x0A, 0x0D: return (enter, false)
        case 0x09: return (tab, false)
        case 0x20: return (space, false)
        default: break
        }
        let table: [Character: (UInt32, Bool)] = [
            "!": (0x1E, true), "@": (0x1F, true), "#": (0x20, true), "$": (0x21, true), "%": (0x22, true),
            "^": (0x23, true), "&": (0x24, true), "*": (0x25, true), "(": (0x26, true), ")": (0x27, true),
            "-": (0x2D, false), "_": (0x2D, true), "=": (0x2E, false), "+": (0x2E, true),
            "[": (0x2F, false), "{": (0x2F, true), "]": (0x30, false), "}": (0x30, true),
            "\\": (0x31, false), "|": (0x31, true), ";": (0x33, false), ":": (0x33, true),
            "'": (0x34, false), "\"": (0x34, true), "`": (0x35, false), "~": (0x35, true),
            ",": (0x36, false), "<": (0x36, true), ".": (0x37, false), ">": (0x37, true),
            "/": (0x38, false), "?": (0x38, true),
        ]
        return table[ch]
    }
}

/// 远程桌面「快捷键」菜单：按被控端平台给出常用组合键。
public struct RemoteShortcut: Identifiable {
    public let id: String          // 文字的键：combo.<id>
    public let events: [InputEvent]
    public let group: Int          // 0 系统，1 编辑
}

public enum RemoteShortcuts {
    public static func list(for platform: Platform) -> [RemoteShortcut] {
        let mac = platform == .macos
        let mod = mac ? Hid.gui : Hid.ctrl
        let edit: [RemoteShortcut] = [
            RemoteShortcut(id: "copy", events: Hid.combo(mod, Hid.letter("c")), group: 1),
            RemoteShortcut(id: "paste", events: Hid.combo(mod, Hid.letter("v")), group: 1),
            RemoteShortcut(id: "cut", events: Hid.combo(mod, Hid.letter("x")), group: 1),
            RemoteShortcut(id: "undo", events: Hid.combo(mod, Hid.letter("z")), group: 1),
            RemoteShortcut(id: "selectAll", events: Hid.combo(mod, Hid.letter("a")), group: 1),
        ]
        if mac {
            return [
                RemoteShortcut(id: "cmdTab", events: Hid.combo(Hid.gui, Hid.tab), group: 0),
                RemoteShortcut(id: "spotlight", events: Hid.combo(Hid.gui, Hid.space), group: 0),
                RemoteShortcut(id: "missionControl", events: Hid.combo(Hid.ctrl, Hid.up), group: 0),
                RemoteShortcut(id: "closeWindow", events: Hid.combo(Hid.gui, Hid.letter("w")), group: 0),
                RemoteShortcut(id: "quitApp", events: Hid.combo(Hid.gui, Hid.letter("q")), group: 0),
                RemoteShortcut(id: "forceQuit", events: Hid.combo(Hid.gui, Hid.alt, Hid.esc), group: 0),
                RemoteShortcut(id: "lock", events: [.specialCombo(combo: .lockScreen)], group: 0),
            ] + edit
        }
        return [
            RemoteShortcut(id: "ctrlAltDel", events: [.specialCombo(combo: .ctrlAltDel)], group: 0),
            RemoteShortcut(id: "win", events: [.specialCombo(combo: .winKey)], group: 0),
            RemoteShortcut(id: "altTab", events: [.specialCombo(combo: .altTab)], group: 0),
            RemoteShortcut(id: "altF4", events: Hid.combo(Hid.alt, Hid.f(4)), group: 0),
            RemoteShortcut(id: "taskManager", events: [.specialCombo(combo: .taskManager)], group: 0),
            RemoteShortcut(id: "showDesktop", events: [.specialCombo(combo: .showDesktop)], group: 0),
            RemoteShortcut(id: "lock", events: [.specialCombo(combo: .lockScreen)], group: 0),
        ] + edit
    }
}

/// 终端特殊键 → 发给远端 shell 的字节（xterm 序列）。
public enum TerminalKey: String, CaseIterable {
    case esc, tab, up, down, left, right, home, end, pageUp, pageDown, insert, delete, backspace, enter
    case f1, f2, f3, f4, f5, f6, f7, f8, f9, f10, f11, f12
}

public enum TerminalKeys {
    static let esc: UInt8 = 0x1B

    /// - appCursor: 远端开了应用光标模式（DECCKM，vim / less 等全屏程序会开）时方向键用 ESC O x
    /// - ctrl / alt / shift：xterm 修饰参数（方向键等带修饰时用 CSI 1;m x）
    public static func bytes(_ key: TerminalKey, appCursor: Bool = false, ctrl: Bool = false, alt: Bool = false, shift: Bool = false) -> [UInt8] {
        let mod: Int = 1 + (shift ? 1 : 0) + (alt ? 2 : 0) + (ctrl ? 4 : 0)
        func csi(_ s: String) -> [UInt8] { [esc] + Array(("[" + s).utf8) }
        func cursor(_ c: String) -> [UInt8] {
            if mod > 1 { return csi("1;\(mod)" + c) }
            return appCursor ? [esc] + Array(("O" + c).utf8) : csi(c)
        }
        func tilde(_ n: Int) -> [UInt8] { mod > 1 ? csi("\(n);\(mod)~") : csi("\(n)~") }
        func ss3(_ c: String) -> [UInt8] { mod > 1 ? csi("1;\(mod)" + c) : [esc] + Array(("O" + c).utf8) }
        var out: [UInt8]
        switch key {
        case .esc: out = [esc]
        case .tab: out = shift ? csi("Z") : [0x09]
        case .enter: out = [0x0D]
        case .backspace: out = [0x7F]
        case .up: out = cursor("A")
        case .down: out = cursor("B")
        case .right: out = cursor("C")
        case .left: out = cursor("D")
        case .home: out = cursor("H")
        case .end: out = cursor("F")
        case .insert: out = tilde(2)
        case .delete: out = tilde(3)
        case .pageUp: out = tilde(5)
        case .pageDown: out = tilde(6)
        case .f1: out = ss3("P")
        case .f2: out = ss3("Q")
        case .f3: out = ss3("R")
        case .f4: out = ss3("S")
        case .f5: out = tilde(15)
        case .f6: out = tilde(17)
        case .f7: out = tilde(18)
        case .f8: out = tilde(19)
        case .f9: out = tilde(20)
        case .f10: out = tilde(21)
        case .f11: out = tilde(23)
        case .f12: out = tilde(24)
        }
        // Alt 加在不带修饰参数的单字节键上：前面补 ESC（meta 前缀）
        if alt && (key == .esc || key == .tab || key == .enter || key == .backspace) { out = [esc] + out }
        return out
    }

    /// 文字在 Ctrl / Alt 粘滞键下的编码：Ctrl+字母 → 0x01–0x1A，Ctrl+@[\]^_ 等 → 控制字符；Alt → 前缀 ESC。
    public static func encode(text: String, ctrl: Bool, alt: Bool) -> [UInt8] {
        var out: [UInt8] = []
        for ch in text {
            var bytes = Array(String(ch).utf8)
            if ctrl, let c = ctrlByte(ch) { bytes = [c] }
            if alt { bytes = [esc] + bytes }
            out += bytes
        }
        return out
    }

    public static func ctrlByte(_ ch: Character) -> UInt8? {
        guard let s = ch.unicodeScalars.first, ch.unicodeScalars.count == 1, s.isASCII else { return nil }
        let v = UInt8(s.value)
        switch v {
        case 0x61...0x7A: return v - 0x60          // a–z
        case 0x40...0x5F: return v - 0x40          // @ A–Z [ \ ] ^ _
        case 0x20: return 0x00                     // Ctrl+Space
        case 0x3F: return 0x7F                     // Ctrl+?
        case 0x32: return 0x00                     // Ctrl+2
        case 0x33...0x37: return v - 0x18          // Ctrl+3..7 → ESC FS GS RS US
        case 0x38: return 0x7F
        default: return nil
        }
    }
}
