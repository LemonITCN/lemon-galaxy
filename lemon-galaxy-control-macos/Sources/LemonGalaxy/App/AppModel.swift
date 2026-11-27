import Foundation
import SwiftUI
import AppKit
import SystemConfiguration

enum NavSection: String, CaseIterable, Identifiable {
    case hosts, sftp, forwards, ai, mcp, snippets, local, settings
    var id: String { rawValue }
    var icon: String {
        switch self {
        case .hosts: return "server.rack"
        case .sftp: return "folder"
        case .forwards: return "arrow.left.arrow.right"
        case .ai: return "wand.and.stars"
        case .mcp: return "point.3.connected.trianglepath.dotted"
        case .snippets: return "chevron.left.forwardslash.chevron.right"
        case .local: return "desktopcomputer"
        case .settings: return "gearshape"
        }
    }
    var titleKey: String { "nav." + rawValue }
}

enum HostsListTab: String, CaseIterable { case hosts, credentials, knownHosts }
enum SettingsPage: String, CaseIterable { case account, appearance, terminal, connection, ai, about }
enum LocalHostPage: String, CaseIterable { case overview, accounts, assist, events, permissions }

/// 界面内的模态卡片（画在主窗口里，不用系统 sheet，这样 selftest 也能离屏渲染出来）
enum ModalKind: Identifiable {
    case connectChooser(deviceId: String)
    case secretPrompt(SecretPromptRequest)
    case hostKey(HostKeyRequest)
    case update
    case assistConnect
    case terminalSessions(deviceId: String, sessions: [TerminalSessionInfo])
    case confirm(ConfirmRequest)
    case portForwardEditor(PortForwardDraft)
    var id: String {
        switch self {
        case .connectChooser(let d): return "chooser-" + d
        case .secretPrompt(let r): return "secret-" + r.id
        case .hostKey(let r): return "hostkey-" + r.id
        case .update: return "update"
        case .assistConnect: return "assist"
        case .terminalSessions(let d, _): return "sessions-" + d
        case .confirm(let c): return "confirm-" + c.id
        case .portForwardEditor(let d): return "pf-" + d.uid
        }
    }

    /// 点遮罩空白处能不能关掉：内核在等回答的卡片、编辑中的表单都不行（免得误点丢掉输入）
    var dismissOnBackground: Bool {
        switch self {
        case .secretPrompt, .hostKey, .portForwardEditor: return false
        default: return true
        }
    }

    /// 内核线程正在等它回答（SecretPrompt），不能被点空白处关掉
    var isKernelPrompt: Bool {
        switch self {
        case .secretPrompt, .hostKey: return true
        default: return false
        }
    }

    /// 被别的卡片顶掉或关掉时，按取消回答内核（否则内核线程一直等）
    func cancelPending() {
        switch self {
        case .secretPrompt(let r): r.respond(nil)
        case .hostKey(let r): r.respond(false)
        default: break
        }
    }
}

struct ConfirmRequest {
    let id = UUID().uuidString
    let title: String
    let message: String
    let confirmTitle: String
    let destructive: Bool
    let action: () -> Void
}

struct Toast: Identifiable, Equatable {
    let id = UUID()
    let text: String
    let isError: Bool
    /// 提示条上的一个操作按钮（如「在访达中显示」）
    var action: ToastAction? = nil
    static func == (a: Toast, b: Toast) -> Bool { a.id == b.id }
}

struct ToastAction {
    let title: String
    let run: () -> Void
}

/// 主窗口的全部状态。界面只通过它和 Bridge 层打交道。
@MainActor
final class AppModel: ObservableObject {
    enum Phase { case login, main }

    let settings: AppSettings
    let core: CoreProvider.Context
    var client: GalaxyClient { core.client }
    let selfTest: Bool

    @Published var phase: Phase = .login
    @Published var clientState: ClientState = .loggedOut
    @Published var user: UserInfo?
    @Published var server: ServerInfo?
    @Published var devices: [DeviceInfo] = []
    @Published var vault: [VaultEntry] = []

    @Published var section: NavSection = .hosts
    @Published var hostsTab: HostsListTab = .hosts
    @Published var selection: String?
    @Published var editing: EditTarget?
    @Published var renamingId: String?
    @Published var searchText = ""
    @Published var tagFilter: String?
    @Published var settingsPage: SettingsPage = .account
    @Published var localPage: LocalHostPage = .overview
    @Published var selectedSnippet: String?

    @Published var tabs: [TabModel] = []
    @Published var activeTabId: String?
    @Published var modal: ModalKind? {
        didSet {
            // 等内核回答的卡片被换掉：按取消回答（回答只认第一次，卡片自己回答过的不受影响）
            if let old = oldValue, old.isKernelPrompt, old.id != modal?.id { old.cancelPending() }
        }
    }
    @Published var toast: Toast?

    let host: LocalHostModel
    let ai: AIHubModel
    /// 本机 MCP 服务（docs/mcp.md）
    let mcp: MCPModel
    let transfers = TransferQueue()
    let update = UpdateModel()
    /// 经设备转发中的 RDP（本机端口），底部提示条可以手动停止；退出登录、应用退出时全部关掉
    @Published private(set) var rdpForwards: [RdpForwardItem] = []
    /// 正在运行的端口映射（加密库条目 id → 句柄）；退出登录、应用退出时全部停掉
    @Published var forwards: [String: RunningForward] = [:]
    /// 正在开启中的端口映射
    @Published var forwardBusy: Set<String> = []
    /// 端口映射上次开启失败的原因（详情页显示）
    @Published var forwardErrors: [String: String] = [:]
    @Published var selectedForward: String?
    /// 本次登录是否已经自动开启过「登录后自动开启」的端口映射
    var forwardsAutoStarted = false
    /// 正在发送唤醒包的设备
    @Published private(set) var waking: Set<String> = []
    /// 远程升级被控端的进度（设备 id → 状态；UI/Hosts/DeviceUpgrade.swift），离开详情再回来还能看到
    @Published var upgrades: [String: DeviceUpgrade] = [:]

    private var peers: [String: PeerConnection] = [:]
    private var assistNames: [String: String] = [:]
    private var sshSessions: [String: SshConnection] = [:]
    /// 已连上的 SSH 主机 → 内核报告的连接路径（direct / device:<名> / ssh:<名>），主机详情与终端顶栏显示
    @Published private(set) var sshVia: [String: String] = [:]
    private var listenerAdapter: ClientListenerAdapter?

    init(settings: AppSettings, selfTest: Bool, forceMock: Bool = false, mockOptions: MockOptions = MockOptions()) {
        self.settings = settings
        self.selfTest = selfTest
        let adapter = ClientListenerAdapter()
        let wantsMock = forceMock || CoreProvider.wantsMock()
        // 模拟内核只存内存；真内核存本机安全存储（临时签名的包存文件不碰钥匙串，见 LocalSecureStore）
        let store: SecureStore = (selfTest || wantsMock) ? MemorySecureStore() : LocalSecureStore(dir: AppModel.dataDir.appendingPathComponent("secrets", isDirectory: true))
        for d in [AppModel.dataDir, AppModel.cacheDir] { try? FileManager.default.createDirectory(at: d, withIntermediateDirectories: true) }
        let config = CoreConfig(dataDir: AppModel.dataDir.path, cacheDir: AppModel.cacheDir.path, logLevel: "info", platform: .macos,
                                deviceKind: .control, appVersion: AppInfo.version, osVersion: AppInfo.osVersion,
                                hostname: AppInfo.computerName, locale: Localization.shared.strings.pack.coreLocale)
        let ctx = CoreProvider.make(config: config, store: store, listener: adapter, forceMock: forceMock, mockOptions: mockOptions)
        core = ctx
        // 被控一侧的远程升级回调：内核初始化后、本机被控服务启动前登记（App/RemoteUpdateHandler.swift）
        RemoteUpdateRequestHandler.register(isMock: ctx.isMock, selfTest: selfTest)
        host = LocalHostModel(selfTest: selfTest)
        ai = AIHubModel()
        mcp = MCPModel()
        listenerAdapter = adapter
        adapter.model = self
        host.app = self
        ai.app = self
        mcp.app = self
        update.app = self
        if client.restore() { didLogin() }
    }

    /// 数据目录（与内核 CoreConfig、--mcp-stdio 桥接读的端点文件同一处）
    nonisolated static var dataDir: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return base.appendingPathComponent("LemonGalaxy", isDirectory: true)
    }

    static var cacheDir: URL {
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0].appendingPathComponent("cn.lemonit.galaxy.control", isDirectory: true)
    }

    // MARK: 登录状态

    func didLogin() {
        user = client.currentUser()
        server = client.server()
        clientState = client.state()
        reloadDevices()
        reloadVault()
        phase = .main
        attachHostIfReady()
        ai.reloadSummaries()
        mcp.didLogin()
        Task { try? await client.refreshDevices() }
        if clientState == .online {
            update.autoCheckIfDue()
            autoStartForwards()
        }
    }

    /// 本机被控服务要等登录上线后内核才给（control 客户端）；上线、重连时都尝试接上
    func attachHostIfReady() {
        guard host.service == nil, let h = client.host() else { return }
        host.attach(h)
    }

    /// 设备列表页打开期间让内核做局域网发现（protocol.md 第 5 节）
    func setLanDiscovery(_ active: Bool) {
        guard phase == .main else { return }
        client.setLanDiscovery(active: active)
    }

    func logout() async {
        closeAllRdp()
        stopAllForwards()
        closeAllTabs()
        try? await client.logout()
        mcp.didLogout()
        phase = .login
        user = nil
        server = nil
        devices = []
        vault = []
        upgrades = [:]
    }

    func handleState(_ s: ClientState) {
        clientState = s
        if s == .loggedOut, phase == .main { phase = .login }
        if s == .online, phase == .main {
            user = client.currentUser()
            server = client.server()
            attachHostIfReady()
            update.autoCheckIfDue()
            autoStartForwards()
        }
    }

    func handleSessionRevoked() {
        closeAllRdp()
        stopAllForwards()
        closeAllTabs()
        mcp.didLogout()
        phase = .login
        showToast(L("login.revoked"), error: true)
    }

    func reloadDevices() { devices = client.devices() }
    func reloadVault() {
        vault = client.vault()
        pruneForwards()
    }

    // MARK: 加密库辅助

    func entry(_ id: String) -> VaultEntry? { vault.first { $0.id == id } }
    func device(_ id: String) -> DeviceInfo? { devices.first { $0.id == id } }

    func meta(for deviceId: String) -> VaultEntry? {
        vault.first { if case .deviceMeta(let d, _, _, _, _) = $0.data { return d == deviceId }; return false }
    }

    func deviceName(_ id: String) -> String {
        if let n = assistNames[id] { return n }
        if let m = meta(for: id), case .deviceMeta(_, let alias?, _, _, _) = m.data, !alias.isEmpty { return alias }
        return device(id)?.hostname ?? id
    }

    func sshLabel(_ id: String) -> String {
        if let e = entry(id), case .sshHost(let label, _, _, _, _, _, _, _, _, _, _) = e.data { return label }
        return id
    }

    var credentials: [VaultEntry] { vault.filter { if case .credential = $0.data { return true }; return false }.sorted { $0.sort < $1.sort } }
    var snippets: [VaultEntry] { vault.filter { if case .snippet = $0.data { return true }; return false }.sorted { $0.sort < $1.sort } }
    var knownHosts: [VaultEntry] { vault.filter { if case .knownHost = $0.data { return true }; return false } }

    var allTags: [String] {
        var set = Set<String>()
        for e in vault {
            switch e.data {
            case .deviceMeta(_, _, let tags, _, _), .sshHost(_, _, _, _, _, _, _, let tags, _, _, _), .rdpHost(_, _, _, _, _, _, _, let tags, _, _):
                tags.forEach { set.insert($0) }
            default: break
            }
        }
        return set.sorted()
    }

    func save(_ entry: VaultEntry) async -> VaultEntry? {
        // 先在本地更新，界面立即响应；内核保存后以返回值为准
        if let i = vault.firstIndex(where: { $0.id == entry.id }), !entry.id.isEmpty { vault[i] = entry }
        do {
            let saved = try await client.vaultSave(entry: entry)
            if let i = vault.firstIndex(where: { $0.id == saved.id }) { vault[i] = saved } else { vault.append(saved) }
            return saved
        } catch {
            showError(error)
            reloadVault()
            return nil
        }
    }

    func delete(_ id: String) async {
        do {
            try await client.vaultDelete(id: id)
            reloadVault()
            if selection == id { selection = nil }
            if case .vault(let eid)? = editing, eid == id { editing = nil }
        } catch { showError(error) }
    }

    /// 设备的别名、标签、收藏存在 DeviceMeta 里；没有就新建一条
    func updateDeviceMeta(_ deviceId: String, _ change: (inout String?, inout [String], inout String, inout Bool, inout String?) -> Void) async {
        var e = meta(for: deviceId) ?? VaultEntry(id: "", folderId: nil, sort: 0,
                                                  data: .deviceMeta(deviceId: deviceId, alias: nil, tags: [], note: "", favorite: false), updatedAt: 0)
        guard case .deviceMeta(_, var alias, var tags, var note, var fav) = e.data else { return }
        var folder = e.folderId
        change(&alias, &tags, &note, &fav, &folder)
        e.folderId = folder
        e.data = .deviceMeta(deviceId: deviceId, alias: alias, tags: tags, note: note, favorite: fav)
        _ = await save(e)
    }

    // MARK: 提示

    func showToast(_ text: String, error: Bool = false, action: ToastAction? = nil) {
        let t = Toast(text: text, isError: error, action: action)
        toast = t
        guard !selfTest else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + (action != nil ? 8 : error ? 5 : 2.5)) { [weak self] in
            if self?.toast == t { self?.toast = nil }
        }
    }

    func showError(_ error: Error) { showToast(Self.describe(error), error: true) }

    nonisolated static func describe(_ error: Error) -> String {
        if let g = error as? GalaxyError {
            switch g {
            case .Network(let m), .Permission(let m), .NotFound(let m), .Protocol(let m), .Other(let m): return m
            case .Auth(_, let m): return m
            case .Cancelled: return L("common.cancelled")
            }
        }
        return error.localizedDescription
    }

    // MARK: 连接池（同一台设备的终端、文件、桌面、AI 共用一条端到端会话）

    func peer(_ deviceId: String) async throws -> PeerConnection {
        if let p = peers[deviceId], p.closedReason == nil, !p.session.isClosed() { return p }
        let session = try await client.connect(deviceId: deviceId)
        let p = PeerConnection(deviceId: deviceId, session: session)
        peers[deviceId] = p
        return p
    }

    func ssh(_ hostId: String) async throws -> SshConnection {
        if let s = sshSessions[hostId], !s.session.isClosed() { return s }
        let prompt = SecretPromptBridge(model: self, title: sshLabel(hostId))
        let session = try await client.connectSsh(hostId: hostId, prompt: prompt)
        let c = SshConnection(hostId: hostId, session: session)
        sshSessions[hostId] = c
        sshVia[hostId] = c.info.via
        return c
    }

    /// 没有标签页在用的 SSH 直连会话断开（SSH 没有断线继承，留着只占服务器连接）
    func releaseUnusedSsh() {
        var used = Set<String>()
        for t in tabs {
            if let term = t as? TerminalTabModel, case .ssh(let h) = term.target { used.insert(h) }
            if let sftp = t as? SFTPTabModel {
                for pane in [sftp.left, sftp.right] { if case .ssh(let h) = pane.endpoint { used.insert(h) } }
            }
        }
        for (id, c) in sshSessions where !used.contains(id) {
            c.session.disconnect()
            sshSessions[id] = nil
            sshVia[id] = nil
        }
    }

    /// SSH 主机的连接路径文字：有直连会话时用内核报告的 via，否则按条目里配置的跳转推断
    func sshViaText(_ hostId: String) -> String {
        if let via = sshVia[hostId] {
            if via.hasPrefix("device:") { return L("ssh.via.device", String(via.dropFirst(7))) }
            if via.hasPrefix("ssh:") { return L("ssh.via.host", String(via.dropFirst(4))) }
            return L("ssh.via.direct")
        }
        guard let e = entry(hostId), case .sshHost(_, _, _, _, _, let jd, let jh, _, _, _, _) = e.data else { return L("ssh.via.direct") }
        if let jd, !jd.isEmpty { return L("ssh.via.device", deviceName(jd)) }
        if let jh, !jh.isEmpty { return L("ssh.via.host", sshLabel(jh)) }
        return L("ssh.via.direct")
    }

    /// 临时协助连上的设备不在本账号设备列表里：用合成 id 放进连接池，标签页照常按 id 取会话
    func registerAssistPeer(_ s: PeerSession) -> String {
        let info = s.info()
        let id = "assist:" + info.deviceId
        peers[id] = PeerConnection(deviceId: id, session: s)
        assistNames[id] = info.hostname + " · " + L("assist.badge")
        return id
    }

    func releaseIfUnused(deviceId: String) {
        guard !tabs.contains(where: { $0.deviceId == deviceId }), let p = peers[deviceId] else { return }
        p.session.disconnect()
        peers[deviceId] = nil
    }

    // MARK: 标签页

    var activeTab: TabModel? { tabs.first { $0.id == activeTabId } }

    func addTab(_ tab: TabModel) {
        tabs.append(tab)
        activeTabId = tab.id
    }

    func closeTab(_ id: String) {
        guard let i = tabs.firstIndex(where: { $0.id == id }) else { return }
        let tab = tabs[i]
        tab.close()
        tabs.remove(at: i)
        if activeTabId == id {
            activeTabId = tabs.isEmpty ? nil : tabs[min(i, tabs.count - 1)].id
        }
        if let d = tab.deviceId { releaseIfUnused(deviceId: d) }
        releaseUnusedSsh()
    }

    func closeAllTabs() {
        ai.closeAll()
        for t in tabs { t.close() }
        tabs.removeAll()
        activeTabId = nil
        for p in peers.values { p.session.disconnect() }
        peers.removeAll()
        for s in sshSessions.values { s.session.disconnect() }
        sshSessions.removeAll()
        sshVia.removeAll()
    }

    func selectHome(_ s: NavSection? = nil) {
        if let s { section = s }
        activeTabId = nil
    }

    // MARK: 打开连接

    func openTerminal(deviceId: String, attach: TerminalSessionInfo? = nil) {
        let tab = TerminalTabModel(app: self, target: .device(deviceId), title: deviceName(deviceId), attach: attach)
        addTab(tab)
        tab.start()
    }

    func openTerminal(sshHostId: String) {
        let tab = TerminalTabModel(app: self, target: .ssh(sshHostId), title: sshLabel(sshHostId), attach: nil)
        addTab(tab)
        tab.start()
    }

    /// 先查被控端保留的会话：有就让用户选接续还是新开
    func openTerminalAsking(deviceId: String) {
        guard settings.reattachTerminals else { openTerminal(deviceId: deviceId); return }
        Task {
            do {
                let p = try await peer(deviceId)
                let list = try await p.session.listTerminals().filter { !$0.attached }
                if list.isEmpty { openTerminal(deviceId: deviceId) } else { modal = .terminalSessions(deviceId: deviceId, sessions: list) }
            } catch {
                showError(error)
            }
        }
    }

    func openSFTP(deviceId: String? = nil, sshHostId: String? = nil) {
        let tab = SFTPTabModel(app: self)
        if let deviceId { tab.title = deviceName(deviceId); tab.deviceId = deviceId }
        if let sshHostId { tab.title = sshLabel(sshHostId) }
        addTab(tab)
        tab.start(remote: deviceId.map { .device($0) } ?? sshHostId.map { .ssh($0) })
    }

    func openRemoteDesktop(deviceId: String) {
        let tab = RemoteDesktopModel(app: self, deviceId: deviceId, title: deviceName(deviceId))
        addTab(tab)
        tab.start()
    }

    /// 从主机页的「AI」入口进来：新对话，输入框里先带上这台设备的引用
    func openAI(targets: [AgentTarget]) {
        section = .ai
        activeTabId = nil
        ai.newChat(refs: targets.map { ai.ref(for: $0) })
    }

    /// RDP 主机：内核准备好地址（设了经由设备时在本机开端口转发），再交给系统里的远程桌面应用（微软 Windows App）。
    /// 密码不放进链接，由用户在 Windows App 里输入。
    func openRdp(_ hostId: String) {
        Task {
            do {
                let t = try await client.openRdp(hostId: hostId)
                if t.forwarded {
                    rdpForwards.removeAll { $0.id == t.forwardId }
                    rdpForwards.append(RdpForwardItem(id: t.forwardId, hostId: hostId, label: t.label, port: t.port, startedAt: Date()))
                }
                guard !selfTest else { return }
                switch RdpLauncher.open(t) {
                case .opened:
                    if t.forwarded { showToast(L("rdp.forwarding", t.label, Int(t.port))) }
                case .noClient:
                    if t.forwarded { closeRdp(t.forwardId) }
                    confirm(L("rdp.noclient.title"), L("rdp.noclient.body"), button: L("rdp.noclient.store"), destructive: false) {
                        NSWorkspace.shared.open(RdpLauncher.appStoreURL)
                    }
                case .badAddress:
                    if t.forwarded { closeRdp(t.forwardId) }
                    showToast(L("rdp.badAddress"), error: true)
                }
            } catch { showError(error) }
        }
    }

    func closeRdp(_ forwardId: String) {
        client.closeRdp(forwardId: forwardId)
        rdpForwards.removeAll { $0.id == forwardId }
    }

    func closeAllRdp() {
        for f in rdpForwards { client.closeRdp(forwardId: f.id) }
        rdpForwards.removeAll()
    }

    /// 双击设备：弹连接方式选择
    func activateDevice(_ id: String) {
        guard let d = device(id), !d.isSelf, d.kind != .mobile else { return }
        modal = .connectChooser(deviceId: id)
    }

    // MARK: 远程开机

    /// 设备离线（服务器与局域网都看不到）、能被控、上报过网卡地址时才显示「远程开机」
    func canWake(_ deviceId: String) -> Bool {
        guard let d = device(deviceId), !d.isSelf, d.kind != .mobile, !d.online, !d.lanSeen else { return false }
        return client.canWake(deviceId: deviceId)
    }

    /// 由本机或同一网络里在线的设备代发唤醒包，结果用提示条告诉用户经由谁发的
    func wake(_ deviceId: String) {
        guard !waking.contains(deviceId) else { return }
        waking.insert(deviceId)
        let name = deviceName(deviceId)
        Task {
            defer { waking.remove(deviceId) }
            do {
                let via = try await client.wakeDevice(deviceId: deviceId).map { $0.isEmpty ? L("wake.viaSelf") : $0 }
                showToast(via.isEmpty ? L("wake.sentPlain", name) : L("wake.sent", name, via.joined(separator: L("wake.joiner"))))
            } catch {
                showToast(L("wake.failed", Self.describe(error)), error: true)
            }
        }
    }
}

/// 一条与被控设备的端到端会话。
@MainActor
final class PeerConnection: ObservableObject {
    let deviceId: String
    let session: PeerSession
    @Published var path: PathInfo
    @Published var info: PeerInfo
    @Published var closedReason: String? {
        didSet { if closedReason != nil { endNapGuard() } }
    }
    private var adapter: PeerListenerAdapter?
    /// 连着时不让系统 App Nap：切到别的窗口后被降速，保活包发不出去，被控端 30 秒收不到就断开会话
    nonisolated(unsafe) private var napGuard: NSObjectProtocol?

    init(deviceId: String, session: PeerSession) {
        self.deviceId = deviceId
        self.session = session
        self.path = session.path()
        self.info = session.info()
        napGuard = ProcessInfo.processInfo.beginActivity(options: .userInitiatedAllowingIdleSystemSleep,
                                                         reason: "LemonGalaxy 远程会话保持连接")
        let a = PeerListenerAdapter()
        a.owner = self
        adapter = a
        session.setListener(listener: a)
    }

    private func endNapGuard() {
        if let g = napGuard { ProcessInfo.processInfo.endActivity(g) }
        napGuard = nil
    }

    deinit {
        if let g = napGuard { ProcessInfo.processInfo.endActivity(g) }
    }
}

/// 一条 SSH 直连会话（同一台主机的终端、SFTP 共用）。
@MainActor
final class SshConnection {
    let hostId: String
    let session: SshSession
    /// 连接时内核给的信息，via 是实际走的路径
    let info: SshInfo
    init(hostId: String, session: SshSession) {
        self.hostId = hostId
        self.session = session
        self.info = session.info()
    }
}

// MARK: - 回调适配：内核线程 → 主线程

final class ClientListenerAdapter: ClientListener, @unchecked Sendable {
    weak var model: AppModel?
    func onState(state: ClientState) { DispatchQueue.main.async { self.model?.handleState(state) } }
    func onDevicesChanged() { DispatchQueue.main.async { self.model?.reloadDevices() } }
    func onVaultChanged() { DispatchQueue.main.async { self.model?.reloadVault() } }
    func onSessionRevoked() { DispatchQueue.main.async { self.model?.handleSessionRevoked() } }
}

final class PeerListenerAdapter: PeerListener, @unchecked Sendable {
    weak var owner: PeerConnection?
    func onPathChanged(path: PathInfo) { DispatchQueue.main.async { self.owner?.path = path } }
    func onClosed(reason: String) { DispatchQueue.main.async { self.owner?.closedReason = reason } }
}

/// 内核等界面回答的一次询问：只认第一次回答（卡片按钮、被别的卡片顶掉、超时可能先后触发）。
final class PromptReply<T>: @unchecked Sendable {
    private let lock = NSLock()
    private let sem = DispatchSemaphore(value: 0)
    private var value: T
    private var done = false
    init(default value: T) { self.value = value }

    func resolve(_ v: T) {
        lock.lock()
        defer { lock.unlock() }
        guard !done else { return }
        done = true
        value = v
        sem.signal()
    }

    /// 阻塞等回答；超时按默认值（取消 / 不信任）
    func wait(timeout: TimeInterval) -> (value: T, timedOut: Bool) {
        let timedOut = sem.wait(timeout: .now() + timeout) == .timedOut
        if timedOut { resolve(value) }
        lock.lock()
        defer { lock.unlock() }
        return (value, timedOut)
    }
}

/// SSH 未保存的密码、私钥口令、键盘交互问题
struct SecretPromptRequest: Identifiable {
    let id = UUID().uuidString
    let title: String
    let prompt: String
    let echo: Bool
    let respond: (String?) -> Void
}

/// 首次连接或主机密钥变化时确认
struct HostKeyRequest: Identifiable {
    let id = UUID().uuidString
    let title: String
    let host: String
    let port: UInt16
    let keyType: String
    let fingerprint: String
    /// 与加密库里记录的不一致（可能被冒充）
    let changed: Bool
    let respond: (Bool) -> Void
}

/// 内核在自己的线程上同步调用 SecretPrompt：这里切到主线程弹卡片，阻塞当前（内核）线程等用户回答。
final class SecretPromptBridge: SecretPrompt, @unchecked Sendable {
    private weak var model: AppModel?
    private let title: String
    /// 用户走开时不让内核线程永远等着
    static let timeout: TimeInterval = 600
    init(model: AppModel, title: String) { self.model = model; self.title = title }

    func ask(prompt: String, echo: Bool) -> String? {
        if Thread.isMainThread { return nil }   // 不能在主线程上等主线程
        let reply = PromptReply<String?>(default: nil)
        let req = SecretPromptRequest(title: title, prompt: prompt, echo: echo) { reply.resolve($0) }
        return present(.secretPrompt(req), reply: reply)
    }

    func confirmHostKey(host: String, port: UInt16, keyType: String, fingerprint: String, changed: Bool) -> Bool {
        if Thread.isMainThread { return false }
        let reply = PromptReply<Bool>(default: false)
        let req = HostKeyRequest(title: title, host: host, port: port, keyType: keyType, fingerprint: fingerprint, changed: changed) { reply.resolve($0) }
        return present(.hostKey(req), reply: reply)
    }

    private func present<T>(_ modal: ModalKind, reply: PromptReply<T>) -> T {
        DispatchQueue.main.async {
            guard let model = self.model else { modal.cancelPending(); return }
            MainActor.assumeIsolated { model.modal = modal }
        }
        let r = reply.wait(timeout: Self.timeout)
        if r.timedOut {
            DispatchQueue.main.async {
                MainActor.assumeIsolated { if let model = self.model, model.modal?.id == modal.id { model.modal = nil } }
            }
        }
        return r.value
    }
}

enum AppInfo {
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
}

/// 一条经设备转发的 RDP（本机 127.0.0.1:port → 那台设备 → 目标的 RDP 端口）
struct RdpForwardItem: Identifiable, Equatable {
    let id: String
    let hostId: String
    let label: String
    let port: UInt16
    let startedAt: Date
}

/// 把 RDP 地址交给系统里的远程桌面应用（微软 Windows App，处理 rdp:// 链接）。密码不放进链接。
enum RdpLauncher {
    enum Result { case opened, noClient, badAddress }
    /// Mac App Store 里的 Windows App（原 Microsoft Remote Desktop）
    static let appStoreURL = URL(string: "https://apps.apple.com/app/windows-app/id1295203466")!

    /// rdp://full%20address=s:<host>:<port>&username=s:<user>，键和值各自 URL 编码（域账号写成 域\用户），
    /// 冒号与方括号按微软文档的写法原样保留（与 iOS、安卓一致）。这种写法 URL(string:) 不接受，用宽松的 CFURL 构造。
    static func url(for t: RdpTarget) -> URL? {
        let host = t.host.contains(":") && !t.host.hasPrefix("[") ? "[\(t.host)]" : t.host
        var params = [("full address", "s:\(host):\(t.port)")]
        var user = t.username
        if let d = t.domain, !d.isEmpty, !user.isEmpty, !user.contains("\\"), !user.contains("@") { user = d + "\\" + user }
        if !user.isEmpty { params.append(("username", "s:" + user)) }
        let allowed = CharacterSet(charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
        let keep = CharacterSet(charactersIn: ":[]")
        let query = params.map { k, v in
            (k.addingPercentEncoding(withAllowedCharacters: allowed) ?? k) + "=" + (v.addingPercentEncoding(withAllowedCharacters: allowed.union(keep)) ?? v)
        }.joined(separator: "&")
        return CFURLCreateWithString(nil, ("rdp://" + query) as CFString, nil) as URL?
    }

    @MainActor
    static func open(_ t: RdpTarget) -> Result {
        guard let url = url(for: t) else { return .badAddress }
        guard NSWorkspace.shared.urlForApplication(toOpen: url) != nil else { return .noClient }
        NSWorkspace.shared.open(url)
        return .opened
    }
}
