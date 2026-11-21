import SwiftUI
import AppKit

/// 界面配色。每套配色有浅色、深色两个变体；界面主题（跟随系统 / 浅色 / 深色）决定用哪一个。
struct Palette: Equatable {
    var id: String
    var isDark: Bool
    var accent: Color
    var accentText: Color          // 强调色上面的文字
    var sidebar: Color
    var sidebarText: Color
    var sidebarSelection: Color
    var list: Color
    var content: Color
    var card: Color
    var elevated: Color            // 输入框、浮层
    var border: Color
    var separator: Color
    var text: Color
    var secondaryText: Color
    var tertiaryText: Color
    var hover: Color
    var selection: Color
    var success: Color
    var warning: Color
    var danger: Color
    var info: Color
    var tabBar: Color
    var tabActive: Color

    var nsAccent: NSColor { NSColor(accent) }
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
        PaletteFamily(id: "galaxy", nameKey: "palette.galaxy", accentLight: "#2F6BEE", accentDark: "#4C82F7", swatch: "#3B76F2"),
        PaletteFamily(id: "teal", nameKey: "palette.teal", accentLight: "#0E9F8E", accentDark: "#22C3AE", swatch: "#14B8A6"),
        PaletteFamily(id: "violet", nameKey: "palette.violet", accentLight: "#6D4AE8", accentDark: "#8B70FF", swatch: "#7C5CFF"),
        PaletteFamily(id: "lemon", nameKey: "palette.lemon", accentLight: "#B58300", accentDark: "#F2C230", swatch: "#E8B400"),
        PaletteFamily(id: "rose", nameKey: "palette.rose", accentLight: "#D63C55", accentDark: "#F2607A", swatch: "#E5484D"),
        PaletteFamily(id: "graphite", nameKey: "palette.graphite", accentLight: "#4B5563", accentDark: "#A1A8B3", swatch: "#6B7280"),
    ]

    static func make(_ id: String, dark: Bool) -> Palette {
        let f = families.first { $0.id == id } ?? families[0]
        let accent = Color(hex: dark ? f.accentDark : f.accentLight)
        // 柠檬、石墨在深色下强调色很亮，上面的字用深色
        let accentText: Color = (dark && (f.id == "lemon" || f.id == "graphite")) ? Color(hex: "#15171C") : .white
        if dark {
            return Palette(id: f.id, isDark: true, accent: accent, accentText: accentText,
                           sidebar: Color(hex: "#14161B"), sidebarText: Color(hex: "#C9CED8"), sidebarSelection: accent.opacity(0.22),
                           list: Color(hex: "#191C22"), content: Color(hex: "#1E2128"), card: Color(hex: "#252932"),
                           elevated: Color(hex: "#2B3039"), border: Color(hex: "#343A45"), separator: Color(hex: "#2A2F38"),
                           text: Color(hex: "#E8EAEE"), secondaryText: Color(hex: "#A3ABB8"), tertiaryText: Color(hex: "#737C8A"),
                           hover: Color.white.opacity(0.05), selection: accent.opacity(0.24),
                           success: Color(hex: "#34C77B"), warning: Color(hex: "#F0A43A"), danger: Color(hex: "#F2555A"), info: Color(hex: "#58A6FF"),
                           tabBar: Color(hex: "#14161B"), tabActive: Color(hex: "#1E2128"))
        }
        return Palette(id: f.id, isDark: false, accent: accent, accentText: accentText,
                       sidebar: Color(hex: "#EEF0F4"), sidebarText: Color(hex: "#3A4250"), sidebarSelection: accent.opacity(0.14),
                       list: Color(hex: "#F6F7F9"), content: Color(hex: "#FFFFFF"), card: Color(hex: "#FFFFFF"),
                       elevated: Color(hex: "#FFFFFF"), border: Color(hex: "#DDE1E7"), separator: Color(hex: "#E6E9EE"),
                       text: Color(hex: "#1A1E26"), secondaryText: Color(hex: "#5B6576"), tertiaryText: Color(hex: "#8A93A2"),
                       hover: Color.black.opacity(0.04), selection: accent.opacity(0.13),
                       success: Color(hex: "#1E9E5A"), warning: Color(hex: "#C77A0A"), danger: Color(hex: "#D93036"), info: Color(hex: "#1F6FEB"),
                       tabBar: Color(hex: "#E7EAEF"), tabActive: Color(hex: "#FFFFFF"))
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

extension NSColor {
    convenience init(hex: String) {
        let s = hex.trimmingCharacters(in: CharacterSet(charactersIn: "#"))
        var v: UInt64 = 0
        Scanner(string: s).scanHexInt64(&v)
        self.init(srgbRed: CGFloat((v >> 16) & 0xff) / 255, green: CGFloat((v >> 8) & 0xff) / 255, blue: CGFloat(v & 0xff) / 255, alpha: 1)
    }
}
