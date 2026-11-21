import Foundation
import UIKit
import CoreVideo
import AVFoundation
import Combine
import GalaxyKit

/// 远程桌面会话：list_displays → open_display(VideoSink) → VideoToolbox 硬件解码 → 显示；手势与键盘 → InputEvent。
@MainActor
final class RemoteSession: ObservableObject, Identifiable {
    enum Status: Equatable {
        case connecting, streaming
        case failed(String)
    }

    let id = UUID().uuidString
    let deviceId: String
    let title: String
    let platform: Platform
    @Published private(set) var status: Status = .connecting
    @Published private(set) var displays: [DisplayInfo] = []
    @Published private(set) var displayId: UInt32?
    @Published private(set) var quality: VideoQuality
    @Published var mouseMode: Bool { didSet { canvas.mouseMode = mouseMode } }
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
    let canvas = RemoteCanvasView()
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

    init(app: AppModel, deviceId: String, title: String, platform: Platform) {
        self.app = app
        self.deviceId = deviceId
        self.title = title
        self.platform = platform
        quality = app.settings.defaultQuality
        mouseMode = app.settings.remoteMouseMode
        canvas.mouseMode = mouseMode
        canvas.platform = platform
        canvas.send = { [weak self] events in self?.send(events) }
        let display = canvas.displayLayer
        decoder.onFrame = { [weak self] pb, _ in
            DispatchQueue.main.async {
                guard let self else { return }
                RemoteCanvasView.enqueue(pb, on: display)
                self.frames += 1
                if self.status != .streaming { self.status = .streaming }
            }
        }
        decoder.onNeedKeyframe = { [weak self] in
            DispatchQueue.main.async { self?.channel?.requestKeyframe() }
        }
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
        Task { [weak self] in
            guard let self, let app = self.app else { return }
            do {
                let p = try await app.account.peer(deviceId)
                peer = p
                closeWatch = p.$closedReason.compactMap { $0 }.receive(on: DispatchQueue.main).sink { [weak self] reason in
                    self?.status = .failed(reason.isEmpty ? L("remote.closed") : reason)
                    // 会话断了，声音通道也随之失效；录像收尾（内核还留着已写的部分）
                    self?.stopRecording(.dropped)
                    self?.stopAudio()
                }
                displays = try await p.session.listDisplays()
                guard let first = displays.first(where: \.primary) ?? displays.first else {
                    status = .failed(L("remote.noDisplay"))
                    return
                }
                await open(first.id)
            } catch {
                status = .failed(describeError(error))
            }
        }
    }

    private func open(_ id: UInt32) async {
        guard let peer else { return }
        // 关画面时内核会自动结束录像但不返回结果：先自己停，拿到文件
        stopRecording(.reopen)
        channel?.stop()
        channel = nil
        displayId = id
        status = .connecting
        if let d = display { canvas.setRemoteSize(Int(d.width), Int(d.height)) }
        let adapter = VideoSinkAdapter()
        let decoder = self.decoder
        adapter.onConfigure = { [weak self] codec, w, h in
            decoder.configure(codec: codec)
            DispatchQueue.main.async {
                guard let self else { return }
                self.codec = codec
                self.videoSize = CGSize(width: Int(w), height: Int(h))
                // 没拿到屏幕信息时按码流尺寸摆放
                if self.display == nil { self.canvas.setRemoteSize(Int(w), Int(h)) }
            }
        }
        adapter.onData = { data, pts, key in decoder.decode(data, ptsUs: pts, keyframe: key) }
        adapter.onCursorMove = { [weak self] x, y, visible in
            DispatchQueue.main.async { self?.canvas.remoteCursor(x, y, visible: visible) }
        }
        adapter.onClose = { [weak self, weak adapter] reason in
            DispatchQueue.main.async {
                guard let self else { return }
                self.status = .failed(reason.isEmpty ? L("remote.closed") : reason)
                // 只收当前画面的录像（换屏幕后旧通道迟到的关闭不算）
                if let adapter, self.sinkAdapter === adapter { self.stopRecording(.dropped) }
            }
        }
        sinkAdapter = adapter
        do {
            channel = try await peer.session.openDisplay(options: DisplayOptions(displayId: id, maxFps: quality.maxFps, bitrateKbps: quality.bitrateKbps,
                                                                                 codecs: VideoDecoder.supportedCodecs, viewOnly: viewOnly),
                                                         sink: adapter)
            canvas.viewOnly = viewOnly
            startAudioIfWanted()
        } catch {
            status = .failed(describeError(error))
        }
    }

    func switchDisplay(_ id: UInt32) {
        guard id != displayId else { return }
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

    func retry() {
        if let id = displayId, peer?.isAlive == true { Task { await open(id) } } else { start() }
    }

    func send(_ events: [InputEvent]) {
        guard let channel, !viewOnly else { return }
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
    /// 声音在后台放不了（没有后台音频模式），先停掉，回来后按开关重新开始
    func setForeground(_ fg: Bool) {
        guard fg != foreground else { return }
        foreground = fg
        if fg {
            canvas.displayLayer.flush()
            channel?.requestKeyframe()
            if channel != nil { startAudioIfWanted() }
        } else {
            stopRecording(.background)
            stopAudio()
        }
    }

    func stop() {
        stopRecording(.closed)
        stopped = true
        stopAudio()
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
