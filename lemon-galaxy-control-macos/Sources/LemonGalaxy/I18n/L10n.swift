import Foundation
import SwiftUI

/// 多语言底座：Resources/Locales/<代码>.json，一种语言一个文件，新增语言只加文件。
/// 文件格式：{ "_meta": { "code": "zh-Hans", "name": "简体中文", "coreLocale": "zh-CN" }, "strings": { "键": "文字", ... } }
/// 文字里的 {0} {1} … 按顺序替换为参数。缺的键回落到英文，再回落到键本身（selftest 会列出缺失的键）。
struct LanguagePack: Identifiable, Equatable {
    let code: String
    let name: String
    let coreLocale: String
    let strings: [String: String]
    var id: String { code }

    static func load(_ url: URL) -> LanguagePack? {
        guard let data = try? Data(contentsOf: url),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let meta = obj["_meta"] as? [String: Any],
              let code = meta["code"] as? String,
              let strings = obj["strings"] as? [String: String] else { return nil }
        return LanguagePack(code: code, name: meta["name"] as? String ?? code,
                            coreLocale: meta["coreLocale"] as? String ?? code, strings: strings)
    }
}

/// 当前语言的字符串表。值类型，放进 SwiftUI 环境：换语言时环境变化，所有用到的视图自动重绘。
struct Strings: Equatable {
    let pack: LanguagePack
    let fallback: LanguagePack?

    var code: String { pack.code }
    var isChinese: Bool { pack.code.hasPrefix("zh") }

    func callAsFunction(_ key: String, _ args: Any...) -> String {
        var s = pack.strings[key] ?? fallback?.strings[key] ?? {
            Localization.shared.noteMissing(key)
            return key
        }()
        for (i, a) in args.enumerated() { s = s.replacingOccurrences(of: "{\(i)}", with: String(describing: a)) }
        return s
    }

    static func == (a: Strings, b: Strings) -> Bool { a.pack == b.pack }
}

final class Localization: ObservableObject {
    static let shared = Localization()

    @Published private(set) var strings: Strings
    private(set) var packs: [LanguagePack] = []
    private var missing = Set<String>()
    private let missingLock = NSLock()

    private init() {
        let packs = AppResources.files(in: "Locales", ext: "json").compactMap(LanguagePack.load)
        self.packs = packs
        let en = packs.first { $0.code == "en" }
        let initial = packs.first ?? LanguagePack(code: "en", name: "English", coreLocale: "en", strings: [:])
        strings = Strings(pack: initial, fallback: en)
    }

    /// "system" 跟随系统：系统首选语言以 zh 开头用简体中文，否则看有没有同名语言包，没有用英文。
    static func resolve(_ setting: String, packs: [LanguagePack]) -> String {
        if setting != "system", packs.contains(where: { $0.code == setting }) { return setting }
        for lang in Locale.preferredLanguages {
            if lang.hasPrefix("zh") { return "zh-Hans" }
            if let p = packs.first(where: { lang.hasPrefix($0.code) }) { return p.code }
        }
        return "en"
    }

    func apply(setting: String) {
        let code = Self.resolve(setting, packs: packs)
        guard let pack = packs.first(where: { $0.code == code }) ?? packs.first(where: { $0.code == "en" }) else { return }
        if pack != strings.pack {
            strings = Strings(pack: pack, fallback: packs.first { $0.code == "en" })
        }
    }

    func noteMissing(_ key: String) {
        missingLock.lock(); missing.insert(key); missingLock.unlock()
    }

    var missingKeys: [String] { missingLock.lock(); defer { missingLock.unlock() }; return missing.sorted() }
}

/// 非视图代码（模型、菜单、弹窗）取当前语言的文字。
var L: Strings { Localization.shared.strings }

private struct StringsKey: EnvironmentKey {
    static let defaultValue: Strings = Localization.shared.strings
}

extension EnvironmentValues {
    var loc: Strings {
        get { self[StringsKey.self] }
        set { self[StringsKey.self] = newValue }
    }
}
