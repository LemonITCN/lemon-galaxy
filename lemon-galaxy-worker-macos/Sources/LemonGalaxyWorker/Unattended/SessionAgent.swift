import AppKit
import ScreenCaptureKit
import CoreMedia
import CoreVideo
import Security

/// 会话代理（--session-agent）：launchd 在登录界面（root）与每个已登录用户的图形会话里各启动一份
/// （/Library/LaunchAgents/cn.lemonit.galaxy.worker.agent.plist，LimitLoadToSessionType = LoginWindow + Aqua），
/// 连到后台服务，替它截屏编码、注入键鼠、读写剪贴板、采集声音、开隐私屏。
/// 与菜单栏应用是同一个可执行文件（同一签名），「屏幕录制」「辅助功能」授权对它同样生效。
enum SessionAgentMain {
    static func run(_ args: [String]) -> Int32 {
        signal(SIGPIPE, SIG_IGN)
        let socket = Unattended.arg(args, "--socket") ?? Unattended.socketPath
        let session = AgentSession.current()
        NSLog("LemonGalaxy 会话代理：启动（%@，用户 %@，会话 %u）", session.type.rawValue, session.userName, session.sessionId)
        // 平时不建 NSApplication：它会让本进程以应用的 Bundle ID 登记到 LaunchServices，双击应用、登录项打开的就成了代理。
        // 只有开隐私屏（要窗口）时才建，用完后空闲时退出、由 launchd 重启一个干净的（AgentAppKit）
        let platform = AgentPlatform(loginWindow: session.type == .loginWindow)
        let client = AgentClient(socketPath: socket, platform: platform, session: { AgentSession.current() },
                                 capabilities: { platform.capabilities() })
        client.keyframe = { platform.requestKeyframe(displayId: $0) }
        client.recycleWhenIdle = { AgentAppKit.started }
        client.shouldExit = {
            // 无人值守已关闭（全局代理配置被删）：正常退出，launchd 不再拉起
            socket == Unattended.defaultSocket && !FileManager.default.fileExists(atPath: Unattended.globalAgentPlist)
        }
        client.start()
        // 快速切换用户时会话进出屏幕：立刻告诉后台服务（另有每秒一次的轮询兜底）
        let ws = NSWorkspace.shared.notificationCenter
        ws.addObserver(forName: NSWorkspace.sessionDidBecomeActiveNotification, object: nil, queue: nil) { _ in client.consoleMayHaveChanged() }
        ws.addObserver(forName: NSWorkspace.sessionDidResignActiveNotification, object: nil, queue: nil) { _ in client.consoleMayHaveChanged() }
        // 主线程跑 RunLoop（隐私屏的窗口、NSWorkspace 通知都要它）；挂一个定时器保证它不会因为没有事件源而返回
        let keepAlive = Timer(timeInterval: 3600, repeats: true) { _ in }
        RunLoop.main.add(keepAlive, forMode: .common)
        withExtendedLifetime((client, platform, keepAlive)) {
            while true { _ = RunLoop.main.run(mode: .default, before: .distantFuture) }
        }
        return 0
    }
}

/// 会话代理里的 AppKit：只在开隐私屏时在主线程建 NSApplication 并进入它的事件循环（嵌套在主 RunLoop 里）
enum AgentAppKit {
    private static let once = OnceFlag()
    private static let flag = Box(false)
    static var started: Bool { flag.value }

    static func ensure(_ mac: MacHostPlatform) {
        guard once.fire() else { return }
        let sem = DispatchSemaphore(value: 0)
        DispatchQueue.main.async {
            let app = NSApplication.shared
            app.setActivationPolicy(.accessory)
            mac.attachAppKit()
            flag.value = true
            sem.signal()
            app.run()
        }
        sem.wait()
    }
}

/// 代理所在会话的信息
struct AgentSession: Equatable {
    var type: AgentSessionType
    var uid: uid_t
    var userName: String
    var sessionId: UInt32
    var onConsole: Bool

    static func current() -> AgentSession {
        let uid = getuid()
        let dict = CGSessionCopyCurrentDictionary() as? [String: Any] ?? [:]
        // 登录界面的代理以 root 运行（Apple DTS：pre-login agent 以 root 启动，用户登录后结束）
        let loginDone = dict["kCGSessionLoginDoneKey"] as? Bool ?? (uid != 0)
        let type: AgentSessionType = (uid == 0 || !loginDone) ? .loginWindow : .aqua
        let onConsole = dict["kCGSSessionOnConsoleKey"] as? Bool ?? false
        var sid = SecuritySessionId()
        var attrs = SessionAttributeBits()
        _ = SessionGetInfo(SecuritySessionId(bitPattern: -1), &sid, &attrs)
        let name = getpwuid(uid).flatMap { $0.pointee.pw_name.map { String(cString: $0) } } ?? "\(uid)"
        return AgentSession(type: type, uid: uid, userName: type == .loginWindow ? "loginwindow" : name, sessionId: sid, onConsole: onConsole)
    }
}

/// 连接后台服务、处理它的请求。平台层可以换（自检里用假的），连接断了自动重连。
final class AgentClient: @unchecked Sendable {
    let socketPath: String
    let platform: HostPlatform
    let session: @Sendable () -> AgentSession
    let capabilities: @Sendable () -> [String]
    /// 画面积压丢帧后要一个关键帧
    var keyframe: (@Sendable (UInt32) -> Void)?
    /// 连不上时检查要不要退出
    var shouldExit: (@Sendable () -> Bool)?
    /// 空闲（没有采集、声音、隐私屏）时是否退出让 launchd 重启（用过 AppKit 后）
    var recycleWhenIdle: (@Sendable () -> Bool)?
    /// 后台服务让本代理退出（restart 为真时以非零退出码退出，由 launchd 重启）
    var exitHandler: @Sendable (Bool) -> Void = { restart in exit(restart ? 1 : 0) }

    private let lock = NSLock()
    private var conn: IPCConnection?
    private var running = false
    private var lastConsole: AgentConsoleState?
    private var capturing = Set<UInt32>()
    private var sinks: [UInt32: ForwardingFrameSink] = [:]
    /// 主控端自己画光标的屏幕（画面里不画）。采集启动途中收到的也记着，启动成功后补设
    private var cursorOff = Set<UInt32>()
    private var audioOn = false
    private var privacyOn = false
    private let wake = DispatchSemaphore(value: 0)

    init(socketPath: String, platform: HostPlatform, session: @escaping @Sendable () -> AgentSession, capabilities: @escaping @Sendable () -> [String]) {
        self.socketPath = socketPath
        self.platform = platform
        self.session = session
        self.capabilities = capabilities
    }

    var isConnected: Bool { lock.locked { conn.map { !$0.isClosed } ?? false } }

    func start() {
        let go = lock.locked { () -> Bool in defer { running = true }; return !running }
        guard go else { return }
        let t = Thread { [self] in loop() }
        t.name = "galaxy.agent.connect"
        t.start()
        // 在不在屏幕上：每秒查一次，有变化就报
        let poll = Thread { [self] in
            while lock.locked({ running }) {
                Thread.sleep(forTimeInterval: 1)
                consoleMayHaveChanged()
            }
        }
        poll.name = "galaxy.agent.console"
        poll.start()
    }

    func stop() {
        let c = lock.locked { () -> IPCConnection? in
            running = false
            defer { conn = nil }
            return conn
        }
        c?.close()
        wake.signal()
    }

    private func loop() {
        var delay = 0.5
        while lock.locked({ running }) {
            do {
                let fd = try IPCSocket.connect(path: socketPath)
                let c = IPCConnection(fd: fd)
                c.label = "daemon"
                // 套接字在 /var/run（只有 root 能建）；正式环境再核对对端是 root
                if socketPath == Unattended.defaultSocket && c.peer.uid != 0 {
                    c.close()
                    throw IPCError.denied("后台服务不是 root")
                }
                let closed = DispatchSemaphore(value: 0)
                c.start(onRequest: { [weak self] m, r in self?.handle(m, r) },
                        onNotify: { [weak self] m in self?.handleNotify(m) },
                        onClose: { closed.signal() })
                let s = session()
                let caps = capabilities()
                let hello = AgentHello(sessionType: s.type, uid: s.uid, userName: s.userName, sessionId: s.sessionId, pid: getpid(),
                                       onConsole: s.onConsole, version: AppInfo.version, build: AppInfo.build, capabilities: caps)
                lock.locked {
                    conn = c
                    lastConsole = AgentConsoleState(onConsole: s.onConsole, capabilities: caps)
                }
                c.notify(IPCMethod.agentHello, hello)
                delay = 0.5
                closed.wait()
                lock.locked { if conn === c { conn = nil } }
                stopAll()
            } catch {
                if shouldExit?() == true {
                    NSLog("LemonGalaxy 会话代理：无人值守已关闭，退出")
                    exitHandler(false)
                    return
                }
            }
            guard lock.locked({ running }) else { break }
            _ = wake.wait(timeout: .now() + delay)
            delay = min(delay * 2, 5)
        }
    }

    /// 会话进出屏幕（快速切换用户）或授权变化时报给后台服务
    func consoleMayHaveChanged() {
        let s = session()
        let state = AgentConsoleState(onConsole: s.onConsole, capabilities: capabilities())
        let c = lock.locked { () -> IPCConnection? in
            guard let c = conn, lastConsole != state else { return nil }
            lastConsole = state
            return c
        }
        c?.notify(IPCMethod.agentConsole, state)
        // 离开屏幕：停掉采集与隐私屏（后台服务也会发 stopAll，这里先停更快）
        if c != nil && !state.onConsole { stopAll() }
    }

    private func stopAll() {
        let (displays, audio, privacy) = lock.locked { () -> (Set<UInt32>, Bool, Bool) in
            defer { capturing.removeAll(); sinks.removeAll(); cursorOff.removeAll(); audioOn = false; privacyOn = false }
            return (capturing, audioOn, privacyOn)
        }
        for d in displays { platform.stopCapture(displayId: d) }
        if audio { platform.stopAudio() }
        if privacy { _ = platform.setPrivacy(on: false) }
        // 本代理不再负责画面（离开屏幕、换会话、与后台服务断开）：它加的虚拟屏一并删掉。
        // 后台服务的内核按负责画面的代理对账，这些屏留着就没人管了
        let virtuals = platform.virtualDisplayIds()
        for id in virtuals { _ = platform.removeVirtualDisplay(displayId: id) }
        if !virtuals.isEmpty { NSLog("LemonGalaxy 会话代理：不再负责画面，删掉本代理加的 %d 块虚拟屏", virtuals.count) }
        recycleIfIdle()
    }

    /// 用过 AppKit（隐私屏）的代理在空闲时退出，launchd 随即重启一个没有登记到 LaunchServices 的。
    /// 还有虚拟屏时不退出（屏随本进程存亡）
    private func recycleIfIdle() {
        guard recycleWhenIdle?() == true else { return }
        let idle = lock.locked { capturing.isEmpty && !audioOn && !privacyOn } && platform.virtualDisplayIds().isEmpty
        guard idle else { return }
        NSLog("LemonGalaxy 会话代理：隐私屏用完且空闲，退出后由 launchd 重启")
        exitHandler(true)
    }

    // MARK: 处理后台服务的消息

    private func handle(_ m: IPCConnection.Message, _ reply: IPCConnection.Reply) {
        do {
            switch m.method {
            case IPCMethod.displays:
                reply.ok(platform.listDisplays())
            case IPCMethod.startCapture:
                let p = try m.decode(PStartCapture.self)
                guard let c = lock.locked({ conn }) else { reply.fail(.other("已断开")); return }
                let sink = ForwardingFrameSink(display: p.display, conn: c, keyframe: keyframe)
                let started = platform.startCapture(displayId: p.display, options: p.options, sink: sink)
                let cursorOff = started == nil ? false : lock.locked { () -> Bool in
                    capturing.insert(p.display)
                    sinks[p.display] = sink
                    return self.cursorOff.contains(p.display)
                }
                // 启动途中已经要求不画光标（通知与请求在不同队列上处理）：补设
                if cursorOff { platform.setCursorInVideo(displayId: p.display, show: false) }
                reply.ok(PStarted(started: started))
            case IPCMethod.screenshot:
                let p = try m.decode(PScreenshot.self)
                if let img = platform.screenshot(displayId: p.display, maxWidth: p.maxWidth, maxHeight: p.maxHeight) {
                    reply.ok(PImageMeta(mime: img.mime, width: img.width, height: img.height), blob: img.data)
                } else {
                    reply.ok(PImageMeta(mime: "", width: 0, height: 0))
                }
            case IPCMethod.uiTree:
                let p = try m.decode(PUiTree.self)
                reply.ok(PText(text: platform.uiTree(displayId: p.display, maxNodes: p.maxNodes)))
            case IPCMethod.computerUse:
                let p = try m.decode(PComputerUse.self)
                if let r = platform.computerUse(requestJson: p.request) {
                    let img = r.image
                    reply.ok(PComputerUseReply(text: r.text, mime: img?.mime ?? "", width: img?.width ?? 0, height: img?.height ?? 0),
                             blob: img?.data ?? Data())
                } else {
                    reply.ok(PComputerUseReply(text: nil, mime: "", width: 0, height: 0))
                }
            case IPCMethod.clipboardGet:
                reply.ok(PClipboard(data: platform.clipboardGet()))
            case IPCMethod.privacy:
                let on = try m.decode(POn.self).on
                let ok = platform.setPrivacy(on: on)
                lock.locked { privacyOn = on && ok }
                reply.ok(PBool(value: ok))
                if !on { recycleIfIdle() }
            case IPCMethod.startAudio:
                guard let c = lock.locked({ conn }) else { reply.fail(.other("已断开")); return }
                let ok = platform.startAudio(sink: ForwardingAudioSink(conn: c))
                lock.locked { audioOn = ok }
                reply.ok(PBool(value: ok))
            case IPCMethod.capabilities:
                reply.ok(PStrings(values: capabilities()))
            case IPCMethod.virtualAdd:
                let p = try m.decode(PVirtualAdd.self)
                let r = platform.addVirtualDisplay(width: p.width, height: p.height, refresh: p.refresh)
                if let d = r.display {
                    reply.ok(d)
                } else {
                    reply.fail(.other(r.error ?? "加虚拟屏失败"))
                }
            case IPCMethod.virtualRemove:
                let p = try m.decode(PDisplay.self)
                // 正在采集这块屏的先停掉
                if lock.locked({ capturing.remove(p.display) != nil }) {
                    lock.locked { sinks[p.display] = nil; cursorOff.remove(p.display) }
                    platform.stopCapture(displayId: p.display)
                }
                if let err = platform.removeVirtualDisplay(displayId: p.display) {
                    reply.fail(.other(err))
                } else {
                    reply.ok()
                }
                recycleIfIdle()
            case IPCMethod.virtualState:
                reply.ok(PVirtualState(ids: platform.virtualDisplayIds(), needsHeadless: platform.needsHeadlessDisplay()))
            default:
                reply.fail(.other("未知方法 \(m.method)"))
            }
        } catch {
            reply.fail(error)
        }
    }

    private func handleNotify(_ m: IPCConnection.Message) {
        switch m.method {
        case IPCMethod.stopCapture:
            guard let p = try? m.decode(PDisplay.self) else { return }
            lock.locked { _ = capturing.remove(p.display); sinks[p.display] = nil; cursorOff.remove(p.display) }
            platform.stopCapture(displayId: p.display)
            recycleIfIdle()
        case IPCMethod.keyframe:
            if let p = try? m.decode(PDisplay.self) { platform.requestKeyframe(displayId: p.display) }
        case IPCMethod.bitrate:
            if let p = try? m.decode(PBitrate.self) { platform.setBitrate(displayId: p.display, kbps: p.kbps) }
        case IPCMethod.cursorInVideo:
            guard let p = try? m.decode(PCursorInVideo.self) else { return }
            lock.locked { if p.show { cursorOff.remove(p.display) } else { cursorOff.insert(p.display) } }
            platform.setCursorInVideo(displayId: p.display, show: p.show)
        case IPCMethod.inject:
            if let p = try? m.decode(PInject.self) { platform.inject(displayId: p.display, event: p.event) }
        case IPCMethod.clipboardSet:
            if let p = try? m.decode(PClipboard.self), let d = p.data { platform.clipboardSet(data: d) }
        case IPCMethod.stopAudio:
            lock.locked { audioOn = false }
            platform.stopAudio()
            recycleIfIdle()
        case IPCMethod.stopAll:
            stopAll()
        case IPCMethod.agentExit:
            let restart = (try? m.decode(PBool.self).value) ?? false
            NSLog("LemonGalaxy 会话代理：后台服务要求退出%@", restart ? "（版本更新，由 launchd 重启）" : "")
            stopAll()
            exitHandler(restart)
        default:
            break
        }
    }
}

/// 编码后的画面帧 → 后台服务。连接积压超过 8 MB 时丢帧，直到下一个关键帧（丢了参考帧后面的帧也解不出来）
final class ForwardingFrameSink: EncodedFrameSink, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("转发对象不能由内核指针构造") }
    let display: UInt32
    private weak var conn: IPCConnection?
    private let keyframe: (@Sendable (UInt32) -> Void)?
    private let lock = NSLock()
    private var dropping = false
    private(set) var dropped = 0

    init(display: UInt32, conn: IPCConnection, keyframe: (@Sendable (UInt32) -> Void)?) {
        self.display = display
        self.conn = conn
        self.keyframe = keyframe
        super.init(noHandle: NoHandle())
    }

    override func push(data: Data, ptsUs: UInt64, keyframe key: Bool) {
        guard let c = conn else { return }
        let skip = lock.locked { dropping && !key }
        if skip { lock.locked { dropped += 1 }; return }
        if c.notify(IPCMethod.agentFrame, AFrame(display: display, pts: ptsUs, key: key), blob: data, droppable: !key) {
            if key { lock.locked { dropping = false } }
        } else {
            let first = lock.locked { () -> Bool in defer { dropping = true; dropped += 1 }; return !dropping }
            if first { keyframe?(display) }
        }
    }

    override func setResolution(width: UInt32, height: UInt32) {
        conn?.notify(IPCMethod.agentResolution, AResolution(display: display, width: width, height: height))
    }

    /// 光标位置很小、只在变化时发，不丢（丢了最后一次位置，光标停下后主控端就一直画在旧位置）
    override func setCursor(x: Int32, y: Int32, visible: Bool) {
        conn?.notify(IPCMethod.agentCursor, ACursor(display: display, x: x, y: y, visible: visible))
    }
}

/// 声音样本 → 后台服务（积压时丢掉，声音晚到不如不到）
final class ForwardingAudioSink: AudioPcmSink, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("转发对象不能由内核指针构造") }
    private weak var conn: IPCConnection?
    init(conn: IPCConnection) {
        self.conn = conn
        super.init(noHandle: NoHandle())
    }
    override func push(pcm: Data, sampleRate: UInt32, channels: UInt32) {
        conn?.notify(IPCMethod.agentAudio, AAudio(rate: sampleRate, channels: channels), blob: pcm, droppable: true, dropAbove: 2 << 20)
    }
}

// MARK: - 代理的平台层

/// 用户会话里与 Mac 主控端的本机被控完全相同（MacHostPlatform）；登录界面里：
///   - 截屏：macOS 14.4 起先试 ScreenCaptureKit（Apple 在 14.4 修好了登录界面前不能用的问题），不行再用 CGDisplayStream；
///     更早的系统直接用 CGDisplayStream。用户会话里 ScreenCaptureKit 失败（授权在但启动失败）时同样退回 CGDisplayStream。
///   - 截图：ScreenCaptureKit 不行时用 CGDisplayCreateImage。
///   - 键鼠：CGEvent 发到 HID 层（root 进程不受「辅助功能」授权限制）。
///   - 不提供隐私屏（登录界面没有可遮的内容）与声音。
final class AgentPlatform: HostPlatform, @unchecked Sendable {
    let loginWindow: Bool
    /// 不碰 NSScreen（见 MacHostPlatform.init(appKit:)）；开隐私屏时才接上 AppKit
    private let mac = MacHostPlatform(appKit: false)
    private let lock = NSLock()
    private var own: [UInt32: AnyObject] = [:]

    init(loginWindow: Bool) {
        self.loginWindow = loginWindow
    }

    private var screenAllowed: Bool { loginWindow || SystemPermissions.screenRecordingGranted }
    private var inputAllowed: Bool { SystemPermissions.accessibilityGranted || getuid() == 0 }

    func capabilities() -> [String] {
        guard loginWindow else { return mac.capabilities() }
        var c = ["clipboard"]
        // 登录界面的两种截屏（ScreenCaptureKit / CGDisplayStream）都能切换画面里的光标
        if screenAllowed { c += ["display", "screenshot", "client_cursor"] }
        if inputAllowed { c.append("input") }
        if SystemPermissions.accessibilityGranted { c.append("ui_tree") }
        let vd = virtualDisplayMax()
        if vd > 0 { c.append(VirtualDisplays.capability(vd)) }
        return c
    }

    // MARK: 虚拟屏
    // 建在本代理里、由本代理持有（CGVirtualDisplay 要在连着窗口服务器的进程里建，后台服务是 launchd 守护进程，建不了）。
    // 登录界面的代理同样能建（没接显示器的 Mac mini 开机停在登录界面时就靠它）；用户登录后登录界面的代理被系统结束，
    // 它的屏随之消失，内核对账后在用户会话的代理里重新加。本代理不再负责画面时由 AgentClient.stopAll 全部删掉。

    func virtualDisplayMax() -> UInt32 {
        guard loginWindow else { return mac.virtualDisplayMax() }
        return VirtualDisplays.shared.available && screenAllowed ? VirtualDisplays.controllerMax : 0
    }

    func addVirtualDisplay(width: UInt32, height: UInt32, refresh: UInt32) -> VirtualDisplayResult {
        mac.addVirtualDisplay(width: width, height: height, refresh: refresh)
    }

    func removeVirtualDisplay(displayId: UInt32) -> String? {
        stopOwn(displayId)
        return mac.removeVirtualDisplay(displayId: displayId)
    }

    func virtualDisplayIds() -> [UInt32] { mac.virtualDisplayIds() }
    func needsHeadlessDisplay() -> Bool { mac.needsHeadlessDisplay() }

    func listDisplays() -> [DisplayInfo] { mac.listDisplays() }

    func startCapture(displayId: UInt32, options: CaptureOptions, sink: EncodedFrameSink) -> CaptureStarted? {
        stopOwn(displayId)
        let forceStream = ProcessInfo.processInfo.environment["LEMONGALAXY_CAPTURE"] == "displaystream"
        if !loginWindow && !forceStream {
            if let s = mac.startCapture(displayId: displayId, options: options, sink: sink) { return s }
            guard SystemPermissions.screenRecordingGranted else { return nil }
            NSLog("LemonGalaxy 会话代理：ScreenCaptureKit 启动失败，改用 CGDisplayStream")
        }
        if loginWindow && !forceStream && ProcessInfo.processInfo.isOperatingSystemAtLeast(OperatingSystemVersion(majorVersion: 14, minorVersion: 4, patchVersion: 0)) {
            let cap = ScreenCapture(displayId: displayId, options: options, sink: sink)
            let sem = DispatchSemaphore(value: 0)
            nonisolated(unsafe) var started: CaptureStarted?
            Task.detached {
                started = await cap.start()
                sem.signal()
            }
            if sem.wait(timeout: .now() + 10) == .success, let s = started {
                lock.locked { own[displayId] = cap }
                NSLog("LemonGalaxy 会话代理：登录界面用 ScreenCaptureKit 截屏")
                return s
            }
            cap.stop()
            NSLog("LemonGalaxy 会话代理：登录界面 ScreenCaptureKit 不可用，改用 CGDisplayStream")
        }
        let ds = DisplayStreamCapture(displayId: displayId, options: options, sink: sink)
        guard let s = ds.start() else {
            NSLog("LemonGalaxy 会话代理：CGDisplayStream 也启动失败（屏幕 %u）", displayId)
            return nil
        }
        lock.locked { own[displayId] = ds }
        return s
    }

    private func stopOwn(_ displayId: UInt32) {
        let o = lock.locked { own.removeValue(forKey: displayId) }
        if let c = o as? ScreenCapture { c.stop() }
        if let d = o as? DisplayStreamCapture { d.stop() }
    }

    func stopCapture(displayId: UInt32) {
        stopOwn(displayId)
        mac.stopCapture(displayId: displayId)
    }

    func requestKeyframe(displayId: UInt32) {
        switch lock.locked({ own[displayId] }) {
        case let c as ScreenCapture: c.requestKeyframe()
        case let d as DisplayStreamCapture: d.requestKeyframe()
        default: mac.requestKeyframe(displayId: displayId)
        }
    }

    func setBitrate(displayId: UInt32, kbps: UInt32) {
        switch lock.locked({ own[displayId] }) {
        case let c as ScreenCapture: c.setBitrate(Int(kbps))
        case let d as DisplayStreamCapture: d.setBitrate(Int(kbps))
        default: mac.setBitrate(displayId: displayId, kbps: kbps)
        }
    }

    func setCursorInVideo(displayId: UInt32, show: Bool) {
        switch lock.locked({ own[displayId] }) {
        case let c as ScreenCapture: c.setCursorInVideo(show)
        case let d as DisplayStreamCapture: d.setCursorInVideo(show)
        default: mac.setCursorInVideo(displayId: displayId, show: show)
        }
    }

    func inject(displayId: UInt32, event: InputEvent) { mac.inject(displayId: displayId, event: event) }

    func screenshot(displayId: UInt32, maxWidth: UInt32, maxHeight: UInt32) -> ImageData? {
        if let img = mac.screenshot(displayId: displayId, maxWidth: maxWidth, maxHeight: maxHeight) { return img }
        guard screenAllowed, let cg = CGDisplayCreateImage(displayId) ?? CGDisplayCreateImage(CGMainDisplayID()) else { return nil }
        return MacHostPlatform.encode(cg, maxWidth: Int(maxWidth), maxHeight: Int(maxHeight))
    }

    func uiTree(displayId: UInt32, maxNodes: UInt32) -> String? { mac.uiTree(displayId: displayId, maxNodes: maxNodes) }
    /// 登录界面不支持；用户会话里与主控端同一份实现。会动 AI 光标的动作先接上 AppKit（光标是个窗口）
    func computerUse(requestJson: String) -> ComputerUseReply? {
        if loginWindow { return nil }
        if ComputerUse.needsCursor(requestJson) { AgentAppKit.ensure(mac) }
        return mac.computerUse(requestJson: requestJson)
    }
    func clipboardGet() -> ClipboardData? { mac.clipboardGet() }
    func clipboardSet(data: ClipboardData) { mac.clipboardSet(data: data) }
    func clipboardSetOffer(offer: ClipOfferInfo, source: ClipSource) -> String? { mac.clipboardSetOffer(offer: offer, source: source) }
    func setPrivacy(on: Bool) -> Bool {
        if loginWindow { return !on }
        if on { AgentAppKit.ensure(mac) }
        return mac.setPrivacy(on: on)
    }
    func startAudio(sink: AudioPcmSink) -> Bool { loginWindow ? false : mac.startAudio(sink: sink) }
    func stopAudio() { mac.stopAudio() }
}

/// CGDisplayStream 截屏 + VideoToolbox 编码（登录界面、ScreenCaptureKit 不可用时）。
/// macOS 14 起标为过时、15 起在部分场景（由 NSTask 启动的进程）会卡 30 秒后失败；这里只作后备。
final class DisplayStreamCapture: @unchecked Sendable {
    let displayId: UInt32
    let options: CaptureOptions
    let sink: EncodedFrameSink
    private var stream: CGDisplayStream?
    private var encoder: VideoEncoder?
    private let queue = DispatchQueue(label: "galaxy.displaystream", qos: .userInteractive)
    private var start0: UInt64?
    /// 实际采集的屏幕、画面尺寸与帧率（重建流时用）
    private var source: (id: CGDirectDisplayID, w: Int, h: Int, fps: Int)?
    /// 画面里画不画光标。CGDisplayStream 不能中途改，切换时用新设置重建流（编码器不变，画面接着走）
    private let lock = NSLock()
    private var showCursor = true
    private var reporter: CursorReporter?
    private var stopped = false
    private let cfgQueue = DispatchQueue(label: "galaxy.displaystream.cfg")
    private static let timebase: mach_timebase_info_data_t = {
        var t = mach_timebase_info_data_t()
        mach_timebase_info(&t)
        return t
    }()

    init(displayId: UInt32, options: CaptureOptions, sink: EncodedFrameSink) {
        self.displayId = displayId
        self.options = options
        self.sink = sink
    }

    func start() -> CaptureStarted? {
        let id = CGDisplayIsOnline(displayId) != 0 ? displayId : CGMainDisplayID()
        guard let mode = CGDisplayCopyDisplayMode(id) else { return nil }
        var w = mode.pixelWidth, h = mode.pixelHeight
        if w > 3840 { h = h * 3840 / w; w = 3840 }
        if h > 2400 { w = w * 2400 / h; h = 2400 }
        w &= ~1; h &= ~1
        guard w > 0, h > 0 else { return nil }
        let fps = Int(max(1, min(options.maxFps, 60)))
        let wanted = options.codecs.isEmpty ? [VideoCodec.h264] : options.codecs
        var chosen: (VideoEncoder, VideoCodec)?
        for c in wanted {
            if let enc = VideoEncoder(codec: c, width: w, height: h, fps: fps, bitrateKbps: Int(options.bitrateKbps),
                                      output: { [sink] data, pts, key in sink.push(data: data, ptsUs: pts, keyframe: key) }) {
                chosen = (enc, c)
                break
            }
        }
        guard let (enc, codec) = chosen else { return nil }
        encoder = enc
        let show = lock.locked { showCursor }
        guard let s = makeStream(id: id, w: w, h: h, fps: fps, showCursor: show) else {
            enc.invalidate()
            encoder = nil
            return nil
        }
        let r = CursorReporter(displayId: id, width: w, height: h, sink: sink)
        let changed = lock.locked { () -> Bool in
            stream = s
            source = (id, w, h, fps)
            reporter = r
            return showCursor != show
        }
        if !show { r.start() }
        if changed { setCursorInVideo(lock.locked { showCursor }) }
        return CaptureStarted(width: UInt32(w), height: UInt32(h), codec: codec)
    }

    private func makeStream(id: CGDirectDisplayID, w: Int, h: Int, fps: Int, showCursor: Bool) -> CGDisplayStream? {
        let props: [CFString: Any] = [
            CGDisplayStream.showCursor: showCursor,
            CGDisplayStream.minimumFrameTime: 1.0 / Double(fps),
            CGDisplayStream.queueDepth: 4,
        ]
        let s = CGDisplayStream(dispatchQueueDisplay: id, outputWidth: w, outputHeight: h,
                                pixelFormat: Int32(bitPattern: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange),
                                properties: props as CFDictionary, queue: queue) { [weak self] status, time, surface, _ in
            guard status == .frameComplete, let surface else { return }
            self?.frame(surface, time)
        }
        guard let s, s.start() == .success else { return nil }
        return s
    }

    /// 画面里画不画光标：不画时开始上报光标位置。重建流在 cfgQueue 上串行进行，以最后一次要求为准
    func setCursorInVideo(_ show: Bool) {
        lock.locked { showCursor = show }
        cfgQueue.async { [weak self] in
            guard let self else { return }
            let (want, src, r) = self.lock.locked { (self.showCursor, self.stopped ? nil : self.source, self.reporter) }
            guard let src else { return }
            if !want { r?.start() }
            guard let s = self.makeStream(id: src.id, w: src.w, h: src.h, fps: src.fps, showCursor: want) else {
                NSLog("LemonGalaxy 会话代理：切换画面里的光标时重建 CGDisplayStream 失败（屏幕 %u）", src.id)
                return
            }
            let old = self.lock.locked { () -> CGDisplayStream? in
                guard !self.stopped else { return s }
                defer { self.stream = s }
                return self.stream
            }
            old?.stop()
            if want { r?.stop() }
        }
    }

    private func frame(_ surface: IOSurfaceRef, _ time: UInt64) {
        var pb: Unmanaged<CVPixelBuffer>?
        guard CVPixelBufferCreateWithIOSurface(kCFAllocatorDefault, surface, nil, &pb) == kCVReturnSuccess, let buf = pb?.takeRetainedValue() else { return }
        if start0 == nil { start0 = time }
        let ticks = time &- start0!
        let us = ticks * UInt64(Self.timebase.numer) / UInt64(Self.timebase.denom) / 1000
        encoder?.encode(buf, ptsUs: us)
    }

    func stop() {
        let (s, r) = lock.locked { () -> (CGDisplayStream?, CursorReporter?) in
            stopped = true
            defer { stream = nil; reporter = nil }
            return (stream, reporter)
        }
        r?.stop()
        s?.stop()
        encoder?.invalidate()
        encoder = nil
    }

    func requestKeyframe() { encoder?.requestKeyframe() }
    func setBitrate(_ kbps: Int) { encoder?.setBitrate(kbps: kbps) }
}
