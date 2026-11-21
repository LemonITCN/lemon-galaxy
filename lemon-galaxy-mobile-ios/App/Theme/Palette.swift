import SwiftUI
import UIKit

/// 界面配色。每套配色有浅色、深色两个变体；界面主题（跟随系统 / 浅色 / 深色）决定用哪一个。
struct Palette: Equatable {
    var id: String
    var isDark: Bool
    var accent: Color
    var accentText: Color
    var background: Color      // 页面底色
    var card: Color            // 列表行、卡片
    var elevated: Color        // 输入框、浮层
    var border: Color
    var text: Color
    var secondaryText: Color
    var tertiaryText: Color
    var success: Color
    var warning: Color
    var danger: Color
    var info: Color
    var lan: Color
    var offline: Color
}

struct PaletteFamily: Identifiable {
    let id: String
    let nameKey: String
    let accentLight: String
    let accentDark: String
    let swatch: String
}

enum Palettes {
    static let families: [PaletteFamily] = [
        PaletteFamily(id: "galaxy", nameKey: "palette.galaxy", accentLight: "#2F6BEE", accentDark: "#5B8CFF", swatch: "#3B76F2"),
        PaletteFamily(id: "lemon", nameKey: "palette.lemon", accentLight: "#5E7A00", accentDark: "#C6E35A", swatch: "#A8C83A"),
        PaletteFamily(id: "teal", nameKey: "palette.teal", accentLight: "#0E8F80", accentDark: "#2CC9B4", swatch: "#14B8A6"),
        PaletteFamily(id: "violet", nameKey: "palette.violet", accentLight: "#6D4AE8", accentDark: "#9A82FF", swatch: "#7C5CFF"),
        PaletteFamily(id: "rose", nameKey: "palette.rose", accentLight: "#D63C55", accentDark: "#FF7A90", swatch: "#E5484D"),
        PaletteFamily(id: "graphite", nameKey: "palette.graphite", accentLight: "#3D4350", accentDark: "#C9CED8", swatch: "#6B7280"),
    ]

    static func make(_ id: String, dark: Bool) -> Palette {
        let f = families.first { $0.id == id } ?? families[0]
        let accent = Color(hex: dark ? f.accentDark : f.accentLight)
        let accentText: Color = (dark && (f.id == "lemon" || f.id == "graphite")) ? Color(hex: "#15171C") : .white
        if dark {
            return Palette(id: f.id, isDark: true, accent: accent, accentText: accentText,
                           background: Color(hex: "#111318"), card: Color(hex: "#1B1E25"), elevated: Color(hex: "#252932"),
                           border: Color(hex: "#30353F"), text: Color(hex: "#E8EAEE"), secondaryText: Color(hex: "#A3ABB8"),
                           tertiaryText: Color(hex: "#6E7785"), success: Color(hex: "#34C77B"), warning: Color(hex: "#F0A43A"),
                           danger: Color(hex: "#F2555A"), info: Color(hex: "#58A6FF"), lan: Color(hex: "#4FA3FF"), offline: Color(hex: "#5C6573"))
        }
        return Palette(id: f.id, isDark: false, accent: accent, accentText: accentText,
                       background: Color(hex: "#F2F3F6"), card: Color(hex: "#FFFFFF"), elevated: Color(hex: "#FFFFFF"),
                       border: Color(hex: "#DDE1E7"), text: Color(hex: "#1A1E26"), secondaryText: Color(hex: "#5B6576"),
                       tertiaryText: Color(hex: "#8A93A2"), success: Color(hex: "#1E9E5A"), warning: Color(hex: "#C77A0A"),
                       danger: Color(hex: "#D93036"), info: Color(hex: "#1F6FEB"), lan: Color(hex: "#1F6FEB"), offline: Color(hex: "#A0A7B3"))
    }
}

private struct PaletteKey: EnvironmentKey {
    static let defaultValue = Palettes.make("galaxy", dark: false)
}

extension EnvironmentValues {
    var palette: Palette {
        get { self[PaletteKey.self] }
        set { self[PaletteKey.self] = newValue }
    }
}

extension Color {
    init(hex: String) {
        let s = hex.trimmingCharacters(in: CharacterSet(charactersIn: "#"))
        var v: UInt64 = 0
        Scanner(string: s).scanHexInt64(&v)
        let r, g, b, a: Double
        if s.count == 8 {
            r = Double((v >> 24) & 0xff) / 255; g = Double((v >> 16) & 0xff) / 255; b = Double((v >> 8) & 0xff) / 255; a = Double(v & 0xff) / 255
        } else {
            r = Double((v >> 16) & 0xff) / 255; g = Double((v >> 8) & 0xff) / 255; b = Double(v & 0xff) / 255; a = 1
        }
        self.init(.sRGB, red: r, green: g, blue: b, opacity: a)
    }
}

extension UIColor {
    convenience init(hex: String) {
        let s = hex.trimmingCharacters(in: CharacterSet(charactersIn: "#"))
        var v: UInt64 = 0
        Scanner(string: s).scanHexInt64(&v)
        self.init(red: CGFloat((v >> 16) & 0xff) / 255, green: CGFloat((v >> 8) & 0xff) / 255, blue: CGFloat(v & 0xff) / 255, alpha: 1)
    }
}
