import SwiftUI
import AppKit
import CoreVideo
import VideoToolbox
import Carbon.HIToolbox
import Combine

/// 远程桌面标签：list_displays → open_display(VideoSink) → 解码显示；鼠标键盘 → InputEvent。
@MainActor
final class RemoteDesktopModel: TabModel {
    /// reconnecting：会话断开或画面意外结束后自动重连中（最后一帧压暗留着）
    enum Phase: Equatable { case connecting, streaming, reconnecting, failed(String) }
    enum ScaleMode: String { case fit, original }

    @Published var phase: Phase = .connecting
    @Published var peer: PeerConnection?
    @Published var displays: [DisplayInfo] = []
    @Published var displayId: UInt32 = 0
    @Published var streamSize = CGSize.zero
    @Published var codec: VideoCodec = .h264
    @Published var scaleMode: ScaleMode = .fit
    @Published var quality: String
    @Published var fps: Int
    @Published var bitrateKbps: Int
    @Published var clipboardSync: Bool {
        didSet {
            guard clipboardSync != oldValue else { return }
            // 关掉同步：正在传的文件作废；重新打开时对方剪贴板里的文件可以再拉一次
            cancelFileClip()
            lastRemoteFiles = nil
        }
    }
    @Published var commandAsCtrl = true
    @Published var measuredFps = 0
    @Published var measuredKbps = 0
    @Published var remoteCursor: CGPoint?
    @Published var focusMode = false
    /// 隐私屏：被控端本地屏幕黑掉、本地键鼠失效（对端能力里有 "privacy" 才显示开关）
    @Published var privacyOn = false
    @Published var privacyBusy = false
    /// 远端声音：对端能力里有 "audio" 才显示开关；开不开记在设置里，打开画面后按设置自动开始
    @Published var audioOn = false
    @Published var audioBusy = false
    @Published var audioVolume: Double
    private(set) var audio: AudioPlayback?
    /// 正在录像（内核把收到的 H.264 / Opus 原样封装成分片 MP4，不重新编码）
    @Published var recording: Recording?
    @Published var recordBusy = false
    /// 虚拟屏：本会话最多能加几块（0 = 不支持或没有控制权限，不显示入口）
    @Published var virtualMax: UInt32 = 0
    /// 本会话加的虚拟屏（id → 加的时候选的模式），只有这些能删；会话换了清空（被控端随旧会话已删掉）
    @Published var myVirtual: [UInt32: VirtualDisplayMode] = [:]
    @Published var virtualBusy = false
    private(set) var closed = false
    private var peerWatch: AnyCancellable?
    /// 自动重连（docs/protocol.md 8.3「画面结束」）：正在进行的重连任务与最近一次失败原因
    private var reconnectTask: Task<Void, Never>?
    private var lastReconnectError: String?
    /// 已用掉的重试次数：真正出了画面才清零（打开了却一帧不来又被关掉时接着用剩下的次数，不会无限循环）
    private var reconnectAttempt = 0
    /// 这个标签出过画面没有：一次画面都没出过就断的直接显示失败，不自动重连
    private var everStreamed = false
    /// 重连间隔：约 2 分钟后放弃，显示失败与重试
    private static let reconnectDelays: [Double] = [1, 2, 4, 8, 15, 15, 15, 15, 15, 15]
    let videoView = RemoteVideoNSView()
    private var channel: DisplayChannel?
    private(set) var sinkAdapter: VideoSinkAdapter?
    nonisolated(unsafe) private let decoder = VideoDecoder()
    private var frameCount = 0
    private var byteCount = 0
    private var statsTimer: Timer?
    private var clipboardTimer: Timer?
    private var lastPasteboardChange = ClipboardFiles.pasteboard.changeCount
    /// 上次和对方同步时（推过去或拉过来）本机剪贴板的 changeCount：之后本机又复制了东西，鼠标回到画面时以本机为准
    private var pasteboardChangeAtSync = ClipboardFiles.pasteboard.changeCount
    /// 上次同步时对方剪贴板里的文字（推过去的或拉过来的）：对方没再复制新文字就不拉
    private(set) var lastRemoteText: String?
    /// 复制粘贴文件的进度条（画面上方）；nil 时不显示
    @Published var fileClip: RemoteFileClip?
    private(set) var clipTransfer: ClipboardTransfer?
    private var clipListener: ClipTransferListener?
    /// 每次开始或作废一次文件传输加一：迟到的回调按它丢弃
    private var clipGen = 0
    /// 上次同步过的对方剪贴板文件列表（已放进本机剪贴板等粘贴的，或推送后对方剪贴板里的暂存路径）：同一列表不重复处理
    private(set) var lastRemoteFiles: [String]?
    /// 本标签放进本机剪贴板的对方文件清单（等着在访达里粘贴）；关标签时撤回
    private(set) var remoteOfferId: UInt64?
    var framesDecoded = 0

    init(app: AppModel, deviceId: String, title: String) {
        let s = app.settings
        quality = s.displayQuality
        fps = s.displayFps
        bitrateKbps = s.displayBitrateKbps
        clipboardSync = s.clipboardSync
        audioVolume = s.remoteAudioVolume
        super.init(app: app, kind: .remote, title: title)
        self.deviceId = deviceId
        videoView.model = self
        decoder.onFrame = { [weak self] pb, _ in
            DispatchQueue.main.async { MainActor.assumeIsolated { self?.show(pb) } }
        }
        decoder.onNeedKeyframe = { [weak self] in
            DispatchQueue.main.async { MainActor.assumeIsolated { self?.channel?.requestKeyframe() } }
        }
    }

    var display: DisplayInfo? { displays.first { $0.id == displayId } }
    var platform: Platform { peer?.info.platform ?? app?.device(deviceId ?? "")?.platform ?? .windows }

    func start() {
        // 手动重试：放弃正在等的自动重连，从头来
        reconnectTask?.cancel()
        reconnectTask = nil
        reconnectAttempt = 0
        Task {
            guard let app, let deviceId else { return }
            do {
                let p = try await app.peer(deviceId)
                if peer !== p { watchPeer(p) }
                peer = p
                virtualMax = p.session.virtualDisplayMax()
                displays = try await p.session.listDisplays()
                guard let first = displays.first(where: \.primary) ?? displays.first else {
                    phase = .failed(L("remote.noDisplay"))
                    live = false
                    return
                }
                await open(first.id)
            } catch {
                phase = .failed(AppModel.describe(error))
                live = false
            }
        }
        statsTimer?.invalidate()
        statsTimer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.measuredFps = self.frameCount
                self.measuredKbps = self.byteCount * 8 / 1000
                self.frameCount = 0
                self.byteCount = 0
            }
        }
        setupClipboardSync()
    }

    /// 打开画面。`reconnecting` 时界面保持「正在重新连接」，失败不改成失败状态（由重连循环决定），返回是否打开了。
    @discardableResult
    func open(_ id: UInt32, reconnecting: Bool = false) async -> Bool {
        guard let peer else { return false }
        // 切屏、改帧率要重开画面：录像随旧画面结束（内核 stop 时也会自动结束），这里先停下来给出保存位置
        if recording != nil { stopRecording() }
        channel?.stop()
        channel = nil
        displayId = id
        if !reconnecting { phase = .connecting }
        let adapter = VideoSinkAdapter(model: self)
        sinkAdapter = adapter
        let codecs: [VideoCodec] = app?.settings.preferH265 == true ? [.h265, .h264] : [.h264]
        do {
            channel = try await peer.session.openDisplay(options: DisplayOptions(displayId: id, maxFps: UInt32(fps), bitrateKbps: UInt32(bitrateKbps), codecs: codecs, viewOnly: !(peer.info.permissions.control)),
                                                         sink: adapter)
            live = true
            // 画面开起来之后再开声音（切屏、改帧率重开画面时声音不断）
            if audio == nil, audioSupported, app?.settings.remoteAudio == true { startAudio() }
            return true
        } catch {
            if reconnecting {
                lastReconnectError = AppModel.describe(error)
            } else {
                phase = .failed(AppModel.describe(error))
                live = false
            }
            return false
        }
    }

    // VideoSink 回调（已切到主线程或解码队列）
    func configured(codec: VideoCodec, width: UInt32, height: UInt32) {
        self.codec = codec
        streamSize = CGSize(width: Int(width), height: Int(height))
        decoder.configure(codec: codec)
        videoView.needsLayout = true
    }

    nonisolated func frameArrived(_ data: Data, pts: UInt64, keyframe: Bool) {
        decoder.decode(data, ptsUs: pts, keyframe: keyframe)
        DispatchQueue.main.async { MainActor.assumeIsolated { self.byteCount += data.count } }
    }

    private func show(_ pb: CVPixelBuffer) {
        frameCount += 1
        framesDecoded += 1
        if phase != .streaming {
            phase = .streaming
            everStreamed = true
            reconnectAttempt = 0
        }
        videoView.display(pb)
    }

    /// 画面不是我们停的却结束了（被控端结束、断线、画面没有响应等）：自动重连，会话还在就先只重开画面
    func displayClosed(_ reason: String) {
        // 内核在我们自己停画面时回 "stopped"（切屏、关标签），不算
        guard !closed, reason != "stopped" else { return }
        if recording != nil { stopRecording() }
        connectionLost(reason.isEmpty ? L("remote.closed") : reason, reopenOnly: true)
    }

    /// 会话断开或画面意外结束：最后一帧压暗留着、显示「正在重新连接」，按间隔自动重连，约 2 分钟仍不行才显示失败
    private func connectionLost(_ reason: String, reopenOnly: Bool) {
        guard !closed else { return }
        if reconnectTask != nil { return }
        channel = nil
        live = false
        // 一次画面都没出过（第一次就没打开）：直接失败，由用户点重试
        guard everStreamed else {
            phase = .failed(reason)
            return
        }
        phase = .reconnecting
        lastReconnectError = nil
        let first = reconnectAttempt
        reconnectTask = Task { [weak self] in
            while let me = self, !Task.isCancelled, !me.closed, me.reconnectAttempt < Self.reconnectDelays.count {
                let i = me.reconnectAttempt
                me.reconnectAttempt += 1
                try? await Task.sleep(nanoseconds: UInt64(Self.reconnectDelays[i] * 1_000_000_000))
                guard let me = self, !Task.isCancelled, !me.closed else { return }
                // 打开了还要等到第一帧才算连上
                if await me.reconnectOnce(reopenOnly: reopenOnly && i == first), await me.waitFirstFrame() {
                    me.reconnectTask = nil
                    return
                }
            }
            guard let me = self, !Task.isCancelled, !me.closed else { return }
            me.reconnectTask = nil
            me.phase = .failed(me.lastReconnectError ?? reason)
        }
    }

    /// 重连打开画面后等第一帧（最多 20 秒）
    private func waitFirstFrame() async -> Bool {
        for _ in 0..<80 {
            if phase == .streaming { return true }
            if closed { return false }
            try? await Task.sleep(nanoseconds: 250_000_000)
        }
        lastReconnectError = L("remote.noResponse")
        return false
    }

    /// 重连一次：会话还在且只是画面断了就只重开画面，否则重新建会话；回到原来那块屏（没了就主屏）
    private func reconnectOnce(reopenOnly: Bool) async -> Bool {
        guard let app, let deviceId else { return false }
        do {
            let alive = peer.map { $0.closedReason == nil && !$0.session.isClosed() } ?? false
            if !(reopenOnly && alive) {
                let p = try await app.peer(deviceId)
                if peer !== p { watchPeer(p) }
                peer = p
                virtualMax = p.session.virtualDisplayMax()
            }
            guard let peer else { return false }
            displays = try await peer.session.listDisplays()
            guard let target = displays.first(where: { $0.id == displayId }) ?? displays.first(where: \.primary) ?? displays.first else {
                lastReconnectError = L("remote.noDisplay")
                return false
            }
            return await open(target.id, reconnecting: true)
        } catch {
            lastReconnectError = AppModel.describe(error)
            return false
        }
    }

    func cursorMoved(x: Int32, y: Int32, visible: Bool) {
        remoteCursor = visible ? CGPoint(x: Int(x), y: Int(y)) : nil
    }

    func send(_ e: InputEvent) {
        // 粘贴键在等本机剪贴板同步过去：后面的输入排在它后面，顺序不乱
        if heldInput != nil { heldInput?.append(e); return }
        channel?.sendInput(event: e)
    }

    /// 正在等剪贴板同步时排队的输入（第一项是粘贴键的按下）
    private var heldInput: [InputEvent]?

    /// 远程画面里按了粘贴（Cmd+V，控制 Windows / Linux 时映射成 Ctrl+V）：本机剪贴板刚变过、每秒一次的轮询还没同步时，
    /// 先立刻同步（文字设过去；被控端支持时文件只发清单），最多等 0.3 秒再把按键发出去，免得对方粘出旧内容
    func sendPasteKey(_ e: InputEvent) {
        if heldInput != nil { heldInput?.append(e); return }
        guard let sync = pollClipboard() else {
            send(e)
            return
        }
        heldInput = [e]
        Task {
            await Self.wait(sync, upTo: 0.3)
            let events = heldInput ?? []
            heldInput = nil
            for ev in events { channel?.sendInput(event: ev) }
        }
    }

    /// 等 task 完成，最多等 seconds 秒（超时不取消它）
    nonisolated static func wait(_ task: Task<Void, Never>, upTo seconds: Double) async {
        final class Once: @unchecked Sendable {
            private let lock = NSLock()
            private var done = false
            func fire() -> Bool { lock.locked { defer { done = true }; return !done } }
        }
        let once = Once()
        await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
            Task {
                await task.value
                if once.fire() { c.resume() }
            }
            Task {
                try? await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
                if once.fire() { c.resume() }
            }
        }
    }

    func applyQuality(_ q: String) {
        quality = q
        let v = AppSettings.quality(q)
        let fpsChanged = v.fps != fps
        fps = v.fps
        bitrateKbps = v.kbps
        if fpsChanged { Task { await open(displayId) } } else { channel?.setBitrate(kbps: UInt32(v.kbps)) }
    }

    func setBitrate(_ kbps: Int) {
        quality = "custom"
        bitrateKbps = kbps
        channel?.setBitrate(kbps: UInt32(kbps))
    }

    func setFps(_ f: Int) {
        quality = "custom"
        fps = f
        Task { await open(displayId) }
    }

    func special(_ c: ComboKey) { send(.specialCombo(combo: c)) }

    /// 新的会话：被控端的隐私屏随旧会话结束已自动关闭；会话断开时开关复位、声音停掉（重连打开画面后按设置再开）
    private func watchPeer(_ p: PeerConnection) {
        privacyOn = false
        myVirtual = [:]
        stopAudio()
        peerWatch = p.$closedReason.sink { [weak self] reason in
            guard let reason, let self, self.peer === p else { return }
            self.privacyOn = false
            self.myVirtual = [:]
            self.stopAudio()
            // 会话断了：录像到此为止
            if self.recording != nil { self.stopRecording() }
            // 以前这里不改界面，画面停在最后一帧还像连着、键鼠发进死流；现在自动重连
            self.connectionLost(reason.isEmpty ? L("remote.closed") : reason, reopenOnly: false)
        }
    }

    // MARK: 远端声音

    var audioSupported: Bool { peer?.info.capabilities.contains("audio") ?? false }

    /// 工具栏开关：记进设置，下次打开远程桌面照此自动开或不开
    func setAudio(_ on: Bool) {
        app?.settings.remoteAudio = on
        if on { startAudio() } else { stopAudio() }
    }

    func setVolume(_ v: Double) {
        audioVolume = min(max(v, 0), 1)
        app?.settings.remoteAudioVolume = audioVolume
        audio?.setVolume(volume: Float(audioVolume))
    }

    private func startAudio() {
        guard let peer, audio == nil, !audioBusy else { return }
        audioBusy = true
        let volume = Float(audioVolume)
        Task {
            do {
                let a = try await peer.session.startAudio(volume: volume)
                // 等待期间标签关了、会话换了或断了、又被关掉了：立刻停
                if closed || self.peer !== peer || peer.closedReason != nil || app?.settings.remoteAudio != true {
                    a.stop()
                } else {
                    a.setVolume(volume: Float(audioVolume))
                    audio = a
                    audioOn = true
                }
            } catch {
                audioOn = false
                if !closed { app?.showToast(L("remote.audio.failed", AppModel.describe(error)), error: true) }
            }
            audioBusy = false
        }
    }

    private func stopAudio() {
        audio?.stop()
        audio = nil
        audioOn = false
    }

    /// 对端支持隐私屏（能力列表里有 "privacy"）
    var privacySupported: Bool { peer?.info.capabilities.contains("privacy") ?? false }

    func setPrivacy(_ on: Bool) {
        guard let peer, !privacyBusy else { return }
        privacyBusy = true
        Task {
            do {
                try await peer.session.setPrivacy(on: on)
                privacyOn = on
                app?.showToast(L(on ? "remote.privacy.on" : "remote.privacy.off"))
            } catch {
                app?.showToast(L("remote.privacy.failed", AppModel.describe(error)), error: true)
            }
            privacyBusy = false
        }
    }

    // MARK: 录像

    struct Recording {
        /// 录在哪个画面通道上（切屏后 channel 换了，停录像要找原来那个）
        let channel: DisplayChannel
        let path: String
        var startedMs: Int64
        let withAudio: Bool
    }

    /// 录像保存位置：~/Movies/LemonGalaxy（selftest 换成临时目录）
    static var recordingDirOverride: URL?
    static var recordingDir: URL {
        recordingDirOverride ?? FileManager.default.urls(for: .moviesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("LemonGalaxy", isDirectory: true)
    }

    /// LemonGalaxy-<设备名>-<yyyyMMdd-HHmmss>.mp4；设备名里不能进文件名的字符换成 -
    nonisolated static func recordingFileName(device: String, at ms: Int64) -> String {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "yyyyMMdd-HHmmss"
        let bad = CharacterSet(charactersIn: "/\\:*?\"<>|").union(.newlines).union(.controlCharacters)
        var name = device.unicodeScalars.map { bad.contains($0) ? "-" : String($0) }.joined().trimmingCharacters(in: .whitespaces)
        if name.isEmpty { name = "device" }
        return "LemonGalaxy-\(name)-\(f.string(from: Date(timeIntervalSince1970: TimeInterval(ms) / 1000))).mp4"
    }

    var canRecord: Bool { channel != nil && phase == .streaming }

    func toggleRecording() {
        if recording != nil { stopRecording() } else { startRecording() }
    }

    /// 开始录像：正在收听声音时一起录
    func startRecording() {
        guard let channel, recording == nil, !recordBusy, phase == .streaming else { return }
        recordBusy = true
        let dir = Self.recordingDir
        let withAudio = audioOn
        let path = dir.appendingPathComponent(Self.recordingFileName(device: title, at: Clock.nowMs)).path
        Task {
            defer { recordBusy = false }
            do {
                try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
                try await channel.startRecording(path: path, withAudio: withAudio)
                // 等待期间标签关了或画面换了：旧画面停掉时内核已自动结束，这里不再记
                if closed || self.channel !== channel {
                    _ = try? channel.stopRecording()
                    return
                }
                recording = Recording(channel: channel, path: path, startedMs: Clock.nowMs, withAudio: withAudio)
            } catch {
                if !closed { app?.showToast(L("remote.record.failed", AppModel.describe(error)), error: true) }
            }
        }
    }

    /// 停止录像并提示保存位置。画面已被内核关掉（断线、切屏）时内核已自动结束录像，stop_recording 会报「没在录」，按已知路径提示
    func stopRecording() {
        guard let rec = recording else { return }
        recording = nil
        var paths = [rec.path]
        var durationMs = UInt64(max(0, Clock.nowMs - rec.startedMs))
        var failure: String?
        do {
            let info = try rec.channel.stopRecording()
            if !info.paths.isEmpty { paths = info.paths }
            durationMs = info.durationMs
        } catch {
            failure = AppModel.describe(error)
        }
        let fm = FileManager.default
        if failure != nil, !paths.contains(where: { fm.fileExists(atPath: $0) }) {
            app?.showToast(L("remote.record.failed", failure ?? ""), error: true)
            return
        }
        let duration = Self.clock(Int64(durationMs))
        let text = paths.count > 1 ? L("remote.record.savedMany", duration, paths.count)
            : L("remote.record.saved", duration, (paths[0] as NSString).lastPathComponent)
        app?.showToast(text, action: ToastAction(title: L("remote.record.reveal")) { Self.reveal(paths) })
    }

    /// 在访达里选中录像文件；文件不在（例如模拟内核）时打开所在文件夹
    static func reveal(_ paths: [String]) {
        let urls = paths.filter { FileManager.default.fileExists(atPath: $0) }.map { URL(fileURLWithPath: $0) }
        if urls.isEmpty {
            let dir = paths.first.map { URL(fileURLWithPath: $0).deletingLastPathComponent() } ?? recordingDir
            NSWorkspace.shared.open(dir)
        } else {
            NSWorkspace.shared.activateFileViewerSelecting(urls)
        }
    }

    /// 毫秒 → 00:12 / 1:02:03
    nonisolated static func clock(_ ms: Int64) -> String {
        let t = max(0, ms / 1000)
        let h = t / 3600, m = t / 60 % 60, sec = t % 60
        return h > 0 ? String(format: "%d:%02d:%02d", h, m, sec) : String(format: "%02d:%02d", m, sec)
    }

    // MARK: 剪贴板同步：本机变化推给远端；鼠标回到画面时拉取远端。文件走内核的复制粘贴文件（protocol.md 8.4.1）：
    // 复制时只交换清单，粘贴时才传；本机粘贴的做法见 Platform/LazyFilePaste.swift
    private func setupClipboardSync() {
        clipboardTimer?.invalidate()
        clipboardTimer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { _ = self?.pollClipboard() }
        }
    }

    /// 对方开放了剪贴板与文件权限才能复制粘贴文件
    var fileClipSupported: Bool {
        guard let info = peer?.info else { return false }
        return info.permissions.clipboard && info.permissions.files
            && info.capabilities.contains("clipboard") && info.capabilities.contains("files")
    }

    /// 被控端支持粘贴时才传（能力 clipboard_offer）：本机复制文件时只发清单，对方粘贴时才传，进度在对方那边
    var clipOfferSupported: Bool {
        fileClipSupported && peer?.info.capabilities.contains("clipboard_offer") == true
    }

    /// 本机剪贴板变了（按 changeCount 判断，同一次复制只处理一次）：文件推给对方，文字直接设过去。
    /// 返回同步对方剪贴板的任务（文字设过去、文件只发清单；粘贴键据此等它完成），没有要同步的返回 nil
    @discardableResult
    func pollClipboard() -> Task<Void, Never>? {
        guard clipboardSync, let peer, phase == .streaming else { return nil }
        let pb = ClipboardFiles.pasteboard
        guard pb.changeCount != lastPasteboardChange else { return nil }
        lastPasteboardChange = pb.changeCount
        // 本应用放进去的标记文件（对方复制的文件等着在本机粘贴，可能是别的标签放的）：不是本机复制的，不推回去
        if LazyPaste.holdsMarker(pb) { return nil }
        // 本机又复制了东西：还在传的文件作废（推送被新内容取代，拉取完成后也不该再盖掉本机剪贴板）
        if fileClip != nil { cancelFileClip() }
        // 访达里复制文件时剪贴板同时带文件名文字，先按文件处理
        let urls = ClipboardFiles.fileURLs(on: pb)
        if !urls.isEmpty {
            pasteboardChangeAtSync = pb.changeCount
            return pushFiles(urls.map(\.path))
        }
        guard let text = pb.string(forType: .string) else { return nil }
        lastRemoteFiles = nil
        lastRemoteText = text
        pasteboardChangeAtSync = pb.changeCount
        let session = peer.session
        return Task { try? await session.clipboardSet(data: .text(text: text)) }
    }

    /// 鼠标回到画面时拉取对方剪贴板。只在「对方上次同步后又复制了新东西，且本机这段时间没再复制」时才放进本机：
    /// 本机后来复制的东西（截图工具放的图片等，图片不同步给对方）不能被对方的旧文字盖掉；两边都变了以本机为准
    /// （鼠标在画面外时复制的总比进画面前在对方那边复制的晚）
    func pullRemoteClipboard() {
        guard clipboardSync, let peer else { return }
        Task {
            guard let data = try? await peer.session.clipboardGet() else { return }
            let pb = ClipboardFiles.pasteboard
            let localChanged = pb.changeCount != pasteboardChangeAtSync
            switch data {
            case .text(let text):
                lastRemoteFiles = nil
                let remoteChanged = text != lastRemoteText
                lastRemoteText = text
                if remoteChanged, !localChanged, pb.string(forType: .string) != text {
                    pb.clearContents()
                    pb.setString(text, forType: .string)
                    lastPasteboardChange = pb.changeCount
                }
                pasteboardChangeAtSync = pb.changeCount
            case .files(let paths):
                if localChanged {
                    // 本机更新：这批对方文件当作已看过，不放进来
                    lastRemoteFiles = paths
                    pasteboardChangeAtSync = pb.changeCount
                    return
                }
                pullFiles(paths)
            case .image:
                lastRemoteFiles = nil
            }
        }
    }

    /// 本机复制了文件 / 文件夹：被控端支持时只发清单（对方粘贴时才传，进度在对方粘贴的那边，这里不显示进度条），返回发清单的任务；
    /// 老被控端整批传到对方暂存区，完成后对方剪贴板就是它们
    @discardableResult
    func pushFiles(_ paths: [String]) -> Task<Void, Never>? {
        guard let peer, !paths.isEmpty else { return nil }
        cancelFileClip()
        let names = paths.map { ($0 as NSString).lastPathComponent }
        guard fileClipSupported else {
            showFileClipResult(RemoteFileClip(direction: .push, names: names, state: .failed(L("remote.files.noPermission"))))
            return nil
        }
        if clipOfferSupported {
            // 对方剪贴板随之换成这份清单（对方读剪贴板时不返回它），之前记下的对方文件作废
            lastRemoteFiles = nil
            let gen = clipGen
            let session = peer.session
            return Task {
                do {
                    _ = try await session.clipboardOfferFiles(localPaths: paths)
                } catch {
                    guard gen == clipGen else { return }
                    showFileClipResult(RemoteFileClip(direction: .push, names: names, state: .failed(AppModel.describe(error))))
                }
            }
        }
        let gen = clipGen
        let listener = makeClipListener(gen)
        fileClip = RemoteFileClip(direction: .push, names: names)
        clipListener = listener
        clipTransfer = peer.session.clipboardPushFiles(localPaths: paths, listener: listener)
        return nil
    }

    /// 对方剪贴板里的文件：只展开成清单（不下载），放进本机剪贴板；在访达里粘贴时才按项读，进度窗口在本机（LazyFilePaste）。
    /// 同一列表只放一次
    func pullFiles(_ remotePaths: [String]) {
        guard let peer, !remotePaths.isEmpty, !ClipboardFiles.sameFiles(remotePaths, lastRemoteFiles) else { return }
        // 正在把本机文件整批推给对方（老被控端）：对方剪贴板马上会被换掉，这次不拉
        if let c = fileClip, c.state == .running, c.direction == .push { return }
        lastRemoteFiles = remotePaths
        let names = remotePaths.map(ClipboardFiles.baseName)
        guard fileClipSupported else {
            showFileClipResult(RemoteFileClip(direction: .pull, names: names, state: .failed(L("remote.files.noPermission"))))
            return
        }
        cancelFileClip()
        let gen = clipGen
        let change = lastPasteboardChange
        let session = peer.session
        Task {
            do {
                let offer = try await session.clipboardRemoteOffer(remotePaths: remotePaths)
                let pb = ClipboardFiles.pasteboard
                // 展开清单期间本机又复制了别的、或关掉了同步：不盖掉本机剪贴板
                guard gen == clipGen, !closed, pb.changeCount == change else {
                    if lastRemoteFiles == remotePaths { lastRemoteFiles = nil }
                    return
                }
                let info = offer.info()
                if let error = LazyPaste.shared.offer(info, open: { index, offset in try offer.open(index: index, offset: offset) }) {
                    showFileClipResult(RemoteFileClip(direction: .pull, names: names, state: .failed(error)))
                    return
                }
                remoteOfferId = info.id
                lastPasteboardChange = pb.changeCount
                pasteboardChangeAtSync = pb.changeCount
            } catch {
                guard gen == clipGen else { return }
                // 下次鼠标进来再试
                lastRemoteFiles = nil
                showFileClipResult(RemoteFileClip(direction: .pull, names: names, state: .failed(AppModel.describe(error))))
            }
        }
    }

    /// 取消正在传的文件并收起进度条（取消时内核报的「已取消」随之丢弃）
    func cancelFileClip() {
        clipGen += 1
        clipTransfer?.cancel()
        clipTransfer = nil
        clipListener = nil
        fileClip = nil
    }

    /// 收起完成或失败的提示
    func dismissFileClip() {
        if fileClip?.state != .running { fileClip = nil }
    }

    private func makeClipListener(_ gen: Int) -> ClipTransferListener {
        ClipTransferListener(progress: { [weak self] done, total in
            guard let self, gen == self.clipGen, var c = self.fileClip, c.state == .running else { return }
            c.done = done
            c.total = total
            self.fileClip = c
        }, finished: { [weak self] error in
            self?.fileClipFinished(gen, error: error)
        })
    }

    private func fileClipFinished(_ gen: Int, error: String?) {
        guard gen == clipGen, var c = fileClip, c.state == .running else { return }
        let transfer = clipTransfer
        clipTransfer = nil
        clipListener = nil
        if let error {
            c.state = .failed(error)
            showFileClipResult(c)
            return
        }
        // 只有整批推送（老被控端）走这里：对方剪贴板现在是暂存区里的这些文件，之后拉取看到同一列表不再放回本机
        lastRemoteFiles = transfer?.paths() ?? []
        c.state = .done
        if c.total > 0 { c.done = c.total }
        showFileClipResult(c)
    }

    /// 显示完成或失败提示，几秒后自动收起（selftest 不收，便于截图）
    private func showFileClipResult(_ c: RemoteFileClip) {
        fileClip = c
        guard app?.selfTest != true else { return }
        let id = c.id
        let seconds: Double = c.state == .done ? 4 : 8
        DispatchQueue.main.asyncAfter(deadline: .now() + seconds) { [weak self] in
            MainActor.assumeIsolated {
                if self?.fileClip?.id == id { self?.fileClip = nil }
            }
        }
    }

    override func close() {
        closed = true
        reconnectTask?.cancel()
        reconnectTask = nil
        // 关标签时录像随之结束并提示保存位置（要在 channel.stop 之前，才拿得到内核给的文件信息）
        if recording != nil { stopRecording() }
        stopAudio()
        // 关掉远程桌面标签时顺手关隐私屏（同一会话可能还被终端、文件标签用着，被控端不会自动关）
        if privacyOn, let peer, peer.closedReason == nil {
            let session = peer.session
            Task { try? await session.setPrivacy(on: false) }
            privacyOn = false
        }
        // 同理删掉本标签加的虚拟屏（会话随后断开的话被控端也会删）
        if !myVirtual.isEmpty, let peer, peer.closedReason == nil {
            let session = peer.session
            let ids = Array(myVirtual.keys)
            Task { for id in ids { try? await session.removeVirtualDisplay(displayId: id) } }
            myVirtual = [:]
        }
        peerWatch = nil
        channel?.stop()
        channel = nil
        decoder.invalidate()
        statsTimer?.invalidate()
        clipboardTimer?.invalidate()
        cancelFileClip()
        // 本标签放进本机剪贴板、还没粘贴的对方文件：撤回（会话可能随之断开，之后粘贴也读不到）；正在拷贝的不受影响
        if let id = remoteOfferId { LazyPaste.shared.withdraw(offerId: id) }
    }

    func disconnect() {
        app?.closeTab(id)
    }
}

final class VideoSinkAdapter: VideoSink, @unchecked Sendable {
    private weak var model: RemoteDesktopModel?
    init(model: RemoteDesktopModel) { self.model = model }
    func onConfig(codec: VideoCodec, width: UInt32, height: UInt32) {
        DispatchQueue.main.async { [weak self] in MainActor.assumeIsolated { self?.model?.configured(codec: codec, width: width, height: height) } }
    }
    func onFrame(data: Data, ptsUs: UInt64, keyframe: Bool) {
        model?.frameArrived(data, pts: ptsUs, keyframe: keyframe)
    }
    func onClosed(reason: String) {
        // 切屏后旧画面迟到的结束通知不算（例如删掉正在看的虚拟屏前已切到别的屏）
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated {
                guard let self, let m = self.model, m.sinkAdapter === self else { return }
                m.displayClosed(reason)
            }
        }
    }
    func onCursor(x: Int32, y: Int32, visible: Bool) {
        DispatchQueue.main.async { [weak self] in MainActor.assumeIsolated { self?.model?.cursorMoved(x: x, y: y, visible: visible) } }
    }
}

// MARK: - 画面视图（AppKit）：显示 + 输入

final class RemoteVideoNSView: NSView, NSTextInputClient {
    weak var model: RemoteDesktopModel?
    private let videoLayer = CALayer()
    private var tracking: NSTrackingArea?
    private var pressedModifiers: Set<UInt16> = []
    /// 已按物理键发出「按下」的键（Mac 键码）：抬起只发给这些键，输入法吃掉的键不发不成对的抬起
    private var keysDown: Set<UInt16> = []
    /// 发过「按下」还没抬起的鼠标键（及按下时的位置）：失去焦点时一并抬起
    private var buttonsDown: [PointerButton: (Int32, Int32)] = [:]

    /// 修饰键在 NSEvent 设备相关标志位里的位置（左右分开）：远端的修饰键状态照这个算，不靠按下 / 松开次数翻转
    private static let modifierBits: [(code: Int, mask: UInt)] = [
        (kVK_Control, 0x0001), (kVK_Shift, 0x0002), (kVK_RightShift, 0x0004), (kVK_Command, 0x0008),
        (kVK_RightCommand, 0x0010), (kVK_Option, 0x0020), (kVK_RightOption, 0x0040), (kVK_RightControl, 0x2000),
    ]

    /// 本机此刻实际按着的修饰键（Mac 键码）
    static func actualModifiers(_ flags: NSEvent.ModifierFlags) -> Set<UInt16> {
        var s = Set<UInt16>()
        for (code, mask) in modifierBits where flags.rawValue & mask != 0 { s.insert(UInt16(code)) }
        // 有的键盘、合成事件不带设备相关位：设备无关位说按着而左右都没有时算左边那个；说没按着时左右都去掉
        let pairs: [(NSEvent.ModifierFlags, Int, Int)] = [(.control, kVK_Control, kVK_RightControl), (.shift, kVK_Shift, kVK_RightShift),
                                                          (.option, kVK_Option, kVK_RightOption), (.command, kVK_Command, kVK_RightCommand)]
        for (flag, left, right) in pairs {
            let l = UInt16(left), r = UInt16(right)
            if !flags.contains(flag) {
                s.remove(l)
                s.remove(r)
            } else if !s.contains(l) && !s.contains(r) {
                s.insert(l)
            }
        }
        return s
    }

    /// 把远端的修饰键对齐本机实际状态（多的抬起、少的按下）。键盘与鼠标事件发出前都调一次：漏了某次 flagsChanged
    /// （窗口不在前台时按下、点进窗口后才松开等）也能马上纠正，远端不会一直卡着 Ctrl / Shift / Option。
    private func syncModifiers(_ flags: NSEvent.ModifierFlags) {
        let want = Self.actualModifiers(flags)
        guard want != pressedModifiers else { return }
        for c in pressedModifiers.subtracting(want) {
            if let h = mapped(c) { model?.send(.key(hidUsage: h, down: false)) }
        }
        for c in want.subtracting(pressedModifiers) {
            if let h = mapped(c) { model?.send(.key(hidUsage: h, down: true)) }
        }
        pressedModifiers = want
    }
    /// 正交给输入法处理的按键；输入法没要（退格、回车、方向键……）时在 doCommand 里按物理键发出
    private var imeEvent: NSEvent?
    private var imeEventSent = false
    private var marked = NSAttributedString()
    /// selftest 下把帧转成 CGImage 放进图层（离屏截图抓不到 IOSurface 内容）
    static var snapshotMode = false
    private(set) var lastImage: CGImage?

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = NSColor.black.cgColor
        videoLayer.contentsGravity = .resize
        videoLayer.magnificationFilter = .linear
        videoLayer.minificationFilter = .trilinear
        layer?.addSublayer(videoLayer)
    }

    required init?(coder: NSCoder) { fatalError() }

    override var acceptsFirstResponder: Bool { true }
    override var isFlipped: Bool { true }

    func display(_ pb: CVPixelBuffer) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        if Self.snapshotMode {
            var img: CGImage?
            VTCreateCGImageFromCVPixelBuffer(pb, options: nil, imageOut: &img)
            lastImage = img
            videoLayer.contents = img
        } else if let surface = CVPixelBufferGetIOSurface(pb)?.takeUnretainedValue() {
            videoLayer.contents = surface
        }
        CATransaction.commit()
        if videoLayer.frame.isEmpty { needsLayout = true }
    }

    /// 画面在视图里的位置（等比缩放或原始大小居中）
    var contentRect: CGRect {
        guard let m = model, m.streamSize.width > 0 else { return bounds }
        let disp = m.display
        let native = CGSize(width: CGFloat(disp?.width ?? UInt32(m.streamSize.width)) / CGFloat(disp?.scale ?? 1),
                            height: CGFloat(disp?.height ?? UInt32(m.streamSize.height)) / CGFloat(disp?.scale ?? 1))
        let aspect = m.streamSize.width / m.streamSize.height
        var size: CGSize
        if m.scaleMode == .original {
            size = native
        } else {
            size = CGSize(width: bounds.width, height: bounds.width / aspect)
            if size.height > bounds.height { size = CGSize(width: bounds.height * aspect, height: bounds.height) }
        }
        return CGRect(x: (bounds.width - size.width) / 2, y: (bounds.height - size.height) / 2, width: size.width, height: size.height).integral
    }

    override func layout() {
        super.layout()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        videoLayer.frame = contentRect
        CATransaction.commit()
    }

    override func updateTrackingAreas() {
        if let t = tracking { removeTrackingArea(t) }
        let t = NSTrackingArea(rect: bounds, options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self)
        addTrackingArea(t)
        tracking = t
        super.updateTrackingAreas()
    }

    // 视图坐标 → 被控屏幕像素
    private func remotePoint(_ event: NSEvent) -> (Int32, Int32)? {
        guard let m = model, let d = m.display else { return nil }
        let p = convert(event.locationInWindow, from: nil)
        let r = contentRect
        guard r.width > 0, r.height > 0 else { return nil }
        let nx = min(max((p.x - r.minX) / r.width, 0), 1)
        let ny = min(max((p.y - r.minY) / r.height, 0), 1)
        return (Int32(nx * CGFloat(d.width - 1)), Int32(ny * CGFloat(d.height - 1)))
    }

    override func mouseEntered(with event: NSEvent) { model?.pullRemoteClipboard() }
    override func mouseMoved(with event: NSEvent) {
        syncModifiers(event.modifierFlags)
        if let (x, y) = remotePoint(event) { model?.send(.mouseMove(x: x, y: y)) }
    }
    override func mouseDragged(with event: NSEvent) { mouseMoved(with: event) }
    override func rightMouseDragged(with event: NSEvent) { mouseMoved(with: event) }
    override func otherMouseDragged(with event: NSEvent) { mouseMoved(with: event) }

    private func button(_ event: NSEvent, _ b: PointerButton, down: Bool) {
        if down { window?.makeFirstResponder(self) }
        // 先把远端的修饰键对齐本机实际状态，不然远端卡着的 Ctrl / Shift 会让这一下变成 Ctrl+点击
        syncModifiers(event.modifierFlags)
        guard let (x, y) = remotePoint(event) else { return }
        if down { buttonsDown[b] = (x, y) } else { buttonsDown[b] = nil }
        model?.send(.mouseButton(button: b, down: down, x: x, y: y))
    }

    override func mouseDown(with event: NSEvent) { button(event, .left, down: true) }
    override func mouseUp(with event: NSEvent) { button(event, .left, down: false) }
    override func rightMouseDown(with event: NSEvent) { button(event, .right, down: true) }
    override func rightMouseUp(with event: NSEvent) { button(event, .right, down: false) }
    override func otherMouseDown(with event: NSEvent) { button(event, otherButton(event), down: true) }
    override func otherMouseUp(with event: NSEvent) { button(event, otherButton(event), down: false) }
    private func otherButton(_ e: NSEvent) -> PointerButton { e.buttonNumber == 3 ? .back : e.buttonNumber == 4 ? .forward : .middle }

    override func scrollWheel(with event: NSEvent) {
        // 像素滚动量；触控板的精确滚动直接用，滚轮按行换算
        let k: CGFloat = event.hasPreciseScrollingDeltas ? 1 : 40
        let dx = Int32(event.scrollingDeltaX * k), dy = Int32(event.scrollingDeltaY * k)
        syncModifiers(event.modifierFlags)
        if dx != 0 || dy != 0 { model?.send(.wheel(dx: dx, dy: dy)) }
    }

    // MARK: 键盘：物理键发 HID 码；输入法提交的文字发 Text

    private var usingInputMethod: Bool {
        guard let src = TISCopyCurrentKeyboardInputSource()?.takeRetainedValue(),
              let ptr = TISGetInputSourceProperty(src, kTISPropertyInputSourceType) else { return false }
        let type = Unmanaged<CFString>.fromOpaque(ptr).takeUnretainedValue() as String
        return type == (kTISTypeKeyboardInputMode as String)
    }

    private func mapped(_ keyCode: UInt16) -> UInt32? {
        guard var hid = HIDKeyMap.hid(forMacKeyCode: keyCode) else { return nil }
        // 控制 Windows / Linux 时，Command 当 Ctrl 用（Cmd+C → Ctrl+C），符合 Mac 用户习惯
        if model?.commandAsCtrl == true, model?.platform != .macos {
            if hid == HIDKeyMap.leftGUI { hid = HIDKeyMap.leftControl }
            if hid == HIDKeyMap.rightGUI { hid = 0xE4 }
        }
        return hid
    }

    private func sendKey(_ event: NSEvent, down: Bool) {
        guard let hid = mapped(event.keyCode) else { return }
        // 按着 Command 的组合键：AppKit 不把这个键的 keyUp 交给我们（macOS 一直如此），按下后不立刻抬起，
        // 远端就一直当这个键按着——Cmd+A 全选后 A 卡住，GNOME Wayland 上每进一个输入框就自动重复输入一串 a。
        // 所以这类键按下即抬起（按住时系统发的重复按下也各算一次）。
        let tap = down && event.modifierFlags.contains(.command)
        if down && !tap { keysDown.insert(event.keyCode) } else { keysDown.remove(event.keyCode) }
        if down && isPasteChord(event) {
            model?.sendPasteKey(.key(hidUsage: hid, down: true))
        } else {
            model?.send(.key(hidUsage: hid, down: down))
        }
        if tap { model?.send(.key(hidUsage: hid, down: false)) }
    }

    /// 把我们发过「按下」还没抬起的键与修饰键全部抬起：失去焦点、窗口不再是前台时（之后的 keyUp / flagsChanged
    /// 会交给别的应用，不抬起的话远端一直当它们按着）
    func releaseAllKeys() {
        for code in keysDown {
            if let hid = mapped(code) { model?.send(.key(hidUsage: hid, down: false)) }
        }
        keysDown.removeAll()
        for code in pressedModifiers {
            if let hid = mapped(code) { model?.send(.key(hidUsage: hid, down: false)) }
        }
        pressedModifiers.removeAll()
        for (b, p) in buttonsDown {
            model?.send(.mouseButton(button: b, down: false, x: p.0, y: p.1))
        }
        buttonsDown.removeAll()
    }

    override func resignFirstResponder() -> Bool {
        releaseAllKeys()
        return super.resignFirstResponder()
    }

    override func viewWillMove(toWindow newWindow: NSWindow?) {
        if let w = window { NotificationCenter.default.removeObserver(self, name: NSWindow.didResignKeyNotification, object: w) }
        if let w = newWindow {
            NotificationCenter.default.addObserver(self, selector: #selector(windowLostKey), name: NSWindow.didResignKeyNotification, object: w)
        }
        super.viewWillMove(toWindow: newWindow)
    }

    @objc private func windowLostKey() { releaseAllKeys() }

    /// 远端会当成粘贴的组合键：Cmd+V（控制 Mac，或控制 Windows / Linux 且 Command 当 Ctrl 用）、控制 Windows / Linux 时的 Ctrl+V
    private func isPasteChord(_ event: NSEvent) -> Bool {
        guard event.keyCode == UInt16(kVK_ANSI_V), let model else { return false }
        let f = event.modifierFlags
        let mac = model.platform == .macos
        return (f.contains(.command) && (mac || model.commandAsCtrl)) || (f.contains(.control) && !mac)
    }

    override func keyDown(with event: NSEvent) {
        syncModifiers(event.modifierFlags)
        if usingInputMethod, event.modifierFlags.intersection([.command, .control]).isEmpty {
            // 输入法在组字时自己处理；没在组字时退格、回车、方向键、Tab、Esc 等会经 doCommand(by:) 交回来，
            // 那时按物理键发给远端（以前 doCommand 是空的，开着中文输入法这些键全被丢掉）
            imeEvent = event
            imeEventSent = false
            let handled = inputContext?.handleEvent(event) ?? false
            // 输入法既没处理也没交回来（如 F1–F12）：照样按物理键发
            if !handled, !imeEventSent, !hasMarkedText() { sendKey(event, down: true) }
            imeEvent = nil
            return
        }
        sendKey(event, down: true)
    }

    override func keyUp(with event: NSEvent) {
        // 只给发过「按下」的键发抬起：输入法吃掉的键（组字、提交成文字的）两头都不发
        if keysDown.contains(event.keyCode) { sendKey(event, down: false) }
        syncModifiers(event.modifierFlags)
    }

    /// 输入法交回来的命令：按当前这个物理键发出去，抬起在 keyUp 里发
    private func imePassthrough() -> Bool {
        guard let e = imeEvent, !imeEventSent, !hasMarkedText() else { return false }
        imeEventSent = true
        sendKey(e, down: true)
        return true
    }

    override func flagsChanged(with event: NSEvent) {
        let code = event.keyCode
        if code == UInt16(kVK_CapsLock) {
            // 大写锁定是开关：macOS 每按一下只来一次 flagsChanged，按「按下 / 抬起」翻转会让远端一直按着它。按一下就发一次按下即抬起
            if let hid = mapped(code) {
                model?.send(.key(hidUsage: hid, down: true))
                model?.send(.key(hidUsage: hid, down: false))
            }
            return
        }
        syncModifiers(event.modifierFlags)
        // 松开 Command 时还记着按下的键（理论上没有，兜底）：一并抬起
        if !event.modifierFlags.contains(.command), code == UInt16(kVK_Command) || code == UInt16(kVK_RightCommand), !keysDown.isEmpty {
            for c in keysDown { if let h = mapped(c) { model?.send(.key(hidUsage: h, down: false)) } }
            keysDown.removeAll()
        }
    }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        // 带 Command 的组合键也交给远端（否则被菜单吃掉），Cmd+Q / Cmd+W 等本机关键操作除外
        guard window?.firstResponder === self else { return false }
        if event.modifierFlags.contains(.command), let c = event.charactersIgnoringModifiers, ["q", "w", "`"].contains(c) { return false }
        keyDown(with: event)
        return true
    }

    // MARK: NSTextInputClient（只为接住输入法提交的文字）

    func insertText(_ string: Any, replacementRange: NSRange) {
        let s = (string as? NSAttributedString)?.string ?? (string as? String) ?? ""
        let wasComposing = hasMarkedText()
        marked = NSAttributedString()
        // 没在组字、插入的是功能键字符（U+F700–U+F8FF，如 F5、Home）：按物理键发，不当文字
        if !wasComposing, !s.isEmpty, s.unicodeScalars.allSatisfy({ (0xF700...0xF8FF).contains($0.value) }), imePassthrough() { return }
        if !s.isEmpty { model?.send(.text(text: s)) }
    }
    func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        marked = (string as? NSAttributedString) ?? NSAttributedString(string: (string as? String) ?? "")
    }
    func unmarkText() { marked = NSAttributedString() }
    func selectedRange() -> NSRange { NSRange(location: NSNotFound, length: 0) }
    func markedRange() -> NSRange { marked.length > 0 ? NSRange(location: 0, length: marked.length) : NSRange(location: NSNotFound, length: 0) }
    func hasMarkedText() -> Bool { marked.length > 0 }
    func attributedSubstring(forProposedRange range: NSRange, actualRange: NSRangePointer?) -> NSAttributedString? { nil }
    func validAttributesForMarkedText() -> [NSAttributedString.Key] { [] }
    func firstRect(forCharacterRange range: NSRange, actualRange: NSRangePointer?) -> NSRect {
        // 候选窗出现在鼠标附近
        guard let w = window else { return .zero }
        let p = w.mouseLocationOutsideOfEventStream
        return w.convertToScreen(NSRect(x: p.x, y: p.y - 20, width: 1, height: 20))
    }
    func characterIndex(for point: NSPoint) -> Int { NSNotFound }
    override func doCommand(by selector: Selector) {
        _ = imePassthrough()
    }
}

struct RemoteVideoHost: NSViewRepresentable {
    let view: RemoteVideoNSView
    func makeNSView(context: Context) -> NSView {
        let c = NSView()
        c.wantsLayer = true
        c.layer?.backgroundColor = NSColor.black.cgColor
        attach(c)
        return c
    }
    func updateNSView(_ nsView: NSView, context: Context) {
        if view.superview !== nsView { attach(nsView) }
        view.needsLayout = true
    }
    private func attach(_ c: NSView) {
        view.removeFromSuperview()
        view.translatesAutoresizingMaskIntoConstraints = false
        c.addSubview(view)
        NSLayoutConstraint.activate([
            view.leadingAnchor.constraint(equalTo: c.leadingAnchor), view.trailingAnchor.constraint(equalTo: c.trailingAnchor),
            view.topAnchor.constraint(equalTo: c.topAnchor), view.bottomAnchor.constraint(equalTo: c.bottomAnchor),
        ])
    }
}

// MARK: - SwiftUI：工具栏 + 画面

struct RemoteDesktopView: View {
    @ObservedObject var tab: RemoteDesktopModel
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(spacing: 0) {
            if !tab.focusMode { RemoteToolbar(tab: tab) }
            ZStack {
                Color.black
                if tab.scaleMode == .original, let d = tab.display {
                    ScrollView([.horizontal, .vertical]) {
                        RemoteVideoHost(view: tab.videoView)
                            .frame(width: CGFloat(d.width) / CGFloat(d.scale), height: CGFloat(d.height) / CGFloat(d.scale))
                    }
                } else {
                    RemoteVideoHost(view: tab.videoView)
                }
                switch tab.phase {
                case .connecting:
                    VStack(spacing: 10) {
                        ProgressView().controlSize(.regular).tint(.white)
                        Text(loc("remote.connecting")).font(.system(size: 12.5)).foregroundStyle(.white.opacity(0.75))
                    }
                case .reconnecting:
                    ZStack {
                        Color.black.opacity(0.55)
                        VStack(spacing: 10) {
                            ProgressView().controlSize(.regular).tint(.white)
                            Text(loc("remote.reconnecting")).font(.system(size: 12.5)).foregroundStyle(.white.opacity(0.85))
                        }
                    }
                case .failed(let e):
                    VStack(spacing: 12) {
                        Image(systemName: "display.trianglebadge.exclamationmark").font(.system(size: 30, weight: .light)).foregroundStyle(.white.opacity(0.7))
                        Text(e).font(.system(size: 13)).foregroundStyle(.white.opacity(0.85)).multilineTextAlignment(.center)
                        Button(loc("common.retry")) { tab.start() }.buttonStyle(PrimaryButtonStyle())
                    }
                    .padding(30)
                case .streaming:
                    EmptyView()
                }
                if let clip = tab.fileClip {
                    VStack {
                        // 外层限宽只约束提议宽度，进度条本身按内容取宽
                        RemoteFileClipPill(tab: tab, clip: clip).frame(maxWidth: 560).padding(.top, 12).padding(.horizontal, 16)
                        Spacer()
                    }
                }
                if tab.focusMode {
                    VStack {
                        HStack {
                            Spacer()
                            Button { tab.focusMode = false; NSApp.keyWindow?.toggleFullScreen(nil) } label: {
                                Label(loc("remote.exitFullscreen"), systemImage: "arrow.down.right.and.arrow.up.left")
                                    .font(.system(size: 12, weight: .medium))
                                    .padding(.horizontal, 10).padding(.vertical, 6)
                                    .background(Capsule().fill(.black.opacity(0.55)))
                                    .foregroundStyle(.white)
                            }
                            .buttonStyle(.plain)
                            .padding(10)
                        }
                        Spacer()
                    }
                }
            }
        }
    }
}

struct RemoteToolbar: View {
    @ObservedObject var tab: RemoteDesktopModel
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        HStack(spacing: 6) {
            // 多屏切换；我加的虚拟屏右键可删
            if tab.displays.count > 1 {
                HStack(spacing: 2) {
                    ForEach(Array(tab.displays.enumerated()), id: \.element.id) { i, d in
                        let mine = tab.isMyVirtual(d.id)
                        Button { Task { await tab.open(d.id) } } label: {
                            HStack(spacing: 4) {
                                Image(systemName: d.isVirtual ? "rectangle.dashed" : "display").font(.system(size: 10.5))
                                Text(verbatim: "\(i + 1)").font(.system(size: 11.5, weight: tab.displayId == d.id ? .semibold : .regular))
                            }
                            .foregroundStyle(tab.displayId == d.id ? p.accent : p.secondaryText)
                            .padding(.horizontal, 8).frame(height: 24)
                            .background(RoundedRectangle(cornerRadius: 6).fill(tab.displayId == d.id ? p.accent.opacity(0.14) : .clear))
                        }
                        .buttonStyle(.plain)
                        .help(loc("remote.screen", i + 1) + " · \(d.name) · \(d.width)×\(d.height)"
                              + (d.isVirtual ? " · " + loc(mine ? "remote.vd.mine.badge" : "remote.vd.badge") : ""))
                        .contextMenu {
                            if mine {
                                Button(loc("remote.vd.remove")) { Task { await tab.removeVirtualDisplay(d.id) } }
                                    .disabled(tab.virtualBusy)
                            }
                        }
                    }
                }
            }
            if tab.virtualSupported { RemoteVirtualDisplayButton(tab: tab) }
            if tab.displays.count > 1 || tab.virtualSupported { divider }
            Picker("", selection: $tab.scaleMode) {
                Image(systemName: "arrow.up.left.and.arrow.down.right").help(loc("remote.fit")).tag(RemoteDesktopModel.ScaleMode.fit)
                Text("1:1").help(loc("remote.original")).tag(RemoteDesktopModel.ScaleMode.original)
            }
            .pickerStyle(.segmented).labelsHidden().frame(width: 84)
            .onChange(of: tab.scaleMode) { _ in tab.videoView.needsLayout = true }
            qualityMenu
            divider
            IconButton(icon: "keyboard.badge.ellipsis", help: "Ctrl+Alt+Del") { tab.special(.ctrlAltDel) }
            IconButton(icon: "lock", help: loc("remote.lock")) { tab.special(.lockScreen) }
            Menu {
                Button("Alt+Tab") { tab.special(.altTab) }
                Button(loc("remote.combo.taskManager")) { tab.special(.taskManager) }
                Button(loc("remote.combo.winKey")) { tab.special(.winKey) }
                Button(loc("remote.combo.showDesktop")) { tab.special(.showDesktop) }
                if tab.platform != .macos {
                    Divider()
                    Toggle(loc("remote.commandAsCtrl"), isOn: $tab.commandAsCtrl)
                }
            } label: {
                Image(systemName: "command").font(.system(size: 12))
            }
            .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
            .help(loc("remote.keys"))
            IconButton(icon: tab.clipboardSync ? "doc.on.clipboard.fill" : "doc.on.clipboard", help: loc("remote.clipboard"), active: tab.clipboardSync) {
                tab.clipboardSync.toggle()
            }
            InfoButton(text: loc("remote.clipboard.info"))
            if tab.audioSupported { RemoteAudioControl(tab: tab) }
            RemoteRecordButton(tab: tab)
            if tab.privacySupported { privacyToggle }
            Spacer(minLength: 8)
            stats
            if let peer = tab.peer { PathBadge(peer: peer) }
            divider
            IconButton(icon: "arrow.up.left.and.arrow.down.right", help: loc("remote.fullscreen")) {
                tab.focusMode = true
                NSApp.keyWindow?.toggleFullScreen(nil)
            }
            Button { tab.disconnect() } label: {
                Text(loc("remote.disconnect")).font(.system(size: 12, weight: .medium)).foregroundStyle(p.danger)
                    .padding(.horizontal, 10).frame(height: 24)
                    .background(RoundedRectangle(cornerRadius: 6).fill(p.danger.opacity(0.12)))
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 10)
        .frame(height: 38)
        .background(p.content)
        .overlay(Rectangle().fill(p.separator).frame(height: 1), alignment: .bottom)
    }

    private var divider: some View { Rectangle().fill(p.separator).frame(width: 1, height: 18).padding(.horizontal, 3) }

    /// 隐私屏开关：开着时醒目显示，提醒对方屏幕现在是黑的
    private var privacyToggle: some View {
        let on = tab.privacyOn
        let canControl = tab.peer?.info.permissions.control ?? false
        return Button { tab.setPrivacy(!on) } label: {
            HStack(spacing: 5) {
                if tab.privacyBusy {
                    ProgressView().controlSize(.mini).frame(width: 12, height: 12)
                } else {
                    Image(systemName: on ? "eye.slash.fill" : "eye.slash").font(.system(size: 11, weight: .medium))
                }
                Text(loc("remote.privacy")).font(.system(size: 11.5, weight: on ? .semibold : .regular))
            }
            .foregroundStyle(on ? Color.white : (canControl ? p.secondaryText : p.tertiaryText))
            .padding(.horizontal, 8).frame(height: 24)
            .background(RoundedRectangle(cornerRadius: 6).fill(on ? p.warning : .clear))
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(on ? .clear : p.border, lineWidth: 0.7))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!canControl || tab.privacyBusy)
        .help(loc(canControl ? "remote.privacy.help" : "remote.privacy.noControl"))
    }

    private var qualityMenu: some View {
        Menu {
            Picker(loc("remote.quality"), selection: Binding(get: { tab.quality }, set: { tab.applyQuality($0) })) {
                Text(loc("remote.quality.smooth")).tag("smooth")
                Text(loc("remote.quality.balanced")).tag("balanced")
                Text(loc("remote.quality.sharp")).tag("sharp")
                if tab.quality == "custom" { Text(loc("remote.quality.custom")).tag("custom") }
            }
            .pickerStyle(.inline)
            Picker(loc("remote.fps"), selection: Binding(get: { tab.fps }, set: { tab.setFps($0) })) {
                ForEach([30, 60, 90, 120, 144], id: \.self) { f in Text(verbatim: "\(f) fps").tag(f) }
            }
            Picker(loc("remote.bitrate"), selection: Binding(get: { tab.bitrateKbps }, set: { tab.setBitrate($0) })) {
                ForEach([2_000, 6_000, 12_000, 20_000, 30_000, 50_000], id: \.self) { k in Text(verbatim: "\(k / 1000) Mbps").tag(k) }
                if ![2_000, 6_000, 12_000, 20_000, 30_000, 50_000].contains(tab.bitrateKbps) {
                    Text(verbatim: "\(tab.bitrateKbps / 1000) Mbps").tag(tab.bitrateKbps)
                }
            }
        } label: {
            HStack(spacing: 4) {
                Image(systemName: "slider.horizontal.3").font(.system(size: 11))
                Text(loc("remote.quality." + tab.quality)).font(.system(size: 11.5))
            }
        }
        .menuStyle(.borderlessButton).fixedSize()
    }

    private var stats: some View {
        let detail = tab.streamSize.width > 0 ? "\(Int(tab.streamSize.width))×\(Int(tab.streamSize.height)) · " + (tab.codec == .h265 ? "H.265" : "H.264") : ""
        return Text(verbatim: "\(tab.measuredFps) fps · " + String(format: "%.1f Mbps", Double(tab.measuredKbps) / 1000))
            .font(.system(size: 11).monospacedDigit())
            .foregroundStyle(p.tertiaryText)
            .lineLimit(1)
            .fixedSize()
            .help(detail)
    }
}

/// 声音开关（与工具栏图标按钮同样式）+ 右侧小箭头弹出音量
struct RemoteAudioControl: View {
    @ObservedObject var tab: RemoteDesktopModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var hover = false
    @State private var showVolume = false

    var body: some View {
        let on = tab.audioOn
        HStack(spacing: 0) {
            Button { tab.setAudio(!on) } label: {
                Group {
                    if tab.audioBusy {
                        ProgressView().controlSize(.mini).frame(width: 12, height: 12)
                    } else if on {
                        // 波纹随音量亮起
                        Image(systemName: "speaker.wave.3.fill", variableValue: tab.audioVolume).font(.system(size: 12.5, weight: .medium))
                    } else {
                        Image(systemName: "speaker.slash").font(.system(size: 12.5, weight: .medium))
                    }
                }
                .frame(width: 26, height: 24)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(tab.audioBusy)
            .help(loc(on ? "remote.audio.turnOff" : "remote.audio.turnOn"))
            Button { showVolume.toggle() } label: {
                Image(systemName: "chevron.down").font(.system(size: 7.5, weight: .bold))
                    .frame(width: 12, height: 24)
                    .padding(.trailing, 2)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(loc("remote.audio.volume"))
            .popover(isPresented: $showVolume, arrowEdge: .bottom) {
                RemoteAudioVolumePanel(tab: tab)
            }
        }
        .foregroundStyle(on ? p.accent : p.secondaryText)
        .background(RoundedRectangle(cornerRadius: 6).fill(on ? p.accent.opacity(0.14) : (hover ? p.hover.opacity(2) : .clear)))
        .onHover { hover = $0 }
    }
}

/// 音量弹出层
struct RemoteAudioVolumePanel: View {
    @ObservedObject var tab: RemoteDesktopModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(loc("remote.audio.volume")).font(.system(size: 12, weight: .medium)).foregroundStyle(p.text)
                Spacer()
                Text(verbatim: "\(Int((tab.audioVolume * 100).rounded()))%")
                    .font(.system(size: 11.5).monospacedDigit())
                    .foregroundStyle(p.secondaryText)
            }
            HStack(spacing: 8) {
                Image(systemName: "speaker.fill").font(.system(size: 10)).foregroundStyle(p.tertiaryText)
                Slider(value: Binding(get: { tab.audioVolume }, set: { tab.setVolume($0) }), in: 0...1)
                    .tint(p.accent)
                Image(systemName: "speaker.wave.3.fill").font(.system(size: 10)).foregroundStyle(p.tertiaryText)
            }
        }
        .padding(14)
        .frame(width: 240)
    }
}

/// 录像按钮：没在录时是圆形录制图标；录制中显示红点与计时，点一下停止
struct RemoteRecordButton: View {
    @ObservedObject var tab: RemoteDesktopModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var hover = false

    var body: some View {
        if let rec = tab.recording {
            Button { tab.stopRecording() } label: {
                HStack(spacing: 5) {
                    Circle().fill(p.danger).frame(width: 7, height: 7)
                    TimelineView(.periodic(from: .now, by: 1)) { _ in
                        Text(verbatim: RemoteDesktopModel.clock(Clock.nowMs - rec.startedMs))
                            .font(.system(size: 11.5, weight: .semibold).monospacedDigit())
                    }
                    Image(systemName: "stop.fill").font(.system(size: 8, weight: .bold)).opacity(hover ? 1 : 0.7)
                }
                .foregroundStyle(p.danger)
                .padding(.horizontal, 8).frame(height: 24)
                .background(RoundedRectangle(cornerRadius: 6).fill(p.danger.opacity(hover ? 0.2 : 0.13)))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .onHover { hover = $0 }
            .help(loc("remote.record.stop"))
        } else if tab.recordBusy {
            ProgressView().controlSize(.mini).frame(width: 26, height: 24)
        } else {
            IconButton(icon: "record.circle", help: loc("remote.record.start")) { tab.startRecording() }
                .disabled(!tab.canRecord)
                .opacity(tab.canRecord ? 1 : 0.45)
        }
    }
}
