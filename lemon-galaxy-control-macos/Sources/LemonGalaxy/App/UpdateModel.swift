import Foundation
import AppKit

/// 检查更新与下载安装包（ffi.md 5.0）。
/// 内核向自己的服务器查询，返回前已验发布签名；下载时核对 SHA-256。这里只管界面状态：
/// 手动检查在「设置 - 关于」；登录上线后每天最多自动检查一次，有新版本在设置图标上标小红点。
/// 下载到「下载」文件夹，完成后打开 DMG，由用户把应用拖进「应用程序」替换。
@MainActor
final class UpdateModel: ObservableObject {
    enum State: Equatable {
        case idle
        case checking
        case upToDate
        case available(UpdateInfo)
        case downloading(UpdateInfo, done: UInt64, total: UInt64)
        case downloaded(UpdateInfo, path: String)
        case failed(String)
    }

    @Published var state: State = .idle
    /// 已知的新版本（设置图标的小红点）；下载安装后仍保留，直到换成新版本重新启动
    @Published var available: UpdateInfo? {
        // 侧栏、设置导航观察的是 AppModel：小红点跟着变
        willSet { app?.objectWillChange.send() }
    }
    @Published var lastChecked: Date?
    weak var app: AppModel?
    private var listener: UpdateDownloadListener?

    static let component = "control"
    static let platform = "macos"

    var busy: Bool {
        switch state {
        case .checking, .downloading: return true
        default: return false
        }
    }

    /// 登录上线后调用：距上次自动检查满一天才查，失败不打扰用户
    func autoCheckIfDue() {
        guard let app, !app.selfTest, !busy else { return }
        let now = Date().timeIntervalSince1970
        guard now - app.settings.lastUpdateCheck >= 86_400 else { return }
        app.settings.lastUpdateCheck = now
        Task { await check(manual: false) }
    }

    /// 手动检查：有新版本时弹出更新卡片，没有时提示已是最新
    func check(manual: Bool) async {
        guard let app, !busy else { return }
        state = .checking
        do {
            let info = try await app.client.checkUpdate(component: Self.component, platform: Self.platform, arch: "", current: AppInfo.version)
            lastChecked = Date(timeIntervalSince1970: TimeInterval(Clock.nowMs) / 1000)
            if let info {
                available = info
                state = .available(info)
                if manual { app.modal = .update }
            } else {
                available = nil
                state = .upToDate
                if manual { app.showToast(L("update.upToDate", AppInfo.version)) }
            }
        } catch {
            lastChecked = Date(timeIntervalSince1970: TimeInterval(Clock.nowMs) / 1000)
            let msg = AppModel.describe(error)
            state = .failed(msg)
            if manual { app.showToast(L("update.checkFailed", msg), error: true) }
        }
    }

    /// 下载到「下载」文件夹（同名旧文件会被覆盖），完成后打开 DMG
    func download() {
        guard let app, case .available(let info) = state else { return }
        let dir = FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
            ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Downloads", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let name = (info.fileName as NSString).lastPathComponent
        let dest = dir.appendingPathComponent(name.isEmpty ? "LemonGalaxy-\(info.version).dmg" : name)
        state = .downloading(info, done: 0, total: info.size)
        let l = UpdateDownloadListener(model: self, info: info, path: dest.path)
        listener = l
        app.client.downloadUpdate(info: info, dest: dest.path, listener: l)
    }

    func progress(_ info: UpdateInfo, done: UInt64, total: UInt64) {
        guard case .downloading = state else { return }
        state = .downloading(info, done: done, total: total == 0 ? info.size : total)
    }

    func finished(_ info: UpdateInfo, path: String, error: String?) {
        listener = nil
        if let error {
            state = .failed(L("update.downloadFailed", error))
            return
        }
        state = .downloaded(info, path: path)
        guard app?.selfTest == false else { return }
        NSWorkspace.shared.open(URL(fileURLWithPath: path))
    }

    func revealDownloaded() {
        guard case .downloaded(_, let path) = state else { return }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
    }

    /// 关掉更新卡片后回到「有新版本」状态（失败、已下载后再点检查可以重来）
    func reset() {
        if busy { return }
        if let a = available { state = .available(a) } else { state = .idle }
    }
}

/// 内核线程 → 主线程
final class UpdateDownloadListener: TransferListener, @unchecked Sendable {
    private weak var model: UpdateModel?
    private let info: UpdateInfo
    private let path: String
    private let lock = NSLock()
    private var lastReport = Date.distantPast

    init(model: UpdateModel, info: UpdateInfo, path: String) {
        self.model = model
        self.info = info
        self.path = path
    }

    func onProgress(done: UInt64, total: UInt64) {
        // 每 0.1 秒最多刷新一次界面
        let now = Date()
        let report: Bool = lock.locked {
            guard now.timeIntervalSince(lastReport) >= 0.1 || done >= total else { return false }
            lastReport = now
            return true
        }
        guard report else { return }
        let info = info
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated { self?.model?.progress(info, done: done, total: total) }
        }
    }

    func onFinished(error: String?) {
        let info = info, path = path
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated { self?.model?.finished(info, path: path, error: error) }
        }
    }
}
