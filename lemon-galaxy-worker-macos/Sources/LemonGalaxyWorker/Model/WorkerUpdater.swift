import Foundation
import AppKit

/// 检查更新与下载安装包（docs/ffi.md 5.0 / 5.1）。
/// WorkerHost.check_update 用任一有效账号的令牌向自己的服务器查询，返回前已验发布签名；download_update 下载时核对 SHA-256。
/// 手动检查在「关于」页；上线后每天最多自动检查一次，有新版本时菜单栏菜单里加一项提示。
/// 下载到「下载」文件夹，完成后打开 DMG，由用户把应用拖进「应用程序」替换。
@MainActor
final class WorkerUpdater: ObservableObject {
    enum State: Equatable {
        case idle
        case checking
        case upToDate
        case available(UpdateInfo)
        case downloading(UpdateInfo, done: UInt64, total: UInt64)
        case downloaded(UpdateInfo, path: String)
        case failed(String)
    }

    static let platform = "macos"

    @Published var state: State = .idle
    /// 已知的新版本（菜单栏菜单提示）；直到换成新版本重新启动前一直保留
    @Published var available: UpdateInfo? {
        // 侧栏「关于」的小红点观察的是 WorkerModel
        willSet { model?.objectWillChange.send() }
    }
    @Published var lastChecked: Date?
    weak var model: WorkerModel?
    private var listener: WorkerDownloadListener?

    var busy: Bool {
        switch state {
        case .checking, .downloading: return true
        default: return false
        }
    }

    /// 上线后调用：距上次自动检查满一天才查，失败不打扰用户
    func autoCheckIfDue() {
        guard let model, !model.selfTest, !busy else { return }
        let now = Date().timeIntervalSince1970
        guard now - model.settings.lastUpdateCheck >= 86_400 else { return }
        model.settings.lastUpdateCheck = now
        Task { await check(manual: false) }
    }

    /// 手动检查：没有新版本或失败时在窗口里提示
    func check(manual: Bool) async {
        guard let model, !busy else { return }
        guard model.hasActiveAccount else {
            state = .failed(L("update.needAccount"))
            return
        }
        state = .checking
        do {
            let info = try await model.worker.checkUpdate(platform: Self.platform, current: AppInfo.version)
            lastChecked = Date(timeIntervalSince1970: TimeInterval(Clock.nowMs) / 1000)
            if let info {
                available = info
                state = .available(info)
            } else {
                available = nil
                state = .upToDate
                if manual { model.show(notice: L("update.upToDate", AppInfo.version)) }
            }
        } catch {
            lastChecked = Date(timeIntervalSince1970: TimeInterval(Clock.nowMs) / 1000)
            state = .failed(L("update.checkFailed", describeError(error)))
        }
        UIHooks.update(model)
    }

    /// 下载到「下载」文件夹（同名旧文件会被覆盖），完成后打开 DMG
    func download() {
        guard let model, case .available(let info) = state else { return }
        let dir = FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
            ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Downloads", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let name = (info.fileName as NSString).lastPathComponent
        let dest = dir.appendingPathComponent(name.isEmpty ? "LemonGalaxyWorker-\(info.version)-macOS.dmg" : name)
        state = .downloading(info, done: 0, total: info.size)
        let l = WorkerDownloadListener(updater: self, info: info, path: dest.path)
        listener = l
        model.worker.downloadUpdate(info: info, dest: dest.path, listener: l)
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
        // 模拟内核只报进度不写文件
        guard let model, !model.selfTest, !model.core.isMock else { return }
        NSWorkspace.shared.open(URL(fileURLWithPath: path))
    }

    func revealDownloaded() {
        guard case .downloaded(_, let path) = state else { return }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
    }

    /// 失败或已下载后回到「有新版本」或初始状态
    func reset() {
        if busy { return }
        if let a = available { state = .available(a) } else { state = .idle }
    }
}

/// 内核线程 → 主线程
final class WorkerDownloadListener: TransferListener, @unchecked Sendable {
    private weak var updater: WorkerUpdater?
    private let info: UpdateInfo
    private let path: String
    private let lock = NSLock()
    private var lastReport = Date.distantPast

    init(updater: WorkerUpdater, info: UpdateInfo, path: String) {
        self.updater = updater
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
            MainActor.assumeIsolated { self?.updater?.progress(info, done: done, total: total) }
        }
    }

    func onFinished(error: String?) {
        let info = info, path = path
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated { self?.updater?.finished(info, path: path, error: error) }
        }
    }
}
