import Foundation
import Combine

/// 资源定位：App 包里在 Bundle.main 的资源目录（Locales/、TerminalThemes/ 是目录引用）；
/// swift test / 冒烟测试时回溯到源码树的 Resources/。环境变量 LEMONGALAXY_RESOURCES 可以覆盖。
public enum AppResources {
    public static var overrideRoot: URL?

    public static var root: URL {
        if let overrideRoot { return overrideRoot }
        if let env = ProcessInfo.processInfo.environment["LEMONGALAXY_RESOURCES"], !env.isEmpty {
            return URL(fileURLWithPath: env, isDirectory: true)
        }
        if let res = Bundle.main.resourceURL,
           FileManager.default.fileExists(atPath: res.appendingPathComponent("Locales").path) {
            return res
        }
        // 本文件位于 lemon-galaxy-mobile-ios/Sources/GalaxyKit/I18n/，往上四级是工程目录
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { url.deleteLastPathComponent() }
        return url.appendingPathComponent("Resources", isDirectory: true)
    }

    public static func url(_ relative: String) -> URL { root.appendingPathComponent(relative) }

    public static func files(in dir: String, ext: String) -> [URL] {
        let d = url(dir)
        let items = (try? FileManager.default.contentsOfDirectory(at: d, includingPropertiesForKeys: nil)) ?? []
        return items.filter { $0.pathExtension == ext }.sorted { $0.lastPathComponent < $1.lastPathComponent }
    }
}

/// 多语言底座：Resources/Locales/<代码>.json，一种语言一个文件，新增语言只加文件。
/// 文件格式：{ "_meta": { "code": "zh-Hans", "name": "简体中文", "coreLocale": "zh-CN" }, "strings": { "键": "文字", ... } }
/// 文字里的 {0} {1} … 按顺序替换为参数。缺的键回落到英文，再回落到键本身（单元测试会检查两种语言的键是否一致）。
public struct LanguagePack: Identifiable, Equatable {
    public let code: String
    public let name: String
    public let coreLocale: String
    public let strings: [String: String]
    public var id: String { code }

    public init(code: String, name: String, coreLocale: String, strings: [String: String]) {
        self.code = code
        self.name = name
        self.coreLocale = coreLocale
        self.strings = strings
    }

    public static func load(_ url: URL) -> LanguagePack? {
        guard let data = try? Data(contentsOf: url) else { return nil }
        return parse(data)
    }

    public static func parse(_ data: Data) -> LanguagePack? {
        guard let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let meta = obj["_meta"] as? [String: Any],
              let code = meta["code"] as? String,
              let strings = obj["strings"] as? [String: String] else { return nil }
        return LanguagePack(code: code, name: meta["name"] as? String ?? code,
                            coreLocale: meta["coreLocale"] as? String ?? code, strings: strings)
    }
}

/// 当前语言的字符串表。值类型：换语言时整体替换，SwiftUI 环境里的所有视图自动重绘。
public struct Strings: Equatable {
    public let pack: LanguagePack
    public let fallback: LanguagePack?

    public init(pack: LanguagePack, fallback: LanguagePack?) {
        self.pack = pack
        self.fallback = fallback
    }

    public var code: String { pack.code }
    public var isChinese: Bool { pack.code.hasPrefix("zh") }

    public func callAsFunction(_ key: String, _ args: Any...) -> String {
        format(key, args)
    }

    public func format(_ key: String, _ args: [Any]) -> String {
        var s = pack.strings[key] ?? fallback?.strings[key] ?? {
            Localization.shared.noteMissing(key)
            return key
        }()
        for (i, a) in args.enumerated() { s = s.replacingOccurrences(of: "{\(i)}", with: String(describing: a)) }
        return s
    }

    public func has(_ key: String) -> Bool { pack.strings[key] != nil || fallback?.strings[key] != nil }

    public static func == (a: Strings, b: Strings) -> Bool { a.pack == b.pack }
}

public final class Localization: ObservableObject {
    public static let shared = Localization()

    @Published public private(set) var strings: Strings
    public private(set) var packs: [LanguagePack] = []
    private var missing = Set<String>()
    private let missingLock = NSLock()

    private init() {
        let packs = AppResources.files(in: "Locales", ext: "json").compactMap(LanguagePack.load)
        self.packs = packs
        let en = packs.first { $0.code == "en" }
        let initial = packs.first { $0.code == Self.resolve("system", packs: packs) } ?? en
            ?? LanguagePack(code: "en", name: "English", coreLocale: "en", strings: [:])
        strings = Strings(pack: initial, fallback: en)
    }

    /// "system" 跟随系统：系统首选语言以 zh 开头用简体中文，否则看有没有同名语言包，没有用英文。
    public static func resolve(_ setting: String, packs: [LanguagePack], preferred: [String] = Locale.preferredLanguages) -> String {
        if setting != "system", packs.contains(where: { $0.code == setting }) { return setting }
        for lang in preferred {
            if lang.hasPrefix("zh"), packs.contains(where: { $0.code == "zh-Hans" }) { return "zh-Hans" }
            if let p = packs.first(where: { lang.hasPrefix($0.code) }) { return p.code }
        }
        return "en"
    }

    public func apply(setting: String) {
        let code = Self.resolve(setting, packs: packs)
        guard let pack = packs.first(where: { $0.code == code }) ?? packs.first(where: { $0.code == "en" }) else { return }
        if pack != strings.pack {
            strings = Strings(pack: pack, fallback: packs.first { $0.code == "en" })
        }
    }

    func noteMissing(_ key: String) {
        missingLock.lock(); missing.insert(key); missingLock.unlock()
    }

    public var missingKeys: [String] { missingLock.lock(); defer { missingLock.unlock() }; return missing.sorted() }
}

/// 非视图代码（模型、菜单、弹窗）取当前语言的文字。
public var L: Strings { Localization.shared.strings }
