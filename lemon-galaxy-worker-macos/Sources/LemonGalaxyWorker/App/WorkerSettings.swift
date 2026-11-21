import Foundation
import AppKit

/// 被控端的本机界面偏好。存 UserDefaults；selftest 下不读不写（persistent = false）。
/// 账号权限不在这里：它们由内核存在数据目录里（HostService.update_account_policy）。
final class WorkerSettings: ObservableObject {
    static let shared = WorkerSettings()
    var persistent = true
    private let defaults = UserDefaults.standard
    private var loading = false

    @Published var appearance: String = "system" { didSet { save("appearance", appearance); applyAppearance() } }   // system | light | dark
    @Published var language: String = "system" { didSet { save("language", language); Localization.shared.apply(setting: language) } }
    /// 有人连入时发系统通知（需要用户允许通知）
    @Published var systemNotifications: Bool = false { didSet { save("systemNotifications", systemNotifications) } }
    /// 有人请求临时协助时播放提示音
    @Published var assistSound: Bool = true { didSet { save("assistSound", assistSound) } }
    @Published var lastServer: String = "" { didSet { save("lastServer", lastServer) } }
    @Published var lastUsername: String = "" { didSet { save("lastUsername", lastUsername) } }
    /// 上次自动检查更新的时间（秒）
    @Published var lastUpdateCheck: Double = 0 { didSet { save("lastUpdateCheck", lastUpdateCheck) } }

    /// 界面配色固定为 galaxy（与主控端默认一致）；被控端不提供换配色
    let palette = "galaxy"

    func load() {
        guard persistent else { return }
        loading = true
        defer { loading = false }
        func s(_ k: String) -> String? { defaults.string(forKey: k) }
        func b(_ k: String) -> Bool? { defaults.object(forKey: k) as? Bool }
        appearance = s("appearance") ?? appearance
        language = s("language") ?? language
        systemNotifications = b("systemNotifications") ?? systemNotifications
        assistSound = b("assistSound") ?? assistSound
        lastServer = s("lastServer") ?? lastServer
        lastUsername = s("lastUsername") ?? lastUsername
        lastUpdateCheck = defaults.object(forKey: "lastUpdateCheck") as? Double ?? lastUpdateCheck
    }

    private func save(_ key: String, _ value: Any) {
        guard persistent, !loading else { return }
        defaults.set(value, forKey: key)
    }

    func applyAppearance() {
        guard let app = NSApp else { return }
        switch appearance {
        case "light": app.appearance = NSAppearance(named: .aqua)
        case "dark": app.appearance = NSAppearance(named: .darkAqua)
        default: app.appearance = nil
        }
    }
}

extension AiMode {
    /// 本机上限可选的档位（从严到宽）
    static let capChoices: [AiMode] = [.off, .readOnly, .confirmAll, .confirmDangerous, .fullTrust]

    var key: String {
        switch self {
        case .off: return "off"
        case .readOnly: return "readOnly"
        case .confirmAll: return "confirmAll"
        case .confirmDangerous: return "confirmDangerous"
        case .fullTrust: return "fullTrust"
        }
    }

    var labelKey: String { "ai.mode." + key }
}
