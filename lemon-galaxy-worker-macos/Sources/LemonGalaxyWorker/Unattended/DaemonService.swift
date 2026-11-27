import Foundation

/// 无人值守的后台服务（LaunchDaemon，root，开机启动、出错自动重启）：
///   - 跑被控内核（WorkerHost + HostService），数据在 /Library/Application Support/LemonGalaxy/worker（只有 root 能读），
///     设备身份存文件（FileSecureStore），没人登录也在线；终端、文件按账号对应的系统用户执行（内核切换用户）。
///   - 画面、键鼠、剪贴板、声音、隐私屏交给当前屏幕上那个会话的会话代理（DaemonPlatform）。
///   - 菜单栏应用连上来当界面：状态、账号、协助、事件都经这里转给内核（RemoteWorkerHost）。
///   - 把全局会话代理写到 /Library/LaunchAgents（登录界面 + 每个已登录用户）。
final class DaemonService: @unchecked Sendable {
    struct Options {
        var socketPath = Unattended.socketPath
        var dataRoot = Unattended.daemonDataRoot
        var cacheDir = Unattended.daemonCacheDir
        /// 用模拟内核（自检与开发用，不连网络）
        var mock = false
        /// 模拟内核预置演示账号（开发时走一遍检查更新、自动更新用）
        var mockAccounts = false
        /// 写 /Library/LaunchAgents、在已登录会话里加载会话代理（只在以 root 运行时）
        var installAgents = false
        /// 会话代理报的 uid 必须与套接字对端一致（自检里一个进程模拟多个会话时关掉）
        var checkAgentUid = true
        /// 导入数据、要求重启时怎么做；默认退出进程由 launchd 重启
        var restart: (@Sendable () -> Void)? = nil
        /// 自动更新替换哪个应用（默认本程序所在的包）、怎么打开新版本（默认由 launchd 重启）；自检时换成临时目录
        var updateTarget: URL? = nil
        var updateLaunch = "none"
    }

    final class Client: @unchecked Sendable {
        let id: Int
        let conn: IPCConnection
        let auth: PeerAuth
        init(id: Int, conn: IPCConnection, auth: PeerAuth) {
            self.id = id
            self.conn = conn
            self.auth = auth
        }
    }

    let opts: Options
    let worker: WorkerHost
    let platform = DaemonPlatform()
    let isMock: Bool
    let coreVersion: String
    let workerDir: URL
    let keyStore: FileSecureStore
    let updater: DaemonUpdater
    private let lock = NSLock()
    private var service: HostService?
    private var clients: [Int: Client] = [:]
    private var nextClient = 1
    private var server: IPCServer?
    private let listener = DaemonListener()
    private var released = false

    init(_ opts: Options, locale: String) {
        self.opts = opts
        umask(0o022)
        let fm = FileManager.default
        try? fm.createDirectory(at: opts.dataRoot, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o755])
        try? fm.createDirectory(at: opts.cacheDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o755])
        workerDir = opts.dataRoot.appendingPathComponent("worker", isDirectory: true)
        try? fm.createDirectory(at: workerDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        chmod(workerDir.path, 0o700)
        keyStore = FileSecureStore(dir: workerDir.appendingPathComponent("keys", isDirectory: true))
        isMock = opts.mock || !WorkerCore.realCoreAvailable
        if isMock {
            worker = MockWorkerHost(options: MockWorkerOptions(latencyScale: 0, presetAccounts: opts.mockAccounts))
            coreVersion = MockWorkerHost.version
        } else {
            #if LEMONGALAXY_REAL_CORE
            WorkerCore.initCoreOnce(WorkerCore.config(locale: locale, dataDir: opts.dataRoot, cacheDir: opts.cacheDir))
            worker = WorkerHost(store: PrefixedSecureStore(base: keyStore, prefix: "worker."))
            coreVersion = LemonGalaxyWorker.coreVersion()
            #else
            worker = MockWorkerHost(options: MockWorkerOptions(latencyScale: 0, presetAccounts: opts.mockAccounts))
            coreVersion = MockWorkerHost.version
            #endif
        }
        // 内核日志目录也只给 root
        chmod(opts.dataRoot.appendingPathComponent("logs").path, 0o700)
        // 自动更新：本程序的签名要求趁包还没被换掉先取好
        _ = AppUpdateCodeSign.own
        var uo = DaemonUpdater.Options(stateDir: opts.dataRoot.appendingPathComponent("update", isDirectory: true),
                                       downloadDir: opts.cacheDir.appendingPathComponent("updates", isDirectory: true))
        if let t = opts.updateTarget { uo.target = t }
        uo.launch = opts.updateLaunch
        updater = DaemonUpdater(worker: worker, opts: uo)
        // 远程升级回调：交给 AppUpdateEntry，本进程里转给上面的 DaemonUpdater（模拟内核不登记；Model/RemoteUpdateHandler.swift）
        RemoteUpdateRequestHandler.register(isMock: isMock, selfTest: false)
        listener.owner = self
        platform.expectedBuild = AppInfo.build
        platform.onAgentsChanged = { [weak self] in self?.broadcastInfo() }
        updater.onChange = { [weak self] in self?.broadcastInfo() }
        updater.restart = { [weak self] in self?.restartSoon() }
    }

    // MARK: 启动

    func start() throws {
        let s = try IPCServer(path: opts.socketPath, mode: 0o666)
        server = s
        s.start { [weak self] conn in self?.accept(conn) }
        // 监听 5 秒后还在：报告启动成功（自动更新后助手在等它，等不到就换回旧版本）
        DispatchQueue.global().asyncAfter(deadline: .now() + 5) { [weak self] in self?.updater.launched() }
        Task { [weak self] in
            guard let self else { return }
            do { _ = try await self.worker.start() } catch { NSLog("LemonGalaxy: 被控服务启动失败：%@", String(describing: error)) }
            self.attachIfNeeded()
        }
        if opts.installAgents, let exe = Bundle.main.executablePath {
            DispatchQueue.global().async {
                if AgentPlist.install(executable: exe, bundleId: AppInfo.bundleId) {
                    NSLog("LemonGalaxy: 已写入 %@", Unattended.globalAgentPlist)
                    AgentPlist.reloadInGuiSessions()
                }
            }
        }
    }

    func stop() {
        server?.stop()
        let cs = lock.locked { Array(clients.values) }
        for c in cs { c.conn.close() }
    }

    func attachIfNeeded() {
        guard let s = worker.service() else { return }
        let fresh = lock.locked { () -> Bool in
            if let cur = service, cur === s { return false }
            service = s
            return true
        }
        if fresh {
            s.setPlatform(platform: platform)
            s.setListener(listener: listener)
        }
        broadcastInfo()
    }

    var currentService: HostService? { lock.locked { service } }

    var claimed: Bool {
        worker.server() != nil || !(currentService?.accounts().isEmpty ?? true)
    }

    func info(for auth: PeerAuth) -> DaemonInfo {
        DaemonInfo(version: AppInfo.version, build: AppInfo.build, coreVersion: coreVersion, deviceId: worker.deviceId(),
                   server: worker.server(), hasService: currentService != nil, claimed: claimed,
                   peerAdmin: auth.isAdmin, peerConsole: auth.isConsole, peerUser: auth.userName,
                   agents: platform.summaries(), isMock: isMock, update: updater.status)
    }

    // MARK: 连接

    /// 第一条消息决定角色：会话代理发 a.hello 通知，界面发 c.hello 请求。10 秒内不表明身份就断开
    private func accept(_ conn: IPCConnection) {
        let decided = OnceFlag()
        conn.label = "pending-\(conn.peer.pid)"
        conn.start(onRequest: { [weak self, weak conn] msg, reply in
            guard let self, let conn else { return }
            guard msg.method == IPCMethod.hello, decided.fire() else { reply.fail(.denied("先发送 hello")); return }
            self.controlHello(conn, msg, reply)
        }, onNotify: { [weak self, weak conn] msg in
            guard let self, let conn, msg.method == IPCMethod.agentHello, decided.fire() else { return }
            self.agentHello(conn, msg)
        }, onClose: nil)
        DispatchQueue.global().asyncAfter(deadline: .now() + 10) { [weak conn] in
            if decided.fire() { conn?.close() }
        }
    }

    private func agentHello(_ conn: IPCConnection, _ msg: IPCConnection.Message) {
        guard let hello = try? msg.decode(AgentHello.self) else { conn.close(); return }
        guard CodeIdentity.peerMatches(auditToken: conn.peer.auditToken) else {
            NSLog("LemonGalaxy: 拒绝会话代理（进程 %d）：代码签名与后台服务不一致", conn.peer.pid)
            platform.noteRejected()
            // 多半是应用刚更新、这个代理还是旧程序（旧包已删，签名对不上）：让它退出，launchd 用包里的新程序重启它。
            // 冒充的程序收到这条也没有用处
            conn.notify(IPCMethod.agentExit, PBool(value: true))
            DispatchQueue.global().asyncAfter(deadline: .now() + 0.5) { conn.close() }
            return
        }
        if opts.checkAgentUid && hello.uid != conn.peer.uid {
            NSLog("LemonGalaxy: 拒绝会话代理（进程 %d）：报的用户与实际不符", conn.peer.pid)
            platform.noteRejected()
            conn.close()
            return
        }
        if lock.locked({ released }) { conn.notify(IPCMethod.agentExit, PBool(value: false)); return }
        platform.addAgent(conn, hello: hello)
    }

    private func controlHello(_ conn: IPCConnection, _ msg: IPCConnection.Message, _ reply: IPCConnection.Reply) {
        let hello = try? msg.decode(ControlHello.self)
        let auth = PeerAuth.evaluate(conn.peer, consoleUid: platform.consoleUid(), daemonUid: getuid())
        // 签名不一致但对端是管理员（多半是应用刚更新、后台服务还是旧程序）：只允许查询与重启
        guard auth.allowed || (auth.isAdmin && !auth.codeOK) else {
            reply.fail(.denied(L("unattended.denied")))
            conn.close()
            return
        }
        let id = lock.locked { () -> Int in defer { nextClient += 1 }; return nextClient }
        let client = Client(id: id, conn: conn, auth: auth)
        conn.label = "ui-\(id)-\(auth.userName)"
        conn.setHandlers(onRequest: { [weak self] m, r in self?.handle(client, m, r) }, onNotify: nil,
                         onClose: { [weak self] in self?.lock.locked { _ = self?.clients.removeValue(forKey: id) } })
        lock.locked { clients[id] = client }
        if let h = hello, !h.locale.isEmpty { applyLocale(h.locale) }
        reply.ok(info(for: auth))
    }

    /// 界面的语言（内核的 locale，如 zh-CN）：内核错误文字与后台服务自己的提示都跟着换
    private func applyLocale(_ locale: String) {
        WorkerCore.setLocale(locale, isMock: isMock)
        if let pack = Localization.shared.packs.first(where: { $0.coreLocale == locale }) {
            Localization.shared.apply(setting: pack.code)
        }
    }

    // MARK: 推送

    func broadcast<T: Encodable>(_ method: String, _ body: T) {
        let cs = lock.locked { Array(clients.values) }
        for c in cs where c.auth.allowed { c.conn.notify(method, body) }
    }

    func broadcastInfo() {
        let cs = lock.locked { Array(clients.values) }
        for c in cs where c.auth.allowed { c.conn.notify(IPCMethod.pushInfo, info(for: c.auth)) }
    }

    // MARK: 界面的请求

    private func handle(_ client: Client, _ m: IPCConnection.Message, _ reply: IPCConnection.Reply) {
        // 「在不在屏幕前」随快速切换用户变化，每个请求重新看
        var auth = client.auth
        auth.isConsole = platform.consoleUid() == auth.uid
        if !auth.allowed && m.method != IPCMethod.restart && m.method != IPCMethod.hello {
            reply.fail(.denied(L("unattended.denied")))
            return
        }
        do {
            switch m.method {
            case IPCMethod.hello:
                reply.ok(info(for: auth))
            case IPCMethod.setLocale:
                applyLocale(try m.decode(CLocale.self).locale)
                reply.ok()
            case IPCMethod.restart:
                reply.ok()
                restartSoon()

            case IPCMethod.login:
                let p = try m.decode(CLogin.self)
                // 后台服务以 root 运行：账号对应到发起登录的用户（终端、文件以他的身份执行），不能对应到 root
                let sys = auth.uid == 0 ? (Unattended.consoleUser()?.name ?? auth.userName) : auth.userName
                Task {
                    do {
                        let r = try await worker.login(server: p.server, username: p.username, password: p.password, totp: p.totp,
                                                       newPassword: p.newPassword, systemUser: sys)
                        attachIfNeeded()
                        reply.ok(CLoginResult(result: r, info: info(for: auth)))
                    } catch { reply.fail(error) }
                }
            case IPCMethod.logout:
                let u = try m.decode(CUser.self)
                if let a = currentService?.accounts().first(where: { $0.userId == u.userId }), !auth.mayManage(a.policy) {
                    throw IPCFault.denied(L("unattended.notYourAccount"))
                }
                Task {
                    do { try await worker.logout(userId: u.userId); reply.ok(CInfo(info: info(for: auth))) } catch { reply.fail(error) }
                }
            case IPCMethod.setServer:
                let s = try m.decode(CServer.self)
                Task {
                    do { _ = try await worker.setServer(server: s.server); attachIfNeeded(); reply.ok(CInfo(info: info(for: auth))) } catch { reply.fail(error) }
                }
            case IPCMethod.start:
                Task {
                    do { _ = try await worker.start(); attachIfNeeded(); reply.ok(CInfo(info: info(for: auth))) } catch { reply.fail(error) }
                }
            case IPCMethod.checkUpdate:
                let c = try m.decode(CUpdateCheck.self)
                Task {
                    do { reply.ok(CUpdateInfo(info: try await worker.checkUpdate(platform: c.platform, current: c.current))) } catch { reply.fail(error) }
                }
            case IPCMethod.downloadUpdate:
                let d = try m.decode(CDownload.self)
                let dir = opts.cacheDir.appendingPathComponent("updates", isDirectory: true)
                try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o755])
                let name = (d.info.fileName as NSString).lastPathComponent.replacingOccurrences(of: "/", with: "_")
                let dest = dir.appendingPathComponent(name.isEmpty ? "LemonGalaxyWorker-update.dmg" : name).path
                worker.downloadUpdate(info: d.info, dest: dest, listener: DaemonDownloadListener(token: d.token, path: dest, conn: client.conn))
                reply.ok()
            case IPCMethod.updateRequest:
                // 预先下载 / 立即更新：进度经概况推送（DaemonInfo.update）
                let u = try m.decode(CUpdateRequest.self)
                try updater.request(u.info, install: u.install)
                NSLog("LemonGalaxy: %@ 要求%@ %@", auth.userName, u.install ? "立即更新到" : "预先下载", u.info.version)
                reply.ok()

            default:
                try handleService(client, m, reply)
            }
        } catch {
            reply.fail(error)
        }
    }

    private func handleService(_ client: Client, _ m: IPCConnection.Message, _ reply: IPCConnection.Reply) throws {
        var auth = client.auth
        auth.isConsole = platform.consoleUid() == auth.uid
        switch m.method {
        case IPCMethod.exportData, IPCMethod.importData:
            guard auth.isAdmin else { throw IPCFault.denied(L("unattended.needAdmin")) }
            try handleMigration(client, m, reply)
            return
        case IPCMethod.release:
            // 不是管理员也能关闭（去掉会话代理），但只有管理员能删掉后台服务里的设备身份与账号
            let wipe = (try m.decode(CRelease.self)).wipe
            guard !wipe || auth.isAdmin else { throw IPCFault.denied(L("unattended.needAdmin")) }
            release(wipe: wipe)
            reply.ok()
            return
        default: break
        }
        guard let s = currentService else { throw IPCFault.other(L("unattended.noService")) }
        // 内核让临时协助会话以本进程的用户执行终端与文件，后台服务里就是 root：只有管理员能开启或允许
        // （内核改为按屏幕前的用户执行之前的保护，见 docs/spec.md 3.0.1）
        if !auth.mayOperateAssist(daemonIsRoot: getuid() == 0) {
            switch m.method {
            case IPCMethod.startAssist, IPCMethod.setAssistCode: throw IPCFault.denied(L("unattended.assistAdmin"))
            case IPCMethod.respondAssist where (try? m.decode(CAssistResponse.self).allow) == true: throw IPCFault.denied(L("unattended.assistAdmin"))
            default: break
            }
        }
        switch m.method {
        case IPCMethod.status: reply.ok(CStatus(status: s.status()))
        case IPCMethod.events: reply.ok(CEvents(events: s.recentEvents(limit: try m.decode(CLimit.self).limit)))
        case IPCMethod.lockOnEnd: reply.ok(PBool(value: s.lockOnEnd()))
        case IPCMethod.setLockOnEnd: s.setLockOnEnd(on: try m.decode(POn.self).on); reply.ok()
        case IPCMethod.headlessDisplay: reply.ok(PText(text: s.headlessDisplay()))
        case IPCMethod.setHeadlessDisplay: reply.ok(PBool(value: s.setHeadlessDisplay(value: try m.decode(PText.self).text ?? "")))
        case IPCMethod.setRemoteEnabled: s.setRemoteEnabled(enabled: try m.decode(CEnabled.self).enabled); reply.ok()
        case IPCMethod.disconnectAll: s.disconnectAll(); reply.ok()
        case IPCMethod.disconnectSession: reply.ok(PBool(value: s.disconnectSession(id: try m.decode(CId.self).id)))
        case IPCMethod.respondAssist:
            let r = try m.decode(CAssistResponse.self)
            s.respondAssist(requestId: r.requestId, allow: r.allow)
            reply.ok()
        case IPCMethod.setAssistCode: s.setAssistCode(code: try m.decode(CCode.self).code); reply.ok()
        case IPCMethod.startAssist:
            Task {
                do { reply.ok(CAssist(info: try await s.startAssist())) } catch { reply.fail(error) }
            }
        case IPCMethod.stopAssist: s.stopAssist(); reply.ok()
        case IPCMethod.stopAi: s.stopAi(); reply.ok()
        case IPCMethod.updatePolicy:
            let p = try m.decode(CPolicy.self)
            guard let old = s.accounts().first(where: { $0.userId == p.userId }) else { throw IPCFault(kind: "notFound", code: nil, detail: L("unattended.noAccount")) }
            guard auth.mayChange(old.policy, to: p.policy) else { throw IPCFault.denied(L("unattended.notYourAccount")) }
            s.updateAccountPolicy(userId: p.userId, policy: p.policy)
            reply.ok()
        default:
            throw IPCFault.other("未知方法 \(m.method)")
        }
    }

    // MARK: 移交

    private func handleMigration(_ client: Client, _ m: IPCConnection.Message, _ reply: IPCConnection.Reply) throws {
        switch m.method {
        case IPCMethod.exportData:
            guard let id = keyStore.get(key: "worker.device_id"), let key = keyStore.get(key: "worker.device_key") else {
                throw IPCFault.other(L("unattended.noIdentity"))
            }
            reply.ok(CMigration(bundle: MigrationBundle(deviceId: id, deviceKey: key, files: Migration.collect(dir: workerDir), sourceUser: "root")))
        case IPCMethod.importData:
            guard !claimed else { throw IPCFault.other(L("unattended.alreadyClaimed")) }
            let b = try m.decode(CMigration.self).bundle
            guard !b.deviceId.isEmpty, b.deviceId.count <= 64, !b.deviceKey.isEmpty, b.deviceKey.count <= 256 else {
                throw IPCFault.other("设备身份不完整")
            }
            for f in b.files where !Migration.safeRelative(f.path) { throw IPCFault.other("不合法的路径：\(f.path)") }
            // 清掉新装时自动生成的数据（保留 keys/ 目录本身），写入移交来的
            let fm = FileManager.default
            for name in (try? fm.contentsOfDirectory(atPath: workerDir.path)) ?? [] where name != "keys" {
                try? fm.removeItem(at: workerDir.appendingPathComponent(name))
            }
            try Migration.write(b.files, to: workerDir)
            keyStore.set(key: "worker.device_id", value: b.deviceId)
            keyStore.set(key: "worker.device_key", value: b.deviceKey)
            NSLog("LemonGalaxy: 已接收用户 %@ 移交的设备身份与 %d 个文件，重启后台服务", b.sourceUser, b.files.count)
            reply.ok()
            restartSoon()
        default:
            break
        }
    }

    /// 关闭无人值守：断开所有会话、拒绝新连接、去掉全局会话代理；wipe 时再删数据（已由界面拿回当前用户）。
    /// 之后等界面注销本服务（launchd 随后结束本进程）
    func release(wipe: Bool) {
        lock.locked { released = true }
        currentService?.setRemoteEnabled(enabled: false)
        currentService?.disconnectAll()
        if wipe { try? FileManager.default.removeItem(at: workerDir) }
        if opts.installAgents { AgentPlist.uninstall() }
        platform.dismissAll()
        NSLog("LemonGalaxy: 无人值守已关闭%@，等待注销", wipe ? "（已删除后台服务的数据）" : "")
    }

    func restartSoon() {
        DispatchQueue.global().asyncAfter(deadline: .now() + 0.3) { [opts] in
            if let r = opts.restart { r() } else { exit(0) }
        }
    }
}

/// 管理后台服务的权限：管理员（或与后台服务同一用户）可以做所有事；
/// 当前坐在这台 Mac 前的普通用户可以查看、开关远程、应答协助、管理对应到自己的账号；其他人一律拒绝。
struct PeerAuth: Equatable {
    var uid: uid_t
    var userName: String
    var isAdmin: Bool
    var isConsole: Bool
    var codeOK: Bool

    var allowed: Bool { codeOK && (isAdmin || isConsole) }

    static func evaluate(_ p: PeerCredentials, consoleUid: uid_t?, daemonUid: uid_t) -> PeerAuth {
        PeerAuth(uid: p.uid, userName: p.userName, isAdmin: p.uid == daemonUid || p.isAdmin,
                 isConsole: consoleUid == p.uid, codeOK: CodeIdentity.peerMatches(auditToken: p.auditToken))
    }

    /// 开启 / 允许临时协助：内核让协助会话以后台服务的身份（root）执行终端与文件，所以以 root 运行时只有管理员可以
    func mayOperateAssist(daemonIsRoot: Bool) -> Bool { !daemonIsRoot || isAdmin }

    /// 账号对应的系统用户就是自己，或者是管理员
    func mayManage(_ policy: HostPolicy) -> Bool { isAdmin || policy.systemUser == userName }

    /// 改权限：能管这个账号；改对应的系统用户只有管理员可以
    func mayChange(_ old: HostPolicy, to new: HostPolicy) -> Bool {
        mayManage(old) && (old.systemUser == new.systemUser || isAdmin)
    }
}

/// 内核回调 → 推给所有界面
final class DaemonListener: HostListener, @unchecked Sendable {
    weak var owner: DaemonService?
    func onStatus(status: HostStatus) { owner?.broadcast(IPCMethod.pushStatus, CStatus(status: status)) }
    func onAssistRequest(requestId: String, fromUser: String, fromDevice: String) {
        owner?.broadcast(IPCMethod.pushAssist, LAssistRequest(requestId: requestId, fromUser: fromUser, fromDevice: fromDevice))
    }
    func onSessionStarted(info: HostSessionInfo) { owner?.broadcast(IPCMethod.pushStarted, info) }
    func onSessionEnded(sessionId: String) { owner?.broadcast(IPCMethod.pushEnded, CId(id: sessionId)) }
    func onAiActivity(sessionId: String, description: String) {
        owner?.broadcast(IPCMethod.pushAi, LAiActivity(sessionId: sessionId, description: description))
    }
}

/// 后台服务替界面下载更新：下到 /Library/Caches 下（所有人可读），界面再拷到自己的「下载」文件夹
final class DaemonDownloadListener: TransferListener, @unchecked Sendable {
    let token: String
    let path: String
    private weak var conn: IPCConnection?
    private let lock = NSLock()
    private var last = Date.distantPast
    init(token: String, path: String, conn: IPCConnection) {
        self.token = token
        self.path = path
        self.conn = conn
    }
    func onProgress(done: UInt64, total: UInt64) {
        let now = Date()
        let send = lock.locked { () -> Bool in
            guard now.timeIntervalSince(last) >= 0.1 || done >= total else { return false }
            last = now
            return true
        }
        guard send else { return }
        conn?.notify(IPCMethod.pushDownload, CDownloadEvent(token: token, done: done, total: total, finished: false, error: nil, path: nil))
    }
    func onFinished(error: String?) {
        if error == nil { chmod(path, 0o644) }
        conn?.notify(IPCMethod.pushDownload, CDownloadEvent(token: token, done: 0, total: 0, finished: true, error: error, path: error == nil ? path : nil))
    }
}

// MARK: - 入口

/// --daemon [--socket 路径] [--data-dir 目录] [--mock [--mock-accounts]]
/// 由 launchd 以 root 启动（包内 Contents/Library/LaunchDaemons/cn.lemonit.galaxy.worker.daemon.plist，SMAppService 登记）。
/// 开发时可以不用 root：指定 --socket 与 --data-dir 到自己能写的地方（不写 /Library/LaunchAgents）。
enum DaemonMain {
    static func run(_ args: [String]) -> Int32 {
        var o = DaemonService.Options()
        if let s = Unattended.arg(args, "--socket") { o.socketPath = s }
        if let d = Unattended.arg(args, "--data-dir") {
            o.dataRoot = URL(fileURLWithPath: d, isDirectory: true)
            o.cacheDir = o.dataRoot.appendingPathComponent("cache", isDirectory: true)
        }
        o.mock = args.contains("--mock")
        o.mockAccounts = args.contains("--mock-accounts")
        o.installAgents = getuid() == 0 && Unattended.arg(args, "--data-dir") == nil
        if getuid() != 0 && Unattended.arg(args, "--data-dir") == nil {
            FileHandle.standardError.write(Data("后台服务需要以 root 运行（由 launchd 启动）；开发时请用 --socket 与 --data-dir 指定自己能写的位置。\n".utf8))
            return 2
        }
        if !o.mock && !WorkerCore.realCoreAvailable {
            FileHandle.standardError.write(Data("没有接入 Rust 内核，只能用 --mock 运行。\n".utf8))
            return 2
        }
        // 后台服务没有用户界面语言：用系统首选语言，界面连上后按界面的语言切换内核错误文字
        Localization.shared.apply(setting: "system")
        let svc = DaemonService(o, locale: Localization.shared.strings.pack.coreLocale)
        do {
            try svc.start()
        } catch {
            FileHandle.standardError.write(Data("后台服务启动失败：\(error)\n".utf8))
            return 1
        }
        NSLog("LemonGalaxy: 后台服务已启动（%@ %@，套接字 %@，数据 %@%@）", AppInfo.version, AppInfo.build, o.socketPath, o.dataRoot.path, svc.isMock ? "，模拟内核" : "")
        signal(SIGPIPE, SIG_IGN)
        // 收到 SIGTERM（注销无人值守、关机）直接退出；内核状态在每次变化时已经写盘
        let term = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
        signal(SIGTERM, SIG_IGN)
        term.setEventHandler {
            svc.stop()
            exit(0)
        }
        term.resume()
        withExtendedLifetime((svc, term)) { dispatchMain() }
    }
}

/// --unattended-status [--socket 路径]：不开窗口，连上后台服务打印概况（排查用，任何用户都能跑；
/// 不是管理员也不在屏幕前的用户会被后台服务拒绝，这本身也说明后台服务在运行）
enum UnattendedStatusCLI {
    static func run(_ args: [String]) -> Int32 {
        let socket = Unattended.arg(args, "--socket") ?? Unattended.socketPath
        Localization.shared.apply(setting: "system")
        if AppInfo.isBundled {
            let st = UnattendedController.daemonService.status
            let text: String = {
                switch st {
                case .enabled: return "已启用"
                case .requiresApproval: return "等待在「系统设置 → 通用 → 登录项」里批准"
                case .notRegistered: return "未登记"
                case .notFound: return "找不到（应用没有放在「应用程序」里、没有被 LaunchServices 登记，或包内缺少配置）"
                @unknown default: return "未知"
                }
            }()
            print("登记状态（SMAppService）：\(text)")
        }
        print("全局会话代理配置：\(FileManager.default.fileExists(atPath: Unattended.globalAgentPlist) ? Unattended.globalAgentPlist : "没有")")
        guard let fd = try? IPCSocket.connect(path: socket) else {
            print("后台服务：连不上 \(socket)")
            return 1
        }
        let c = IPCConnection(fd: fd)
        c.start()
        defer { c.close() }
        print("后台服务：已连接 \(socket)（进程 \(c.peer.pid)，用户 \(c.peer.userName)）")
        let info: DaemonInfo
        do {
            info = try c.call(IPCMethod.hello, ControlHello(version: AppInfo.version, build: AppInfo.build, locale: ""), timeout: 5).decode(DaemonInfo.self)
        } catch {
            print("后台服务拒绝了本次查询：\(error)")
            return 2
        }
        print("版本 \(info.version)（\(info.build)），内核 \(info.coreVersion)\(info.isMock ? "（模拟）" : "")")
        print("设备号 \(groupedId(info.deviceId))")
        print("服务器 \(info.server ?? "—")，被控服务\(info.hasService ? "运行中" : "未启动")")
        print("你的身份：\(info.peerUser)\(info.peerAdmin ? "，管理员" : "")\(info.peerConsole ? "，正在使用这台 Mac" : "")")
        if info.build != AppInfo.build { print("注意：后台服务版本与本程序（\(AppInfo.build)）不同，打开应用会让它重启") }
        if info.agents.isEmpty { print("会话代理：没有（登录界面或用户会话的代理还没连上）") }
        for a in info.agents {
            let who = a.sessionType == .loginWindow ? "登录界面" : "用户 \(a.userName)"
            print("会话代理：\(who)，\(a.onConsole ? "在屏幕上" : "不在屏幕上")\(a.active ? "，负责画面" : "")，版本 \(a.version)")
        }
        if info.hasService, let m = try? c.call(IPCMethod.status, IPCEmpty(), timeout: 5), let s = try? m.decode(CStatus.self).status {
            print("在线 \(s.online ? "是" : "否")，允许远程 \(s.remoteEnabled ? "是" : "否")，账号 \(s.accounts.map { "\($0.username)→\($0.policy.systemUser)" }.joined(separator: ", "))，会话 \(s.sessions.count) 个")
        }
        return 0
    }
}
