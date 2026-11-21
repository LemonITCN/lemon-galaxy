import Foundation

/// 资源定位：正式包里在 Contents/Resources；开发时（swift run / swift build 直接运行）回溯到源码树的 Resources/。
/// 不用 SwiftPM 的 Bundle.module：它要求资源包放在 .app 根目录，会破坏代码签名。
enum AppResources {
    static let root: URL = {
        if let env = ProcessInfo.processInfo.environment["LEMONGALAXY_RESOURCES"], !env.isEmpty {
            return URL(fileURLWithPath: env, isDirectory: true)
        }
        if let res = Bundle.main.resourceURL,
           FileManager.default.fileExists(atPath: res.appendingPathComponent("Locales").path) {
            return res
        }
        // 本文件位于 lemon-galaxy-control-macos/Sources/LemonGalaxy/App/，往上三级是工程目录
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
