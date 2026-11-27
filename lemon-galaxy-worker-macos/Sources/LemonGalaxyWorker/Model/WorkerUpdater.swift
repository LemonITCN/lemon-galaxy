import Foundation
import AppKit

/// 检查、下载与自动安装更新（docs/ffi.md 5.0 / 5.1；安装流程见 Shared/AppUpdateInstaller.swift，docs/spec.md「Mac 自动更新」）。
/// WorkerHost.check_update 用任一有效账号的令牌向自己的服务器查询，返回前已验发布签名；download_update 下载时核对 SHA-256。
/// 上线后与之后每 6 小时在后台检查一次，有新版本时菜单栏菜单里加一项提示；「自动更新」开着（默认）时先下载好。
/// 不会自己装：用户点「立即更新」（本机或远程控制这台 Mac 时都一样）后一路自动做完，中间不弹任何要点的东西：
///   - 没开无人值守：本应用自己下载到缓存目录、核对、替换、退出，助手打开新版本（同 Mac 主控端）；
///   - 开了无人值守：交给后台服务（root）下载与替换（Unattended/DaemonUpdater.swift），后台服务与会话代理由 launchd
///     用新程序重启，本界面重连时发现磁盘上的应用换了版本就自己重新打开（UnattendedController.relaunchIfStale）。
/// 新版本没能正常启动时助手自动换回旧版本，这个版本以后不再自动下载。
@MainActor
final class WorkerUpdater: ObservableObject {
    enum State: Equatable {
        case idle
        case checking
        case upToDate
        case available(UpdateInfo)
        case downloading(UpdateInfo, done: UInt64, total: UInt64)
        /// 已下载（本机缓存目录，或无人值守时后台服务的缓存目录），点「立即更新」就装
        case ready(UpdateInfo)
        case installing(UpdateInfo, AppUpdate.Step)
        case failed(String)
    }

    static let platform = "macos"
    /// 自动检查的间隔
    static let checkInterval: Double = 6 * 3600
    /// 计划、启动成功标记、结果、不再自动下载的版本（没开无人值守时用；后台服务有自己的）
    static var stateDir: URL { AppInfo.dataDir.appendingPathComponent("update", isDirectory: true) }
    static var downloadDir: URL { AppInfo.cacheDir.appendingPathComponent("updates", isDirectory: true) }

    @Published var state: State = .idle
    /// 已知的新版本（菜单栏菜单提示）；直到换成新版本重新启动前一直保留
    @Published var available: UpdateInfo? {
        // 侧栏「关于」的小红点观察的是 WorkerModel
        willSet { model?.objectWillChange.send() }
    }
    @Published var lastChecked: Date?
    /// 下载中点了「立即更新」：下载完接着装
    @Published var installWhenReady = false
    /// 应用所在位置不能自动替换的原因（没开无人值守时由本应用替换；开了由后台服务替换，原因在点了之后报）
    @Published var blocker: AppUpdateBlocker?
    weak var model: WorkerModel?
    private var listener: WorkerDownloadListener?
    private var timer: Timer?
    private var downloadPath: String?
    private var lastDaemonStatus: DaemonUpdateStatus?

    var busy: Bool {
        switch state {
        case .checking, .downloading, .installing: return true
        default: return false
        }
    }

    var isInstalling: Bool { if case .installing = state { return true }; return false }
    private var remote: RemoteWorkerHost? { model?.worker as? RemoteWorkerHost }
    var updateState: AppUpdateState { AppUpdateState(dir: Self.stateDir) }

    // MARK: 启动

    /// 应用启动后调一次：没开无人值守时 5 秒后报告「启动成功」（自动更新后助手在等它）、看上次更新的结果、清理；
    /// 之后每半小时看要不要检查
    func start() {
        guard let model, !model.selfTest else { return }
        AppUpdateEntry.register(self)
        _ = AppUpdateCodeSign.own
        if remote == nil {
            blocker = AppUpdate.blocker()
            let st = updateState, downloads = Self.downloadDir, ref = WorkerUpdaterRef(self)
            DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 5) {
                let fresh = AppUpdate.reportLaunched(state: st)
                AppUpdate.cleanupLeftovers(target: Bundle.main.bundleURL, state: st)
                AppUpdate.pruneDownloads(downloads, current: AppInfo.version)
                let r = st.takeResult()
                DispatchQueue.main.async { MainActor.assumeIsolated { ref.updater?.show(result: r, fresh: fresh) } }
            }
        }
        timer = Timer.scheduledTimer(withTimeInterval: 1800, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.autoCheckIfDue() }
        }
    }

    private func show(result r: AppUpdateResult?, fresh: Bool) {
        guard let model else { return }
        if fresh { model.show(notice: L("update.installed", AppInfo.version)) }
        guard let r else { return }
        if r.outcome == "ok" {
            if fresh { return }
            if r.toVersion == AppInfo.version { model.show(notice: L("update.installed", AppInfo.version)) }
        } else {
            state = .failed(L("update.rolledBack", r.toVersion, r.fromVersion))
        }
        UIHooks.update(model)
    }

    // MARK: 检查

    /// 上线后、定时器每半小时调用：距上次自动检查满 6 小时才查，失败不打扰用户
    func autoCheckIfDue() {
        guard let model, !model.selfTest, !busy, model.hasActiveAccount, model.status.online else { return }
        let now = Date().timeIntervalSince1970
        guard now - model.settings.lastUpdateCheck >= Self.checkInterval else { return }
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
                // 自动更新开着：先在后台下载好（换回过的版本不再自动下载；无人值守时后台服务自己判断）
                if model.settings.autoUpdate, !model.selfTest, remote != nil || !updateState.failedVersions.contains(info.version) {
                    download(install: false)
                }
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

    // MARK: 下载

    /// 下载到缓存目录（无人值守时由后台服务下载到它的缓存目录）。已下载过就直接就绪
    func download(install: Bool) {
        guard let model, let info = available else { return }
        if install { installWhenReady = true }
        switch state {
        case .downloading, .installing, .checking: return
        default: break
        }
        if remote != nil {
            // 进度由后台服务推来（换回过的版本它不会下载，界面就停在「有新版本」）
            sendToDaemon(info, install: install)
            return
        }
        let dir = Self.downloadDir
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let name = (info.fileName as NSString).lastPathComponent
        let dest = dir.appendingPathComponent(name.isEmpty ? "LemonGalaxyWorker-\(info.version)-macOS.dmg" : name)
        if FileManager.default.fileExists(atPath: dest.path) {
            finished(info, path: dest.path, error: nil)
            return
        }
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
            installWhenReady = false
            state = .failed(L("update.downloadFailed", error))
            return
        }
        // 模拟内核只报进度不写文件（自检、演示）：照样显示「已下载」，装不了
        guard FileManager.default.fileExists(atPath: path) || model?.core.isMock == true else {
            installWhenReady = false
            state = .failed(L("install.err.noFile"))
            return
        }
        downloadPath = path
        state = .ready(info)
        if installWhenReady { install() }
    }

    // MARK: 安装

    /// 零交互升级（AppUpdateEntry.startUpdateNow 在主线程上调到这里）：把 info 当成要装的新版本，然后和「立即更新」一样
    func startUpdateNow(_ info: UpdateInfo) {
        guard AppVersion.newer(info.version, than: AppInfo.version) else {
            NSLog("LemonGalaxyWorker: 要求升级到 %@，不比当前 %@ 新，不处理", info.version, AppInfo.version)
            return
        }
        switch state {
        case .installing:
            return
        case .checking:
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self] in self?.startUpdateNow(info) }
            return
        case .downloading where remote == nil:
            installWhenReady = true
            return
        case .ready(let cur) where cur.version == info.version:
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
        guard let model, !model.selfTest else { return }
        if remote != nil {
            // 无人值守：后台服务下载（已下载就跳过，正在下载就下载完接着装）、核对、替换、重启
            guard let info = available else { return }
            switch state {
            case .installing, .checking: return
            case .downloading: break
            default: state = .installing(info, .verifying)
            }
            installWhenReady = true
            sendToDaemon(info, install: true)
            return
        }
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

    /// 没开无人值守：本应用自己核对、替换、起助手，然后退出（助手打开新版本）
    private func install() {
        guard case .ready(let info) = state, let path = downloadPath, let model, !model.selfTest, remote == nil else { return }
        installWhenReady = false
        guard let req = AppUpdateCodeSign.ownRequirement, let bundleId = Bundle.main.bundleIdentifier else {
            state = .failed(AppUpdateBlocker.devBuild.message)
            return
        }
        state = .installing(info, .verifying)
        let r = AppUpdate.Request(dmg: URL(fileURLWithPath: path), target: Bundle.main.bundleURL, bundleId: bundleId,
                                  currentVersion: AppInfo.version, currentBuild: AppInfo.build, requirement: req, state: updateState)
        let ref = WorkerUpdaterRef(self)
        DispatchQueue.global(qos: .userInitiated).async {
            do {
                _ = try AppUpdate.install(r) { step in
                    DispatchQueue.main.async { MainActor.assumeIsolated { ref.updater?.stepped(info, step) } }
                }
                DispatchQueue.main.async { MainActor.assumeIsolated { ref.updater?.quitForUpdate() } }
            } catch {
                let e = error as? AppUpdateError
                // 包本身有问题（版本不对、签名不对、不完整、打不开）：删掉缓存，下次重新下载
                if let k = e?.kind, [.notNewer, .signature, .badBundle, .noApp, .mount].contains(k) { try? FileManager.default.removeItem(atPath: path) }
                let msg = L("update.installFailed", e?.detail ?? "\(error)")
                DispatchQueue.main.async { MainActor.assumeIsolated { ref.updater?.state = .failed(msg) } }
            }
        }
    }

    private func stepped(_ info: UpdateInfo, _ step: AppUpdate.Step) {
        if isInstalling { state = .installing(info, step) }
    }

    /// 新版本已换上、助手已起：退出（助手等本进程退出后打开新版本）。退出被卡住时 10 秒后直接结束
    private func quitForUpdate() {
        NSLog("LemonGalaxyWorker: 已换上新版本，退出后由更新助手重新打开")
        DispatchQueue.global().asyncAfter(deadline: .now() + 10) { exit(0) }
        NSApp.terminate(nil)
    }

    // MARK: 无人值守：交给后台服务、后台服务推来的进度

    private func sendToDaemon(_ info: UpdateInfo, install: Bool) {
        guard let remote else { return }
        // 之后的推送哪怕和上次一样（例如同样的失败原因）也要照着显示
        lastDaemonStatus = nil
        Task {
            do {
                _ = try await remote.call(IPCMethod.updateRequest, CUpdateRequest(info: info, install: install), timeout: 30)
            } catch {
                installWhenReady = false
                state = .failed(L(install ? "update.installFailed" : "update.downloadFailed", describeError(error)))
            }
        }
    }

    func daemonStatus(_ s: DaemonUpdateStatus?) {
        guard remote != nil, let s, s != lastDaemonStatus else { return }
        lastDaemonStatus = s
        let info: UpdateInfo
        if let a = available, a.version == s.version { info = a } else {
            info = UpdateInfo(version: s.version, fileName: "", size: s.total, sha256: "", url: "", notes: "")
        }
        switch s.phase {
        case "downloading":
            state = .downloading(info, done: s.done, total: s.total)
        case "ready":
            if case .installing = state { return }
            state = .ready(info)
        case "installing":
            let step: AppUpdate.Step = s.message == "replacing" ? .replacing : s.message == "restarting" ? .restarting : .verifying
            state = .installing(info, step)
        case "restarting":
            state = .installing(info, .restarting)
        case "failed":
            installWhenReady = false
            state = .failed(s.message ?? "")
        case "rolledBack":
            installWhenReady = false
            state = .failed(L("update.rolledBack", s.version, s.message ?? AppInfo.version))
        case "updated":
            if s.version == AppInfo.version { model?.show(notice: L("update.installed", AppInfo.version)) }
        default:
            break
        }
        if let model { UIHooks.update(model) }
    }

    /// 失败后回到「有新版本」或初始状态
    func reset() {
        if busy { return }
        if case .ready = state { return }
        if let a = available { state = .available(a) } else { state = .idle }
    }
}

/// 零交互升级的唯一入口：界面上的「立即更新」与主控端远程要求「升级被控端」（内核回调，参数是查到的新版本）走同一条路径。
/// 任意线程可调、立即返回，在后台一路做完，中间不弹任何要点的东西：
///   - 在无人值守的后台服务进程里（内核在这里，root）：交给 DaemonUpdater——下载到 /Library/Caches、核对、替换
///     /Applications 里的应用、后台服务退出由 launchd 用新程序重启，会话代理与菜单栏应用随后各自换成新版本；
///   - 在菜单栏应用里（没开无人值守，内核在这里）：交给 WorkerUpdater——下载到缓存目录、核对、替换、退出，助手打开新版本；
///     开了无人值守时菜单栏应用里没有内核，界面上的「立即更新」经本机通信交给后台服务。
/// 新版本没能正常启动时助手自动换回旧版本。应用还没起好（模型没登记）时先记下，登记时接着做。
enum AppUpdateEntry {
    @MainActor private static weak var updater: WorkerUpdater?
    @MainActor private static var queued: UpdateInfo?

    static func startUpdateNow(info: UpdateInfo) {
        if let d = DaemonUpdater.current {
            d.startUpdateNow(info)
            return
        }
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                if let u = updater { u.startUpdateNow(info) } else { queued = info }
            }
        }
    }

    @MainActor static func register(_ u: WorkerUpdater) {
        updater = u
        if let q = queued {
            queued = nil
            u.startUpdateNow(q)
        }
    }
}

/// 后台线程回到主线程时找回模型（不把 MainActor 对象直接抓进并发闭包）
private final class WorkerUpdaterRef: @unchecked Sendable {
    weak var updater: WorkerUpdater?
    init(_ u: WorkerUpdater) { updater = u }
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
