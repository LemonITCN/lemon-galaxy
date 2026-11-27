import Foundation

/// 无人值守下的自动更新（后台服务一侧，root；docs/spec.md「Mac 自动更新」）。
/// 菜单栏应用点「立即更新」（本机或远程控制这台 Mac 时都一样）后，由后台服务一路做完，不弹任何东西：
///   1. 下载到 /Library/Caches/cn.lemonit.galaxy.worker/updates（只有 root 能写；内核已验发布签名与 SHA-256）；
///   2. 挂载、用本程序的指定要求严格核对新应用、拷到 /Applications 旁边的隐藏位置、原子对换（AppUpdateInstaller.swift）；
///      新包的属主改成和原来一样，关闭无人值守后菜单栏应用照样能自己更新；
///   3. 用旧程序起助手（root，新会话），后台服务退出；launchd 按登记的 BundleProgram（包内主程序的路径没变）用新程序重启它；
///   4. 新后台服务开始监听 5 秒后报告启动成功，助手再等一会儿（让还是旧程序的会话代理重连时被要求重启）后删掉旧包；
///      150 秒内没报告：助手结束新后台服务与会话代理、换回旧包、让 launchd 立刻启动旧后台服务，记下这个版本不再自动下载；
///   5. 会话代理重连时版本不同（或旧包已删、签名对不上）被要求退出，launchd 用包里的新程序重启它（/Library/LaunchAgents 里写的是绝对路径）；
///      菜单栏应用重连时发现磁盘上的应用换了版本，自己重新打开成新版本（UnattendedController.relaunchIfStale）。
/// 谁能点：管理员与正坐在这台 Mac 前的用户（与检查更新相同）。只能装比当前新、签名身份与当前应用相同、
/// 发布签名有效的版本（和 Linux 被控服务每 6 小时自己更新是同样的信任范围），不能拿它装任意程序。
final class DaemonUpdater: @unchecked Sendable {
    struct Options {
        var target = Bundle.main.bundleURL
        var stateDir: URL
        var downloadDir: URL
        /// none：由 launchd 重启；自检里用 probe
        var launch = "none"
        var timeout: Double = 150
        var label: String? = Unattended.daemonLabel
    }

    /// 本进程里的后台服务（AppUpdateEntry 在后台服务进程里把零交互升级交给它）
    static var current: DaemonUpdater? { currentLock.locked { currentRef.value } }
    private static let currentLock = NSLock()
    nonisolated(unsafe) private static var currentRef = WeakDaemonUpdater()

    let opts: Options
    let worker: WorkerHost
    /// 进度变化（推给界面）
    var onChange: (@Sendable () -> Void)?
    /// 换好包、助手已起：让后台服务退出（launchd 重启）
    var restart: (@Sendable () -> Void)?
    private let lock = NSLock()
    private var status_: DaemonUpdateStatus?
    private var downloading: String?
    private var installAfterDownload = false
    private var lastProgress = Date.distantPast

    init(worker: WorkerHost, opts: Options) {
        self.worker = worker
        self.opts = opts
        try? FileManager.default.createDirectory(at: opts.downloadDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o755])
        state.ensure()
        Self.currentLock.locked { Self.currentRef.value = self }
    }

    /// 零交互升级（AppUpdateEntry.startUpdateNow 在后台服务进程里调到这里；任意线程，立即返回）。
    /// 和界面上的「立即更新」同一条路径；不能做时把原因放进进度（界面显示）并记日志
    func startUpdateNow(_ info: UpdateInfo) {
        NSLog("LemonGalaxy: 收到升级到 %@ 的要求", info.version)
        // 已经在装：不打断，也不把进度改成失败
        if let p = status?.phase, p == "installing" || p == "restarting" { return }
        do {
            try request(info, install: true)
        } catch {
            let msg = (error as? IPCFault)?.detail ?? "\(error)"
            NSLog("LemonGalaxy: 不能升级到 %@：%@", info.version, msg)
            set(DaemonUpdateStatus(phase: "failed", version: info.version, message: L("update.installFailed", msg)))
        }
    }

    var state: AppUpdateState { AppUpdateState(dir: opts.stateDir) }
    var status: DaemonUpdateStatus? { lock.locked { status_ } }

    private func set(_ s: DaemonUpdateStatus?) {
        lock.locked { status_ = s }
        onChange?()
    }

    /// 后台服务开始监听 5 秒后：报告启动成功（助手在等）、清理上次留下的、看上次更新的结果
    func launched() {
        let st = state
        let fresh = AppUpdate.reportLaunched(state: st)
        AppUpdate.cleanupLeftovers(target: opts.target, state: st)
        AppUpdate.pruneDownloads(opts.downloadDir, current: AppInfo.version)
        // 自己就是刚换上的新版本：告诉界面「已更新」（菜单栏应用重新打开后提示）
        if fresh {
            NSLog("LemonGalaxy: 已更新到 %@（%@）", AppInfo.version, AppInfo.build)
            set(DaemonUpdateStatus(phase: "updated", version: AppInfo.version))
        }
        guard let r = st.takeResult() else { return }
        NSLog("LemonGalaxy: 上次更新 %@ → %@：%@ %@", r.fromVersion, r.toVersion, r.outcome, r.reason)
        if r.outcome == "ok" {
            set(DaemonUpdateStatus(phase: "updated", version: r.toVersion))
        } else {
            set(DaemonUpdateStatus(phase: "rolledBack", version: r.toVersion, message: r.fromVersion))
        }
    }

    func file(for info: UpdateInfo) -> URL {
        let name = (info.fileName as NSString).lastPathComponent
        return opts.downloadDir.appendingPathComponent(name.isEmpty ? "LemonGalaxyWorker-\(info.version)-macOS.dmg" : name)
    }

    /// 界面要求预先下载（install 为假，换回过的版本不再自动下载）或立即更新（下载完接着装）
    func request(_ info: UpdateInfo, install: Bool) throws {
        guard AppVersion.newer(info.version, than: AppInfo.version) else {
            throw IPCFault.other(L("install.err.notNewer", info.version, AppInfo.version))
        }
        if install, let b = AppUpdate.blocker(for: opts.target) { throw IPCFault.other(b.message) }
        if !install && state.failedVersions.contains(info.version) { return }
        let busy = lock.locked { () -> Bool in
            guard let p = status_?.phase else { return false }
            return p == "installing" || p == "restarting"
        }
        if busy { throw IPCFault.other(L("install.err.busy")) }
        let dest = file(for: info)
        if FileManager.default.fileExists(atPath: dest.path) {
            set(DaemonUpdateStatus(phase: "ready", version: info.version, done: info.size, total: info.size))
            if install { startInstall(info, dest) }
            return
        }
        let start = lock.locked { () -> Bool in
            if install { installAfterDownload = true }
            guard downloading == nil else { return false }
            downloading = info.version
            return true
        }
        guard start else { return }
        set(DaemonUpdateStatus(phase: "downloading", version: info.version, done: 0, total: info.size))
        worker.downloadUpdate(info: info, dest: dest.path, listener: DaemonUpdateListener(owner: self, info: info, path: dest))
    }

    fileprivate func progress(_ info: UpdateInfo, done: UInt64, total: UInt64) {
        let now = Date()
        let send = lock.locked { () -> Bool in
            // 完成之后才到的进度不算（不能把「已下载 / 失败」又盖回「下载中」）
            guard downloading == info.version else { return false }
            guard now.timeIntervalSince(lastProgress) >= 0.25 || done >= total else { return false }
            lastProgress = now
            return true
        }
        if send { set(DaemonUpdateStatus(phase: "downloading", version: info.version, done: done, total: total == 0 ? info.size : total)) }
    }

    fileprivate func finished(_ info: UpdateInfo, path: URL, error: String?) {
        let install = lock.locked { () -> Bool in
            defer { downloading = nil; installAfterDownload = false }
            return installAfterDownload
        }
        if let error {
            set(DaemonUpdateStatus(phase: "failed", version: info.version, message: L("update.downloadFailed", error)))
            return
        }
        guard FileManager.default.fileExists(atPath: path.path) else {
            set(DaemonUpdateStatus(phase: "failed", version: info.version, message: L("install.err.noFile")))
            return
        }
        chmod(path.path, 0o644)
        set(DaemonUpdateStatus(phase: "ready", version: info.version, done: info.size, total: info.size))
        if install { startInstall(info, path) }
    }

    private func startInstall(_ info: UpdateInfo, _ dmg: URL) {
        guard let req = AppUpdateCodeSign.ownRequirement, let id = Bundle.main.bundleIdentifier else {
            set(DaemonUpdateStatus(phase: "failed", version: info.version, message: AppUpdateBlocker.devBuild.message))
            return
        }
        set(DaemonUpdateStatus(phase: "installing", version: info.version, message: "verifying"))
        let opts = opts, st = state
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            var r = AppUpdate.Request(dmg: dmg, target: opts.target, bundleId: id, currentVersion: AppInfo.version, currentBuild: AppInfo.build,
                                      requirement: req, state: st)
            r.mode = "daemon"
            r.launch = opts.launch
            r.timeout = opts.timeout
            r.launchdLabel = opts.label
            // 挂载点放在只有 root 能进的状态目录里
            r.workDir = opts.stateDir
            do {
                let plan = try AppUpdate.install(r) { step in
                    let s: String
                    switch step {
                    case .verifying: s = "verifying"
                    case .replacing: s = "replacing"
                    case .restarting: s = "restarting"
                    }
                    self?.set(DaemonUpdateStatus(phase: "installing", version: info.version, message: s))
                }
                NSLog("LemonGalaxy: 已换成 %@（%@），后台服务退出后由 launchd 用新程序重启", plan.toVersion, plan.toBuild)
                self?.set(DaemonUpdateStatus(phase: "restarting", version: plan.toVersion))
                // 让「正在重启」推到界面再退出
                DispatchQueue.global().asyncAfter(deadline: .now() + 0.8) { self?.restart?() }
            } catch {
                let e = error as? AppUpdateError
                NSLog("LemonGalaxy: 自动更新失败：%@", String(describing: error))
                if let k = e?.kind, [.notNewer, .signature, .badBundle, .noApp, .mount].contains(k) { try? FileManager.default.removeItem(at: dmg) }
                self?.set(DaemonUpdateStatus(phase: "failed", version: info.version, message: L("update.installFailed", e?.detail ?? "\(error)")))
            }
        }
    }
}

final class WeakDaemonUpdater {
    weak var value: DaemonUpdater?
}

final class DaemonUpdateListener: TransferListener, @unchecked Sendable {
    private weak var owner: DaemonUpdater?
    private let info: UpdateInfo
    private let path: URL
    init(owner: DaemonUpdater, info: UpdateInfo, path: URL) {
        self.owner = owner
        self.info = info
        self.path = path
    }
    func onProgress(done: UInt64, total: UInt64) { owner?.progress(info, done: done, total: total) }
    func onFinished(error: String?) { owner?.finished(info, path: path, error: error) }
}

// MARK: - 命令行

/// --unattended-update [--socket 路径]：不开窗口，让无人值守的后台服务检查新版本并立即更新（和界面上的「立即更新」同一条路径，
/// 零交互），等它换成新版本重新上线后退出。管理员或正坐在这台 Mac 前的用户可用（例如 ssh 登录后运行）。
/// 退出码：0 已更新或已是最新；1 失败或新版本没起来（已换回旧版本）。
enum UnattendedUpdateCLI {
    static func run(_ args: [String]) -> Int32 {
        setvbuf(stdout, nil, _IOLBF, 0)
        let socket = Unattended.arg(args, "--socket") ?? Unattended.socketPath
        Localization.shared.apply(setting: "system")
        let latest = Box<DaemonInfo?>(nil)

        func connect() -> (IPCConnection, DaemonInfo)? {
            guard let fd = try? IPCSocket.connect(path: socket) else { return nil }
            let c = IPCConnection(fd: fd)
            c.start(onNotify: { m in
                if m.method == IPCMethod.pushInfo, let i = try? m.decode(DaemonInfo.self) { latest.value = i }
            })
            // 正式套接字只认 root 的后台服务
            if socket == Unattended.defaultSocket && c.peer.uid != 0 { c.close(); return nil }
            guard let i = try? c.call(IPCMethod.hello, ControlHello(version: AppInfo.version, build: AppInfo.build, locale: ""), timeout: 5).decode(DaemonInfo.self) else {
                c.close()
                return nil
            }
            latest.value = i
            return (c, i)
        }

        guard let (c, first) = connect() else {
            print("连不上后台服务（\(socket)），或它拒绝了本次请求（要管理员或正坐在这台 Mac 前的用户）")
            return 1
        }
        print("后台服务 \(first.version)（\(first.build)）")
        let info: UpdateInfo
        do {
            guard let i = try c.call(IPCMethod.checkUpdate, CUpdateCheck(platform: "macos", current: first.version), timeout: 60)
                    .decode(CUpdateInfo.self).info else {
                print("已是最新版本")
                c.close()
                return 0
            }
            info = i
            _ = try c.call(IPCMethod.updateRequest, CUpdateRequest(info: i, install: true), timeout: 30)
        } catch {
            print("失败：\((error as? IPCError).map { "\($0)" } ?? describeError(error))")
            c.close()
            return 1
        }
        print("开始更新到 \(info.version)")
        var conn: IPCConnection? = c
        var lastPhase = ""
        let end = Date().addingTimeInterval(15 * 60)
        while Date() < end {
            if let cur = conn, cur.isClosed { conn = nil; print("后台服务正在重启…") }
            if conn == nil {
                if let (c2, i2) = connect() {
                    conn = c2
                    if i2.version != first.version || i2.build != first.build {
                        print("已更新：后台服务现在是 \(i2.version)（\(i2.build)）")
                        c2.close()
                        return 0
                    }
                }
            }
            if let u = latest.value?.update, u.version == info.version || u.phase == "rolledBack" {
                // 下载进度只报一次「下载中」，其余阶段变了就打印
                let key = u.phase + (u.message ?? "")
                if key != lastPhase {
                    lastPhase = key
                    print(u.phase == "downloading" ? "下载中…" : u.phase + (u.message.map { "：" + $0 } ?? ""))
                }
                if u.phase == "failed" || u.phase == "rolledBack" {
                    conn?.close()
                    return 1
                }
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
        print("等待超时")
        return 1
    }
}
