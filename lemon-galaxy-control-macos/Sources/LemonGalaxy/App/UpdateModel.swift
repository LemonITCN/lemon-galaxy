import Foundation
import AppKit

/// 检查、下载与自动安装更新（ffi.md 5.0；安装流程见 Platform/AppUpdateInstaller.swift，docs/spec.md「Mac 自动更新」）。
/// 内核向自己的服务器查询，返回前已验发布签名；下载时核对 SHA-256。
/// 登录上线后与之后每 6 小时在后台检查一次，有新版本在设置图标上标小红点；「自动更新」开着（默认）时先下载到缓存目录。
/// 不会自己装：用户点「立即更新」（本机或远程控制这台 Mac 时都一样）后一路自动做完——没下载的先下载、核对签名、
/// 替换应用、关掉旧的、打开新的，中间不弹任何要点的东西。新版本没能正常启动时助手自动换回旧版本，这个版本以后不再自动下载。
@MainActor
final class UpdateModel: ObservableObject {
    enum State: Equatable {
        case idle
        case checking
        case upToDate
        case available(UpdateInfo)
        case downloading(UpdateInfo, done: UInt64, total: UInt64)
        /// 已下载到缓存目录（内核核对过 SHA-256），点「立即更新」就装
        case ready(UpdateInfo, path: String)
        case installing(UpdateInfo, AppUpdate.Step)
        case failed(String)
    }

    @Published var state: State = .idle
    /// 已知的新版本（设置图标的小红点）；直到换成新版本重新启动前一直保留
    @Published var available: UpdateInfo? {
        // 侧栏、设置导航观察的是 AppModel：小红点跟着变
        willSet { app?.objectWillChange.send() }
    }
    @Published var lastChecked: Date?
    /// 下载中点了「立即更新」：下载完接着装
    @Published var installWhenReady = false
    /// 应用所在位置不能自动替换的原因（设置里说明，「立即更新」不可用）
    @Published var blocker: AppUpdateBlocker?
    weak var app: AppModel?
    private var listener: UpdateDownloadListener?
    private var timer: Timer?

    static let component = "control"
    static let platform = "macos"
    /// 自动检查的间隔
    static let checkInterval: Double = 6 * 3600
    /// 计划、启动成功标记、结果、不再自动下载的版本（只有本用户能读写）
    static var stateDir: URL { AppModel.dataDir.appendingPathComponent("update", isDirectory: true) }
    /// 下载的安装包（不放「下载」文件夹）
    static var downloadDir: URL { AppModel.cacheDir.appendingPathComponent("updates", isDirectory: true) }
    var updateState: AppUpdateState { AppUpdateState(dir: Self.stateDir) }

    var busy: Bool {
        switch state {
        case .checking, .downloading, .installing: return true
        default: return false
        }
    }

    var isInstalling: Bool { if case .installing = state { return true }; return false }

    // MARK: 启动

    /// 应用启动后调一次：5 秒后报告「启动成功」（自动更新后助手在等它）、看上次更新的结果、清理；之后每半小时看要不要检查
    func start() {
        guard let app, !app.selfTest else { return }
        AppUpdateEntry.register(self)
        _ = AppUpdateCodeSign.own
        blocker = AppUpdate.blocker()
        let st = updateState, downloads = Self.downloadDir, ref = UpdateModelRef(self)
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 5) {
            let fresh = AppUpdate.reportLaunched(state: st)
            AppUpdate.cleanupLeftovers(target: Bundle.main.bundleURL, state: st)
            AppUpdate.pruneDownloads(downloads, current: AppInfo.version)
            let r = st.takeResult()
            DispatchQueue.main.async { MainActor.assumeIsolated { ref.model?.show(result: r, fresh: fresh) } }
        }
        timer = Timer.scheduledTimer(withTimeInterval: 1800, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.autoCheckIfDue() }
        }
    }

    private func show(result r: AppUpdateResult?, fresh: Bool) {
        guard let app else { return }
        if fresh { app.showToast(L("update.installed", AppInfo.version)) }
        guard let r else { return }
        if r.outcome == "ok" {
            if fresh { return }
            if r.toVersion == AppInfo.version { app.showToast(L("update.installed", AppInfo.version)) }
        } else {
            state = .failed(L("update.rolledBack", r.toVersion, r.fromVersion))
        }
    }

    // MARK: 检查

    /// 登录上线后、定时器每半小时调用：距上次自动检查满 6 小时才查，失败不打扰用户
    func autoCheckIfDue() {
        guard let app, !app.selfTest, !busy, app.clientState == .online else { return }
        let now = Date().timeIntervalSince1970
        guard now - app.settings.lastUpdateCheck >= Self.checkInterval else { return }
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
                // 自动更新开着：先在后台下载好（换回过的版本不再自动下载）
                if app.settings.autoUpdate, !app.selfTest, !updateState.failedVersions.contains(info.version) { download(install: false) }
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

    // MARK: 下载

    /// 下载到缓存目录。已下载过（内核核对 SHA-256 后才改成正式文件名）就直接就绪
    func download(install: Bool) {
        guard let app, let info = available else { return }
        if install { installWhenReady = true }
        switch state {
        case .downloading, .installing, .checking: return
        default: break
        }
        let dir = Self.downloadDir
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let name = (info.fileName as NSString).lastPathComponent
        let dest = dir.appendingPathComponent(name.isEmpty ? "LemonGalaxy-\(info.version)-macOS.dmg" : name)
        if FileManager.default.fileExists(atPath: dest.path) {
            finished(info, path: dest.path, error: nil)
            return
        }
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
            installWhenReady = false
            state = .failed(L("update.downloadFailed", error))
            return
        }
        state = .ready(info, path: path)
        if installWhenReady { install() }
    }

    // MARK: 安装

    /// 零交互升级（AppUpdateEntry.startUpdateNow 在主线程上调到这里）：把 info 当成要装的新版本，然后和「立即更新」一样
    func startUpdateNow(_ info: UpdateInfo) {
        guard AppVersion.newer(info.version, than: AppInfo.version) else {
            NSLog("LemonGalaxy: 要求升级到 %@，不比当前 %@ 新，不处理", info.version, AppInfo.version)
            return
        }
        switch state {
        case .installing:
            return
        case .checking:
            // 正在检查：查完再来（检查不会很久）
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self] in self?.startUpdateNow(info) }
            return
        case .downloading:
            // 正在下载（也是比当前新的版本）：下载完接着装
            installWhenReady = true
            return
        case .ready(let cur, _) where cur.version == info.version:
            installNow()
            return
        default:
            break
        }
        available = info
        state = .available(info)
        installNow()
    }

    /// 「立即更新」：没下载的先下载，下载中的下载完接着装，已下载的马上装
    func installNow() {
        guard let app, !app.selfTest else { return }
        if let b = AppUpdate.blocker() {
            blocker = b
            state = .failed(b.message)
            return
        }
        switch state {
        case .ready: install()
        case .downloading: installWhenReady = true
        case .installing, .checking: break
        default: if available != nil { download(install: true) }
        }
    }

    /// 核对、替换、起助手，然后退出（助手打开新版本）
    private func install() {
        guard case .ready(let info, let path) = state, let app, !app.selfTest else { return }
        installWhenReady = false
        guard let req = AppUpdateCodeSign.ownRequirement, let bundleId = Bundle.main.bundleIdentifier else {
            state = .failed(AppUpdateBlocker.devBuild.message)
            return
        }
        state = .installing(info, .verifying)
        let r = AppUpdate.Request(dmg: URL(fileURLWithPath: path), target: Bundle.main.bundleURL, bundleId: bundleId,
                                  currentVersion: AppInfo.version, currentBuild: AppInfo.build, requirement: req, state: updateState)
        let ref = UpdateModelRef(self)
        DispatchQueue.global(qos: .userInitiated).async {
            do {
                _ = try AppUpdate.install(r) { step in
                    DispatchQueue.main.async { MainActor.assumeIsolated { ref.model?.stepped(info, step) } }
                }
                DispatchQueue.main.async { MainActor.assumeIsolated { ref.model?.quitForUpdate() } }
            } catch {
                let e = error as? AppUpdateError
                // 包本身有问题（版本不对、签名不对、不完整、打不开）：删掉缓存，下次重新下载
                if let k = e?.kind, [.notNewer, .signature, .badBundle, .noApp, .mount].contains(k) { try? FileManager.default.removeItem(atPath: path) }
                let msg = L("update.installFailed", e?.detail ?? "\(error)")
                DispatchQueue.main.async { MainActor.assumeIsolated { ref.model?.state = .failed(msg) } }
            }
        }
    }

    private func stepped(_ info: UpdateInfo, _ step: AppUpdate.Step) {
        if isInstalling { state = .installing(info, step) }
    }

    /// 新版本已换上、助手已起：退出（助手等本进程退出后打开新版本）。退出被卡住时 10 秒后直接结束
    private func quitForUpdate() {
        NSLog("LemonGalaxy: 已换上新版本，退出后由更新助手重新打开")
        DispatchQueue.global().asyncAfter(deadline: .now() + 10) { exit(0) }
        NSApp.terminate(nil)
    }

    /// 关掉更新卡片后回到「有新版本 / 已下载」状态（失败后再点可以重来）
    func reset() {
        if busy { return }
        if case .ready = state { return }
        if let a = available { state = .available(a) } else { state = .idle }
    }
}

/// 零交互升级的唯一入口：界面上的「立即更新」与主控端远程要求「升级被控端」（内核回调，参数是查到的新版本）走同一条路径。
/// 任意线程可调、立即返回；在后台一路做完：没下载的先下载到缓存目录、核对签名与版本、替换应用、关闭、
/// 助手打开新版本并等它报告启动成功（不成功就换回旧版本）。中间不弹任何要点的东西。
/// 应用还没起好（模型没登记）时先记下，登记时接着做；不能自动替换（位置不可写等）时只在设置里显示原因。
enum AppUpdateEntry {
    @MainActor private static weak var model: UpdateModel?
    @MainActor private static var queued: UpdateInfo?

    static func startUpdateNow(info: UpdateInfo) {
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                if let m = model { m.startUpdateNow(info) } else { queued = info }
            }
        }
    }

    @MainActor static func register(_ m: UpdateModel) {
        model = m
        if let q = queued {
            queued = nil
            m.startUpdateNow(q)
        }
    }
}

/// 后台线程回到主线程时找回模型（不把 MainActor 对象直接抓进并发闭包）
private final class UpdateModelRef: @unchecked Sendable {
    weak var model: UpdateModel?
    init(_ m: UpdateModel) { model = m }
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
