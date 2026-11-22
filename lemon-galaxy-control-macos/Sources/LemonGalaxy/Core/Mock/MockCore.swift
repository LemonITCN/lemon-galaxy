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
    private lazy var hostService = MockHostService(client: self)
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

    override func connectSsh(hostId: String, prompt: SecretPrompt) async throws -> SshSession {
        await delay(500)
        guard let e = vault().first(where: { $0.id == hostId }),
              case let .sshHost(label, host, port, username, auth, jumpDevice, jumpHost, _, _, _, _) = e.data else {
            throw GalaxyError.NotFound(detail: zh ? "主机不存在" : "Host not found")
        }
        // 与真内核一样：内核在自己的线程上同步询问（界面阻塞等回答），不能占着协作线程池
        try await Task.detached(priority: .userInitiated) { [self] in
            // 主机密钥：没记录过或与记录不一致时请用户确认，确认后存进加密库
            let fp = Self.fakeFingerprint(host)
            let known = self.vault().first { if case .knownHost(let h, let p, _, _) = $0.data { return h.caseInsensitiveCompare(host) == .orderedSame && p == port }; return false }
            var knownFp: String?
            if let known, case .knownHost(_, _, _, let f) = known.data { knownFp = f }
            if knownFp != fp {
                guard prompt.confirmHostKey(host: host, port: port, keyType: "ssh-ed25519", fingerprint: fp, changed: knownFp != nil) else {
                    throw GalaxyError.Cancelled
                }
                let entry = VaultEntry(id: known?.id ?? "", folderId: nil, sort: 0,
                                       data: .knownHost(host: host, port: port, keyType: "ssh-ed25519", fingerprint: fp), updatedAt: 0)
                _ = try await self.vaultSave(entry: entry)
            }
            switch auth {
            case .password(let pw) where pw == nil || pw!.isEmpty:
                // 没保存密码：通过 SecretPrompt 问用户（界面弹遮挡的输入框）
                let answer = prompt.ask(prompt: self.zh ? "\(username)@\(host) 的密码：" : "Password for \(username)@\(host):", echo: false)
                guard let answer, !answer.isEmpty else { throw GalaxyError.Cancelled }
                if answer == "wrong" { throw GalaxyError.Auth(code: "invalid_credentials", detail: self.zh ? "密码错误" : "Permission denied") }
            case .interactive:
                guard prompt.ask(prompt: self.zh ? "验证码（键盘交互）：" : "Verification code (keyboard-interactive):", echo: true) != nil else {
                    throw GalaxyError.Cancelled
                }
            case .agent:
                throw GalaxyError.Permission(detail: self.zh ? "暂不支持 SSH Agent，请改用私钥或密码" : "SSH agent is not supported yet; use a private key or password")
            default:
                break
            }
        }.value
        var via = "direct"
        if let d = jumpDevice, !d.isEmpty { via = "device:" + (device(d)?.hostname ?? d) }
        else if let j = jumpHost, !j.isEmpty, let je = vault().first(where: { $0.id == j }), case .sshHost(let jl, _, _, _, _, _, _, _, _, _, _) = je.data { via = "ssh:" + jl }
        return MockSshSession(client: self, info: SshInfo(hostId: hostId, label: label, host: host, port: port, username: username, via: via))
    }

    /// 模拟主机密钥指纹：MockData 里记录过的用记录值，其余按主机名稳定生成（43 位 base64 样式）
    static func fakeFingerprint(_ host: String) -> String {
        if let fp = MockData.knownFingerprints[host] { return fp }
        let alphabet = Array("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/")
        var x: UInt64 = 0xcbf2_9ce4_8422_2325
        for b in host.utf8 { x = (x ^ UInt64(b)) &* 0x100_0000_01b3 }
        var out = "SHA256:"
        for _ in 0..<43 {
            x = x &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
            out.append(alphabet[Int(x >> 58)])
        }
        return out
    }

    // MARK: RDP

    private var rdpForwards: Set<String> = []

    override func openRdp(hostId: String) async throws -> RdpTarget {
        await delay(300)
        guard let e = vault().first(where: { $0.id == hostId }),
              case let .rdpHost(label, host, port, username, password, domain, jump, _, _, _) = e.data else {
            throw GalaxyError.NotFound(detail: zh ? "主机不存在" : "Host not found")
        }
        let p = port == 0 ? 3389 : port
        guard let jump, !jump.isEmpty else {
            return RdpTarget(label: label, host: host, port: p, username: username, domain: domain, password: password, forwarded: false, forwardId: "")
        }
        guard let d = device(jump), d.online || d.lanSeen else {
            throw GalaxyError.Network(detail: zh ? "经由的设备离线" : "The via device is offline")
        }
        let id = UUID().uuidString.lowercased()
        lock.locked { _ = rdpForwards.insert(id) }
        return RdpTarget(label: label, host: "127.0.0.1", port: 53_389, username: username, domain: domain, password: password, forwarded: true, forwardId: id)
    }

    override func closeRdp(forwardId: String) { lock.locked { _ = rdpForwards.remove(forwardId) } }

    // MARK: 远程开机

    override func canWake(deviceId: String) -> Bool {
        guard let d = device(deviceId) else { return false }
        return !d.online && !d.lanSeen && d.kind != .mobile && !d.isSelf && MockData.wakeRelays[deviceId] != nil
    }

    /// 模拟：经同一网络里的在线设备代发唤醒包；后台模拟打开时 6 秒后设备上线
    override func wakeDevice(deviceId: String) async throws -> [String] {
        await delay(500)
        guard let d = device(deviceId) else { throw GalaxyError.NotFound(detail: zh ? "设备不存在" : "Device not found") }
        if d.online || d.lanSeen { throw GalaxyError.Other(detail: zh ? "设备在线，不需要远程开机" : "The device is already online") }
        guard let relay = MockData.wakeRelays[deviceId] else {
            throw GalaxyError.Other(detail: zh ? "这台设备还没有上报网卡地址，无法远程开机" : "This device has not reported a network adapter address yet")
        }
        if options.liveSimulation {
            queue.asyncAfter(deadline: .now() + 6) { [weak self] in
                guard let self else { return }
                self.lock.locked {
                    if let i = self._devices.firstIndex(where: { $0.id == deviceId }) {
                        self._devices[i].online = true
                        self._devices[i].lastSeenAt = MockData.now
                    }
                }
                self.listener?.onDevicesChanged()
            }
        }
        return [relay]
    }

    // MARK: 端口映射

    /// 模拟映射正在占用的本机端口
    private var forwardPorts: Set<UInt16> = []

    func releaseForwardPort(_ p: UInt16) { lock.locked { _ = forwardPorts.remove(p) } }

    override func startPortForward(entryId: String, prompt: SecretPrompt) async throws -> PortForward {
        await delay(250)
        guard let e = vault().first(where: { $0.id == entryId }),
              case let .portForward(_, viaDevice, viaSsh, localPort, remoteHost, remotePort, _, _) = e.data else {
            throw GalaxyError.NotFound(detail: zh ? "端口映射规则不存在" : "Port forward rule not found")
        }
        guard !remoteHost.trimmingCharacters(in: .whitespaces).isEmpty, remotePort != 0 else {
            throw GalaxyError.Other(detail: zh ? "目标地址或端口没填" : "Target address or port is missing")
        }
        if let d = viaDevice, !d.isEmpty {
            guard let dev = device(d) else { throw GalaxyError.NotFound(detail: zh ? "设备不存在" : "Device not found") }
            guard dev.online || dev.lanSeen else { throw GalaxyError.Network(detail: zh ? "经由的设备离线" : "The via device is offline") }
        } else if let h = viaSsh, !h.isEmpty {
            // 经 SSH 主机：与连接 SSH 主机同一套询问（主机密钥、未保存的密码）
            let s = try await connectSsh(hostId: h, prompt: prompt)
            s.disconnect()
        } else {
            throw GalaxyError.Other(detail: zh ? "没有指定经由哪台设备或 SSH 主机" : "No via device or SSH host is set")
        }
        let port = localPort == 0 ? 50_000 + remotePort % 1000 : localPort
        try lock.locked {
            if forwardPorts.contains(port) {
                throw GalaxyError.Network(detail: zh ? "本机端口 \(port) 已被占用" : "Local port \(port) is already in use")
            }
            forwardPorts.insert(port)
        }
        return MockPortForward(port: port, client: self)
    }

    // MARK: 更新

    /// 模拟服务器上的最新版本
    static let mockUpdate = UpdateInfo(version: "0.2.0", fileName: "LemonGalaxy-0.2.0.dmg", size: 19_284_112,
                                       sha256: String(repeating: "ab", count: 32), url: "/updates/files/control/macos/LemonGalaxy-0.2.0.dmg",
                                       notes: "")

    override func checkUpdate(component: String, platform: String, arch: String, current: String) async throws -> UpdateInfo? {
        await delay(400)
        guard current.compare(Self.mockUpdate.version, options: .numeric) == .orderedAscending else { return nil }
        var u = Self.mockUpdate
        u.notes = zh
            ? "- SSH 直连：主机密钥确认、连接路径显示\n- RDP 主机经设备转发，用 Windows App 打开\n- 远程桌面加隐私屏开关\n- 修复若干问题"
            : "- SSH direct: host key confirmation and route display\n- RDP hosts forwarded through a device, opened in Windows App\n- Privacy screen toggle in remote desktop\n- Bug fixes"
        return u
    }

    /// 模拟内核不提供真的安装包：走一段进度后报错
    override func downloadUpdate(info: UpdateInfo, dest: String, listener: TransferListener) {
        let total = info.size
        let zh = zh
        queue.async {
            for i in 1...10 { listener.onProgress(done: total / 10 * UInt64(i), total: total); Thread.sleep(forTimeInterval: 0.05) }
            listener.onFinished(error: zh ? "模拟内核不提供真实安装包" : "The mock core has no real update package")
        }
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

    override func accountActivity(limit: UInt32) async throws -> [ActivityItem] {
        await delay(150)
        let me = currentUser()?.displayName ?? ""
        let items = [
            ActivityItem(at: MockData.now - 600_000, action: "user.login", actorName: me, targetType: "", targetId: "", targetName: "", ip: "203.0.113.8", detail: ""),
            ActivityItem(at: MockData.now - 1_800_000, action: "connection.start", actorName: me, targetType: "device", targetId: "dev-build", targetName: "build-server-01", ip: "203.0.113.8", detail: "terminal"),
            ActivityItem(at: MockData.now - 40_000_000, action: "connection.start", actorName: me, targetType: "device", targetId: "dev-office", targetName: zh ? "办公室电脑" : "Office PC", ip: "203.0.113.8", detail: "display"),
        ]
        return Array(items.prefix(Int(limit)))
    }

    override func host() -> HostService? { state() == .online ? hostService : nil }
    var mockHost: MockHostService { hostService }
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
    /// 模拟的对方剪贴板：开始时是对方复制好的两项（演示「对方复制 → 本机粘贴」），推送文字或文件后跟着变
    private let clipLock = NSLock()
    private var _remoteClipboard: ClipboardData?
    var remoteClipboard: ClipboardData? {
        get { clipLock.locked { _remoteClipboard } }
        set { clipLock.locked { _remoteClipboard = newValue } }
    }

    init(client: MockGalaxyClient, device: DeviceInfo) {
        self.client = client
        self.device = device
        self.currentPath = MockData.path(for: device)
        super.init(noHandle: NoHandle())
        _remoteClipboard = .files(paths: MockClipboardTransfer.remoteSampleFiles(device: device, zh: client.zh))
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
        return remoteClipboard
    }

    override func clipboardSet(data: ClipboardData) async throws {
        await client.delay(80)
        remoteClipboard = data
    }

    override func clipboardPushFiles(localPaths: [String], listener: TransferListener) -> ClipboardTransfer {
        MockClipboardTransfer.push(session: self, localPaths: localPaths, listener: listener)
    }

    override func clipboardPullFiles(remotePaths: [String], localDir: String, listener: TransferListener) -> ClipboardTransfer {
        MockClipboardTransfer.pull(session: self, remotePaths: remotePaths, localDir: localDir, listener: listener)
    }

    override func forwardPort(localPort: UInt16, remoteHost: String, remotePort: UInt16) async throws -> PortForward {
        await client.delay(200)
        return MockPortForward(port: localPort == 0 ? 50_000 + remotePort % 1000 : localPort)
    }

    override func setPrivacy(on: Bool) async throws {
        await client.delay(200)
        guard MockData.capabilities(for: device).contains("privacy") else {
            throw GalaxyError.Permission(detail: client.zh ? "被控端不支持隐私屏" : "The remote device does not support the privacy screen")
        }
        guard MockData.permissions(for: device).control else {
            throw GalaxyError.Permission(detail: client.zh ? "没有控制权限" : "No control permission")
        }
    }

    override func startAudio(volume: Float) async throws -> AudioPlayback {
        await client.delay(150)
        guard MockData.capabilities(for: device).contains("audio") else {
            throw GalaxyError.Permission(detail: client.zh ? "对方没有给你听声音的权限" : "The remote owner has not granted audio access")
        }
        return MockAudioPlayback(volume: volume)
    }
}

/// 模拟复制粘贴文件：按大小推进进度，传完再改对方剪贴板（推送）或在本机目录写真实的小文件（拉取，界面放进剪贴板的路径是真的）。
/// 名字里带 fail 的在 60% 处失败；带 hold 的停在 45% 等取消（selftest 截进度条用）。超过 4 GB 或 1 万个文件时与内核一样报错。
final class MockClipboardTransfer: ClipboardTransfer, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("模拟对象不能由内核指针构造") }
    static let maxBytes: UInt64 = 4 << 30
    static let maxFiles = 10_000

    private let queue = DispatchQueue(label: "mock.clipboard.transfer")
    private var timer: DispatchSourceTimer?
    /// 传完之前一直持有回调（与内核一致）
    private var listener: TransferListener?
    private let total: UInt64
    private var done: UInt64 = 0
    private var finished = false
    private let hold: Bool
    private let fail: Bool
    private let zh: Bool
    private let work: () throws -> [String]
    private let lock = NSLock()
    private var result: [String] = []
    /// selftest 核对用
    private(set) var cancelled = false

    private init(total: UInt64, names: [String], zh: Bool, speedScale: Double, listener: TransferListener, immediateError: String?, work: @escaping () throws -> [String]) {
        self.total = max(total, 1)
        self.hold = names.contains { $0.lowercased().contains("hold") }
        self.fail = names.contains { $0.lowercased().contains("fail") }
        self.zh = zh
        self.listener = listener
        self.work = work
        super.init(noHandle: NoHandle())
        if let immediateError {
            queue.async { [self] in finish(immediateError) }
            return
        }
        let t = DispatchSource.makeTimerSource(queue: queue)
        // 模拟约 1.2 秒传完；selftest（倍率 0）几毫秒走完
        let step = speedScale == 0 ? self.total / 3 + 1 : max(self.total / 12, 1)
        t.schedule(deadline: .now() + 0.05, repeating: speedScale == 0 ? 0.002 : 0.1)
        t.setEventHandler { [weak self] in self?.tick(step) }
        timer = t
        queue.async { [self] in listener.onProgress(done: 0, total: self.total) }
        t.resume()
    }

    deinit { timer?.cancel() }

    private func tick(_ step: UInt64) {
        guard !finished else { return }
        done = min(total, done + step)
        if hold, done * 100 >= total * 45 {
            done = (total * 45 + 99) / 100
            listener?.onProgress(done: done, total: total)
            timer?.cancel()
            return
        }
        listener?.onProgress(done: done, total: total)
        if fail, done * 10 >= total * 6 {
            finish(zh ? "连接中断（模拟失败）" : "Connection lost (simulated failure)")
        } else if done >= total {
            do {
                let paths = try work()
                lock.locked { result = paths }
                finish(nil)
            } catch {
                finish(error.localizedDescription)
            }
        }
    }

    private func finish(_ error: String?) {
        guard !finished else { return }
        finished = true
        timer?.cancel()
        listener?.onFinished(error: error)
        listener = nil
    }

    override func cancel() {
        queue.async { [self] in
            guard !finished else { return }
            cancelled = true
            finish(zh ? "已取消" : "Cancelled")
        }
    }

    override func paths() -> [String] { lock.locked { result } }

    private static func tooBig(_ zh: Bool) -> String {
        zh ? "要复制的文件太多或太大（超过 4 GB 或 1 万个），请用文件管理传输" : "Too many or too large files to copy (over 4 GB or 10,000 files). Use file transfer instead."
    }

    /// 本机文件 → 对方暂存区 ~/.lemon-galaxy/clipboard/<时间戳>-<随机数>/，传完对方剪贴板就是它们
    static func push(session: MockPeerSession, localPaths: [String], listener: TransferListener) -> MockClipboardTransfer {
        let zh = session.client.zh
        let fm = FileManager.default
        var bytes: UInt64 = 0, files = 0
        var error: String?
        if localPaths.isEmpty { error = zh ? "没有要复制的文件" : "Nothing to copy" }
        for p in localPaths where error == nil {
            var isDir: ObjCBool = false
            guard fm.fileExists(atPath: p, isDirectory: &isDir) else {
                error = zh ? "读不了 \(p)：文件不存在" : "Cannot read \(p): no such file"
                break
            }
            let all = isDir.boolValue ? (fm.enumerator(atPath: p)?.allObjects as? [String] ?? []).map { p + "/" + $0 } : [p]
            for f in all {
                let a = try? fm.attributesOfItem(atPath: f)
                guard (a?[.type] as? FileAttributeType) != .typeDirectory else { continue }
                files += 1
                bytes += (a?[.size] as? NSNumber)?.uint64Value ?? 0
            }
            if bytes > maxBytes || files > maxFiles { error = tooBig(zh) }
        }
        let stage = "\(home(session.device))/.lemon-galaxy/clipboard/\(Clock.nowMs)-\(String(format: "%08x", UInt32.random(in: 0...UInt32.max)))"
        let names = localPaths.map { ($0 as NSString).lastPathComponent }
        return MockClipboardTransfer(total: bytes, names: names, zh: zh, speedScale: session.client.options.latencyScale,
                                     listener: listener, immediateError: error) { [weak session] in
            let staged = names.map { "\(stage)/\($0)" }
            session?.remoteClipboard = .files(paths: staged)
            return staged
        }
    }

    /// 对方剪贴板里的文件 → 本机目录：文件写一份小文本，文件夹（名字没有扩展名）里放两份
    static func pull(session: MockPeerSession, remotePaths: [String], localDir: String, listener: TransferListener) -> MockClipboardTransfer {
        let zh = session.client.zh
        let names = remotePaths.map(baseName)
        let error: String? = remotePaths.isEmpty ? (zh ? "远端剪贴板里没有文件" : "No files in the remote clipboard") : nil
        // 每项按 1.8 MB 算进度
        return MockClipboardTransfer(total: UInt64(remotePaths.count) * 1_800_000, names: names, zh: zh,
                                     speedScale: session.client.options.latencyScale, listener: listener, immediateError: error) {
            let fm = FileManager.default
            let dir = URL(fileURLWithPath: localDir, isDirectory: true)
            try fm.createDirectory(at: dir, withIntermediateDirectories: true)
            var out: [String] = []
            for (remote, name) in zip(remotePaths, names) {
                let dest = dir.appendingPathComponent(name)
                let note = "LemonGalaxy mock\n\(remote)\n"
                if (name as NSString).pathExtension.isEmpty {
                    try fm.createDirectory(at: dest, withIntermediateDirectories: true)
                    try Data(("# " + name + "\n\n" + note).utf8).write(to: dest.appendingPathComponent("README.md"))
                    try Data(("Write-Host \"deploy\"\n# " + note).utf8).write(to: dest.appendingPathComponent("deploy.ps1"))
                } else {
                    try Data(note.utf8).write(to: dest)
                }
                out.append(dest.path)
            }
            return out
        }
    }

    /// 对方主目录（协议格式：Windows 是 /C:/Users/x）
    private static func home(_ d: DeviceInfo) -> String {
        let user = MockData.username(for: d)
        switch d.platform {
        case .windows: return "/C:/Users/\(user)"
        case .macos: return "/Users/\(user)"
        default: return "/home/\(user)"
        }
    }

    /// 远端路径的文件名（兼容反斜杠与结尾的斜杠）
    private static func baseName(_ path: String) -> String {
        path.split(whereSeparator: { $0 == "/" || $0 == "\\" }).last.map(String.init) ?? path
    }

    /// 模拟设备剪贴板里一开始的两项（协议格式路径）
    static func remoteSampleFiles(device: DeviceInfo, zh: Bool) -> [String] {
        let home = home(device)
        return zh ? ["\(home)/Documents/会议纪要 2026-11-20.txt", "\(home)/Desktop/部署脚本"]
            : ["\(home)/Documents/Meeting notes 2026-11-20.txt", "\(home)/Desktop/Deploy scripts"]
    }
}

/// 模拟的远端声音：不出声，只记住音量与静音，供界面与 selftest 核对
final class MockAudioPlayback: AudioPlayback, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("模拟对象不能由内核指针构造") }
    private let lock = NSLock()
    private var _volume: Float
    private var _muted = false
    private var active = true
    var volume: Float { lock.locked { _volume } }
    var muted: Bool { lock.locked { _muted } }
    init(volume: Float) { _volume = volume; super.init(noHandle: NoHandle()) }
    override func setVolume(volume: Float) { lock.locked { _volume = min(max(volume, 0), 1) } }
    override func setMuted(muted: Bool) { lock.locked { _muted = muted } }
    override func isActive() -> Bool { lock.locked { active } }
    override func stop() { lock.locked { active = false } }
}

final class MockPortForward: PortForward, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("模拟对象不能由内核指针构造") }
    let port: UInt16
    private weak var client: MockGalaxyClient?
    private(set) var stopped = false
    init(port: UInt16, client: MockGalaxyClient? = nil) { self.port = port; self.client = client; super.init(noHandle: NoHandle()) }
    override func localPort() -> UInt16 { port }
    override func stop() {
        guard !stopped else { return }
        stopped = true
        client?.releaseForwardPort(port)
    }
}

// MARK: - SSH 直连

final class MockSshSession: SshSession, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("模拟对象不能由内核指针构造") }
    let client: MockGalaxyClient
    let sshInfo: SshInfo
    let device: DeviceInfo
    private var closed = false
    lazy var fileClient = MockFileClient(client: client, platform: .linux, username: sshInfo.username)

    init(client: MockGalaxyClient, info: SshInfo) {
        self.client = client
        self.sshInfo = info
        self.device = DeviceInfo(id: "ssh-\(info.host)", kind: .worker, platform: .linux, hostname: info.label, osVersion: "Debian GNU/Linux 12",
                                 arch: "x86_64", appVersion: "", online: true, lanSeen: false, lastSeenAt: MockData.now, isSelf: false)
        super.init(noHandle: NoHandle())
    }

    override func info() -> SshInfo { sshInfo }
    override func isClosed() -> Bool { closed }

    override func openTerminal(options: TerminalOptions, sink: TerminalSink) async throws -> TerminalChannel {
        await client.delay(200)
        // SSH 没有断线继承：不登记到被控端保留会话里
        return MockTerminalChannel(client: client, device: device, options: options, sink: sink, usernameOverride: sshInfo.username, register: false)
    }

    override func files() -> FileClient { fileClient }

    override func exec(request: ExecRequest) async throws -> ExecResult {
        await client.delay(200)
        let out = MockShell.run(command: request.command, device: device, cwd: request.cwd ?? "~", zh: client.zh)
        return ExecResult(code: out.code, stdout: out.text, stderr: "", timedOut: false, truncated: false, durationMs: 30)
    }

    override func forwardPort(localPort: UInt16, remoteHost: String, remotePort: UInt16) async throws -> PortForward {
        await client.delay(100)
        return MockPortForward(port: localPort == 0 ? 50_000 + remotePort % 1000 : localPort)
    }

    override func disconnect() { closed = true }
}
