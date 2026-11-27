import SwiftUI
import AppKit
import Combine

enum WorkerPage: String, CaseIterable {
    case status, accounts, assist, events, permissions, general, about

    var icon: String {
        switch self {
        case .status: return "dot.radiowaves.left.and.right"
        case .accounts: return "person.2"
        case .assist: return "person.line.dotted.person"
        case .events: return "list.bullet.rectangle"
        case .permissions: return "lock.shield"
        case .general: return "gearshape"
        case .about: return "info.circle"
        }
    }
}

/// 被控端的界面状态：WorkerHost（登录多个账号）+ HostService（被控服务）。
/// 内核回调在任意线程触发，经 WorkerListener 切回主线程。
@MainActor
final class WorkerModel: ObservableObject {
    let core: WorkerCore.Context
    let settings: WorkerSettings
    let selfTest: Bool
    var worker: WorkerHost { core.worker }

    @Published var page: WorkerPage = .status
    @Published var status = HostStatus(online: false, remoteEnabled: true, deviceId: "", publicIp: "", accounts: [], sessions: [], assist: nil)
    @Published var accounts: [HostAccount] = []
    @Published var events: [HostEvent] = []
    @Published var aiActivity: [String: String] = [:]          // session_id → 正在做什么
    @Published var pendingRequest: AssistRequest?
    @Published var screenGranted = false
    @Published var accessibilityGranted = false
    @Published var customCode = ""
    @Published var eventFilter = "all"
    @Published var serviceStarting = true
    @Published var startError: String?
    @Published var loginForm: LoginForm?
    @Published var confirmLogout: HostAccount?
    @Published var loggingOut: Set<UInt64> = []
    @Published var assistBusy = false
    @Published var notice: String?
    @Published var loginItem = LoginItem.State.notRegistered
    /// 还没有服务器时，临时协助页里填的服务器地址
    @Published var assistServer = ""
    @Published var assistError: String?
    @Published var disconnecting: Set<String> = []
    /// 远程看过桌面的会话都结束 10 秒后自动锁屏（被控服务里的本机设置，默认关）
    @Published var lockOnEnd = false
    /// 无头兜底屏设置（""=默认 / "off" / "宽x高@刷新率"，见 HeadlessDisplaySetting）
    @Published var headlessDisplay = ""
    let updater: WorkerUpdater
    let unattended: UnattendedController
    private var cancellables: Set<AnyCancellable> = []
    private(set) var service: HostService?
    private var listener: WorkerListener?
    private var platform: MacHostPlatform?
    private var permTimer: Timer?
    private var refreshTimer: Timer?
    private var noticeTask: Task<Void, Never>?

    struct AssistRequest: Identifiable, Equatable {
        let id: String
        let user: String
        let device: String
    }

    init(settings: WorkerSettings, selfTest: Bool, forceMock: Bool = false, mockOptions: MockWorkerOptions = MockWorkerOptions()) {
        self.settings = settings
        self.selfTest = selfTest
        // 无人值守已开启：内核在后台服务里，这里只当界面（模拟内核与自检不走这条路）
        let remote = !selfTest && !forceMock && !CommandLine.arguments.contains("--mock")
            && ProcessInfo.processInfo.environment["LEMONGALAXY_MOCK"] != "1" && UnattendedController.clientModeAtLaunch()
        core = WorkerCore.make(locale: Localization.shared.strings.pack.coreLocale, forceMock: forceMock, mockOptions: mockOptions, remote: remote)
        // 远程升级回调：内核在本进程时登记（开了无人值守时内核在后台服务里，由后台服务登记；Model/RemoteUpdateHandler.swift）
        RemoteUpdateRequestHandler.register(isMock: core.isMock || core.isRemote, selfTest: selfTest)
        unattended = UnattendedController(clientMode: remote, selfTest: selfTest)
        updater = WorkerUpdater()
        updater.model = self
        unattended.model = self
        unattended.objectWillChange.sink { [weak self] _ in self?.objectWillChange.send() }.store(in: &cancellables)
        if let r = core.worker as? RemoteWorkerHost {
            r.onInfo = { [weak self] info in
                DispatchQueue.main.async { MainActor.assumeIsolated { self?.daemonInfo(info) } }
            }
            r.staleCheck = { UnattendedController.relaunchIfStale() }
            r.prepare = { [weak self] info, c in
                UnattendedController.prepare(info, c, migrating: {
                    DispatchQueue.main.async { MainActor.assumeIsolated { self?.unattended.phase = .migrating } }
                })
            }
        }
        assistServer = settings.lastServer
        refreshPermissions()
        loginItem = selfTest ? .notRegistered : LoginItem.state
        unattended.refresh()
    }

    /// 后台服务连上、重连或概况变化（无人值守下）
    private func daemonInfo(_ info: DaemonInfo?) {
        unattended.received(info)
        updater.daemonStatus(info?.update)
        guard info != nil else { refresh(); return }
        if let s = worker.service() {
            attach(s)
            startError = nil
        }
        serviceStarting = false
        refresh()
    }

    /// 关于页显示的内核版本与数据目录（无人值守下是后台服务的）
    var coreVersionText: String { core.isRemote ? (unattended.daemon?.coreVersion ?? "—") : core.coreVersion }
    var dataDirText: String {
        core.isRemote ? Unattended.daemonDataRoot.appendingPathComponent("worker").path
            : AppInfo.dataDir.path.replacingOccurrences(of: NSHomeDirectory(), with: "~")
    }

    var deviceId: String { status.deviceId.isEmpty ? worker.deviceId() : status.deviceId }
    var serverAddress: String? { worker.server() }
    var hasAccounts: Bool { !accounts.isEmpty }
    var needsPermissions: Bool { !screenGranted || !accessibilityGranted }
    var aiActive: Bool { status.sessions.contains { $0.aiActive } }
    var needsRelogin: Bool { accounts.contains { $0.status == .needsRelogin } }
    var hasActiveAccount: Bool { accounts.contains { $0.status == .active } }

    /// 总状态（菜单栏图标与状态页标题用）
    enum Overall { case starting, noAccount, offline, online, controlled, ai, disabled }
    var overall: Overall {
        if serviceStarting && service == nil { return .starting }
        if !hasAccounts && status.assist == nil { return .noAccount }
        if !status.remoteEnabled { return .disabled }
        if aiActive { return .ai }
        if !status.sessions.isEmpty { return .controlled }
        return status.online ? .online : .offline
    }

    // MARK: 启动

    /// 应用启动时调一次：有账号就启动被控服务
    func boot() {
        serviceStarting = true
        Task {
            do {
                if let s = try await worker.start() { attach(s) }
                startError = nil
            } catch {
                startError = describeError(error)
            }
            serviceStarting = false
            refresh()
        }
    }

    func attach(_ s: HostService) {
        if let cur = service, cur === s { return }
        service = s
        let l = WorkerListener()
        l.owner = self
        listener = l
        s.setListener(listener: l)
        // 无人值守下画面由后台服务的会话代理负责，界面进程不提供平台层
        if !selfTest && !core.isRemote {
            let p = platform ?? MacHostPlatform()
            platform = p
            // 换了被控服务（换服务器重启）：旧服务加的虚拟屏没人管了，删掉（新服务需要时会再加）
            if !VirtualDisplays.shared.ids.isEmpty { DispatchQueue.global().async { VirtualDisplays.shared.removeAll() } }
            s.setPlatform(platform: p)
        }
        refresh()
    }

    /// 登录后内核可能刚启动了被控服务，或换了服务器重启了服务：接上回调与平台层（同一个服务重接也无妨）
    private func attachIfNeeded() {
        if let s = worker.service() { attach(s) }
    }

    func refresh() {
        guard let s = service else {
            UIHooks.update(self)
            return
        }
        status = s.status()
        accounts = status.accounts
        events = s.recentEvents(limit: 300)
        lockOnEnd = s.lockOnEnd()
        headlessDisplay = s.headlessDisplay()
        let live = Set(status.sessions.map(\.id))
        aiActivity = aiActivity.filter { live.contains($0.key) }
        UIHooks.update(self)
    }

    func refreshPermissions() {
        screenGranted = SystemPermissions.screenRecordingGranted
        accessibilityGranted = SystemPermissions.accessibilityGranted
    }

    /// 设置窗口打开期间定时刷新（授权状态、公网 IP、事件）
    func setWindowVisible(_ on: Bool) {
        refreshTimer?.invalidate()
        refreshTimer = nil
        guard on, !selfTest else { return }
        refresh()
        refreshPermissions()
        loginItem = LoginItem.state
        unattended.refresh()
        refreshTimer = Timer.scheduledTimer(withTimeInterval: 3, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.refresh()
                self?.refreshPermissions()
            }
        }
    }

    /// 授权页打开时每 1 秒刷新授权状态（用户在系统设置里勾选后自动变绿）
    func watchPermissions(_ on: Bool) {
        permTimer?.invalidate()
        permTimer = nil
        guard on, !selfTest else { return }
        permTimer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refreshPermissions() }
        }
    }

    func show(notice text: String) {
        notice = text
        noticeTask?.cancel()
        guard !selfTest else { return }
        noticeTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 6_000_000_000)
            if !Task.isCancelled { self?.notice = nil }
        }
    }

    // MARK: 会话与账号的对应

    /// 会话所属账号（内核给的 account_id；临时协助没有账号）
    func account(for s: HostSessionInfo) -> HostAccount? {
        guard let id = s.accountId else { return nil }
        return accounts.first { $0.userId == id }
    }

    /// 需要提示的会话：临时协助、找不到账号的、账号开了「被控时提示」的
    var noticeSessions: [HostSessionInfo] {
        status.sessions.filter { s in account(for: s)?.policy.notify ?? true }
    }

    // MARK: 操作

    func setRemoteEnabled(_ on: Bool) {
        service?.setRemoteEnabled(enabled: on)
        refresh()
    }

    func disconnectAll() { service?.disconnectAll(); aiActivity = [:]; refresh() }

    func setLockOnEnd(_ on: Bool) {
        guard let service else { return }
        service.setLockOnEnd(on: on)
        lockOnEnd = service.lockOnEnd()
    }

    /// 改无头兜底屏设置；被控服务不接受（写法不对）时返回 false
    @discardableResult
    func setHeadlessDisplay(_ v: String) -> Bool {
        guard let service else { return false }
        let ok = service.setHeadlessDisplay(value: v)
        headlessDisplay = service.headlessDisplay()
        return ok
    }

    /// 断开一个会话
    func disconnect(_ s: HostSessionInfo) {
        guard let service else { return }
        disconnecting.insert(s.id)
        if !service.disconnectSession(id: s.id) { show(notice: L("status.sessionGone")) }
        aiActivity[s.id] = nil
        disconnecting.remove(s.id)
        refresh()
    }
    func stopAi() { service?.stopAi(); aiActivity = [:]; refresh() }

    /// 开启临时协助。还没有被控服务（全新安装、没登录过账号）时先用填的服务器地址 set_server。
    func startAssist() {
        guard !assistBusy else { return }
        assistError = nil
        let server = assistServer.trimmingCharacters(in: .whitespacesAndNewlines)
        if service == nil {
            guard LoginForm.looksLikeServer(server) else { assistError = L("login.server.invalid"); return }
        }
        assistBusy = true
        Task {
            defer { assistBusy = false; refresh() }
            do {
                if service == nil {
                    guard let s = try await worker.setServer(server: server) else { throw GalaxyError.Other(detail: L("assist.serverFailed")) }
                    settings.lastServer = server
                    attach(s)
                }
                guard let s = service else { return }
                _ = try await s.startAssist()
            } catch {
                assistError = describeError(error)
            }
        }
    }

    func stopAssist() { service?.stopAssist(); refresh() }
    func refreshCode() { service?.setAssistCode(code: nil); refresh() }

    var customCodeValid: Bool {
        let c = customCode.trimmingCharacters(in: .whitespaces)
        return (8...16).contains(c.count) && c.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber) }
    }

    func applyCustomCode() {
        guard customCodeValid else { return }
        service?.setAssistCode(code: customCode.trimmingCharacters(in: .whitespaces))
        customCode = ""
        refresh()
    }

    func respond(_ allow: Bool) {
        guard let r = pendingRequest else { return }
        service?.respondAssist(requestId: r.id, allow: allow)
        pendingRequest = nil
        AssistRequestPanel.shared.close()
        refresh()
    }

    func updatePolicy(_ account: HostAccount, _ change: (inout HostPolicy) -> Void) {
        var p = account.policy
        change(&p)
        guard p != account.policy else { return }
        service?.updateAccountPolicy(userId: account.userId, policy: p)
        refresh()
    }

    func openLogin(username: String? = nil) {
        let f = LoginForm()
        if let server = serverAddress {
            f.server = server
            // 已有账号时所有账号必须在同一台服务器上：地址先锁着，点「更换服务器」才能改，提交时确认后先退出这些账号（与 Linux 被控端一致）；
            // 只有临时协助在用时可以直接换（会结束协助、按新服务器重启被控服务）
            f.serverLocked = hasAccounts
            f.originalServer = hasAccounts ? server : nil
            f.serverInUseOnly = !hasAccounts && service != nil
        } else {
            f.server = settings.lastServer
        }
        f.username = username ?? (hasAccounts ? "" : settings.lastUsername)
        if core.isMock && f.server.isEmpty && !selfTest {
            // 模拟内核：预填演示地址（任意密码可登录，用户名含 totp / new 可演示两步验证与首次改密码）
            f.server = MockWorkerHost.demoServer
        }
        loginForm = f
        page = .accounts
    }

    func submitLogin(_ f: LoginForm) {
        guard !f.busy else { return }
        f.error = nil
        let server = f.server.trimmingCharacters(in: .whitespacesAndNewlines)
        guard LoginForm.looksLikeServer(server) else { f.error = L("login.server.invalid"); return }
        if f.step == .changePassword {
            guard f.newPassword.count >= 8 else { f.error = L("login.change.tooShort"); return }
            guard f.newPassword == f.confirmPassword else { f.error = L("login.change.mismatch"); return }
        }
        // 换服务器：先在表单里确认（会退出本机已登录的账号）
        let switching = f.originalServer.map { !LoginForm.sameServer($0, server) } ?? false
        if switching && !f.switchConfirmed {
            f.switchPending = true
            return
        }
        f.busy = true
        let user = f.username.trimmingCharacters(in: .whitespaces)
        let totp = f.totp.isEmpty ? nil : f.totp
        let newPassword = f.step == .changePassword ? f.newPassword : nil
        Task {
            defer { f.busy = false }
            do {
                if switching && hasAccounts {
                    // 所有账号必须在同一台服务器上：先退出旧服务器上的账号，内核才会换到新服务器
                    for a in accounts { try await worker.logout(userId: a.userId) }
                    f.originalServer = nil
                    refresh()
                }
                let r = try await worker.login(server: server, username: user, password: f.password, totp: totp, newPassword: newPassword, systemUser: nil)
                switch r {
                case .success(_, let name):
                    settings.lastServer = server
                    settings.lastUsername = name
                    loginForm = nil
                    attachIfNeeded()
                    refresh()
                    show(notice: L("accounts.signedIn", name))
                case .needTotp:
                    if f.step == .totp { f.error = L("login.totp.wrong") }
                    f.step = .totp
                case .mustChangePassword:
                    f.step = .changePassword
                }
            } catch {
                f.error = describeError(error)
            }
        }
    }

    func logout(_ a: HostAccount) {
        confirmLogout = nil
        loggingOut.insert(a.userId)
        Task {
            do {
                try await worker.logout(userId: a.userId)
                show(notice: L("accounts.signedOut", a.username))
            } catch {
                show(notice: describeError(error))
            }
            loggingOut.remove(a.userId)
            refresh()
        }
    }

    func setLoginItem(_ on: Bool) {
        guard !selfTest else { return }
        if let err = LoginItem.set(on) { show(notice: err) }
        loginItem = LoginItem.state
    }

    func setSystemNotifications(_ on: Bool) {
        settings.systemNotifications = on
        if on && !selfTest { Notifier.requestAuthorization() }
    }

    // MARK: 回调（主线程）

    func received(status s: HostStatus) {
        status = s
        accounts = s.accounts
        events = service?.recentEvents(limit: 300) ?? events
        let live = Set(s.sessions.map(\.id))
        aiActivity = aiActivity.filter { live.contains($0.key) }
        UIHooks.update(self)
        // 上线后每 6 小时最多自动检查一次更新
        if s.online && hasActiveAccount { updater.autoCheckIfDue() }
    }

    func assistRequested(id: String, user: String, device: String) {
        pendingRequest = AssistRequest(id: id, user: user, device: device)
        // 内核最多等 2 分钟，超时按拒绝处理；到时收起弹窗
        if !selfTest {
            Task { [weak self] in
                try? await Task.sleep(nanoseconds: 121_000_000_000)
                guard let self, self.pendingRequest?.id == id else { return }
                self.pendingRequest = nil
                AssistRequestPanel.shared.close()
                self.refresh()
            }
        }
        if !selfTest {
            AssistRequestPanel.shared.show(self)
            if settings.systemNotifications { Notifier.post(title: L("assist.request.title"), body: L("assist.request.body", user, Self.shortDevice(device))) }
        }
    }

    func sessionStarted(_ info: HostSessionInfo) {
        refresh()
        guard !selfTest, settings.systemNotifications else { return }
        if account(for: info)?.policy.notify ?? true {
            Notifier.post(title: L("notify.connected.title"), body: L("notify.connected.body", info.fromUser, Self.shortDevice(info.fromDevice)),
                          id: "session-" + info.id, category: Notifier.sessionCategory, session: info.id)
        }
    }

    func sessionEnded(_ id: String) {
        if !selfTest { Notifier.remove(id: "session-" + id) }
        aiActivity[id] = nil
        refresh()
    }

    func aiActivity(_ id: String, _ text: String) {
        aiActivity[id] = text
        UIHooks.update(self)
    }

    static func shortDevice(_ d: String) -> String {
        d.count > 12 ? String(d.prefix(8)) + "…" : d
    }

    var filteredEvents: [HostEvent] {
        switch eventFilter {
        case "ai": return events.filter(\.ai)
        case "warning": return events.filter { $0.level == .warning }
        case "connect": return events.filter { ["connect", "disconnect", "reject", "auth_failed"].contains($0.kind) }
        case "all": return events
        default: return events.filter { $0.kind == eventFilter }
        }
    }
}

/// 登录表单（服务器地址、用户名密码 → 两步验证 → 首次改密码）
@MainActor
final class LoginForm: ObservableObject, Identifiable {
    enum Step { case credentials, totp, changePassword }
    nonisolated let id = UUID()
    @Published var step: Step = .credentials
    @Published var server = ""
    @Published var serverLocked = false
    /// 没有账号、但临时协助等已经在用这台服务器（可以换，换了会结束协助）
    @Published var serverInUseOnly = false
    @Published var username = ""
    @Published var password = ""
    @Published var totp = ""
    @Published var newPassword = ""
    @Published var confirmPassword = ""
    @Published var busy = false
    @Published var error: String?
    /// 已有账号所在的服务器（换成别的要确认）
    var originalServer: String?
    /// 显示「更换服务器？」确认
    @Published var switchPending = false
    var switchConfirmed = false

    /// 是不是同一台服务器：主机不分大小写，端口与指纹要一样
    static func sameServer(_ a: String, _ b: String) -> Bool {
        func norm(_ s: String) -> String {
            let t = s.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let hash = t.firstIndex(of: "#") else { return t.lowercased() }
            return t[..<hash].lowercased() + t[hash...]
        }
        return norm(a) == norm(b)
    }

    /// 粗查格式：主机:端口#sha256-指纹（详细校验由内核做，错误文字由内核给）
    static func looksLikeServer(_ s: String) -> Bool {
        guard let hash = s.firstIndex(of: "#") else { return false }
        let hostPort = s[..<hash], fp = s[s.index(after: hash)...]
        guard fp.hasPrefix("sha256-"), fp.count > 20, let colon = hostPort.lastIndex(of: ":") else { return false }
        guard let port = Int(hostPort[hostPort.index(after: colon)...]), (1...65535).contains(port) else { return false }
        return colon > hostPort.startIndex
    }

    var canSubmit: Bool {
        switch step {
        case .credentials: return !server.isEmpty && !username.trimmingCharacters(in: .whitespaces).isEmpty && !password.isEmpty
        case .totp: return totp.count >= 6
        case .changePassword: return !newPassword.isEmpty && !confirmPassword.isEmpty
        }
    }
}

/// 内核回调 → 主线程
final class WorkerListener: HostListener, @unchecked Sendable {
    weak var owner: WorkerModel?
    private func main(_ f: @escaping @MainActor (WorkerModel) -> Void) {
        DispatchQueue.main.async { [weak self] in
            guard let o = self?.owner else { return }
            MainActor.assumeIsolated { f(o) }
        }
    }
    func onStatus(status: HostStatus) { main { $0.received(status: status) } }
    func onAssistRequest(requestId: String, fromUser: String, fromDevice: String) { main { $0.assistRequested(id: requestId, user: fromUser, device: fromDevice) } }
    func onSessionStarted(info: HostSessionInfo) { main { $0.sessionStarted(info) } }
    func onSessionEnded(sessionId: String) { main { $0.sessionEnded(sessionId) } }
    func onAiActivity(sessionId: String, description: String) { main { $0.aiActivity(sessionId, description) } }
}

/// 模型变化后刷新菜单栏图标与被控提示条（selftest 下不碰）
@MainActor
enum UIHooks {
    static func update(_ m: WorkerModel) {
        guard !m.selfTest else { return }
        StatusMenu.shared.update(m)
        ControlBanner.shared.update(m)
    }
}
