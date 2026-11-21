import Foundation
import Combine

/// 一条与被控设备的端到端会话。同一台设备的终端、文件、桌面、AI 共用它。
@MainActor
public final class PeerConnection: ObservableObject {
    public let deviceId: String
    public let session: PeerSession
    @Published public private(set) var path: PathInfo
    @Published public private(set) var info: PeerInfo
    @Published public private(set) var closedReason: String?
    private let adapter = PeerListenerAdapter()

    public init(deviceId: String, session: PeerSession) {
        self.deviceId = deviceId
        self.session = session
        self.path = session.path()
        self.info = session.info()
        adapter.onPath = { [weak self] p in onMain { self?.path = p } }
        adapter.onClose = { [weak self] r in onMain { self?.closedReason = r } }
        session.setListener(listener: adapter)
    }

    public var isAlive: Bool { closedReason == nil && !session.isClosed() }
}

/// 账号、设备列表、加密库与连接池。界面只通过它和内核打交道（AI、传输各有自己的模型）。
@MainActor
public final class GalaxyAccount: ObservableObject {
    public enum Phase: Equatable { case login, main }

    public let core: CoreProvider.Context
    public var client: GalaxyClient { core.client }
    public var isMock: Bool { core.isMock }

    @Published public private(set) var phase: Phase = .login
    @Published public private(set) var clientState: ClientState = .loggedOut
    @Published public private(set) var user: UserInfo?
    @Published public private(set) var server: ServerInfo?
    @Published public private(set) var devices: [DeviceInfo] = []
    @Published public private(set) var vault: [VaultEntry] = []
    @Published public private(set) var refreshing = false
    /// 进行中的 RDP 端口转发
    @Published public internal(set) var rdpForwards: [RdpForward] = []

    /// 服务器吊销了本机会话（回登录页，界面提示一下）
    public var onSessionRevoked: (() -> Void)?
    /// 登录完成（含启动时恢复）
    public var onLoggedIn: (() -> Void)?
    /// 登出或被吊销：界面关掉所有会话
    public var onLoggedOut: (() -> Void)?

    private var peers: [String: PeerConnection] = [:]
    private var sshSessions: [String: SshConnection] = [:]
    /// SSH 认证要问用户时（密码、主机密钥），界面在这里弹窗；没设置就按取消处理
    public var secretPresenter: (@MainActor (SecretRequest) -> Void)?
    private var assistNames: [String: String] = [:]
    private let listener: ClientListenerAdapter

    public init(config: CoreConfig, store: SecureStore, forceMock: Bool = false, mockOptions: MockOptions = MockOptions()) {
        let adapter = ClientListenerAdapter()
        listener = adapter
        core = CoreProvider.make(config: config, store: store, listener: adapter, forceMock: forceMock, mockOptions: mockOptions)
        adapter.onStateChange = { [weak self] s in onMain { self?.handleState(s) } }
        adapter.onDevices = { [weak self] in onMain { self?.reloadDevices() } }
        adapter.onVault = { [weak self] in onMain { self?.reloadVault() } }
        adapter.onRevoked = { [weak self] in onMain { self?.handleRevoked() } }
    }

    /// 启动时恢复登录（离线也能显示缓存的设备与加密库）
    @discardableResult
    public func restore() -> Bool {
        guard client.restore() else { return false }
        didLogin()
        return true
    }

    public var hosts: HostsData { HostsData(devices: devices, vault: vault) }

    // MARK: 登录

    public func login(server: String, username: String, password: String, totp: String?) async throws -> LoginResult {
        let r = try await client.login(server: server, username: username, password: password, totp: totp)
        if r == .success { didLogin() }
        return r
    }

    public func completeFirstLogin(newPassword: String) async throws {
        try await client.completeFirstLogin(newPassword: newPassword)
        didLogin()
    }

    public func changePassword(old: String, new: String, totp: String?) async throws {
        try await client.changePassword(oldPassword: old, newPassword: new, totp: totp)
    }

    public func totpSetup() async throws -> TotpSetup { try await client.totpSetup() }

    public func totpVerify(_ code: String) async throws {
        try await client.totpVerify(code: code)
        user = client.currentUser()
    }

    public func totpDisable(_ code: String) async throws {
        try await client.totpDisable(code: code)
        user = client.currentUser()
    }

    public func logout() async {
        closeAllPeers()
        onLoggedOut?()
        do { try await client.logout() } catch {}
        resetState()
    }

    private func resetState() {
        phase = .login
        user = nil
        server = nil
        devices = []
        vault = []
        clientState = client.state()
    }

    private func didLogin() {
        user = client.currentUser()
        server = client.server()
        clientState = client.state()
        reloadDevices()
        reloadVault()
        phase = .main
        onLoggedIn?()
        Task { try? await client.refreshDevices() }
    }

    private func handleState(_ s: ClientState) {
        clientState = s
        if s == .loggedOut, phase == .main {
            closeAllPeers()
            onLoggedOut?()
            resetState()
        }
        if s == .online, phase == .main {
            user = client.currentUser()
            server = client.server()
        }
    }

    private func handleRevoked() {
        closeAllPeers()
        onLoggedOut?()
        resetState()
        onSessionRevoked?()
    }

    // MARK: 设备

    public func reloadDevices() { devices = client.devices() }
    public func reloadVault() { vault = client.vault() }

    public func refreshDevices() async throws {
        refreshing = true
        defer { refreshing = false }
        try await client.refreshDevices()
        reloadDevices()
    }

    /// 设备列表页打开期间让内核做局域网发现
    public func setLanDiscovery(_ active: Bool) {
        guard phase == .main else { return }
        client.setLanDiscovery(active: active)
    }

    public func removeDevice(_ id: String) async throws {
        try await client.removeDevice(deviceId: id)
        if let m = hosts.metaFor(id) { try? await client.vaultDelete(id: m.id) }
        reloadDevices()
        reloadVault()
    }

    /// 能不能远程开机：服务器认为离线、局域网里也没发现，且内核说能唤醒（能被控、上报过网卡地址）
    public func canWake(_ id: String) -> Bool {
        guard let d = device(id), !d.online, !d.lanSeen, !d.isSelf else { return false }
        return client.canWake(deviceId: id)
    }

    /// 远程开机：返回代发设备名（空字符串表示本机）
    public func wake(_ id: String) async throws -> [String] {
        try await client.wakeDevice(deviceId: id)
    }

    public func device(_ id: String) -> DeviceInfo? { devices.first { $0.id == id } }
    public func entry(_ id: String) -> VaultEntry? { vault.first { $0.id == id } }

    public func deviceName(_ id: String) -> String {
        if let n = assistNames[id] { return n }
        return hosts.deviceName(id)
    }

    // MARK: 加密库

    @discardableResult
    public func save(_ entry: VaultEntry) async throws -> VaultEntry {
        // 先改本地，界面立即响应；内核保存后以返回值为准
        if !entry.id.isEmpty, let i = vault.firstIndex(where: { $0.id == entry.id }) { vault[i] = entry }
        do {
            let saved = try await client.vaultSave(entry: entry)
            if let i = vault.firstIndex(where: { $0.id == saved.id }) { vault[i] = saved } else { vault.append(saved) }
            return saved
        } catch {
            reloadVault()
            throw error
        }
    }

    public func saveData(id: String?, folderId: String?, data: VaultData) async throws {
        if let id, var e = entry(id) {
            e.data = data
            e.folderId = folderId
            try await save(e)
        } else {
            let sort = (vault.map(\.sort).max() ?? 0) + 1
            try await save(VaultEntry(id: "", folderId: folderId, sort: sort, data: data, updatedAt: Clock.nowMs))
        }
    }

    public func delete(_ id: String) async throws {
        try await client.vaultDelete(id: id)
        reloadVault()
    }

    /// 设备的别名、标签、备注、收藏、所在文件夹存在 DeviceMeta 里；还没有就新建一条
    public func updateDeviceMeta(_ deviceId: String, folder: String?? = nil,
                                 _ change: (inout String?, inout [String], inout String, inout Bool) -> Void = { _, _, _, _ in }) async throws {
        var e = hosts.metaFor(deviceId) ?? VaultEntry(id: "", folderId: nil, sort: 0,
                                                       data: .deviceMeta(deviceId: deviceId, alias: nil, tags: [], note: "", favorite: false), updatedAt: 0)
        guard case .deviceMeta(_, var alias, var tags, var note, var fav) = e.data else { return }
        change(&alias, &tags, &note, &fav)
        if let folder { e.folderId = folder }
        e.data = .deviceMeta(deviceId: deviceId, alias: alias, tags: tags, note: note, favorite: fav)
        try await save(e)
    }

    public func toggleFavorite(_ item: HostItem) async throws {
        switch item {
        case .device(let d, _):
            try await updateDeviceMeta(d.id) { _, _, _, fav in fav.toggle() }
        case .ssh(var e):
            if case .sshHost(let a, let b, let c, let d, let f, let g, let h, let i, let j, let fav, let k) = e.data {
                e.data = .sshHost(label: a, host: b, port: c, username: d, auth: f, jumpDeviceId: g, jumpHostId: h, tags: i, note: j, favorite: !fav, terminalTheme: k)
                try await save(e)
            }
        case .rdp(var e):
            if case .rdpHost(let a, let b, let c, let d, let f, let g, let h, let i, let j, let fav) = e.data {
                e.data = .rdpHost(label: a, host: b, port: c, username: d, password: f, domain: g, jumpDeviceId: h, tags: i, note: j, favorite: !fav)
                try await save(e)
            }
        }
    }

    public func move(_ item: HostItem, to folderId: String?) async throws {
        switch item {
        case .device(let d, _): try await updateDeviceMeta(d.id, folder: .some(folderId))
        case .ssh(var e), .rdp(var e):
            e.folderId = folderId
            try await save(e)
        }
    }

    public func moveFolder(_ folder: VaultEntry, to parent: String?) async throws {
        guard parent == nil || !HostTree.descendants(hosts, of: folder.id).contains(parent!) else { return }
        var f = folder
        f.folderId = parent
        try await save(f)
    }

    public func setFolderExpanded(_ folder: VaultEntry, _ expanded: Bool) {
        guard case .folder(let name, let color, _) = folder.data else { return }
        var f = folder
        f.data = .folder(name: name, color: color, expanded: expanded)
        if let i = vault.firstIndex(where: { $0.id == f.id }) { vault[i] = f }
        Task { try? await save(f) }
    }

    // MARK: 连接池

    public func peer(_ deviceId: String) async throws -> PeerConnection {
        if let p = peers[deviceId], p.isAlive { return p }
        let session = try await client.connect(deviceId: deviceId)
        let p = PeerConnection(deviceId: deviceId, session: session)
        peers[deviceId] = p
        return p
    }

    /// 已经连着的会话（不新建）
    public func existingPeer(_ deviceId: String) -> PeerConnection? {
        guard let p = peers[deviceId], p.isAlive else { return nil }
        return p
    }

    /// 临时协助：输入设备码与验证码，等被控端本人点允许
    public func connectAssist(deviceCode: String, verificationCode: String) async throws -> String {
        let s = try await client.connectAssist(deviceCode: deviceCode, verificationCode: verificationCode)
        return registerAssistPeer(s)
    }

    /// 临时协助连上的设备不在本账号设备列表里：用合成 id 放进连接池，会话照常按 id 取
    public func registerAssistPeer(_ s: PeerSession) -> String {
        let info = s.info()
        let id = "assist:" + info.deviceId
        peers[id] = PeerConnection(deviceId: id, session: s)
        assistNames[id] = info.hostname
        return id
    }

    public func isAssist(_ id: String) -> Bool { id.hasPrefix("assist:") }

    public func assistPlatform(_ id: String) -> Platform? { peers[id]?.info.platform }

    /// 没有界面在用了就断开（inUse 由界面判断：还有没有终端、桌面、文件页用这台设备）
    public func release(_ deviceId: String) {
        guard let p = peers[deviceId] else { return }
        p.session.disconnect()
        peers[deviceId] = nil
    }

    public func closeAllPeers() {
        closeAllRdp()
        for p in peers.values { p.session.disconnect() }
        peers.removeAll()
        for s in sshSessions.values { s.session.disconnect() }
        sshSessions.removeAll()
    }

    // MARK: SSH 直连

    /// 连接加密库里的 SSH 主机（可经由设备或另一台 SSH 主机跳转）。认证与主机密钥确认经 secretPresenter 问用户
    public func ssh(_ hostId: String) async throws -> SshConnection {
        if let s = sshSessions[hostId], s.isAlive { return s }
        let bridge = SecretPromptBridge(title: hosts.sshLabel(hostId)) { [weak self] req in
            if let present = self?.secretPresenter { present(req) } else { req.cancel() }
        }
        let session = try await client.connectSsh(hostId: hostId, prompt: bridge)
        let c = SshConnection(hostId: hostId, session: session)
        sshSessions[hostId] = c
        return c
    }

    public func existingSsh(_ hostId: String) -> SshConnection? {
        guard let s = sshSessions[hostId], s.isAlive else { return nil }
        return s
    }

    public func releaseSsh(_ hostId: String) {
        sshSessions.removeValue(forKey: hostId)?.session.disconnect()
    }

    public func release(_ t: RemoteTarget) {
        switch t {
        case .device(let d): release(d)
        case .ssh(let h): releaseSsh(h)
        }
    }
}
