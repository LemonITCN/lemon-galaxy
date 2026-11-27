import AppKit
import ServiceManagement

/// 菜单栏应用里的「无人值守」开关：登记 / 注销后台服务（SMAppService.daemon，macOS 13 起），
/// 引导用户在「系统设置 → 通用 → 登录项」里批准，开启后把账号与设备身份移交给后台服务、自己改当界面；关闭时拿回来。
/// 两种运行方式切换时重新打开应用（菜单栏应用自己的内核没法中途停掉）。
@MainActor
final class UnattendedController: ObservableObject {
    enum Phase: Equatable {
        case unavailable(String)
        case off
        case needsApproval
        case starting
        case migrating
        case running
        case error(String)
    }

    @Published var phase: Phase = .off
    @Published var daemon: DaemonInfo?
    @Published var busy = false
    @Published var confirmDisable = false
    /// 菜单栏应用正在当后台服务的界面（启动时决定）
    @Published private(set) var clientMode: Bool
    let selfTest: Bool
    weak var model: WorkerModel?
    private var pollTimer: Timer?
    private var relaunching = false

    nonisolated static var daemonService: SMAppService { SMAppService.daemon(plistName: Unattended.daemonPlistName) }

    /// 启动时决定运行方式：后台服务已登记并启用 → 当它的界面。
    /// 开发：LEMONGALAXY_CLIENT=1 时连 LEMONGALAXY_SOCKET 指定的后台服务（不用打包、不用 root）。
    nonisolated static func clientModeAtLaunch() -> Bool {
        if ProcessInfo.processInfo.environment["LEMONGALAXY_CLIENT"] == "1" { return true }
        guard AppInfo.isBundled else { return false }
        return daemonService.status == .enabled
    }

    init(clientMode: Bool, selfTest: Bool) {
        self.clientMode = clientMode
        self.selfTest = selfTest
        if clientMode { phase = .starting }
    }

    // MARK: 状态

    var isOn: Bool {
        switch phase {
        case .needsApproval, .starting, .migrating, .running: return true
        case .error: return clientMode
        case .off, .unavailable: return false
        }
    }

    var appInApplications: Bool { Bundle.main.bundlePath.hasPrefix("/Applications/") }

    /// 当前负责画面的会话
    var screenDescription: String {
        guard let a = daemon?.activeAgent else { return L("unattended.screen.none") }
        return a.sessionType == .loginWindow ? L("unattended.screen.loginWindow") : L("unattended.screen.user", a.userName)
    }

    func refresh() {
        guard !selfTest else { return }
        if !AppInfo.isBundled && !clientMode {
            phase = .unavailable(L("unattended.unavailable.notBundled"))
            return
        }
        let status: SMAppService.Status = clientMode && !AppInfo.isBundled ? .enabled : Self.daemonService.status
        switch status {
        case .enabled:
            if clientMode {
                if case .migrating = phase { return }
                phase = daemon != nil ? .running : .starting
            } else {
                phase = .starting
                probeAndSwitch()
            }
        case .requiresApproval:
            phase = .needsApproval
        case .notRegistered, .notFound:
            phase = clientMode ? .error(L("unattended.removedElsewhere")) : .off
        @unknown default:
            phase = .off
        }
        updatePolling()
    }

    /// 后台服务推来的概况（RemoteWorkerHost.onInfo，已切回主线程）
    func received(_ info: DaemonInfo?) {
        daemon = info
        if info != nil, case .migrating = phase { phase = .running }
        refresh()
    }

    private func updatePolling() {
        let need: Bool
        switch phase {
        case .needsApproval, .starting: need = true
        case .error: need = clientMode
        default: need = false
        }
        if need && pollTimer == nil {
            pollTimer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
                MainActor.assumeIsolated { self?.refresh() }
            }
        } else if !need {
            pollTimer?.invalidate()
            pollTimer = nil
        }
    }

    /// 本地模式下后台服务已启用：等它能连上，就重新打开应用改当界面（移交在新进程里做）
    private func probeAndSwitch() {
        Task.detached {
            let ok = UnattendedController.probe()
            await MainActor.run { if ok { self.relaunch() } }
        }
    }

    nonisolated static func probe(socket: String = Unattended.socketPath) -> Bool {
        guard let fd = try? IPCSocket.connect(path: socket) else { return false }
        let c = IPCConnection(fd: fd)
        c.start()
        defer { c.close() }
        return (try? c.call(IPCMethod.hello, ControlHello(version: AppInfo.version, build: AppInfo.build, locale: ""), timeout: 3)) != nil
    }

    // MARK: 开关

    func enable() {
        guard !busy else { return }
        if !AppInfo.isBundled { phase = .unavailable(L("unattended.unavailable.notBundled")); return }
        busy = true
        defer { busy = false }
        do {
            try Self.daemonService.register()
        } catch {
            // 第一次登记一般会报「不允许」并进入「需要批准」，这是正常流程
            if Self.daemonService.status != .requiresApproval {
                phase = .error(L("unattended.failed", error.localizedDescription))
                return
            }
        }
        if Self.daemonService.status == .requiresApproval { SMAppService.openSystemSettingsLoginItems() }
        refresh()
    }

    func openLoginItems() { SMAppService.openSystemSettingsLoginItems() }

    /// 开关拨到关：已经在用就先确认
    func requestDisable() {
        if clientMode || phase == .running { confirmDisable = true } else { Task { await disable() } }
    }

    /// 关闭：管理员把数据拿回当前用户；不是管理员时用移交前留下的备份。然后注销后台服务、重新打开应用
    func disable() async {
        confirmDisable = false
        guard !busy else { return }
        busy = true
        if clientMode {
            let remote = model?.worker as? RemoteWorkerHost
            if let remote, let info = daemon, info.peerAdmin {
                do {
                    let b = try await remote.call(IPCMethod.exportData, IPCEmpty(), timeout: 30).decode(CMigration.self).bundle
                    try Migration.restoreLocal(b)
                    _ = try await remote.call(IPCMethod.release, CRelease(wipe: true), timeout: 30)
                } catch {
                    busy = false
                    phase = .error(L("unattended.disableFailed", describeError(error)))
                    return
                }
            } else {
                _ = Migration.restoreMigratedBackup()
                // 去掉会话代理（数据留在后台服务里，以后管理员再开启时接着用）
                _ = try? await remote?.call(IPCMethod.release, CRelease(wipe: false), timeout: 10)
            }
            remote?.disconnect()
        }
        do {
            try await Self.daemonService.unregister()
        } catch {
            if Self.daemonService.status == .enabled {
                busy = false
                phase = .error(L("unattended.disableFailed", error.localizedDescription))
                return
            }
        }
        busy = false
        if clientMode { relaunch() } else { refresh() }
    }

    /// 换运行方式：1 秒后重新打开自己，然后退出
    func relaunch() {
        guard !relaunching, !selfTest else { return }
        relaunching = true
        model?.show(notice: L("unattended.relaunching"))
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/sh")
        // -n：即使有本应用的进程在跑（会话代理开过隐私屏时会登记到 LaunchServices）也新开界面
        p.arguments = ["-c", "sleep 1; /usr/bin/open -n \"$0\"", Bundle.main.bundlePath]
        try? p.run()
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { NSApp.terminate(nil) }
    }

    // MARK: 连上后台服务时（连接线程上）

    private nonisolated static let restartOnce = OnceFlag()
    private nonisolated static let relaunchOnce = OnceFlag()

    /// 每次连上后台服务、打招呼之前：磁盘上的应用已经换成别的版本（后台服务刚自动更新完，或新版本没起来又换回了旧版本，
    /// 或用户自己拖了新版本进来），本界面是过时的程序——重新打开自己（新程序再连上时版本一致；后台服务若是旧的由新界面让它重启）。
    /// 返回 true 表示正在重新打开，不用这条连接
    nonisolated static func relaunchIfStale() -> Bool {
        guard let disk = AppUpdate.diskBuild(), disk != AppInfo.build else { return false }
        if relaunchOnce.fire() {
            NSLog("LemonGalaxy: 磁盘上的应用已是 %@（本界面 %@），重新打开", disk, AppInfo.build)
            DispatchQueue.main.async { MainActor.assumeIsolated { relaunchSelf() } }
        }
        return true
    }

    /// 本进程退出后由脱离出去的 sh 等它退出再 open -n 打开磁盘上的应用；退出被卡住时 10 秒后直接结束
    static func relaunchSelf() {
        AppUpdate.relaunchAfterExit()
        DispatchQueue.global().asyncAfter(deadline: .now() + 10) { exit(0) }
        NSApp.terminate(nil)
    }

    /// 版本不一致（应用更新后后台服务还是旧程序）就让它重启；后台服务是新的、本机有旧数据就移交（只有管理员能做）。
    /// 返回 true 表示后台服务马上重启，这条连接会断
    nonisolated static func prepare(_ info: DaemonInfo, _ c: IPCConnection, migrating: @Sendable () -> Void = {}) -> Bool {
        if info.build != AppInfo.build && restartOnce.fire() {
            NSLog("LemonGalaxy: 后台服务版本 %@ 与应用 %@ 不同，让它重启", info.build, AppInfo.build)
            return (try? c.call(IPCMethod.restart, timeout: 5)) != nil
        }
        guard !info.claimed, info.peerAdmin, let b = Migration.localBundle() else { return false }
        migrating()
        do {
            _ = try c.call(IPCMethod.importData, CMigration(bundle: b), timeout: 30)
            NSLog("LemonGalaxy: 已把设备 %@ 与 %d 个文件移交给后台服务", Migration.deviceIdHex(b.deviceId), b.files.count)
            Migration.markLocalMigrated()
            return true
        } catch {
            NSLog("LemonGalaxy: 移交给后台服务失败：%@", String(describing: error))
            return false
        }
    }

    // MARK: 自检

    func fake(_ phase: Phase, daemon: DaemonInfo? = nil, clientMode: Bool = false) {
        self.phase = phase
        self.daemon = daemon
        self.clientMode = clientMode
    }
}
