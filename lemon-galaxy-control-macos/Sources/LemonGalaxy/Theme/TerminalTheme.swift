import AppKit
import SwiftUI
import SwiftTerm

/// 终端配色主题：Resources/TerminalThemes/*.json，一个主题一个文件，加新主题只加文件。
/// 字段：id、name、dark、order（排序）、foreground、background、cursor、selection、ansi（16 个颜色，0–7 普通，8–15 高亮）。
struct TerminalTheme: Identifiable, Equatable {
    let id: String
    let name: String
    let dark: Bool
    let order: Int
    let foreground: String
    let background: String
    let cursor: String
    let selection: String
    let ansi: [String]

    static let fallback = TerminalTheme(id: "galaxy-dark", name: "Galaxy Dark", dark: true, order: 0, foreground: "#E3E6EB",
                                        background: "#171A21", cursor: "#4C82F7", selection: "#2C3A55",
                                        ansi: ["#1F232B", "#F2555A", "#34C77B", "#F0B43A", "#4C82F7", "#B07CF2", "#2EC4D6", "#D5D9E0",
                                               "#5C6575", "#FF7A7E", "#5BE09A", "#FFD066", "#79A2FF", "#CDA0FF", "#5EE0EE", "#FFFFFF"])

    static let all: [TerminalTheme] = {
        let list = AppResources.files(in: "TerminalThemes", ext: "json").compactMap { url -> TerminalTheme? in
            guard let data = try? Data(contentsOf: url),
                  let o = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let id = o["id"] as? String, let name = o["name"] as? String,
                  let fg = o["foreground"] as? String, let bg = o["background"] as? String,
                  let ansi = o["ansi"] as? [String], ansi.count == 16 else { return nil }
            return TerminalTheme(id: id, name: name, dark: o["dark"] as? Bool ?? true, order: o["order"] as? Int ?? 99,
                                 foreground: fg, background: bg, cursor: o["cursor"] as? String ?? fg,
                                 selection: o["selection"] as? String ?? "#44475A", ansi: ansi)
        }
        return list.isEmpty ? [fallback] : list.sorted { ($0.order, $0.name) < ($1.order, $1.name) }
    }()

    static func named(_ id: String?) -> TerminalTheme {
        all.first { $0.id == id } ?? all.first { $0.id == "galaxy-dark" } ?? all[0]
    }

    var fgColor: SwiftUI.Color { SwiftUI.Color(hex: foreground) }
    var bgColor: SwiftUI.Color { SwiftUI.Color(hex: background) }

    /// 应用到 SwiftTerm 终端视图
    func apply(to view: TerminalView) {
        view.installColors(ansi.map { Self.stColor($0) })
        view.nativeForegroundColor = NSColor(hex: foreground)
        view.nativeBackgroundColor = NSColor(hex: background)
        view.caretColor = NSColor(hex: cursor)
        view.selectedTextBackgroundColor = NSColor(hex: selection)
        view.layer?.backgroundColor = NSColor(hex: background).cgColor
    }

    static func stColor(_ hex: String) -> SwiftTerm.Color {
        let ns = NSColor(hex: hex)
        return SwiftTerm.Color(red: UInt16(ns.redComponent * 65535), green: UInt16(ns.greenComponent * 65535), blue: UInt16(ns.blueComponent * 65535))
    }
}

/// 终端字体：本机所有等宽字体族。
enum TerminalFonts {
    static let preferred = ["SF Mono", "Menlo", "JetBrains Mono", "Fira Code", "Source Code Pro", "Monaco", "Courier New"]

    static var families: [String] {
        let fm = NSFontManager.shared
        let mono = fm.availableFontFamilies.filter { fam in
            guard let f = NSFont(name: fam, size: 12) ?? fm.font(withFamily: fam, traits: [], weight: 5, size: 12) else { return false }
            return f.isFixedPitch || fam.lowercased().contains("mono")
        }
        // SF Mono 不一定作为字体族安装，但系统等宽字体总能用，固定放第一个
        var out = ["SF Mono"] + preferred.dropFirst().filter { mono.contains($0) }
        for f in mono.sorted() where !out.contains(f) { out.append(f) }
        return out
    }

    static func font(family: String, size: CGFloat) -> NSFont {
        if family == "SF Mono" || family.isEmpty { return NSFont.monospacedSystemFont(ofSize: size, weight: .regular) }
        if let f = NSFontManager.shared.font(withFamily: family, traits: [], weight: 5, size: size) { return f }
        return NSFont(name: family, size: size) ?? NSFont.monospacedSystemFont(ofSize: size, weight: .regular)
    }
}
