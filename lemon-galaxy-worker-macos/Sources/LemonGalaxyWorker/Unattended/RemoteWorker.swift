import Foundation

/// 无人值守开启后，菜单栏应用不再自己跑内核，而是当后台服务的界面：
/// RemoteWorkerHost / RemoteHostService 与真内核、模拟内核同样是 WorkerHost / HostService 的子类，界面代码不用改。
/// 连接断了自动重连（后台服务重启、导入数据后重启）；状态靠后台服务推送，读状态不阻塞主线程。
final class RemoteWorkerHost: WorkerHost, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("远程对象不能由内核指针构造") }

    let socketPath: String
    let remote: RemoteHostService
    private let lock = NSLock()
    private var conn: IPCConnection?
    private var running = false
    private var cachedInfo: DaemonInfo?
    private var ready = false
    private var restartRequested = false
    private var downloads: [String: TransferListener] = [:]
    private let wake = DispatchSemaphore(value: 0)

    /// 连上（含重连）并握手后、界面可用之前调用：做移交、版本不一致时让后台服务重启。返回 true 表示连接会被后台服务重启断开
    var prepare: (@Sendable (DaemonInfo, IPCConnection) -> Bool)?
    /// 后台服务概况变化（连上、代理增减、服务启动），任意线程
    var onInfo: (@Sendable (DaemonInfo?) -> Void)?
    /// 连上之后、打招呼之前：本界面程序已过时（磁盘上的应用换了版本）时重新打开自己并返回 true，这条连接不用
    var staleCheck: (@Sendable () -> Bool)?

    init(socketPath: String = Unattended.socketPath) {
        self.socketPath = socketPath
        remote = RemoteHostService()
        super.init(noHandle: NoHandle())
        remote.host = self
    }

    var info: DaemonInfo? { lock.locked { cachedInfo } }
    var isReady: Bool { lock.locked { ready } }

    func connect() {
        let go = lock.locked { () -> Bool in defer { running = true }; return !running }
        guard go else { return }
        let t = Thread { [self] in loop() }
        t.name = "galaxy.ui.daemon"
        t.start()
    }

    func disconnect() {
        let c = lock.locked { () -> IPCConnection? in
            running = false
            defer { conn = nil; ready = false }
            return conn
        }
        c?.close()
        wake.signal()
    }

    private func loop() {
        var delay = 0.3
        while lock.locked({ running }) {
            if let fd = try? IPCSocket.connect(path: socketPath) {
                let c = IPCConnection(fd: fd)
                c.label = "daemon"
                let closed = DispatchSemaphore(value: 0)
                c.start(onRequest: { _, r in r.fail(.other("界面不处理请求")) },
                        onNotify: { [weak self] m in self?.push(m) },
                        onClose: { closed.signal() })
                let locale = Localization.shared.strings.pack.coreLocale
                if socketPath == Unattended.defaultSocket && c.peer.uid != 0 {
                    c.close()
                } else if staleCheck?() == true {
                    // 旧界面的签名可能已对不上新后台服务（旧包已删），打招呼会被拒绝：先换成新程序
                    c.close()
                } else if let m = try? c.call(IPCMethod.hello, ControlHello(version: AppInfo.version, build: AppInfo.build, locale: locale), timeout: 5),
                          let info = try? m.decode(DaemonInfo.self) {
                    let restarting = prepare?(info, c) ?? false
                    if !restarting {
                        lock.locked {
                            conn = c
                            cachedInfo = info
                            ready = true
                        }
                        remote.connected(c)
                        onInfo?(info)
                        delay = 0.3
                    }
                } else {
                    c.close()
                }
                closed.wait()
                let was = lock.locked { () -> Bool in
                    defer { if conn === c { conn = nil; ready = false } }
                    return conn === c
                }
                if was {
                    remote.disconnected()
                    onInfo?(nil)
                }
            }
            guard lock.locked({ running }) else { break }
            _ = wake.wait(timeout: .now() + delay)
            delay = min(delay * 2, 3)
        }
    }

    private func push(_ m: IPCConnection.Message) {
        switch m.method {
        case IPCMethod.pushInfo:
            guard let i = try? m.decode(DaemonInfo.self) else { return }
            lock.locked { cachedInfo = i }
            onInfo?(i)
        case IPCMethod.pushDownload:
            guard let e = try? m.decode(CDownloadEvent.self) else { return }
            let l = lock.locked { () -> TransferListener? in
                defer { if e.finished { downloads[e.token] = nil } }
                return downloads[e.token]
            }
            guard let l else { return }
            if !e.finished {
                l.onProgress(done: e.done, total: e.total)
            } else {
                (l as? RemoteDownloadListener)?.finish(path: e.path, error: e.error)
            }
        default:
            remote.push(m)
        }
    }

    /// 当前连接；没连上时等一会儿（应用刚启动、后台服务正在重启）
    func connection(wait: Double = 0) throws -> IPCConnection {
        let end = Date().addingTimeInterval(wait)
        while true {
            if let c = lock.locked({ ready ? conn : nil }), !c.isClosed { return c }
            if Date() >= end { throw GalaxyError.Network(detail: L("unattended.unreachable")) }
            Thread.sleep(forTimeInterval: 0.1)
        }
    }

    func call<T: Encodable>(_ method: String, _ body: T, timeout: Double = 120) async throws -> IPCConnection.Message {
        do {
            return try await connection(wait: 3).callAsync(method, body, timeout: timeout)
        } catch let e as IPCError {
            if case .remote(let f) = e { throw f.galaxyError }
            throw GalaxyError.Network(detail: L("unattended.unreachable"))
        }
    }

    func callSync<T: Encodable>(_ method: String, _ body: T, timeout: Double = 3) throws -> IPCConnection.Message {
        do {
            return try connection().call(method, body, timeout: timeout)
        } catch let e as IPCError {
            if case .remote(let f) = e { throw f.galaxyError }
            throw GalaxyError.Network(detail: L("unattended.unreachable"))
        }
    }

    private func update(_ i: DaemonInfo) {
        lock.locked { cachedInfo = i }
        onInfo?(i)
    }

    /// 界面换语言时告诉后台服务（内核错误文字跟着变）
    func setLocale(_ locale: String) {
        DispatchQueue.global().async { [weak self] in _ = try? self?.callSync(IPCMethod.setLocale, CLocale(locale: locale)) }
    }

    // MARK: WorkerHost

    override func deviceId() -> String { info?.deviceId ?? "" }
    override func server() -> String? { info?.server }
    override func service() -> HostService? { (info?.hasService ?? false) ? remote : nil }

    override func start() async throws -> HostService? {
        connect()
        // 最多等 8 秒连上（后台服务可能刚被 launchd 拉起，或正在导入数据后重启）
        let m = try await call(IPCMethod.start, IPCEmpty(), timeout: 60)
        update(try m.decode(CInfo.self).info)
        if remote.lastStatus == nil { remote.refreshStatus() }
        return service()
    }

    override func login(server: String, username: String, password: String, totp: String?, newPassword: String?, systemUser: String?) async throws -> WorkerLoginResult {
        let m = try await call(IPCMethod.login, CLogin(server: server, username: username, password: password, totp: totp, newPassword: newPassword))
        let r = try m.decode(CLoginResult.self)
        update(r.info)
        remote.refreshStatus()
        return r.result
    }

    override func logout(userId: UInt64) async throws {
        let m = try await call(IPCMethod.logout, CUser(userId: userId))
        update(try m.decode(CInfo.self).info)
        remote.refreshStatus()
    }

    override func setServer(server: String) async throws -> HostService? {
        let m = try await call(IPCMethod.setServer, CServer(server: server))
        update(try m.decode(CInfo.self).info)
        remote.refreshStatus()
        return service()
    }

    override func checkUpdate(platform: String, current: String) async throws -> UpdateInfo? {
        try await call(IPCMethod.checkUpdate, CUpdateCheck(platform: platform, current: current)).decode(CUpdateInfo.self).info
    }

    /// 后台服务下到它的缓存目录，完成后拷到 dest（当前用户的「下载」）
    override func downloadUpdate(info: UpdateInfo, dest: String, listener: TransferListener) {
        let token = UUID().uuidString
        let l = RemoteDownloadListener(inner: listener, dest: dest)
        lock.locked { downloads[token] = l }
        Task {
            do {
                _ = try await call(IPCMethod.downloadUpdate, CDownload(token: token, info: info), timeout: 10)
            } catch {
                lock.locked { downloads[token] = nil }
                listener.onFinished(error: describeError(error))
            }
        }
    }
}

final class RemoteDownloadListener: TransferListener, @unchecked Sendable {
    let inner: TransferListener
    let dest: String
    init(inner: TransferListener, dest: String) {
        self.inner = inner
        self.dest = dest
    }
    func onProgress(done: UInt64, total: UInt64) { inner.onProgress(done: done, total: total) }
    func onFinished(error: String?) { inner.onFinished(error: error) }
    func finish(path: String?, error: String?) {
        if let error { inner.onFinished(error: error); return }
        guard let path else { inner.onFinished(error: L("unattended.unreachable")); return }
        do {
            let fm = FileManager.default
            if fm.fileExists(atPath: dest) { try fm.removeItem(atPath: dest) }
            try fm.copyItem(atPath: path, toPath: dest)
            inner.onFinished(error: nil)
        } catch {
            inner.onFinished(error: error.localizedDescription)
        }
    }
}

/// 后台服务里的被控服务（状态从推送缓存里读）
final class RemoteHostService: HostService, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("远程对象不能由内核指针构造") }
    weak var host: RemoteWorkerHost?
    private let lock = NSLock()
    private var listener: HostListener?
    private var status_: HostStatus?
    private var lockOnEnd_ = false
    private var headless_ = ""
    private var events_: [HostEvent] = []

    init() { super.init(noHandle: NoHandle()) }

    var lastStatus: HostStatus? { lock.locked { status_ } }

    fileprivate func connected(_ c: IPCConnection) {
        refreshStatus()
    }

    fileprivate func disconnected() {
        // 断开时显示离线、没有会话（后台服务重启中）
        let s = lock.locked { () -> HostStatus? in
            guard var s = status_ else { return nil }
            s.online = false
            s.sessions = []
            status_ = s
            return s
        }
        if let s, let l = lock.locked({ listener }) { l.onStatus(status: s) }
    }

    /// 后台线程上拉一次状态并通知界面
    func refreshStatus() {
        DispatchQueue.global().async { [weak self] in
            guard let self, let host = self.host,
                  let m = try? host.callSync(IPCMethod.status, IPCEmpty()), let s = try? m.decode(CStatus.self).status else { return }
            let lo = (try? host.callSync(IPCMethod.lockOnEnd, IPCEmpty()).decode(PBool.self).value) ?? false
            let hd = try? host.callSync(IPCMethod.headlessDisplay, IPCEmpty()).decode(PText.self).text
            let l = self.lock.locked { () -> HostListener? in
                self.status_ = s
                self.lockOnEnd_ = lo
                if let hd { self.headless_ = hd }
                return self.listener
            }
            l?.onStatus(status: s)
        }
    }

    fileprivate func push(_ m: IPCConnection.Message) {
        let l = lock.locked { listener }
        switch m.method {
        case IPCMethod.pushStatus:
            guard let s = try? m.decode(CStatus.self).status else { return }
            lock.locked { status_ = s }
            l?.onStatus(status: s)
        case IPCMethod.pushAssist:
            guard let a = try? m.decode(LAssistRequest.self) else { return }
            l?.onAssistRequest(requestId: a.requestId, fromUser: a.fromUser, fromDevice: a.fromDevice)
        case IPCMethod.pushStarted:
            guard let i = try? m.decode(HostSessionInfo.self) else { return }
            l?.onSessionStarted(info: i)
        case IPCMethod.pushEnded:
            guard let i = try? m.decode(CId.self) else { return }
            l?.onSessionEnded(sessionId: i.id)
        case IPCMethod.pushAi:
            guard let a = try? m.decode(LAiActivity.self) else { return }
            l?.onAiActivity(sessionId: a.sessionId, description: a.description)
        default:
            break
        }
    }

    private func fire<T: Encodable>(_ method: String, _ body: T) {
        _ = try? host?.callSync(method, body)
    }

    override func setListener(listener: HostListener) { lock.locked { self.listener = listener } }
    /// 画面由后台服务的会话代理负责，界面进程不提供平台层
    override func setPlatform(platform: HostPlatform) {}

    override func status() -> HostStatus {
        if let s = lock.locked({ status_ }) { return s }
        if let m = try? host?.callSync(IPCMethod.status, IPCEmpty(), timeout: 2), let s = try? m.decode(CStatus.self).status {
            lock.locked { status_ = s }
            return s
        }
        return HostStatus(online: false, remoteEnabled: true, deviceId: host?.deviceId() ?? "", publicIp: "", accounts: [], sessions: [], assist: nil)
    }

    override func accounts() -> [HostAccount] { status().accounts }

    override func recentEvents(limit: UInt32) -> [HostEvent] {
        if let m = try? host?.callSync(IPCMethod.events, CLimit(limit: limit), timeout: 2), let e = try? m.decode(CEvents.self).events {
            lock.locked { events_ = e }
            return e
        }
        return lock.locked { events_ }
    }

    override func lockOnEnd() -> Bool { lock.locked { lockOnEnd_ } }

    override func setLockOnEnd(on: Bool) {
        fire(IPCMethod.setLockOnEnd, POn(on: on))
        if let v = try? host?.callSync(IPCMethod.lockOnEnd, IPCEmpty()).decode(PBool.self).value { lock.locked { lockOnEnd_ = v } }
    }

    override func headlessDisplay() -> String { lock.locked { headless_ } }

    override func setHeadlessDisplay(value: String) -> Bool {
        let ok = (try? host?.callSync(IPCMethod.setHeadlessDisplay, PText(text: value)).decode(PBool.self).value) ?? false
        if let v = try? host?.callSync(IPCMethod.headlessDisplay, IPCEmpty()).decode(PText.self).text { lock.locked { headless_ = v } }
        return ok
    }

    override func setRemoteEnabled(enabled: Bool) { fire(IPCMethod.setRemoteEnabled, CEnabled(enabled: enabled)) }
    override func disconnectAll() { fire(IPCMethod.disconnectAll, IPCEmpty()) }

    override func disconnectSession(id: String) -> Bool {
        (try? host?.callSync(IPCMethod.disconnectSession, CId(id: id)).decode(PBool.self).value) ?? false
    }

    override func respondAssist(requestId: String, allow: Bool) { fire(IPCMethod.respondAssist, CAssistResponse(requestId: requestId, allow: allow)) }
    override func setAssistCode(code: String?) { fire(IPCMethod.setAssistCode, CCode(code: code)) }

    override func startAssist() async throws -> AssistInfo {
        guard let host else { throw GalaxyError.Network(detail: L("unattended.unreachable")) }
        return try await host.call(IPCMethod.startAssist, IPCEmpty(), timeout: 60).decode(CAssist.self).info
    }

    override func stopAssist() { fire(IPCMethod.stopAssist, IPCEmpty()) }
    override func stopAi() { fire(IPCMethod.stopAi, IPCEmpty()) }

    override func updateAccountPolicy(userId: UInt64, policy: HostPolicy) {
        do {
            _ = try host?.callSync(IPCMethod.updatePolicy, CPolicy(userId: userId, policy: policy))
        } catch {
            NSLog("LemonGalaxy: 修改账号权限失败：%@", describeError(error))
        }
    }
}
