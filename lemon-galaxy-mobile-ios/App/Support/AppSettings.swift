import Foundation
import SwiftUI
import GalaxyKit

enum ThemeMode: String, CaseIterable, Identifiable {
    case system, light, dark
    var id: String { rawValue }
    var colorScheme: ColorScheme? {
        switch self {
        case .system: return nil
        case .light: return .light
        case .dark: return .dark
        }
    }
}

/// 本机界面偏好（UserDefaults）。修改立即写盘并推给界面，主题、终端配色、字号即时生效。
@MainActor
final class AppSettings: ObservableObject {
    private let d: UserDefaults

    @Published var themeMode: ThemeMode { didSet { d.set(themeMode.rawValue, forKey: "themeMode") } }
    @Published var palette: String { didSet { d.set(palette, forKey: "palette") } }
    @Published var terminalTheme: String { didSet { d.set(terminalTheme, forKey: "terminalTheme") } }
    @Published var terminalFontSize: Double { didSet { d.set(terminalFontSize, forKey: "terminalFontSize") } }
    @Published var language: String { didSet { d.set(language, forKey: "language") } }
    @Published var appLock: Bool { didSet { d.set(appLock, forKey: "appLock") } }
    @Published var defaultQuality: VideoQuality { didSet { d.set(defaultQuality.rawValue, forKey: "defaultQuality") } }
    /// AI 输入框上次选的档位（新对话默认用它，不超过管理员上限）
    @Published var defaultAiMode: AiMode { didSet { d.set(defaultAiMode.key, forKey: "defaultAiMode") } }
    /// AI 输入框上次选的模型（「供应商 ID/模型 ID」）
    @Published var aiLastModel: String { didSet { d.set(aiLastModel, forKey: "aiLastModel") } }
    /// AI 输入框的「先出计划」
    @Published var aiPlanFirst: Bool { didSet { d.set(aiPlanFirst, forKey: "aiPlanFirst") } }
    /// 完全信任的时限（分钟），0 表示本对话内一直有效
    @Published var aiTrustMinutes: Int { didSet { d.set(aiTrustMinutes, forKey: "aiTrustMinutes") } }
    /// 远程桌面操作方式：鼠标（触控板式，默认）或触控；菜单里切换后记住
    @Published var remoteMouseMode: Bool {
        didSet {
            d.set(remoteMouseMode ? "mouse" : "touch", forKey: "remoteInputMode")
            d.removeObject(forKey: "remoteMouseMode")
        }
    }
    /// 远程桌面播放被控端声音（默认开，记住用户的选择）
    @Published var remoteAudio: Bool { didSet { d.set(remoteAudio, forKey: "remoteAudio") } }
    @Published var terminalInputBox: Bool { didSet { d.set(terminalInputBox, forKey: "terminalInputBox") } }
    @Published var showHiddenFiles: Bool { didSet { d.set(showHiddenFiles, forKey: "showHiddenFiles") } }
    @Published var fileSort: FileSort { didSet { d.set(fileSort.rawValue, forKey: "fileSort") } }
    @Published var fileSortDescending: Bool { didSet { d.set(fileSortDescending, forKey: "fileSortDescending") } }
    @Published var askResumeTerminal: Bool { didSet { d.set(askResumeTerminal, forKey: "askResumeTerminal") } }
    @Published var lastServer: String { didSet { d.set(lastServer, forKey: "lastServer") } }
    @Published var lastUsername: String { didSet { d.set(lastUsername, forKey: "lastUsername") } }

    static let fontRange: ClosedRange<Double> = 8...28

    init(defaults: UserDefaults = .standard) {
        d = defaults
        themeMode = ThemeMode(rawValue: d.string(forKey: "themeMode") ?? "") ?? .system
        palette = d.string(forKey: "palette") ?? "galaxy"
        terminalTheme = d.string(forKey: "terminalTheme") ?? "galaxy-dark"
        let fs = d.double(forKey: "terminalFontSize")
        terminalFontSize = fs == 0 ? 13 : min(max(fs, Self.fontRange.lowerBound), Self.fontRange.upperBound)
        language = d.string(forKey: "language") ?? "system"
        appLock = d.bool(forKey: "appLock")
        defaultQuality = VideoQuality(rawValue: d.string(forKey: "defaultQuality") ?? "") ?? .auto
        defaultAiMode = AiMode.fromKey(d.string(forKey: "defaultAiMode")) ?? .confirmDangerous
        aiLastModel = d.string(forKey: "aiLastModel") ?? ""
        aiPlanFirst = d.bool(forKey: "aiPlanFirst")
        aiTrustMinutes = d.object(forKey: "aiTrustMinutes") as? Int ?? 60
        // 新键（与安卓一致）：旧键 remoteMouseMode 默认是触控，沿用它就改不成默认鼠标
        remoteMouseMode = d.string(forKey: "remoteInputMode").map { $0 != "touch" } ?? true
        remoteAudio = d.object(forKey: "remoteAudio") as? Bool ?? true
        terminalInputBox = d.bool(forKey: "terminalInputBox")
        showHiddenFiles = d.bool(forKey: "showHiddenFiles")
        fileSort = FileSort(rawValue: d.string(forKey: "fileSort") ?? "") ?? .name
        fileSortDescending = d.bool(forKey: "fileSortDescending")
        askResumeTerminal = d.object(forKey: "askResumeTerminal") as? Bool ?? true
        lastServer = d.string(forKey: "lastServer") ?? ""
        lastUsername = d.string(forKey: "lastUsername") ?? ""
    }
}

enum AppInfo {
    static var version: String { Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.1.0" }
    static var build: String { Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "1" }
    static var osVersion: String {
        let v = ProcessInfo.processInfo.operatingSystemVersion
        let name = UIDevice.current.userInterfaceIdiom == .pad ? "iPadOS" : "iOS"
        return "\(name) \(v.majorVersion).\(v.minorVersion).\(v.patchVersion)"
    }
    /// 设备名（iOS 16 起没有特殊授权时只给型号名，如 iPhone / iPad）
    static var deviceName: String { UIDevice.current.name }
    static let sourceURL = URL(string: "https://github.com/LemonITCN/lemon-galaxy")!
}
