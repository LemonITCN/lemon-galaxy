import UIKit
import SwiftUI
import SwiftTerm
import GalaxyKit

/// 终端配色主题：Resources/TerminalThemes/*.json，一个主题一个文件，加新主题只加文件（与 Mac 端共用同一套文件）。
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
    var uiBackground: UIColor { UIColor(hex: background) }

    /// 应用到 SwiftTerm 终端视图
    func apply(to view: TerminalView) {
        view.installColors(ansi.map { Self.stColor($0) })
        view.nativeForegroundColor = UIColor(hex: foreground)
        view.nativeBackgroundColor = UIColor(hex: background)
        view.backgroundColor = UIColor(hex: background)
        view.caretColor = UIColor(hex: cursor)
        view.selectedTextBackgroundColor = UIColor(hex: selection)
        view.keyboardAppearance = dark ? .dark : .light
    }

    static func stColor(_ hex: String) -> SwiftTerm.Color {
        let s = hex.trimmingCharacters(in: CharacterSet(charactersIn: "#"))
        var v: UInt64 = 0
        Scanner(string: s).scanHexInt64(&v)
        func c(_ shift: UInt64) -> UInt16 { UInt16((v >> shift) & 0xff) * 257 }
        return SwiftTerm.Color(red: c(16), green: c(8), blue: c(0))
    }
}
