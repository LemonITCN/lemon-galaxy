import Foundation
import UIKit
import CoreVideo
import AVFoundation
import Combine
import GalaxyKit

/// 远程桌面会话：list_displays → open_display(VideoSink) → VideoToolbox 硬件解码 → 显示；手势与键盘 → InputEvent。
/// 断了自动重连（四端一致，与安卓对齐）：出过画面后会话断开、画面被被控端结束 / 出错 / 没有响应时按 ReconnectSchedule 重试；
/// 第一次就没打开（从没出过画面）的直接显示失败；后台不重连，回前台从 1 秒那次重新开始；
/// 自己停的画面（换屏幕、改帧率、离开）内核不通知，不会触发。
@MainActor
final class RemoteSession: ObservableObject, Identifiable {
    enum Status: Equatable {
        case connecting, streaming
        /// 断了正在自动重连：保留最后一帧（压暗），输入丢弃
        case reconnecting
        case failed(String)
    }

    /// 一次打开画面的结果。superseded：等待期间又开了别的画面、开始了重连或离开了界面，这次结果不要了
    private enum OpenResult {
        case opened, failed(String), superseded
    }

    let id = UUID().uuidString
    let deviceId: String
    let title: String
    let platform: Platform
    @Published private(set) var status: Status = .connecting { didSet { canvas.streaming = status == .streaming } }
    @Published private(set) var displays: [DisplayInfo] = []
    @Published private(set) var displayId: UInt32?
    @Published private(set) var quality: VideoQuality
    @Published var mouseMode: Bool {
        didSet {
            canvas.mouseMode = mouseMode
            if !mouseMode { mouseKeys.cancel() }
            applyClientCursor()
        }
    }
    /// 画面左侧的悬浮鼠标按键（只在鼠标模式下显示；开关记在设置里）
    @Published var mouseKeysOn: Bool {
        didSet { if !mouseKeysOn { mouseKeys.cancel() } }
    }
    let mouseKeys = MouseKeysModel()
    @Published private(set) var videoSize = CGSize.zero
    @Published private(set) var codec: VideoCodec = .h264
    @Published private(set) var peer: PeerConnection?
    @Published private(set) var fps = 0
    @Published var keyboardShown = false
    /// 隐私屏：被控端本地屏幕黑掉、本地键鼠失效（会话结束时被控端自动关闭）
    @Published private(set) var privacyOn = false
    @Published private(set) var privacyBusy = false
    /// 远端声音：内核用本机默认输出播放；开关记在设置里（默认开），画面打开后自动开始
    @Published private(set) var audioOn = false
    @Published private(set) var audioBusy = false
    /// 录像：内核把收到的 H.264（与正在收听的声音）直接封装成 MP4。开始时间非空即录制中
    @Published private(set) var recordingStartedAt: Date?
    @Published private(set) var recordingBusy = false
    /// 停止后要给用户看的录像（弹「已保存」，可分享或存到相册）
    @Published var savedRecording: SavedRecording?
    /// 主控最多能加几块虚拟屏（0 = 对方不支持或没有控制权限，不显示入口）；每次拿到新会话重新取
    @Published private(set) var virtualMax = 0
    /// 本会话加的虚拟屏（DisplayInfo 不带归属，只能自己记；只有这些能删）。会话换了或断开时清空
    @Published private(set) var myVirtual = MyVirtualDisplays()
    @Published private(set) var virtualAdding = false
    @Published private(set) var virtualRemoving: Set<UInt32> = []
    /// 画面是否放大 / 是否原始像素（菜单「视图」用；只在这几项变化时发布，捏合过程中不跟着每帧刷新）
    @Published private(set) var viewFlags = ViewportFlags()
    let canvas = RemoteCanvasView()
    /// 画面上方的短暂提示（缩放倍数、切换显示器、操作方式）
    let hud = RemoteHud()
    weak var app: AppModel?

    private var channel: DisplayChannel?
    private var sinkAdapter: VideoSinkAdapter?
    private let decoder = VideoDecoder()
    private var frames = 0
    private var statsTimer: Timer?
    private var foreground = true
    private var closeWatch: AnyCancellable?
    private var audio: AudioPlayback?
    /// 每次开始或停止声音加一：开到一半被关掉、切后台或会话结束时，迟到的结果直接丢弃
    private var audioGeneration = 0
    /// 正在录像的画面通道（换屏幕、重开画面后就不是当前通道了）
    private var recordingChannel: DisplayChannel?
    /// 每次开始或停止录像加一：开始到一半被停掉时，迟到的结果作废
    private var recordingGeneration = 0
    private var stopped = false

    // 自动重连
    /// 等着的下一次尝试，或新画面等第一帧的超时；结束一轮（成功、放弃、手动重试、离开）时取消
    private var reconnectTask: Task<Void, Never>?
    /// 每结束一轮加一：取消后迟到的定时与结果作废
    private var reconnectGeneration = 0
    /// 本轮已经试了几次（新画面出了第一帧才算成功、清零）
    private var reconnectAttempts = 0
    /// 正在试（连会话、取屏幕、开画面）：回前台不再另起一次，免得同时建两个会话
    private var reconnectInFlight = false
    /// 本轮新画面已经打开，在等第一帧
    private var reconnectAwaitingFrame = false
    /// 放弃时显示的原因：最近一次失败的原因，没有就是断开的原因
    private var reconnectReason = ""
    /// 现在的「连不上」是重连放弃了：重试按钮重新开始一轮；之后迟到的会话断开不再自动开始
    private var reconnectGaveUp = false
    /// 出过画面（第一帧）：从没出过画面就断的直接显示失败，不自动重连
    private var everStreamed = false
    private var foregroundWatch: AnyCancellable?

    init(app: AppModel, deviceId: String, title: String, platform: Platform) {
        self.app = app
        self.deviceId = deviceId
        self.title = title
        self.platform = platform
        quality = app.settings.defaultQuality
        mouseMode = app.settings.remoteMouseMode
        mouseKeysOn = app.settings.remoteMouseKeys
        canvas.mouseMode = mouseMode
        let keys = mouseKeys
        keys.send = { [weak self] presses in self?.canvas.mouseKeys(presses) }
        canvas.keysBusy = { [weak keys] in keys?.busy ?? false }
        canvas.beforePointerMove = { [weak keys] in keys?.beforeMove() }
        canvas.platform = platform
        canvas.send = { [weak self] events in self?.send(events) }
        canvas.onViewportChange = { [weak self] v, reason in
            guard let self else { return }
            let flags = ViewportFlags(v)
            if flags != self.viewFlags { self.viewFlags = flags }
            if reason == .gesture { self.hud.show(L("remote.zoomLevel", Int((v.zoom * 100).rounded()))) }
        }
        let display = canvas.displayLayer
        decoder.onFrame = { [weak self] pb, _ in
            DispatchQueue.main.async {
                guard let self else { return }
                RemoteCanvasView.enqueue(pb, on: display)
                self.frames += 1
                self.gotFrame()
            }
        }
        decoder.onNeedKeyframe = { [weak self] in
            DispatchQueue.main.async { self?.channel?.requestKeyframe() }
        }
        // 回前台（比 scenePhase 的 active 早到，两边都调 setForeground，只生效一次）
        foregroundWatch = NotificationCenter.default.publisher(for: UIApplication.willEnterForegroundNotification)
            .receive(on: DispatchQueue.main)
            .sink { [weak self] _ in self?.setForeground(true) }
    }

    /// 解出一帧：连接中或重连中的新画面出图了就回到正常（重连成功不提示）
    private func gotFrame() {
        switch status {
        case .streaming:
            return
        case .reconnecting:
            // 断开时丢掉的旧画面迟到的帧不算：本次尝试开始打开新画面（sinkAdapter 非空）之后的才算
            guard sinkAdapter != nil else { return }
            endReconnect()
        case .connecting, .failed:
            break
        }
        reconnectGaveUp = false
        everStreamed = true
        status = .streaming
    }

    var display: DisplayInfo? { displays.first { $0.id == displayId } }
    /// 对端能力里有 "privacy" 才显示隐私屏开关
    var supportsPrivacy: Bool { peer?.info.capabilities.contains("privacy") ?? false }

    func setPrivacy(_ on: Bool) {
        guard let peer, !privacyBusy else { return }
        privacyBusy = true
        Task {
            do {
                try await peer.session.setPrivacy(on: on)
                privacyOn = on
                app?.showToast(L(on ? "remote.privacyOn" : "remote.privacyOff"))
            } catch {
                app?.showToast(L("remote.privacyFailed", describeError(error)), error: true)
            }
            privacyBusy = false
        }
    }
    var viewOnly: Bool { !(peer?.info.permissions.control ?? true) }

    /// 对端能力里有 "client_cursor"：鼠标模式下被控端画面里不画光标，由本机画（跟手）
    var supportsClientCursor: Bool { peer?.info.capabilities.contains("client_cursor") ?? false }

    /// 鼠标模式本机画光标、被控端画面里不画；触控模式画回画面里。
    /// 每路画面各自记着（默认画），打开画面后要再发一次。只对支持的被控端发（老版本收到不认识的消息会关流）
    private func applyClientCursor() {
        guard supportsClientCursor, let channel else { return }
        channel.setClientCursor(on: mouseMode)
    }

    /// 对端能力里有 "audio"（对方给了听声音的权限且系统能采集）才显示声音开关
    var supportsAudio: Bool { peer?.info.capabilities.contains("audio") ?? false }

    /// 用户点开关：记住选择并立即开始或停止
    func setAudio(_ on: Bool) {
        app?.settings.remoteAudio = on
        if on { startAudio() } else { stopAudio() }
    }

    private func startAudioIfWanted() {
        if app?.settings.remoteAudio ?? false { startAudio() }
    }

    private func startAudio() {
        guard let peer, supportsAudio, audio == nil, !audioBusy, foreground, !stopped else { return }
        audioBusy = true
        audioGeneration += 1
        let gen = audioGeneration
        Task {
            var playback: AudioPlayback?
            var failure: Error?
            do {
                try RemoteAudioSession.acquire()
                do {
                    playback = try await peer.session.startAudio(volume: 1)
                } catch {
                    RemoteAudioSession.release()
                    throw error
                }
            } catch {
                failure = error
            }
            guard gen == audioGeneration else {
                // 等待期间被关掉了：这一路不要了
                if let playback {
                    playback.stop()
                    RemoteAudioSession.release()
                }
                return
            }
            audioBusy = false
            if let playback {
                audio = playback
                audioOn = true
            } else if let failure, !isCancellation(failure) {
                app?.showToast(L("remote.soundFailed", describeError(failure)), error: true)
            }
        }
    }

    /// 停止播放（不改设置里的开关）
    private func stopAudio() {
        audioGeneration += 1
        audioBusy = false
        audioOn = false
        guard let a = audio else { return }
        audio = nil
        a.stop()
        RemoteAudioSession.release()
    }

    func start() {
        statsTimer?.invalidate()
        statsTimer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            DispatchQueue.main.async {
                guard let self else { return }
                self.fps = self.frames
                self.frames = 0
            }
        }
        stopAudio()
        endReconnect()
        reconnectGaveUp = false
        status = .connecting
        Task { [weak self] in
            guard let self else { return }
            // 等待期间开始了重连：交给重连，不标失败
            if case .failed(let m) = await self.connect(prefer: nil), self.status != .reconnecting {
                self.status = .failed(m)
            }
        }
    }

    /// 经连接池拿会话（旧会话断了会建新会话）、取屏幕列表，打开 prefer 那块（没有了就主屏，再没有就第一块）
    private func connect(prefer: UInt32?) async -> OpenResult {
        guard let app else { return .superseded }
        do {
            let p = try await app.account.peer(deviceId)
            guard !stopped else {
                // 等待期间界面关了（关时已释放过）：刚建的会话没人用就断开，免得留在连接池里
                app.releaseIfUnused(.device(deviceId))
                return .superseded
            }
            adopt(p)
            let list = try await p.session.listDisplays()
            guard !stopped, p === peer else { return .superseded }
            applyDisplays(list)
            guard let target = list.first(where: { $0.id == prefer }) ?? list.first(where: \.primary) ?? list.first else {
                return .failed(L("remote.noDisplay"))
            }
            return await openChannel(target.id)
        } catch {
            return .failed(describeError(error))
        }
    }

    /// 换上（或重新绑定）会话。换了会话：上一个会话加的虚拟屏已由被控端删掉
    private func adopt(_ p: PeerConnection) {
        if p !== peer { myVirtual.reset() }
        peer = p
        virtualMax = Int(p.session.virtualDisplayMax())
        closeWatch = p.$closedReason.compactMap { $0 }.receive(on: DispatchQueue.main).sink { [weak self, weak p] reason in
            // 只算当前会话：重连换了新会话后，旧会话迟到的断开不管
            guard let self, let p, p === self.peer else { return }
            self.sessionLost(reason)
        }
    }

    /// 会话断了：声音通道随之失效，录像收尾（内核还留着已写的部分），被控端随会话删掉本会话加的虚拟屏；然后重连
    private func sessionLost(_ reason: String, restart: Bool = false) {
        stopRecording(.dropped)
        stopAudio()
        resetVirtual()
        connectionLost(reason.isEmpty ? L("remote.closed") : reason, restart: restart)
    }

    // MARK: 自动重连

    /// 断了（会话断开、画面被结束或没有响应）：进入重连，最后一帧留着（压暗）。
    /// 从没出过画面的直接显示失败。已经在重连时只记原因；本轮新画面还没出第一帧就又断了（含内核看门狗判定没有响应），
    /// 算这次失败、接着用剩下的次数（不从头算，免得无限循环）。
    /// 重连放弃后迟到的断开不再自动开始；restart（重试按钮、回前台）照样开始。immediate：第一次不等（只有重试按钮）
    private func connectionLost(_ reason: String, restart: Bool = false, immediate: Bool = false) {
        guard !stopped, !reconnectGaveUp || restart else { return }
        guard everStreamed else {
            dropChannel()
            endReconnect()
            status = .failed(reason)
            return
        }
        reconnectReason = reason
        if status == .reconnecting {
            if reconnectAwaitingFrame {
                reconnectAwaitingFrame = false
                dropChannel()
                scheduleReconnect()
            }
            return
        }
        dropChannel()
        endReconnect()
        reconnectGaveUp = false
        status = .reconnecting
        scheduleReconnect(immediate: immediate)
    }

    /// 丢掉当前画面通道：之后这一路迟到的回调都不算（sinkAdapter 不再是它），输入也不再发
    private func dropChannel() {
        channel?.stop()
        channel = nil
        sinkAdapter = nil
    }

    /// 排下一次尝试；次数用完就放弃。在后台不排（系统会挂起 App），回前台从 1 秒那次重新开始
    private func scheduleReconnect(immediate: Bool = false) {
        reconnectTask?.cancel()
        reconnectTask = nil
        guard foreground else { return }
        guard let wait = ReconnectSchedule.delay(before: reconnectAttempts) else {
            giveUpReconnect()
            return
        }
        let gen = reconnectGeneration
        let ns = immediate ? 0 : UInt64(wait * 1_000_000_000)
        reconnectTask = Task { [weak self] in
            if ns > 0 { try? await Task.sleep(nanoseconds: ns) }
            guard !Task.isCancelled, let self, gen == self.reconnectGeneration else { return }
            await self.reconnectAttempt(gen)
        }
    }

    private func reconnectAttempt(_ gen: Int) async {
        guard foreground, !stopped, status == .reconnecting, !reconnectInFlight else { return }
        reconnectInFlight = true
        reconnectAttempts += 1
        let result = await reconnectOnce()
        reconnectInFlight = false
        guard gen == reconnectGeneration, !stopped, status == .reconnecting else { return }
        switch result {
        case .opened:
            // 新画面开了，等第一帧（gotFrame 里结束重连）；一直不来、也没有结束通知，算这次失败
            reconnectAwaitingFrame = true
            reconnectTask = Task { [weak self] in
                try? await Task.sleep(nanoseconds: UInt64(ReconnectSchedule.firstFrameTimeout * 1_000_000_000))
                guard !Task.isCancelled, let self, gen == self.reconnectGeneration, self.reconnectAwaitingFrame else { return }
                self.reconnectAwaitingFrame = false
                self.reconnectReason = L("remote.failed")
                self.dropChannel()
                self.scheduleReconnect()
            }
        case .failed(let m):
            reconnectReason = m
            scheduleReconnect()
        case .superseded:
            // 同一轮里不该出现（别的打开都会结束这一轮）；万一出现也接着排，免得停在重连中
            scheduleReconnect()
        }
    }

    /// 一次重连：会话还在就先在同一块屏上重开画面；不行（或会话断了）再经连接池重连会话，
    /// 打开同一块屏（没有了就主屏）。声音、录像、虚拟屏照原来的规则处理
    private func reconnectOnce() async -> OpenResult {
        if let peer, peer.isAlive, let id = displayId {
            switch await openChannel(id) {
            case .opened: return .opened
            case .superseded: return .superseded
            case .failed: break
            }
        }
        return await connect(prefer: displayId)
    }

    /// 试满还不行：回到「连不上」（原因 + 重试，重试重新开始一轮）
    private func giveUpReconnect() {
        endReconnect()
        dropChannel()
        reconnectGaveUp = true
        status = .failed(reconnectReason.isEmpty ? L("remote.closed") : reconnectReason)
    }

    /// 结束这一轮（成功、放弃、手动重试、离开）：取消等着的尝试，迟到的定时与结果作废
    private func endReconnect() {
        reconnectGeneration += 1
        reconnectTask?.cancel()
        reconnectTask = nil
        reconnectAttempts = 0
        reconnectAwaitingFrame = false
    }

    /// 回前台：重连中（后台里断的或在后台停着）次数归零、从 1 秒那次重新开始；
    /// 会话已断（还没收到通知，或重连已放弃）开始一轮重连（从没出过画面的照样只显示失败）
    private func reconnectOnForeground() {
        guard !stopped else { return }
        if status == .reconnecting {
            reconnectAttempts = 0
            // 正在试的那次跑完会自己排下一次；新画面已开在等第一帧就接着等
            guard !reconnectInFlight, !reconnectAwaitingFrame else { return }
            scheduleReconnect()
        } else if status != .connecting, let peer, !peer.isAlive {
            sessionLost(peer.closedReason ?? "", restart: true)
        }
    }

    /// 切后台：不重连，等着的那次取消（正在试的让它跑完、新画面在等第一帧的照样等；回前台从 1 秒那次重新开始）
    private func pauseReconnect() {
        guard status == .reconnecting, !reconnectInFlight, !reconnectAwaitingFrame else { return }
        reconnectTask?.cancel()
        reconnectTask = nil
    }

    /// 用户发起的打开画面（换屏幕、改画质、删虚拟屏后切走、重试）。重连中不开：重连会用最新的画质，屏幕按原来的
    private func open(_ id: UInt32) async {
        guard status != .reconnecting else { return }
        if case .failed(let m) = await openChannel(id) { status = .failed(m) }
    }

    private func openChannel(_ id: UInt32) async -> OpenResult {
        guard let peer else { return .superseded }
        // 关画面时内核会自动结束录像但不返回结果：先自己停，拿到文件
        stopRecording(.reopen)
        channel?.stop()
        channel = nil
        let switched = id != displayId
        displayId = id
        if status != .reconnecting { status = .connecting }
        if let d = display { canvas.setRemoteSize(Int(d.width), Int(d.height)) }
        // 换了屏幕：光标先放屏幕中心
        if switched { canvas.resetPointer() }
        canvas.clientCursor = supportsClientCursor
        let adapter = VideoSinkAdapter()
        let decoder = self.decoder
        adapter.onConfigure = { [weak self] codec, w, h in
            decoder.configure(codec: codec)
            DispatchQueue.main.async {
                guard let self else { return }
                let size = CGSize(width: Int(w), height: Int(h))
                let changed = self.status == .streaming && self.videoSize != .zero && self.videoSize != size
                self.codec = codec
                self.videoSize = size
                // 没拿到屏幕信息时按码流尺寸摆放
                if self.display == nil { self.canvas.setRemoteSize(Int(w), Int(h)) }
                // 画面中途变了尺寸（对方改分辨率）：重新取屏幕信息，输入坐标和画面比例跟着更新
                if changed { self.refreshDisplays() }
            }
        }
        adapter.onData = { data, pts, key in decoder.decode(data, ptsUs: pts, keyframe: key) }
        adapter.onCursorMove = { [weak self] x, y, visible in
            DispatchQueue.main.async {
                guard let self else { return }
                // 回报是画面像素，输入与界面用被控屏幕像素：画面被缩放过时换算一下（相等时原样）
                let v = self.canvas.viewport
                let (rx, ry) = LocalPointer.videoToRemote(x, y, video: self.videoSize, remoteW: v.remoteW, remoteH: v.remoteH)
                self.canvas.remoteCursor(rx, ry, visible: visible)
            }
        }
        adapter.onClose = { [weak self, weak adapter] reason in
            DispatchQueue.main.async {
                guard let self else { return }
                // 画面被被控端结束、出错或没有响应（自己停的内核不通知）。只算当前画面：
                // 换屏幕、改帧率、重连换下的旧通道迟到的回调不把新画面标成断开，也不收新画面的录像
                guard let adapter, self.sinkAdapter === adapter else { return }
                self.stopRecording(.dropped)
                self.connectionLost(reason.isEmpty ? L("remote.closed") : reason)
            }
        }
        sinkAdapter = adapter
        do {
            let ch = try await peer.session.openDisplay(options: DisplayOptions(displayId: id, maxFps: quality.maxFps, bitrateKbps: quality.bitrateKbps,
                                                                                codecs: VideoDecoder.supportedCodecs, viewOnly: viewOnly),
                                                        sink: adapter)
            // 等待期间又开了别的画面、开始了重连或离开了界面：这一路不要了
            guard sinkAdapter === adapter, !stopped else {
                ch.stop()
                return .superseded
            }
            channel = ch
            canvas.viewOnly = viewOnly
            applyClientCursor()
            startAudioIfWanted()
            return .opened
        } catch {
            guard sinkAdapter === adapter, !stopped else { return .superseded }
            return .failed(describeError(error))
        }
    }

    /// 重新取屏幕列表（分辨率变化后）；失败不影响画面
    private func refreshDisplays() {
        guard let peer else { return }
        Task {
            guard let list = try? await peer.session.listDisplays(), !list.isEmpty else { return }
            applyDisplays(list)
        }
    }

    /// 换上新的屏幕列表：被控端那边已经没有的虚拟屏不再算我加的
    private func applyDisplays(_ list: [DisplayInfo]) {
        displays = list
        myVirtual.keepOnly(listed: list.map(\.id))
        if let d = display { canvas.setRemoteSize(Int(d.width), Int(d.height)) }
    }

    // MARK: 虚拟屏

    var virtualQuota: VirtualDisplayQuota { VirtualDisplayQuota(added: myVirtual.count, max: virtualMax) }

    private func resetVirtual() {
        virtualMax = 0
        myVirtual.reset()
        virtualRemoving.removeAll()
    }

    /// 在被控端加一块虚拟屏：记入我加的、刷新屏幕列表、直接切过去看。成功返回 true（表单收起），失败提示原因、表单不关
    func addVirtualDisplay(_ r: VirtualDisplayRequest) async -> Bool {
        guard let peer, !virtualAdding, !virtualQuota.full else { return false }
        virtualAdding = true
        defer { virtualAdding = false }
        do {
            let d = try await peer.session.addVirtualDisplay(width: r.width, height: r.height, refresh: r.refresh)
            // 等待期间会话换了或界面关了：被控端随旧会话删屏，这里不再处理
            guard peer === self.peer, !stopped else { return false }
            myVirtual.add(d.id)
            var list = (try? await peer.session.listDisplays()) ?? displays
            if !list.contains(where: { $0.id == d.id }) { list.append(d) }
            guard peer === self.peer, !stopped else { return false }
            applyDisplays(list)
            switchDisplay(d.id)
            app?.showToast(L("remote.vdAdded", "\(d.width)×\(d.height)"))
            return true
        } catch {
            if !isCancellation(error) { app?.showToast(L("remote.vdAddFailed", describeError(error)), error: true) }
            return false
        }
    }

    /// 删掉本会话加的虚拟屏（不用二次确认：会话结束本来也会删，误删可再加）。
    /// 正在看的就是它时先切回主屏（没有主屏就第一块）再删，免得被控端关掉这路画面时闪出错误界面（与安卓端一致）
    func removeVirtualDisplay(_ id: UInt32) {
        guard let peer, myVirtual.contains(id), !virtualRemoving.contains(id) else { return }
        virtualRemoving.insert(id)
        Task {
            defer { virtualRemoving.remove(id) }
            do {
                if displayId == id, let next = MyVirtualDisplays.fallback(removing: id, in: displays) { await open(next) }
                try await peer.session.removeVirtualDisplay(displayId: id)
                guard peer === self.peer, !stopped else { return }
                myVirtual.remove(id)
                let list = ((try? await peer.session.listDisplays()) ?? displays).filter { $0.id != id }
                guard peer === self.peer, !stopped else { return }
                if !list.isEmpty { applyDisplays(list) }
                app?.showToast(L("remote.vdRemoved"))
            } catch {
                if !isCancellation(error) { app?.showToast(L("remote.vdRemoveFailed", describeError(error)), error: true) }
            }
        }
    }

    func switchDisplay(_ id: UInt32) {
        guard id != displayId, status != .reconnecting else { return }
        Task { await open(id) }
    }

    func setQuality(_ q: VideoQuality) {
        let old = quality
        quality = q
        // 自动档与帧率变化要重开画面；只改码率直接调
        if q == .auto || old == .auto || q.maxFps != old.maxFps, let id = displayId {
            Task { await open(id) }
        } else {
            channel?.setBitrate(kbps: q.bitrateKbps)
        }
    }

    /// 「连不上」里的重试：取消等着的重连；重连放弃后重新开始一轮（立即试第一次），其余照旧重开画面或重连会话
    func retry() {
        endReconnect()
        if reconnectGaveUp {
            connectionLost(reconnectReason, restart: true, immediate: true)
            return
        }
        if let id = displayId, peer?.isAlive == true { Task { await open(id) } } else { start() }
    }

    func send(_ events: [InputEvent]) {
        // 重连中的输入丢弃（新画面出图前也不发）
        guard let channel, !viewOnly, status != .reconnecting else { return }
        for e in events { channel.sendInput(event: e) }
    }

    // MARK: 录像

    enum RecordingStop {
        /// 用户点停止、换屏幕或画质重开画面、断线、切后台：界面还在（或回来时还在），弹「已保存」
        case user, reopen, dropped, background
        /// 离开画面、关闭会话：界面已经不在了，只给个提示条
        case left, closed
    }

    var isRecording: Bool { recordingStartedAt != nil }

    func toggleRecording() {
        if isRecording { stopRecording(.user) } else { startRecording() }
    }

    func startRecording() {
        guard let ch = channel, status == .streaming, recordingChannel == nil, !recordingBusy, foreground, !stopped else { return }
        let url: URL
        do {
            url = try RecordingFiles.newURL(device: title)
        } catch {
            app?.showToast(L("remote.recordFailed", error.localizedDescription), error: true)
            return
        }
        recordingBusy = true
        recordingGeneration += 1
        let gen = recordingGeneration
        // 正在收听声音就一起录
        let withAudio = audioOn
        Task {
            do {
                try await ch.startRecording(path: url.path, withAudio: withAudio)
                guard gen == recordingGeneration else {
                    // 开始到一半被停掉（离开画面、切后台、断线）：这段不要了
                    Self.discard(try? ch.stopRecording(), fallback: url)
                    return
                }
                recordingChannel = ch
                recordingStartedAt = Date()
            } catch {
                if gen == recordingGeneration {
                    app?.showToast(L("remote.recordFailed", describeError(error)), error: true)
                }
                try? FileManager.default.removeItem(at: url)
            }
            if gen == recordingGeneration { recordingBusy = false }
        }
    }

    /// 停止录像并按原因提示。没在录时什么也不做；开始到一半时让那次开始作废
    func stopRecording(_ reason: RecordingStop) {
        recordingGeneration += 1
        recordingBusy = false
        guard let ch = recordingChannel else { return }
        recordingChannel = nil
        recordingStartedAt = nil
        let info: RecordingInfo
        do {
            info = try ch.stopRecording()
        } catch {
            app?.showToast(L("remote.recordSaveFailed", describeError(error)), error: true)
            return
        }
        let urls = info.paths.map { URL(fileURLWithPath: $0) }.filter { FileManager.default.fileExists(atPath: $0.path) }
        guard !urls.isEmpty else {
            app?.showToast(L("remote.recordEmpty"))
            return
        }
        switch reason {
        case .left, .closed:
            app?.showToast(L("remote.recordAutoSaved"))
        case .user, .reopen, .dropped, .background:
            savedRecording = SavedRecording(urls: urls, durationMs: info.durationMs, bytes: info.bytes)
        }
    }

    private static func discard(_ info: RecordingInfo?, fallback: URL) {
        let paths = info?.paths ?? [fallback.path]
        for p in paths { try? FileManager.default.removeItem(atPath: p) }
    }

    func sendClipboard() {
        guard let text = UIPasteboard.general.string, !text.isEmpty else {
            app?.showToast(L("remote.clipboardEmpty"), error: true)
            return
        }
        Task {
            do {
                try await peer?.session.clipboardSet(data: .text(text: text))
                app?.showToast(L("remote.clipboardSent"))
            } catch { app?.showError(error) }
        }
    }

    func fetchClipboard() {
        Task {
            do {
                switch try await peer?.session.clipboardGet() {
                case .text(let t)?:
                    UIPasteboard.general.string = t
                    app?.showToast(L("remote.clipboardReceived"))
                case .image(let img)?:
                    if let ui = UIImage(data: img.data) { UIPasteboard.general.image = ui }
                    app?.showToast(L("remote.clipboardReceived"))
                default:
                    app?.showToast(L("remote.clipboardEmpty"), error: true)
                }
            } catch { app?.showError(error) }
        }
    }

    /// 切后台：系统会让解码会话失效，回来后重建并请求关键帧；
    /// 声音在后台放不了（没有后台音频模式），先停掉，回来后按开关重新开始。
    /// 系统在后台会挂起 App，会话往往就断了：后台不重连，回来时会话已断或正在重连，从 1 秒那次重新开始。
    /// 两个来源：willEnterForeground 通知与 scenePhase（AppModel），只生效一次
    func setForeground(_ fg: Bool) {
        guard fg != foreground else { return }
        foreground = fg
        if fg {
            canvas.displayLayer.flush()
            channel?.requestKeyframe()
            if channel != nil { startAudioIfWanted() }
            reconnectOnForeground()
        } else {
            stopRecording(.background)
            stopAudio()
            pauseReconnect()
        }
    }

    func stop() {
        // 按着或锁定的鼠标键先抬起
        mouseKeys.cancel()
        stopRecording(.closed)
        stopped = true
        // 离开：取消等着的重连
        endReconnect()
        closeWatch = nil
        foregroundWatch = nil
        stopAudio()
        // 同一会话可能还被终端、文件用着：删掉本会话界面加的虚拟屏（会话随后断开的话被控端也会删）
        if myVirtual.count > 0, let peer, peer.closedReason == nil {
            let session = peer.session
            let ids = myVirtual.ids
            Task { for id in ids { try? await session.removeVirtualDisplay(displayId: id) } }
            myVirtual.reset()
        }
        statsTimer?.invalidate()
        channel?.stop()
        channel = nil
        decoder.invalidate()
        canvas.displayLayer.flushAndRemoveImage()
    }
}

/// 远端声音用的系统音频会话。
/// 类别用 .playback：声音是用户在工具栏里主动开的，静音键打开时也应当听得到；
/// 加 .mixWithOthers：远程桌面的声音只是辅助，不打断本机正在播放的音乐或播客，也不抢「正在播放」。
/// 几个远程桌面同时开声音时按引用计数，最后一个停了才交还会话（通知其他 App 可以恢复）。
@MainActor
enum RemoteAudioSession {
    private static var users = 0

    static func acquire() throws {
        if users == 0 {
            let s = AVAudioSession.sharedInstance()
            try s.setCategory(.playback, mode: .default, options: [.mixWithOthers])
            try s.setActive(true)
        }
        users += 1
    }

    static func release() {
        guard users > 0 else { return }
        users -= 1
        if users == 0 {
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        }
    }
}

/// 停止后的一段录像（分辨率变化时会分成几个文件）
struct SavedRecording: Identifiable {
    let id = UUID()
    let urls: [URL]
    let durationMs: UInt64
    let bytes: UInt64
}
