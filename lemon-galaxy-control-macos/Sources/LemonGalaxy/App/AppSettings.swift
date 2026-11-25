import Foundation
import SwiftUI
import AppKit

/// 本机界面偏好。存 UserDefaults；selftest 下不读不写（persistent = false）。
/// 改动即时生效：主题、配色、语言、终端外观都通过 @Published 推到界面。
final class AppSettings: ObservableObject {
    static let shared = AppSettings()
    var persistent = true
    private let defaults = UserDefaults.standard

    @Published var appearance: String = "system" { didSet { save("appearance", appearance); applyAppearance() } }   // system | light | dark
    @Published var palette: String = "galaxy" { didSet { save("palette", palette) } }
    @Published var language: String = "system" { didSet { save("language", language); Localization.shared.apply(setting: language) } }
    @Published var terminalTheme: String = "galaxy-dark" { didSet { save("terminalTheme", terminalTheme) } }
    @Published var terminalFont: String = "SF Mono" { didSet { save("terminalFont", terminalFont) } }
    @Published var terminalFontSize: Double = 13 { didSet { save("terminalFontSize", terminalFontSize) } }
    @Published var terminalCursorBlink: Bool = true { didSet { save("terminalCursorBlink", terminalCursorBlink) } }
    @Published var terminalOptionAsMeta: Bool = true { didSet { save("terminalOptionAsMeta", terminalOptionAsMeta) } }

    // 连接
    @Published var displayQuality: String = "balanced" { didSet { save("displayQuality", displayQuality) } }        // smooth | balanced | sharp | custom
    @Published var displayFps: Int = 60 { didSet { save("displayFps", displayFps) } }
    @Published var displayBitrateKbps: Int = 12_000 { didSet { save("displayBitrateKbps", displayBitrateKbps) } }
    @Published var preferH265: Bool = true { didSet { save("preferH265", preferH265) } }
    @Published var clipboardSync: Bool = true { didSet { save("clipboardSync", clipboardSync) } }
    /// 远程桌面里收听对方的声音（对方支持时打开画面后自动开始）与音量 0...1
    @Published var remoteAudio: Bool = true { didSet { save("remoteAudio", remoteAudio) } }
    @Published var remoteAudioVolume: Double = 1 { didSet { save("remoteAudioVolume", remoteAudioVolume) } }
    @Published var reattachTerminals: Bool = true { didSet { save("reattachTerminals", reattachTerminals) } }

    // AI
    @Published var aiDefaultMode: String = "confirmDangerous" { didSet { save("aiDefaultMode", aiDefaultMode) } }
    @Published var aiPlanFirst: Bool = false { didSet { save("aiPlanFirst", aiPlanFirst) } }
    @Published var aiLastModel: String = "" { didSet { save("aiLastModel", aiLastModel) } }

    // 本机 MCP 服务（docs/mcp.md）：默认关闭；端口默认 47821；档位默认危险操作确认
    @Published var mcpEnabled: Bool = false { didSet { save("mcpEnabled", mcpEnabled) } }
    @Published var mcpPort: Int = Int(mcpDefaultPort()) { didSet { save("mcpPort", mcpPort) } }
    @Published var mcpMode: String = "confirmDangerous" { didSet { save("mcpMode", mcpMode) } }
    /// 完全信任选的时限（分钟），0 为不限时
    @Published var mcpTrustMinutes: Int = 60 { didSet { save("mcpTrustMinutes", mcpTrustMinutes) } }
    /// 限时完全信任到期的时间（毫秒），0 为没有限时；重启应用后接着算
    @Published var mcpTrustUntil: Double = 0 { didSet { save("mcpTrustUntil", mcpTrustUntil) } }

    // 登录与窗口
    @Published var lastServer: String = "" { didSet { save("lastServer", lastServer) } }
    @Published var lastUsername: String = "" { didSet { save("lastUsername", lastUsername) } }
    @Published var sidebarCompact: Bool = false { didSet { save("sidebarCompact", sidebarCompact) } }
    @Published var listHidden: Bool = false { didSet { save("listHidden", listHidden) } }
    /// 切到终端、远程桌面等会话标签时自动收起中间列表，回到首页再出现
    @Published var autoHideList: Bool = true { didSet { save("autoHideList", autoHideList) } }
    @Published var welcomedHost: Bool = false { didSet { save("welcomedHost", welcomedHost) } }
    /// 上次自动检查更新的时间（秒），每天最多自动检查一次
    @Published var lastUpdateCheck: Double = 0 { didSet { save("lastUpdateCheck", lastUpdateCheck) } }

    private var loading = false

    func load() {
        guard persistent else { return }
        loading = true
        defer { loading = false }
        func s(_ k: String) -> String? { defaults.string(forKey: k) }
        func b(_ k: String) -> Bool? { defaults.object(forKey: k) as? Bool }
        func d(_ k: String) -> Double? { defaults.object(forKey: k) as? Double }
        func i(_ k: String) -> Int? { defaults.object(forKey: k) as? Int }
        appearance = s("appearance") ?? appearance
        palette = s("palette") ?? palette
        language = s("language") ?? language
        terminalTheme = s("terminalTheme") ?? terminalTheme
        terminalFont = s("terminalFont") ?? terminalFont
        terminalFontSize = d("terminalFontSize") ?? terminalFontSize
        terminalCursorBlink = b("terminalCursorBlink") ?? terminalCursorBlink
        terminalOptionAsMeta = b("terminalOptionAsMeta") ?? terminalOptionAsMeta
        displayQuality = s("displayQuality") ?? displayQuality
        displayFps = i("displayFps") ?? displayFps
        displayBitrateKbps = i("displayBitrateKbps") ?? displayBitrateKbps
        preferH265 = b("preferH265") ?? preferH265
        clipboardSync = b("clipboardSync") ?? clipboardSync
        remoteAudio = b("remoteAudio") ?? remoteAudio
        remoteAudioVolume = min(max(d("remoteAudioVolume") ?? remoteAudioVolume, 0), 1)
        reattachTerminals = b("reattachTerminals") ?? reattachTerminals
        aiDefaultMode = s("aiDefaultMode") ?? aiDefaultMode
        aiPlanFirst = b("aiPlanFirst") ?? aiPlanFirst
        aiLastModel = s("aiLastModel") ?? aiLastModel
        mcpEnabled = b("mcpEnabled") ?? mcpEnabled
        if let p = i("mcpPort"), (1024...65535).contains(p) { mcpPort = p }
        mcpMode = s("mcpMode") ?? mcpMode
        mcpTrustMinutes = i("mcpTrustMinutes") ?? mcpTrustMinutes
        mcpTrustUntil = d("mcpTrustUntil") ?? mcpTrustUntil
        lastServer = s("lastServer") ?? lastServer
        lastUsername = s("lastUsername") ?? lastUsername
        sidebarCompact = b("sidebarCompact") ?? sidebarCompact
        listHidden = b("listHidden") ?? listHidden
        autoHideList = b("autoHideList") ?? autoHideList
        welcomedHost = b("welcomedHost") ?? welcomedHost
        lastUpdateCheck = d("lastUpdateCheck") ?? lastUpdateCheck
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

    var aiMode: AiMode {
        get { AiMode.from(aiDefaultMode) }
        set { aiDefaultMode = newValue.key }
    }

    /// 画质预设 → 帧率与码率
    static func quality(_ q: String) -> (fps: Int, kbps: Int) {
        switch q {
        case "smooth": return (60, 6_000)
        case "sharp": return (60, 30_000)
        case "balanced": return (60, 12_000)
        default: return (60, 12_000)
        }
    }
}

extension AiMode {
    static let selectable: [AiMode] = [.readOnly, .confirmAll, .confirmDangerous, .fullTrust]

    var key: String {
        switch self {
        case .off: return "off"
        case .readOnly: return "readOnly"
        case .confirmAll: return "confirmAll"
        case .confirmDangerous: return "confirmDangerous"
        case .fullTrust: return "fullTrust"
        }
    }

    static func from(_ s: String) -> AiMode {
        switch s {
        case "off": return .off
        case "readOnly": return .readOnly
        case "confirmAll": return .confirmAll
        case "fullTrust": return .fullTrust
        default: return .confirmDangerous
        }
    }

    /// off < read_only < confirm_all < confirm_dangerous < full_trust（ai-agent.md 第 5 节）
    var rank: Int {
        switch self {
        case .off: return 0
        case .readOnly: return 1
        case .confirmAll: return 2
        case .confirmDangerous: return 3
        case .fullTrust: return 4
        }
    }

    var labelKey: String { "ai.mode." + key }
}
