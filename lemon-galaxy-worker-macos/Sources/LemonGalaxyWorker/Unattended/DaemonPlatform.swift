import Foundation

/// 后台服务这一侧的 HostPlatform：自己不碰屏幕（launchd 守护进程不该连窗口服务器），
/// 把内核的请求转给「当前显示在屏幕上的会话」里的会话代理（登录界面或某个已登录用户），回来的画面帧、声音交给内核。
/// 会话切换（登录、注销、快速切换用户）时换到新会话的代理，正在进行的画面、声音、隐私屏自动续上（与 Windows 的 WinPlatform 相同）。
final class DaemonPlatform: HostPlatform, @unchecked Sendable {
    final class Agent: @unchecked Sendable {
        let id: Int
        let conn: IPCConnection
        var hello: AgentHello
        var onConsole: Bool
        var consoleSince: Date
        var capabilities: [String]
        init(id: Int, conn: IPCConnection, hello: AgentHello) {
            self.id = id
            self.conn = conn
            self.hello = hello
            onConsole = hello.onConsole
            consoleSince = Date()
            capabilities = hello.capabilities
        }
    }

    private struct Capture {
        let sink: EncodedFrameSink
        var options: CaptureOptions
    }

    private let lock = NSLock()
    private var agents: [Int: Agent] = [:]
    private var nextAgentId = 1
    private var activeId: Int?
    private var captures: [UInt32: Capture] = [:]
    private var audioSink: AudioPcmSink?
    private var privacyOn = false
    /// 切换会话时重新开始采集，串行进行
    private let switchQueue = DispatchQueue(label: "galaxy.daemon.switch")
    /// 代理增减、换会话时通知界面
    var onAgentsChanged: (@Sendable () -> Void)?
    /// 控制台用户（测试时替换）
    var consoleUid: @Sendable () -> uid_t? = { Unattended.consoleUser()?.uid }
    /// 代理的版本与后台服务不同时（应用更新后）让它退出、由 launchd 用新程序重启
    var expectedBuild: String?

    // MARK: 代理登记

    /// 会话代理连上并发来 hello 后调用。同一安全会话里的旧代理被替换
    func addAgent(_ conn: IPCConnection, hello: AgentHello) {
        if let want = expectedBuild, !hello.build.isEmpty, hello.build != want {
            NSLog("LemonGalaxy: 会话代理版本 %@ 与后台服务 %@ 不同，让它退出后由 launchd 重启", hello.build, want)
            conn.notify(IPCMethod.agentExit, PBool(value: true))
            return
        }
        let id = lock.locked { () -> Int in defer { nextAgentId += 1 }; return nextAgentId }
        let agent = Agent(id: id, conn: conn, hello: hello)
        conn.label = "agent-\(id)-\(hello.sessionType.rawValue)-\(hello.userName)"
        conn.setHandlers(onRequest: { _, reply in reply.fail(.other("后台服务不处理会话代理的请求")) },
                         onNotify: { [weak self] m in self?.agentMessage(id, m) },
                         onClose: { [weak self] in self?.removeAgent(id) })
        let replaced = lock.locked { () -> [Agent] in
            let old = agents.values.filter { $0.hello.sessionId == hello.sessionId && hello.sessionId != 0 }
            for o in old { agents[o.id] = nil }
            agents[id] = agent
            return old
        }
        // 同一会话里的旧代理：让它正常退出（launchd 不再拉起），避免两份来回顶替
        for o in replaced {
            o.conn.notify(IPCMethod.agentExit, PBool(value: false))
            DispatchQueue.global().asyncAfter(deadline: .now() + 0.5) { o.conn.close() }
        }
        // 接上处理函数之前就断了：不留死代理
        if conn.isClosed { lock.locked { _ = agents.removeValue(forKey: id) } }
        NSLog("LemonGalaxy: 会话代理已连接（%@，用户 %@，会话 %u，%@）", hello.sessionType.rawValue, hello.userName, hello.sessionId,
              hello.onConsole ? "在屏幕上" : "不在屏幕上")
        reselect()
    }

    private func removeAgent(_ id: Int) {
        let removed = lock.locked { agents.removeValue(forKey: id) != nil }
        guard removed else { return }
        NSLog("LemonGalaxy: 会话代理 %d 已断开", id)
        reselect()
    }

    private func agentMessage(_ id: Int, _ m: IPCConnection.Message) {
        switch m.method {
        case IPCMethod.agentFrame:
            guard let f = try? m.decode(AFrame.self) else { return }
            let sink = lock.locked { activeId == id ? captures[f.display]?.sink : nil }
            sink?.push(data: m.blob, ptsUs: f.pts, keyframe: f.key)
        case IPCMethod.agentResolution:
            guard let r = try? m.decode(AResolution.self) else { return }
            let sink = lock.locked { activeId == id ? captures[r.display]?.sink : nil }
            sink?.setResolution(width: r.width, height: r.height)
        case IPCMethod.agentAudio:
            guard let a = try? m.decode(AAudio.self) else { return }
            let sink = lock.locked { activeId == id ? audioSink : nil }
            sink?.push(pcm: m.blob, sampleRate: a.rate, channels: a.channels)
        case IPCMethod.agentConsole:
            guard let c = try? m.decode(AgentConsoleState.self) else { return }
            lock.locked {
                guard let a = agents[id] else { return }
                if a.onConsole != c.onConsole { a.consoleSince = Date() }
                a.onConsole = c.onConsole
                a.hello.onConsole = c.onConsole
                a.capabilities = c.capabilities
            }
            reselect()
        case IPCMethod.agentLog:
            if let l = try? m.decode(ALog.self) { NSLog("LemonGalaxy 会话代理 %d：%@", id, l.text) }
        default:
            break
        }
    }

    /// 选出应该负责画面的代理：自己报告在屏幕上的（多个时取最近切过来的）；
    /// 都没报告时按控制台用户找，没人登录时用登录界面的代理
    private func pick() -> Agent? {
        let all = Array(agents.values)
        if let a = all.filter(\.onConsole).max(by: { $0.consoleSince < $1.consoleSince }) { return a }
        if let uid = consoleUid() {
            return all.first { $0.hello.sessionType == .aqua && $0.hello.uid == uid }
        }
        return all.filter { $0.hello.sessionType == .loginWindow }.max { $0.id < $1.id }
    }

    private func reselect() {
        let (old, new) = lock.locked { () -> (Agent?, Agent?) in
            let old = activeId.flatMap { agents[$0] }
            let new = pick()
            activeId = new?.id
            return (old, new)
        }
        if old?.id != new?.id {
            NSLog("LemonGalaxy: 画面改由%@负责", new.map { "\($0.hello.sessionType.rawValue)（\($0.hello.userName)）的会话代理" } ?? "（暂无会话代理）")
            // 旧会话停掉采集与隐私屏（它可能还连着，只是不在屏幕上了）
            old?.conn.notify(IPCMethod.stopAll)
            if new != nil { switchQueue.async { [weak self] in self?.resume() } }
        }
        onAgentsChanged?()
    }

    /// 新会话接手：重开正在进行的画面、声音、隐私屏
    private func resume() {
        let (caps, audio, privacy) = lock.locked { (captures, audioSink, privacyOn) }
        if let audio, !startAudioOnActive(audio) { NSLog("LemonGalaxy: 换会话后重新采集声音失败") }
        if privacy, !setPrivacyOnActive(true) { NSLog("LemonGalaxy: 换会话后重新打开隐私屏失败") }
        for (display, c) in caps {
            if let s = startOnActive(display, c.options) {
                c.sink.setResolution(width: s.width, height: s.height)
            } else {
                NSLog("LemonGalaxy: 换会话后屏幕 %u 重新采集失败", display)
            }
        }
    }

    func summaries() -> [AgentSummary] {
        lock.locked {
            agents.values.sorted { $0.id < $1.id }.map {
                AgentSummary(sessionType: $0.hello.sessionType, userName: $0.hello.userName, uid: $0.hello.uid,
                             onConsole: $0.onConsole, active: $0.id == activeId, version: $0.hello.version)
            }
        }
    }

    /// 关闭无人值守：所有会话代理正常退出（launchd 不再拉起）
    func dismissAll() {
        let all = lock.locked { () -> [Agent] in
            defer { agents.removeAll(); activeId = nil }
            return Array(agents.values)
        }
        for a in all { a.conn.notify(IPCMethod.agentExit, PBool(value: false)) }
        onAgentsChanged?()
    }

    /// 被拒绝的会话代理次数（签名或用户对不上；自检与日志用）
    private var rejected = 0
    var rejectedAgents: Int { lock.locked { rejected } }
    func noteRejected() { lock.locked { rejected += 1 } }

    /// 自检用：某块屏幕当前的内核输出
    func debugSink(_ display: UInt32) -> EncodedFrameSink? { lock.locked { captures[display]?.sink } }

    var activeAgent: AgentHello? { lock.locked { activeId.flatMap { agents[$0]?.hello } } }

    // MARK: 转发

    /// 当前负责画面的代理；wait 秒内等它出现（开机后登录界面的代理比后台服务晚几秒连上）
    private func active(wait: Double = 0) -> Agent? {
        let end = Date().addingTimeInterval(wait)
        while true {
            if let a = lock.locked({ activeId.flatMap { agents[$0] } }) { return a }
            if Date() >= end { return nil }
            Thread.sleep(forTimeInterval: 0.1)
        }
    }

    private func request<T: Encodable>(_ method: String, _ body: T, wait: Double = 0, timeout: Double) -> IPCConnection.Message? {
        guard let a = active(wait: wait) else { return nil }
        do {
            return try a.conn.call(method, body, timeout: timeout)
        } catch {
            NSLog("LemonGalaxy: 会话代理 %@ 失败：%@", method, String(describing: error))
            return nil
        }
    }

    private func notifyActive<T: Encodable>(_ method: String, _ body: T) {
        active()?.conn.notify(method, body)
    }

    private func startOnActive(_ display: UInt32, _ options: CaptureOptions) -> CaptureStarted? {
        guard let m = request(IPCMethod.startCapture, PStartCapture(display: display, options: options), wait: 8, timeout: 40),
              let r = try? m.decode(PStarted.self) else { return nil }
        return r.started
    }

    private func startAudioOnActive(_ sink: AudioPcmSink) -> Bool {
        guard let m = request(IPCMethod.startAudio, IPCEmpty(), wait: 3, timeout: 10) else { return false }
        return (try? m.decode(PBool.self).value) ?? false
    }

    private func setPrivacyOnActive(_ on: Bool) -> Bool {
        guard let m = request(IPCMethod.privacy, POn(on: on), wait: on ? 3 : 0, timeout: 10) else { return !on }
        return (try? m.decode(PBool.self).value) ?? false
    }

    // MARK: HostPlatform

    func listDisplays() -> [DisplayInfo] {
        guard let m = request(IPCMethod.displays, IPCEmpty(), wait: 5, timeout: 5) else { return [] }
        return (try? m.decode([DisplayInfo].self)) ?? []
    }

    func startCapture(displayId: UInt32, options: CaptureOptions, sink: EncodedFrameSink) -> CaptureStarted? {
        lock.locked { captures[displayId] = Capture(sink: sink, options: options) }
        let started = startOnActive(displayId, options)
        if started == nil { lock.locked { if captures[displayId]?.sink === sink { captures[displayId] = nil } } }
        return started
    }

    func stopCapture(displayId: UInt32) {
        lock.locked { _ = captures.removeValue(forKey: displayId) }
        notifyActive(IPCMethod.stopCapture, PDisplay(display: displayId))
    }

    func requestKeyframe(displayId: UInt32) { notifyActive(IPCMethod.keyframe, PDisplay(display: displayId)) }

    func setBitrate(displayId: UInt32, kbps: UInt32) {
        lock.locked { captures[displayId]?.options.bitrateKbps = kbps }
        notifyActive(IPCMethod.bitrate, PBitrate(display: displayId, kbps: kbps))
    }

    func inject(displayId: UInt32, event: InputEvent) {
        notifyActive(IPCMethod.inject, PInject(display: displayId, event: event))
    }

    func screenshot(displayId: UInt32, maxWidth: UInt32, maxHeight: UInt32) -> ImageData? {
        guard let m = request(IPCMethod.screenshot, PScreenshot(display: displayId, maxWidth: maxWidth, maxHeight: maxHeight), wait: 3, timeout: 15),
              !m.blob.isEmpty, let meta = try? m.decode(PImageMeta.self) else { return nil }
        return ImageData(mime: meta.mime, data: m.blob, width: meta.width, height: meta.height)
    }

    func uiTree(displayId: UInt32, maxNodes: UInt32) -> String? {
        guard let m = request(IPCMethod.uiTree, PUiTree(display: displayId, maxNodes: maxNodes), timeout: 15) else { return nil }
        return (try? m.decode(PText.self))?.text
    }

    func clipboardGet() -> ClipboardData? {
        guard let m = request(IPCMethod.clipboardGet, IPCEmpty(), timeout: 5) else { return nil }
        return (try? m.decode(PClipboard.self))?.data
    }

    func clipboardSet(data: ClipboardData) { notifyActive(IPCMethod.clipboardSet, PClipboard(data: data)) }

    func setPrivacy(on: Bool) -> Bool {
        let ok = setPrivacyOnActive(on)
        lock.locked { privacyOn = on && ok }
        return ok
    }

    func startAudio(sink: AudioPcmSink) -> Bool {
        lock.locked { audioSink = sink }
        let ok = startAudioOnActive(sink)
        if !ok { lock.locked { if audioSink === sink { audioSink = nil } } }
        return ok
    }

    func stopAudio() {
        lock.locked { audioSink = nil }
        notifyActive(IPCMethod.stopAudio, IPCEmpty())
    }

    func capabilities() -> [String] {
        // 刚开机、登录界面的代理还没连上时稍等（主控端握手时要用）；最终以代理自己报告的为准
        guard let a = active(wait: 5) else { return [] }
        if let m = try? a.conn.call(IPCMethod.capabilities, IPCEmpty(), timeout: 2), let v = try? m.decode(PStrings.self) {
            lock.locked { a.capabilities = v.values }
            return v.values
        }
        return lock.locked { a.capabilities }
    }
}
