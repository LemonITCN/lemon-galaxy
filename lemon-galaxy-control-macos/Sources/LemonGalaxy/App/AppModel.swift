import Foundation
import SwiftUI
import AppKit
import SystemConfiguration

enum NavSection: String, CaseIterable, Identifiable {
    case hosts, sftp, ai, snippets, local, settings
    var id: String { rawValue }
    var icon: String {
        switch self {
        case .hosts: return "server.rack"
        case .sftp: return "folder"
        case .ai: return "wand.and.stars"
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
    case assistConnect
    case terminalSessions(deviceId: String, sessions: [TerminalSessionInfo])
    case confirm(ConfirmRequest)
    var id: String {
        switch self {
        case .connectChooser(let d): return "chooser-" + d
        case .secretPrompt(let r): return "secret-" + r.id
        case .assistConnect: return "assist"
        case .terminalSessions(let d, _): return "sessions-" + d
        case .confirm(let c): return "confirm-" + c.id
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
    @Published var modal: ModalKind?
    @Published var toast: Toast?

    let host: LocalHostModel
    let ai: AIHubModel
    let transfers = TransferQueue()

    private var peers: [String: PeerConnection] = [:]
    private var assistNames: [String: String] = [:]
    private var sshSessions: [String: SshConnection] = [:]
    private var listenerAdapter: ClientListenerAdapter?

    init(settings: AppSettings, selfTest: Bool, forceMock: Bool = false, mockOptions: MockOptions = MockOptions()) {
        self.settings = settings
        self.selfTest = selfTest
        let adapter = ClientListenerAdapter()
        let wantsMock = forceMock || CoreProvider.wantsMock()
        // 模拟内核不碰钥匙串（临时签名的开发包每次重编都会让钥匙串弹授权框）
        let store: SecureStore = (selfTest || wantsMock) ? MemorySecureStore() : KeychainStore()
        for d in [AppModel.dataDir, AppModel.cacheDir] { try? FileManager.default.createDirectory(at: d, withIntermediateDirectories: true) }
        let config = CoreConfig(dataDir: AppModel.dataDir.path, cacheDir: AppModel.cacheDir.path, logLevel: "info", platform: .macos,
                                deviceKind: .control, appVersion: AppInfo.version, osVersion: AppInfo.osVersion,
                                hostname: AppInfo.computerName, locale: Localization.shared.strings.pack.coreLocale)
        core = CoreProvider.make(config: config, store: store, listener: adapter, forceMock: forceMock, mockOptions: mockOptions)
        host = LocalHostModel(selfTest: selfTest)
        ai = AIHubModel()
        listenerAdapter = adapter
        adapter.model = self
        host.app = self
        ai.app = self
        if client.restore() { didLogin() }
    }

    static var dataDir: URL {
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
        Task { try? await client.refreshDevices() }
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
        closeAllTabs()
        try? await client.logout()
        phase = .login
        user = nil
        server = nil
        devices = []
        vault = []
    }

    func handleState(_ s: ClientState) {
        clientState = s
        if s == .loggedOut, phase == .main { phase = .login }
        if s == .online, phase == .main {
            user = client.currentUser()
            server = client.server()
            attachHostIfReady()
        }
    }

    func handleSessionRevoked() {
        closeAllTabs()
        phase = .login
        showToast(L("login.revoked"), error: true)
    }

    func reloadDevices() { devices = client.devices() }
    func reloadVault() { vault = client.vault() }

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

    func showToast(_ text: String, error: Bool = false) {
        let t = Toast(text: text, isError: error)
        toast = t
        guard !selfTest else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + (error ? 5 : 2.5)) { [weak self] in
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
        if let s = sshSessions[hostId], !s.closed { return s }
        // SSH 直连还不在 galaxy-ffi 里（见 Core/Bridge/Proposed.swift），模拟内核实现了
        guard let sshClient = client as? GalaxyClientSshProposed else {
            throw GalaxyError.Other(message: L("ssh.unsupported"))
        }
        let prompt = SecretPromptBridge(model: self, title: sshLabel(hostId))
        let session = try await sshClient.connectSsh(hostId: hostId, secretPrompt: prompt)
        let c = SshConnection(hostId: hostId, session: session)
        sshSessions[hostId] = c
        return c
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
    }

    func closeAllTabs() {
        for t in tabs { t.close() }
        tabs.removeAll()
        activeTabId = nil
        for p in peers.values { p.session.disconnect() }
        peers.removeAll()
        for s in sshSessions.values { s.session.disconnect() }
        sshSessions.removeAll()
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

    func openAI(targets: [AgentTarget]) {
        ai.draftTargets = targets
        section = .ai
        activeTabId = nil
    }

    /// 当前内核能不能 SSH 直连（真内核暂不支持，按钮置灰）
    var sshSupported: Bool { client is GalaxyClientSshProposed }

    func openRdp(_ hostId: String) {
        guard let e = entry(hostId), case .rdpHost(let label, let host, let port, let username, _, let domain, let jump, _, _, _) = e.data else { return }
        Task {
            var addr = "\(host):\(port)"
            if let jump {
                do {
                    let p = try await peer(jump)
                    let fwd = try await p.session.forwardPort(localPort: 0, remoteHost: host, remotePort: port)
                    addr = "127.0.0.1:\(fwd.localPort())"
                } catch { showError(error); return }
            }
            RdpLauncher.open(label: label, address: addr, username: username, domain: domain)
        }
    }

    /// 双击设备：弹连接方式选择
    func activateDevice(_ id: String) {
        guard let d = device(id), !d.isSelf, d.kind != .mobile else { return }
        modal = .connectChooser(deviceId: id)
    }
}

/// 一条与被控设备的端到端会话。
@MainActor
final class PeerConnection: ObservableObject {
    let deviceId: String
    let session: PeerSession
    @Published var path: PathInfo
    @Published var info: PeerInfo
    @Published var closedReason: String?
    private var adapter: PeerListenerAdapter?

    init(deviceId: String, session: PeerSession) {
        self.deviceId = deviceId
        self.session = session
        self.path = session.path()
        self.info = session.info()
        let a = PeerListenerAdapter()
        a.owner = self
        adapter = a
        session.setListener(listener: a)
    }
}

@MainActor
final class SshConnection {
    let hostId: String
    let session: SshSessionProposed
    var closed = false
    init(hostId: String, session: SshSessionProposed) { self.hostId = hostId; self.session = session }
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

/// SSH 键盘交互、私钥口令、未保存的密码：内核在自己的线程上同步调用 ask，这里切到主线程弹框并等用户回答。
struct SecretPromptRequest: Identifiable {
    let id = UUID().uuidString
    let title: String
    let prompt: String
    let echo: Bool
    let respond: (String?) -> Void
}

final class SecretPromptBridge: SecretPrompt {
    private weak var model: AppModel?
    private let title: String
    init(model: AppModel, title: String) { self.model = model; self.title = title }

    func ask(prompt: String, echo: Bool) -> String? {
        if Thread.isMainThread { return nil }   // 不能在主线程上等主线程
        let sem = DispatchSemaphore(value: 0)
        var answer: String?
        DispatchQueue.main.async {
            guard let model = self.model else { sem.signal(); return }
            model.modal = .secretPrompt(SecretPromptRequest(title: self.title, prompt: prompt, echo: echo) { value in
                answer = value
                sem.signal()
            })
        }
        sem.wait()
        return answer
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

/// RDP 直连：macOS 自带客户端要到后续期才有（ffi.md 未导出），先生成 .rdp 文件交给系统里装的远程桌面应用打开。
enum RdpLauncher {
    @MainActor
    static func open(label: String, address: String, username: String, domain: String?) {
        var lines = ["full address:s:\(address)", "username:s:\(domain.map { "\($0)\\" } ?? "")\(username)",
                     "prompt for credentials:i:1", "screen mode id:i:1", "use multimon:i:0"]
        lines.append("")
        let safe = label.replacingOccurrences(of: "/", with: "-")
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("\(safe).rdp")
        do {
            try lines.joined(separator: "\r\n").write(to: url, atomically: true, encoding: .utf8)
        } catch { return }
        if NSWorkspace.shared.urlForApplication(toOpen: url) != nil {
            NSWorkspace.shared.open(url)
        } else {
            let alert = NSAlert()
            alert.messageText = L("rdp.noclient.title")
            alert.informativeText = L("rdp.noclient.body")
            alert.runModal()
        }
    }
}
