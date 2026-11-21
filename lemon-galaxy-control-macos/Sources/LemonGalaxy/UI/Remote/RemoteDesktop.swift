import SwiftUI
import AppKit
import CoreVideo
import VideoToolbox
import Carbon.HIToolbox
import Combine

/// 远程桌面标签：list_displays → open_display(VideoSink) → 解码显示；鼠标键盘 → InputEvent。
@MainActor
final class RemoteDesktopModel: TabModel {
    enum Phase: Equatable { case connecting, streaming, failed(String) }
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
    @Published var clipboardSync: Bool
    @Published var commandAsCtrl = true
    @Published var measuredFps = 0
    @Published var measuredKbps = 0
    @Published var remoteCursor: CGPoint?
    @Published var focusMode = false
    /// 隐私屏：被控端本地屏幕黑掉、本地键鼠失效（对端能力里有 "privacy" 才显示开关）
    @Published var privacyOn = false
    @Published var privacyBusy = false
    private var peerWatch: AnyCancellable?
    let videoView = RemoteVideoNSView()
    private var channel: DisplayChannel?
    private var sinkAdapter: VideoSinkAdapter?
    nonisolated(unsafe) private let decoder = VideoDecoder()
    private var frameCount = 0
    private var byteCount = 0
    private var statsTimer: Timer?
    private var clipboardTimer: Timer?
    private var lastPasteboardChange = NSPasteboard.general.changeCount
    var framesDecoded = 0

    init(app: AppModel, deviceId: String, title: String) {
        let s = app.settings
        quality = s.displayQuality
        fps = s.displayFps
        bitrateKbps = s.displayBitrateKbps
        clipboardSync = s.clipboardSync
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
        Task {
            guard let app, let deviceId else { return }
            do {
                let p = try await app.peer(deviceId)
                if peer !== p { watchPeer(p) }
                peer = p
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

    func open(_ id: UInt32) async {
        guard let peer else { return }
        channel?.stop()
        channel = nil
        displayId = id
        phase = .connecting
        let adapter = VideoSinkAdapter(model: self)
        sinkAdapter = adapter
        let codecs: [VideoCodec] = app?.settings.preferH265 == true ? [.h265, .h264] : [.h264]
        do {
            channel = try await peer.session.openDisplay(options: DisplayOptions(displayId: id, maxFps: UInt32(fps), bitrateKbps: UInt32(bitrateKbps), codecs: codecs, viewOnly: !(peer.info.permissions.control)),
                                                         sink: adapter)
            live = true
        } catch {
            phase = .failed(AppModel.describe(error))
            live = false
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
        if phase != .streaming { phase = .streaming }
        videoView.display(pb)
    }

    /// 被控端结束了画面（对方断开、权限收回、屏幕拔掉等）
    func displayClosed(_ reason: String) {
        phase = .failed(reason.isEmpty ? L("remote.closed") : reason)
        live = false
    }

    func cursorMoved(x: Int32, y: Int32, visible: Bool) {
        remoteCursor = visible ? CGPoint(x: Int(x), y: Int(y)) : nil
    }

    func send(_ e: InputEvent) { channel?.sendInput(event: e) }

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

    /// 新的会话：被控端的隐私屏随旧会话结束已自动关闭；会话断开时开关复位
    private func watchPeer(_ p: PeerConnection) {
        privacyOn = false
        peerWatch = p.$closedReason.sink { [weak self] reason in
            if reason != nil { self?.privacyOn = false }
        }
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

    // MARK: 剪贴板同步（文字）：本机变化推给远端；切回窗口时拉取远端
    private func setupClipboardSync() {
        clipboardTimer?.invalidate()
        clipboardTimer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.pollClipboard() }
        }
    }

    private func pollClipboard() {
        guard clipboardSync, let peer, phase == .streaming else { return }
        let pb = NSPasteboard.general
        guard pb.changeCount != lastPasteboardChange else { return }
        lastPasteboardChange = pb.changeCount
        guard let text = pb.string(forType: .string) else { return }
        Task { try? await peer.session.clipboardSet(data: .text(text: text)) }
    }

    func pullRemoteClipboard() {
        guard clipboardSync, let peer else { return }
        Task {
            guard let data = try? await peer.session.clipboardGet(), case .text(let text) = data else { return }
            let pb = NSPasteboard.general
            if pb.string(forType: .string) != text {
                pb.clearContents()
                pb.setString(text, forType: .string)
                lastPasteboardChange = pb.changeCount
            }
        }
    }

    override func close() {
        // 关掉远程桌面标签时顺手关隐私屏（同一会话可能还被终端、文件标签用着，被控端不会自动关）
        if privacyOn, let peer, peer.closedReason == nil {
            let session = peer.session
            Task { try? await session.setPrivacy(on: false) }
            privacyOn = false
        }
        peerWatch = nil
        channel?.stop()
        channel = nil
        decoder.invalidate()
        statsTimer?.invalidate()
        clipboardTimer?.invalidate()
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
        DispatchQueue.main.async { [weak self] in MainActor.assumeIsolated { self?.model?.displayClosed(reason) } }
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
    override func mouseMoved(with event: NSEvent) { if let (x, y) = remotePoint(event) { model?.send(.mouseMove(x: x, y: y)) } }
    override func mouseDragged(with event: NSEvent) { mouseMoved(with: event) }
    override func rightMouseDragged(with event: NSEvent) { mouseMoved(with: event) }
    override func otherMouseDragged(with event: NSEvent) { mouseMoved(with: event) }

    private func button(_ event: NSEvent, _ b: PointerButton, down: Bool) {
        if down { window?.makeFirstResponder(self) }
        guard let (x, y) = remotePoint(event) else { return }
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

    override func keyDown(with event: NSEvent) {
        if usingInputMethod, event.modifierFlags.intersection([.command, .control]).isEmpty {
            inputContext?.handleEvent(event)
            return
        }
        if let hid = mapped(event.keyCode) { model?.send(.key(hidUsage: hid, down: true)) }
    }

    override func keyUp(with event: NSEvent) {
        if usingInputMethod, hasMarkedText() { return }
        if let hid = mapped(event.keyCode) { model?.send(.key(hidUsage: hid, down: false)) }
    }

    override func flagsChanged(with event: NSEvent) {
        let code = event.keyCode
        guard HIDKeyMap.modifierKeyCodes.contains(Int(code)), let hid = mapped(code) else { return }
        let down = !pressedModifiers.contains(code)
        if down { pressedModifiers.insert(code) } else { pressedModifiers.remove(code) }
        model?.send(.key(hidUsage: hid, down: down))
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
        marked = NSAttributedString()
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
    override func doCommand(by selector: Selector) {}
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
            // 多屏切换
            if tab.displays.count > 1 {
                HStack(spacing: 2) {
                    ForEach(Array(tab.displays.enumerated()), id: \.element.id) { i, d in
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
                        .help(loc("remote.screen", i + 1) + " · \(d.name) · \(d.width)×\(d.height)")
                    }
                }
                divider
            }
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
