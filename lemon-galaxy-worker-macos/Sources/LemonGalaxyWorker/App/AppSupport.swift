import Foundation
import AppKit
import SystemConfiguration

/// 资源定位：正式包里在 Contents/Resources；开发时（swift build 后直接运行）回溯到源码树的 Resources/。
/// 共享的语言包底座（Shared/L10n.swift）通过这里找 Locales。不用 SwiftPM 的 Bundle.module：它要求资源包放在 .app 根目录，会破坏代码签名。
enum AppResources {
    static let root: URL = {
        if let env = ProcessInfo.processInfo.environment["LEMONGALAXY_RESOURCES"], !env.isEmpty {
            return URL(fileURLWithPath: env, isDirectory: true)
        }
        if let res = Bundle.main.resourceURL,
           FileManager.default.fileExists(atPath: res.appendingPathComponent("Locales").path) {
            return res
        }
        // 本文件位于 lemon-galaxy-worker-macos/Sources/LemonGalaxyWorker/App/，去掉文件名与三级目录就是工程目录
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { url.deleteLastPathComponent() }
        return url.appendingPathComponent("Resources", isDirectory: true)
    }()

    static func url(_ relative: String) -> URL { root.appendingPathComponent(relative) }

    static func files(in dir: String, ext: String) -> [URL] {
        let d = url(dir)
        let items = (try? FileManager.default.contentsOfDirectory(at: d, includingPropertiesForKeys: nil)) ?? []
        return items.filter { $0.pathExtension == ext }.sorted { $0.lastPathComponent < $1.lastPathComponent }
    }
}

enum AppInfo {
    static let name = "LemonGalaxyWorker"
    static let bundleId = "cn.lemonit.galaxy.worker"
    static let releasesURL = URL(string: "https://github.com/LemonITCN/lemon-galaxy/releases/latest")!
    static let repoURL = URL(string: "https://github.com/LemonITCN/lemon-galaxy")!

    /// 电脑名（系统设置里的名称）。不用 Host.current()：它可能做 DNS 反查而卡住启动
    static var computerName: String {
        (SCDynamicStoreCopyComputerName(nil, nil) as String?) ?? "Mac"
    }
    static var version: String { Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.1.0" }
    static var build: String { Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "dev" }
    static var osVersion: String {
        let v = ProcessInfo.processInfo.operatingSystemVersion
        return "macOS \(v.majorVersion).\(v.minorVersion).\(v.patchVersion)"
    }
    /// 当前 macOS 用户（被控端以这个用户身份运行终端、文件与命令）
    static var userName: String { NSUserName() }

    /// 是否作为打好的 .app 运行（通知、开机启动只在正式包里可用）
    static var isBundled: Bool {
        Bundle.main.bundleIdentifier == bundleId && Bundle.main.bundleURL.pathExtension == "app"
    }

    /// 数据目录：~/Library/Application Support/LemonGalaxyWorker（内核在其下建 worker/），与主控端分开
    static var dataDir: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("LemonGalaxyWorker", isDirectory: true)
    }

    static var cacheDir: URL {
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0].appendingPathComponent(bundleId, isDirectory: true)
    }

    /// 临时签名的包存设备身份等机密的目录（LocalSecureStore 的文件模式），与内核的 worker/ 分开，移交后台服务时不跟着改名
    static var secretsDir: URL { dataDir.appendingPathComponent("secrets", isDirectory: true) }
}

/// 给钥匙串里的条目名加前缀：被控端的设备身份与同一台 Mac 上主控端的分开存（docs/ffi.md 5.1）。
final class PrefixedSecureStore: SecureStore, @unchecked Sendable {
    private let base: SecureStore
    private let prefix: String
    init(base: SecureStore, prefix: String) {
        self.base = base
        self.prefix = prefix
    }
    func get(key: String) -> Data? { base.get(key: prefix + key) }
    func set(key: String, value: Data) { base.set(key: prefix + key, value: value) }
    func remove(key: String) { base.remove(key: prefix + key) }
}

/// 把内核错误变成可显示的文字（内核返回的文字已本地化）
func describeError(_ error: Error) -> String {
    if let g = error as? GalaxyError {
        switch g {
        case .Network(let m), .Permission(let m), .NotFound(let m), .Protocol(let m), .Other(let m): return m
        case .Auth(_, let m): return m
        case .Cancelled: return L("common.cancelled")
        }
    }
    return error.localizedDescription
}

/// 设备 ID 分组显示：每 4 位一组
func groupedId(_ id: String, by n: Int = 4) -> String {
    var out = ""
    for (i, c) in id.enumerated() {
        if i > 0 && i % n == 0 { out += " " }
        out.append(c)
    }
    return out
}

func copyToPasteboard(_ s: String) {
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(s, forType: .string)
}
