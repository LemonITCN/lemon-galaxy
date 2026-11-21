import Foundation

// 模拟内核：假账号、假设备、假加密库。只给开发与 selftest 用，界面通过 Bridge 类型访问，不依赖这里的内部细节。
// 登录规则（便于手动演示各个步骤）：
//   - 密码为 wrong：账号或密码错误；
//   - 用户名含 totp：先要求两步验证码（任意 6 位数字通过）；
//   - 用户名含 new：登录后必须改密码；
//   - 其余任意用户名密码都能登录。

final class MockGalaxyClient: GalaxyClient, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("模拟对象不能由内核指针构造") }
    static let version = "0.1.0-mock"

    let store: SecureStore
    var listener: ClientListener?
    let zh: Bool
    let options: MockOptions
    private let lock = NSLock()
    private var _state: ClientState = .loggedOut
    private var _user: UserInfo?
    private var _server: ServerInfo?
    private var _devices: [DeviceInfo]
    private var _vault: [VaultEntry]
    private var pendingLogin: (server: String, username: String, mustChange: Bool)?
    private var _totpEnabled = false
    private var savedConversations: [String: MockAgentConversation] = [:]
    var timers: [DispatchSourceTimer] = []
    let queue = DispatchQueue(label: "mock.core.callbacks")

    init(store: SecureStore, listener: ClientListener, locale: String, options: MockOptions) {
        self.store = store
        self.listener = listener
        self.zh = locale.hasPrefix("zh")
        self.options = options
        self._devices = MockData.devices(zh: zh)
        self._vault = MockData.vault(zh: zh)
        super.init(noHandle: NoHandle())
    }

    deinit { timers.forEach { $0.cancel() } }

    func delay(_ ms: Int) async {
        let scaled = Double(ms) * options.latencyScale
        if scaled <= 0 { return }
        try? await Task.sleep(nanoseconds: UInt64(scaled * 1_000_000))
    }

    private func setState(_ s: ClientState) {
        lock.locked { _state = s }
        let l = listener
        queue.async { l?.onState(state: s) }
    }

    // MARK: 状态

    override func state() -> ClientState { lock.lock(); defer { lock.unlock() }; return _state }
    override func currentUser() -> UserInfo? { lock.lock(); defer { lock.unlock() }; return _user }
    override func server() -> ServerInfo? { lock.lock(); defer { lock.unlock() }; return _server }

    // MARK: 登录

    override func login(server: String, username: String, password: String, totp: String?) async throws -> LoginResult {
        await delay(450)
        guard MockData.isValidServer(server) else {
            throw GalaxyError.Network(detail: zh ? "无法连接服务器：地址格式不对" : "Cannot reach the server: invalid address")
        }
        if password == "wrong" {
            throw GalaxyError.Auth(code: "invalid_credentials", detail: zh ? "用户名或密码错误" : "Incorrect username or password")
        }
        if username.lowercased().contains("totp") && (totp ?? "").isEmpty {
            return .needTotp
        }
        if let t = totp, !t.isEmpty, t.count != 6 || !t.allSatisfy(\.isNumber) {
            throw GalaxyError.Auth(code: "totp_invalid", detail: zh ? "验证码不正确" : "Incorrect verification code")
        }
        if username.lowercased().contains("new") {
            lock.locked { pendingLogin = (server, username, true) }
            return .mustChangePassword
        }
        finishLogin(server: server, username: username)
        return .success
    }

    private func finishLogin(server: String, username: String) {
        let display = username.prefix(1).uppercased() + username.dropFirst()
        lock.lock()
        _user = UserInfo(id: 3, username: username, displayName: display, aiEnabled: true, totpEnabled: _totpEnabled)
        _server = ServerInfo(address: server, name: zh ? "LemonIT 总部" : "LemonIT HQ", version: "0.1.0")
        pendingLogin = nil
        lock.unlock()
        store.set(key: "mock.session", value: Data("\(server)\n\(username)".utf8))
        setState(.online)
        startLiveSimulation()
    }

    override func completeFirstLogin(newPassword: String) async throws {
        await delay(400)
        guard newPassword.count >= 8 else {
            throw GalaxyError.Auth(code: "weak_password", detail: zh ? "新密码至少 8 位" : "The new password must be at least 8 characters")
        }
        let p = lock.locked { pendingLogin }
        guard let p else { throw GalaxyError.Other(detail: zh ? "请先登录" : "Please sign in first") }
        finishLogin(server: p.server, username: p.username)
    }

    override func changePassword(oldPassword: String, newPassword: String, totp: String?) async throws {
        await delay(400)
        if oldPassword == "wrong" {
            throw GalaxyError.Auth(code: "invalid_credentials", detail: zh ? "原密码不正确" : "The current password is incorrect")
        }
        guard newPassword.count >= 8 else {
            throw GalaxyError.Auth(code: "weak_password", detail: zh ? "新密码至少 8 位" : "The new password must be at least 8 characters")
        }
    }

    override func logout() async throws {
        await delay(150)
        store.remove(key: "mock.session")
        lock.locked { _user = nil; _server = nil }
        timers.forEach { $0.cancel() }; timers.removeAll()
        setState(.loggedOut)
    }

    override func restore() -> Bool {
        guard let data = store.get(key: "mock.session"), let s = String(data: data, encoding: .utf8) else { return false }
        let parts = s.split(separator: "\n", maxSplits: 1).map(String.init)
        guard parts.count == 2 else { return false }
        finishLogin(server: parts[0], username: parts[1])
        return true
    }

    // MARK: 设备与加密库

    override func devices() -> [DeviceInfo] { lock.lock(); defer { lock.unlock() }; return _devices }

    override func refreshDevices() async throws {
        await delay(300)
        let l = listener
        queue.async { l?.onDevicesChanged() }
    }

    override func removeDevice(deviceId: String) async throws {
        await delay(200)
        lock.locked { _devices.removeAll { $0.id == deviceId } }
        let l = listener
        queue.async { l?.onDevicesChanged() }
    }

    override func vault() -> [VaultEntry] { lock.lock(); defer { lock.unlock() }; return _vault }

    override func vaultSave(entry: VaultEntry) async throws -> VaultEntry {
        await delay(120)
        var e = entry
        e.updatedAt = Int64(Date().timeIntervalSince1970 * 1000)
        lock.locked {
            if e.id.isEmpty {
                e.id = "v-" + UUID().uuidString.prefix(8).lowercased()
                _vault.append(e)
            } else if let i = _vault.firstIndex(where: { $0.id == e.id }) {
                _vault[i] = e
            } else {
                _vault.append(e)
            }
        }
        let l = listener
        queue.async { l?.onVaultChanged() }
        return e
    }

    override func vaultDelete(id: String) async throws {
        await delay(120)
        lock.locked {
            // 删除文件夹时把里面的条目移到上一级（界面会先确认）
            if let f = _vault.first(where: { $0.id == id }), case .folder = f.data {
                for i in _vault.indices where _vault[i].folderId == id { _vault[i].folderId = f.folderId }
            }
            _vault.removeAll { $0.id == id }
        }
        let l = listener
        queue.async { l?.onVaultChanged() }
    }

    // MARK: 连接

    func device(_ id: String) -> DeviceInfo? { devices().first { $0.id == id } }

    override func connect(deviceId: String) async throws -> PeerSession {
        await delay(600)
        guard let d = device(deviceId) else {
            throw GalaxyError.NotFound(detail: zh ? "设备不存在" : "Device not found")
        }
        guard d.online || d.lanSeen else {
            throw GalaxyError.Network(detail: zh ? "设备离线" : "The device is offline")
        }
        guard d.kind != .mobile else {
            throw GalaxyError.Permission(detail: zh ? "手机端不能被控制" : "Mobile devices cannot be controlled")
        }
        return MockPeerSession(client: self, device: d)
    }

    override func connectAssist(deviceCode: String, verificationCode: String) async throws -> PeerSession {
        await delay(900)
        let digits = deviceCode.filter(\.isNumber)
        guard digits.count == 9 else {
            throw GalaxyError.NotFound(detail: zh ? "设备码不存在" : "Unknown device code")
        }
        guard verificationCode.count >= 6 else {
            throw GalaxyError.Auth(code: "invalid_credentials", detail: zh ? "验证码不正确" : "Incorrect verification code")
        }
        let d = DeviceInfo(id: "assist-" + digits, kind: .worker, platform: .windows, hostname: "DESKTOP-7Q2K9LM",
                           osVersion: "Windows 11 23H2", arch: "x86_64", appVersion: "0.1.0", online: true, lanSeen: false,
                           lastSeenAt: MockData.now, isSelf: false)
        return MockPeerSession(client: self, device: d)
    }

    // MARK: RDP

    private var rdpForwards: Set<String> = []

    override func openRdp(hostId: String) async throws -> RdpTarget {
        await delay(200)
        guard let e = vault().first(where: { $0.id == hostId }),
              case let .rdpHost(label, host, port, username, password, domain, jump, _, _, _) = e.data else {
            throw GalaxyError.NotFound(detail: zh ? "主机不存在" : "Host not found")
        }
        let p = port == 0 ? 3389 : port
        if let jump, !jump.isEmpty {
            guard let d = device(jump), d.online || d.lanSeen else {
                throw GalaxyError.Network(detail: zh ? "经由的设备离线" : "The relay device is offline")
            }
            let id = UUID().uuidString.lowercased()
            lock.locked { _ = rdpForwards.insert(id) }
            return RdpTarget(label: label, host: "127.0.0.1", port: 50_000 + p % 1000, username: username, domain: domain,
                             password: password, forwarded: true, forwardId: id)
        }
        return RdpTarget(label: label, host: host, port: p, username: username, domain: domain, password: password, forwarded: false, forwardId: "")
    }

    override func closeRdp(forwardId: String) {
        lock.locked { _ = rdpForwards.remove(forwardId) }
    }

    var openRdpForwardCount: Int { lock.locked { rdpForwards.count } }

    override func connectSsh(hostId: String, prompt: SecretPrompt) async throws -> SshSession {
        try await mockConnectSsh(hostId: hostId, prompt: prompt)
    }

    // MARK: AI

    override func aiModels() async throws -> AiModels {
        await delay(200)
        return AiModels(enabled: true, maxMode: .fullTrust, items: [
            AiModel(providerId: 1, providerName: "OpenCode Go", id: "kimi-k2", name: "Kimi K2", vision: false, tools: true, contextLength: 131_072),
            AiModel(providerId: 1, providerName: "OpenCode Go", id: "qwen3-vl-plus", name: "Qwen3 VL Plus", vision: true, tools: true, contextLength: 262_144),
            AiModel(providerId: 1, providerName: "OpenCode Go", id: "glm-4.6", name: "GLM 4.6", vision: false, tools: true, contextLength: 200_000),
            AiModel(providerId: 2, providerName: zh ? "公司网关" : "Company gateway", id: "deepseek-v3.2", name: "DeepSeek V3.2", vision: false, tools: true, contextLength: 128_000),
        ])
    }

    override func newConversation(config: ConversationConfig, listener: AgentListener) async throws -> AgentConversation {
        await delay(300)
        let c = MockAgentConversation(client: self, config: config, listener: listener)
        lock.locked { savedConversations[c.id()] = c }
        return c
    }

    // 手机端只能主控：没有本机被控服务
    override func host() -> HostService? { nil }
    override func deviceId() -> String { "dev-self" }
    override func setLanDiscovery(active: Bool) {}

    // MARK: 两步验证、历史对话

    override func totpSetup() async throws -> TotpSetup {
        await delay(200)
        let secret = "JBSWY3DPEHPK3PXPLGALAXY"
        return TotpSetup(secret: secret, otpauthUri: "otpauth://totp/LemonGalaxy:\(currentUser()?.username ?? "user")?secret=\(secret)&issuer=LemonGalaxy")
    }
    override func totpVerify(code: String) async throws {
        await delay(200)
        guard code.count == 6, code.allSatisfy(\.isNumber) else {
            throw GalaxyError.Auth(code: "totp_invalid", detail: zh ? "验证码不正确" : "Incorrect verification code")
        }
        lock.locked { _totpEnabled = true; _user?.totpEnabled = true }
    }
    override func totpDisable(code: String) async throws {
        await delay(200)
        guard code.count == 6 else {
            throw GalaxyError.Auth(code: "totp_invalid", detail: zh ? "验证码不正确" : "Incorrect verification code")
        }
        lock.locked { _totpEnabled = false; _user?.totpEnabled = false }
    }
    override func listConversations() -> [ConversationSummary] {
        let all = lock.locked { Array(savedConversations.values) }
        return all.map { $0.summary() }.sorted { $0.updatedAt > $1.updatedAt }
    }
    override func openConversation(id: String, config: ConversationConfig?, listener: AgentListener) async throws -> AgentConversation {
        await delay(150)
        let c = lock.locked { savedConversations[id] }
        guard let c else { throw GalaxyError.NotFound(detail: zh ? "对话不存在" : "Conversation not found") }
        c.listener = listener
        return c
    }
    override func deleteConversation(id: String) {
        lock.locked { savedConversations[id] = nil }
    }

    // MARK: 后台模拟

    private func startLiveSimulation() {
        guard options.liveSimulation, timers.isEmpty else { return }
        // 每 25 秒让一台设备上下线一次，演示状态点变化
        let t = DispatchSource.makeTimerSource(queue: queue)
        t.schedule(deadline: .now() + 25, repeating: 25)
        t.setEventHandler { [weak self] in
            guard let self else { return }
            self.lock.lock()
            if let i = self._devices.firstIndex(where: { $0.id == "dev-nas" }) {
                self._devices[i].online.toggle()
                self._devices[i].lastSeenAt = MockData.now
            }
            self.lock.unlock()
            self.listener?.onDevicesChanged()
        }
        t.resume()
        timers.append(t)
    }

    func targetName(_ t: AgentTarget) -> String {
        switch t.kind {
        case .device:
            let d = device(t.id)
            if let meta = vault().first(where: { if case .deviceMeta(let id, _, _, _, _) = $0.data { return id == t.id }; return false }),
               case .deviceMeta(_, let alias?, _, _, _) = meta.data { return alias }
            return d?.hostname ?? t.id
        case .sshHost:
            if let e = vault().first(where: { $0.id == t.id }), case .sshHost(let label, _, _, _, _, _, _, _, _, _, _) = e.data { return label }
            return t.id
        }
    }

    func targetPlatform(_ t: AgentTarget) -> Platform {
        if t.kind == .device, let d = device(t.id) { return d.platform }
        return .linux
    }
}

// MARK: - PeerSession

final class MockPeerSession: PeerSession, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("模拟对象不能由内核指针构造") }
    let client: MockGalaxyClient
    let device: DeviceInfo
    private weak var listener: PeerListener?
    private var currentPath: PathInfo
    private var closed = false
    lazy var fileClient = MockFileClient(client: client, platform: device.platform, username: MockData.username(for: device))
    var timer: DispatchSourceTimer?

    init(client: MockGalaxyClient, device: DeviceInfo) {
        self.client = client
        self.device = device
        self.currentPath = MockData.path(for: device)
        super.init(noHandle: NoHandle())
    }

    deinit { timer?.cancel() }

    override func info() -> PeerInfo {
        PeerInfo(deviceId: device.id, hostname: device.hostname, platform: device.platform, osVersion: device.osVersion,
                 arch: device.arch, systemUser: MockData.username(for: device), shell: device.platform == .windows ? "powershell" : "/bin/bash",
                 permissions: MockData.permissions(for: device),
                 capabilities: MockData.capabilities(for: device), path: currentPath)
    }

    override func path() -> PathInfo { currentPath }

    override func setListener(listener: PeerListener) {
        self.listener = listener
        // 中转起步的连接 4 秒后「升级」成直连，演示路径无缝切换
        guard client.options.liveSimulation, currentPath.kind == .relayUdp else { return }
        let t = DispatchSource.makeTimerSource(queue: client.queue)
        t.schedule(deadline: .now() + 4)
        t.setEventHandler { [weak self] in
            guard let self, !self.closed else { return }
            self.currentPath = PathInfo(kind: .direct, rttMs: 21, remoteAddr: "203.0.113.24:51820")
            self.listener?.onPathChanged(path: self.currentPath)
        }
        t.resume()
        timer = t
    }

    override func isClosed() -> Bool { closed }
    override func wakeOnLan(mac: String) async throws { await client.delay(100) }

    override func setPrivacy(on: Bool) async throws {
        await client.delay(100)
        guard MockData.capabilities(for: device).contains("privacy") else {
            throw GalaxyError.Other(detail: client.zh ? "这台设备不支持隐私屏" : "This device does not support the privacy screen")
        }
    }

    override func disconnect() {
        guard !closed else { return }
        closed = true
        timer?.cancel()
    }

    override func openTerminal(options: TerminalOptions, sink: TerminalSink) async throws -> TerminalChannel {
        await client.delay(250)
        guard MockData.permissions(for: device).terminal else {
            throw GalaxyError.Permission(detail: client.zh ? "对方没有给你终端权限" : "The remote owner has not granted terminal access")
        }
        return MockTerminalChannel(client: client, device: device, options: options, sink: sink)
    }

    override func listTerminals() async throws -> [TerminalSessionInfo] {
        await client.delay(150)
        return MockTerminalRegistry.shared.sessions(for: device.id)
    }

    override func killTerminal(sessionId: String) async throws {
        await client.delay(100)
        MockTerminalRegistry.shared.remove(sessionId)
    }

    override func files() -> FileClient { fileClient }

    override func exec(request: ExecRequest) async throws -> ExecResult {
        await client.delay(300)
        let out = MockShell.run(command: request.command, device: device, cwd: request.cwd ?? "~", zh: client.zh)
        return ExecResult(code: out.code, stdout: out.text, stderr: "", timedOut: false, truncated: false, durationMs: 42)
    }

    override func listDisplays() async throws -> [DisplayInfo] {
        await client.delay(150)
        return MockData.displays(for: device)
    }

    override func openDisplay(options: DisplayOptions, sink: VideoSink) async throws -> DisplayChannel {
        await client.delay(350)
        guard MockData.permissions(for: device).view else {
            throw GalaxyError.Permission(detail: client.zh ? "对方没有给你查看画面的权限" : "The remote owner has not granted screen access")
        }
        let display = MockData.displays(for: device).first { $0.id == options.displayId } ?? MockData.displays(for: device)[0]
        return MockDisplayChannel(device: device, display: display, options: options, sink: sink, zh: client.zh)
    }

    override func screenshot(displayId: UInt32, maxWidth: UInt32) async throws -> ImageData {
        await client.delay(200)
        let display = MockData.displays(for: device).first { $0.id == displayId } ?? MockData.displays(for: device)[0]
        let renderer = MockDesktopRenderer(device: device, display: display, zh: client.zh)
        return renderer.pngSnapshot(maxWidth: Int(maxWidth))
    }

    override func clipboardGet() async throws -> ClipboardData? {
        await client.delay(80)
        return .text(text: client.zh ? "远端剪贴板里的文字" : "Text from the remote clipboard")
    }

    override func clipboardSet(data: ClipboardData) async throws { await client.delay(80) }

    override func forwardPort(localPort: UInt16, remoteHost: String, remotePort: UInt16) async throws -> PortForward {
        await client.delay(200)
        return MockPortForward(port: localPort == 0 ? 50_000 + remotePort % 1000 : localPort)
    }
}

final class MockPortForward: PortForward, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("模拟对象不能由内核指针构造") }
    let port: UInt16
    init(port: UInt16) { self.port = port; super.init(noHandle: NoHandle()) }
    override func localPort() -> UInt16 { port }
    override func stop() {}
}
