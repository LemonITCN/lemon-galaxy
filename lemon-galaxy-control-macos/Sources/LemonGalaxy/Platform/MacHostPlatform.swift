import AppKit
import ScreenCaptureKit
import CoreMedia
import CoreVideo
import ApplicationServices
import Carbon.HIToolbox

/// docs/ffi.md 第 5 节 HostPlatform 的 macOS 实现（真实可用，不是模拟）：
///   - 截屏：ScreenCaptureKit → VideoToolbox H.264/H.265 低延迟硬件编码 → Annex B → EncodedFrameSink
///   - 输入：CGEvent 注入鼠标、滚轮、键盘（HID 用法码 → 虚拟键码）、输入法文字（Unicode 字符串事件）
///   - 截图、辅助功能控件树（JSON）、剪贴板（文字 / 图片 / 文件）
///   - 系统声音：ScreenCaptureKit 只采声音（画面部分最小化后丢弃）→ 交错浮点 PCM → AudioPcmSink
///   - 隐私屏：每块屏幕盖一个黑色无边框窗口（从截屏里排除）+ 会话级事件拦截丢掉本地键鼠，只放行自己注入的事件
/// 内核在自己的线程上调用这些方法；需要「屏幕录制」「辅助功能」授权，见 Permissions。
final class MacHostPlatform: HostPlatform, @unchecked Sendable {
    private let lock = NSLock()
    private var captures: [UInt32: ScreenCapture] = [:]
    private var lastDisplay: UInt32 = CGMainDisplayID()
    private let injector = InputInjector()
    private var audio: AudioCapture?
    private let privacy = PrivacyScreen()

    init() {
        DisplayNames.refresh()
        NotificationCenter.default.addObserver(forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main) { [weak self] _ in
            DisplayNames.refresh()
            // 隐私屏开着时屏幕增减或分辨率变化：重新铺满
            MainActor.assumeIsolated { self?.privacy.screensChanged() }
        }
    }

    deinit {
        privacy.turnOff()
        audio?.stop()
    }

    // MARK: 能力（内核据此告诉主控端能用什么）

    func capabilities() -> [String] {
        var caps = ["clipboard"]
        let screen = SystemPermissions.screenRecordingGranted
        let ax = SystemPermissions.accessibilityGranted
        // 系统声音也走 ScreenCaptureKit，只要「屏幕录制」授权
        if screen { caps += ["display", "screenshot", "audio"] }
        if ax { caps += ["input", "ui_tree"] }
        // 隐私屏：拦截本地键鼠要「辅助功能」；没有画面时开隐私屏没有意义
        if screen && ax { caps.append("privacy") }
        return caps
    }

    // MARK: 屏幕

    /// 只用 CoreGraphics（线程安全）：内核在自己的线程上调用，不能同步等主线程，否则主线程正等内核时会互锁
    func listDisplays() -> [DisplayInfo] {
        var count: UInt32 = 0
        CGGetActiveDisplayList(0, nil, &count)
        var ids = [CGDirectDisplayID](repeating: 0, count: Int(count))
        CGGetActiveDisplayList(count, &ids, &count)
        let main = CGMainDisplayID()
        return ids.prefix(Int(count)).map { id in
            let b = CGDisplayBounds(id)
            let scale = displayScale(id)
            let name = DisplayNames.name(id)
            return DisplayInfo(id: id, name: name, width: UInt32(b.width * scale), height: UInt32(b.height * scale),
                               scale: Float(scale), primary: id == main, isVirtual: isVirtual(id, name: name))
        }
    }

    private func isVirtual(_ id: CGDirectDisplayID, name: String) -> Bool {
        // 系统虚拟显示（CGVirtualDisplay / 隔空投送副屏）没有厂商号
        CGDisplayVendorNumber(id) == 0 && CGDisplayIsBuiltin(id) == 0 || name.lowercased().contains("virtual")
    }

    func startCapture(displayId: UInt32, options: CaptureOptions, sink: EncodedFrameSink) -> CaptureStarted? {
        guard SystemPermissions.screenRecordingGranted else { return nil }
        stopCapture(displayId: displayId)
        let cap = ScreenCapture(displayId: displayId, options: options, sink: sink)
        lock.locked { captures[displayId] = cap; lastDisplay = displayId }
        let sem = DispatchSemaphore(value: 0)
        nonisolated(unsafe) var started: CaptureStarted?
        Task.detached {
            started = await cap.start()
            sem.signal()
        }
        sem.wait()
        if started == nil {
            lock.locked { if captures[displayId] === cap { captures[displayId] = nil } }
        } else if privacy.isOn {
            // 启动期间隐私屏刚铺上的黑窗口可能没赶上这次的排除列表，补一次
            Task.detached { await cap.refreshFilter(content: nil) }
        }
        return started
    }

    /// 隐私屏窗口出现后，把它们从所有正在进行的截屏里排除（等到它们出现在可共享内容里，最多约 0.5 秒）
    func refreshCaptureFilters(expecting ids: Set<CGWindowID>) async {
        let caps = lock.locked { Array(captures.values) }
        guard !caps.isEmpty else { return }
        var content: SCShareableContent?
        for _ in 0..<15 {
            content = try? await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
            if let c = content, ids.isSubset(of: Set(c.windows.map(\.windowID))) { break }
            try? await Task.sleep(nanoseconds: 30_000_000)
        }
        for c in caps { await c.refreshFilter(content: content) }
    }

    func stopCapture(displayId: UInt32) {
        let cap = lock.locked { captures.removeValue(forKey: displayId) }
        cap?.stop()
    }

    func requestKeyframe(displayId: UInt32) { lock.locked { captures[displayId] }?.requestKeyframe() }
    func setBitrate(displayId: UInt32, kbps: UInt32) { lock.locked { captures[displayId] }?.setBitrate(Int(kbps)) }

    // MARK: 输入

    func inject(displayId: UInt32, event: InputEvent) {
        // 0 或已经不存在的屏幕：落在最近一次画面所在的屏幕
        let online = displayId != 0 && CGDisplayIsOnline(displayId) != 0
        let display = online ? displayId : lock.locked { lastDisplay }
        injector.inject(event, display: display)
    }

    // MARK: 截图

    func screenshot(displayId: UInt32, maxWidth: UInt32, maxHeight: UInt32) -> ImageData? {
        guard SystemPermissions.screenRecordingGranted else { return nil }
        var image: CGImage?
        // 隐私屏的黑窗口不进截图
        let hidden = privacy.windowIds
        if #available(macOS 14.0, *) {
            let sem = DispatchSemaphore(value: 0)
            Task.detached {
                do {
                    let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
                    if let d = content.displays.first(where: { $0.displayID == displayId }) ?? content.displays.first {
                        let filter = SCContentFilter(display: d, excludingWindows: content.windows.filter { hidden.contains($0.windowID) })
                        let cfg = SCStreamConfiguration()
                        let scale = CGFloat(filter.pointPixelScale)
                        cfg.width = Int(CGFloat(d.width) * scale)
                        cfg.height = Int(CGFloat(d.height) * scale)
                        cfg.showsCursor = true
                        image = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: cfg)
                    }
                } catch {}
                sem.signal()
            }
            sem.wait()
        } else if let cover = privacy.windowId(display: displayId) {
            // macOS 13：隐私屏在最上层，取它下面的所有窗口合成的画面
            image = CGWindowListCreateImage(CGDisplayBounds(displayId), .optionOnScreenBelowWindow, cover, [.bestResolution])
        } else {
            image = CGDisplayCreateImage(displayId)
        }
        guard let src = image else { return nil }
        return Self.encode(src, maxWidth: Int(maxWidth), maxHeight: Int(maxHeight))
    }

    static func encode(_ src: CGImage, maxWidth: Int, maxHeight: Int = 0) -> ImageData? {
        var img = src
        var k = 1.0
        if maxWidth > 0, src.width > maxWidth { k = min(k, Double(maxWidth) / Double(src.width)) }
        if maxHeight > 0, src.height > maxHeight { k = min(k, Double(maxHeight) / Double(src.height)) }
        if k < 1 {
            let w = max(1, Int(Double(src.width) * k)), h = max(1, Int(Double(src.height) * k))
            guard let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                                      space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
            ctx.interpolationQuality = .high
            ctx.draw(src, in: CGRect(x: 0, y: 0, width: w, height: h))
            guard let scaled = ctx.makeImage() else { return nil }
            img = scaled
        }
        let rep = NSBitmapImageRep(cgImage: img)
        guard let data = rep.representation(using: .png, properties: [:]) else { return nil }
        return ImageData(mime: "image/png", data: data, width: UInt32(img.width), height: UInt32(img.height))
    }

    // MARK: 控件树

    func uiTree(displayId: UInt32, maxNodes: UInt32) -> String? {
        guard SystemPermissions.accessibilityGranted else { return nil }
        return AXTreeExporter(displayId: displayId, maxNodes: Int(maxNodes)).export()
    }

    // MARK: 剪贴板

    /// 一次只取一种内容：文件 > 文字 > 图片
    func clipboardGet() -> ClipboardData? {
        let pb = NSPasteboard.general
        let files = (pb.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL])?.map(\.path) ?? []
        if !files.isEmpty { return .files(paths: files) }
        if let text = pb.string(forType: .string) { return .text(text: text) }
        if let raw = pb.data(forType: .png) ?? pb.data(forType: .tiff), let rep = NSBitmapImageRep(data: raw),
           let png = rep.representation(using: .png, properties: [:]) {
            return .image(image: ImageData(mime: "image/png", data: png, width: UInt32(rep.pixelsWide), height: UInt32(rep.pixelsHigh)))
        }
        return nil
    }

    func clipboardSet(data: ClipboardData) {
        let pb = NSPasteboard.general
        pb.clearContents()
        switch data {
        case .text(let t): pb.setString(t, forType: .string)
        case .image(let img): if let ns = NSImage(data: img.data) { pb.writeObjects([ns]) }
        case .files(let paths): pb.writeObjects(paths.map { URL(fileURLWithPath: $0) as NSURL })
        }
    }

    // MARK: 隐私屏

    /// 开：先装好本地键鼠拦截（做不到就返回 false，不铺黑窗口），再异步铺黑窗口；关：全部撤掉。
    /// 不同步等主线程（内核线程调用，主线程可能正等内核）。
    func setPrivacy(on: Bool) -> Bool {
        guard on else {
            privacy.turnOff()
            return true
        }
        guard SystemPermissions.screenRecordingGranted, SystemPermissions.accessibilityGranted else { return false }
        return privacy.turnOn { [weak self] ids in await self?.refreshCaptureFilters(expecting: ids) }
    }

    // MARK: 系统声音

    func startAudio(sink: AudioPcmSink) -> Bool {
        guard SystemPermissions.screenRecordingGranted else { return false }
        stopAudio()
        let cap = AudioCapture(sink: sink)
        lock.locked { audio = cap }
        let sem = DispatchSemaphore(value: 0)
        nonisolated(unsafe) var ok = false
        Task.detached {
            ok = await cap.start()
            sem.signal()
        }
        sem.wait()
        if !ok { lock.locked { if audio === cap { audio = nil } } }
        return ok
    }

    func stopAudio() {
        let cap = lock.locked { () -> AudioCapture? in
            let a = audio
            audio = nil
            return a
        }
        cap?.stop()
    }
}

// MARK: - ScreenCaptureKit 截屏 + 编码

final class ScreenCapture: NSObject, SCStreamOutput, SCStreamDelegate {
    let displayId: UInt32
    let options: CaptureOptions
    let sink: EncodedFrameSink
    private var stream: SCStream?
    private var encoder: VideoEncoder?
    private let queue = DispatchQueue(label: "galaxy.capture", qos: .userInteractive)
    private var startTime: CMTime?

    init(displayId: UInt32, options: CaptureOptions, sink: EncodedFrameSink) {
        self.displayId = displayId
        self.options = options
        self.sink = sink
    }

    func start() async -> CaptureStarted? {
        do {
            let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
            guard let d = content.displays.first(where: { $0.displayID == displayId }) else { return nil }
            let filter = Self.filter(display: d, content: content)
            let cfg = SCStreamConfiguration()
            let scale = displayScale(displayId)
            var w = Int(CGFloat(d.width) * scale), h = Int(CGFloat(d.height) * scale)
            // 上限 4K：超过时等比缩小（spec 3.1）
            if w > 3840 { h = h * 3840 / w; w = 3840 }
            if h > 2400 { w = w * 2400 / h; h = 2400 }
            w &= ~1; h &= ~1
            cfg.width = w
            cfg.height = h
            cfg.minimumFrameInterval = CMTime(value: 1, timescale: CMTimeScale(max(1, min(options.maxFps, 144))))
            cfg.pixelFormat = kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
            // 光标画进画面（HostPlatform 没有单独上报光标的通道）
            cfg.showsCursor = true
            cfg.queueDepth = 4
            cfg.colorSpaceName = CGColorSpace.sRGB
            // 按主控端给的顺序挑第一个本机能硬件编码的格式
            let wanted = options.codecs.isEmpty ? [VideoCodec.h264] : options.codecs
            var chosen: (VideoEncoder, VideoCodec)?
            for c in wanted {
                if let enc = VideoEncoder(codec: c, width: w, height: h, fps: Int(max(1, options.maxFps)), bitrateKbps: Int(options.bitrateKbps),
                                          output: { [sink] data, pts, key in sink.push(data: data, ptsUs: pts, keyframe: key) }) {
                    chosen = (enc, c)
                    break
                }
            }
            guard let (enc, codec) = chosen else { return nil }
            encoder = enc
            let s = SCStream(filter: filter, configuration: cfg, delegate: self)
            try s.addStreamOutput(self, type: .screen, sampleHandlerQueue: queue)
            try await s.startCapture()
            stream = s
            return CaptureStarted(width: UInt32(w), height: UInt32(h), codec: codec)
        } catch {
            NSLog("LemonGalaxy: 截屏启动失败 %@", String(describing: error))
            return nil
        }
    }

    /// 不把自己的窗口（被控提示条、隐私屏黑窗口）录进去
    static func filter(display: SCDisplay, content: SCShareableContent) -> SCContentFilter {
        let pid = ProcessInfo.processInfo.processIdentifier
        let own = content.windows.filter { $0.owningApplication?.processID == pid }
        return SCContentFilter(display: display, excludingWindows: own)
    }

    /// 按最新的窗口列表更新排除名单（隐私屏刚铺上黑窗口时）；content 为空时自己取（含还没显示出来的窗口）
    func refreshFilter(content: SCShareableContent?) async {
        guard let s = stream else { return }
        do {
            let c: SCShareableContent
            if let content { c = content } else { c = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false) }
            guard let d = c.displays.first(where: { $0.displayID == displayId }) else { return }
            try await s.updateContentFilter(Self.filter(display: d, content: c))
        } catch {
            NSLog("LemonGalaxy: 更新截屏排除窗口失败 %@", String(describing: error))
        }
    }

    func stop() {
        let s = stream
        stream = nil
        Task { try? await s?.stopCapture() }
        encoder?.invalidate()
        encoder = nil
    }

    func requestKeyframe() { encoder?.requestKeyframe() }
    func setBitrate(_ kbps: Int) { encoder?.setBitrate(kbps: kbps) }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen, sampleBuffer.isValid, let pb = sampleBuffer.imageBuffer else { return }
        // 只编码「完整」帧；画面没变时 ScreenCaptureKit 发 idle 帧，跳过
        if let atts = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
           let raw = atts.first?[.status] as? Int, let status = SCFrameStatus(rawValue: raw), status != .complete { return }
        let pts = sampleBuffer.presentationTimeStamp
        if startTime == nil { startTime = pts }
        let us = UInt64(max(0, (pts - startTime!).seconds * 1_000_000))
        encoder?.encode(pb, ptsUs: us)
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        NSLog("LemonGalaxy: 截屏中断 %@", String(describing: error))
    }
}

// MARK: - 系统声音

/// 用 ScreenCaptureKit 采系统声音（不含本进程自己放的声音：主控端正在播放的远端声音不会再传回去）。
/// ScreenCaptureKit 必须带一个屏幕过滤器，画面部分设成 2×2、每秒 1 帧，收到就丢，不影响正在进行的截屏。
final class AudioCapture: NSObject, SCStreamOutput, SCStreamDelegate, @unchecked Sendable {
    private let sink: AudioPcmSink
    private let lock = NSLock()
    private var scStream: SCStream?
    private var stopped = false
    private var restarts = 0
    private let queue = DispatchQueue(label: "galaxy.audio", qos: .userInitiated)

    init(sink: AudioPcmSink) { self.sink = sink }

    func start() async -> Bool {
        do {
            let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
            guard let d = content.displays.first(where: { $0.displayID == CGMainDisplayID() }) ?? content.displays.first else { return false }
            let cfg = SCStreamConfiguration()
            cfg.capturesAudio = true
            cfg.excludesCurrentProcessAudio = true
            cfg.sampleRate = 48_000
            cfg.channelCount = 2
            cfg.width = 2
            cfg.height = 2
            cfg.minimumFrameInterval = CMTime(value: 1, timescale: 1)
            cfg.showsCursor = false
            cfg.queueDepth = 3
            let s = SCStream(filter: SCContentFilter(display: d, excludingWindows: []), configuration: cfg, delegate: self)
            // 画面输出也要接上，否则 ScreenCaptureKit 每帧都报找不到输出
            try s.addStreamOutput(self, type: .screen, sampleHandlerQueue: queue)
            try s.addStreamOutput(self, type: .audio, sampleHandlerQueue: queue)
            try await s.startCapture()
            let keep = lock.locked { () -> Bool in
                guard !stopped else { return false }
                scStream = s
                return true
            }
            if !keep { try? await s.stopCapture() }
            return keep
        } catch {
            NSLog("LemonGalaxy: 声音采集启动失败 %@", String(describing: error))
            return false
        }
    }

    func stop() {
        let s = lock.locked { () -> SCStream? in
            stopped = true
            defer { scStream = nil }
            return scStream
        }
        if let s { Task { try? await s.stopCapture() } }
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .audio, sampleBuffer.isValid, !lock.locked({ stopped }),
              let (pcm, rate, channels) = Self.interleavedPCM(sampleBuffer) else { return }
        sink.push(pcm: pcm, sampleRate: rate, channels: channels)
    }

    /// 中断（主屏幕换了等）：隔 1 秒重开；重开失败或连续中断 5 次就放弃（对方可以关掉声音再打开）
    func stream(_ stream: SCStream, didStopWithError error: Error) {
        NSLog("LemonGalaxy: 声音采集中断 %@", String(describing: error))
        let retry = lock.locked { () -> Bool in
            guard !stopped, scStream === stream, restarts < 5 else { return false }
            restarts += 1
            scStream = nil
            return true
        }
        guard retry else { return }
        Task.detached { [weak self] in
            try? await Task.sleep(nanoseconds: 1_000_000_000)
            guard let self, !self.lock.locked({ self.stopped }) else { return }
            if await self.start() { self.lock.locked { self.restarts = 0 } }
        }
    }

    /// CMSampleBuffer（ScreenCaptureKit 给的一般是非交错 Float32）→ 交错 Float32 小端字节、采样率、声道数
    static func interleavedPCM(_ sb: CMSampleBuffer) -> (Data, UInt32, UInt32)? {
        guard let asbd = sb.formatDescription?.audioStreamBasicDescription, asbd.mFormatID == kAudioFormatLinearPCM else { return nil }
        let channels = Int(asbd.mChannelsPerFrame)
        let frames = sb.numSamples
        let flags = asbd.mFormatFlags
        let isFloat = flags & kAudioFormatFlagIsFloat != 0
        let bits = Int(asbd.mBitsPerChannel)
        guard channels > 0, frames > 0, asbd.mSampleRate > 0, flags & kAudioFormatFlagIsBigEndian == 0,
              (isFloat && bits == 32) || (!isFloat && bits == 16 && flags & kAudioFormatFlagIsSignedInteger != 0) else { return nil }
        let nonInterleaved = flags & kAudioFormatFlagIsNonInterleaved != 0
        let width = bits / 8
        func sample(_ p: UnsafeMutableRawPointer, _ i: Int) -> Float {
            isFloat ? p.loadUnaligned(fromByteOffset: i * 4, as: Float.self) : Float(p.loadUnaligned(fromByteOffset: i * 2, as: Int16.self)) / 32768
        }
        var out = [Float](repeating: 0, count: frames * channels)
        do {
            try sb.withAudioBufferList { list, _ in
                out.withUnsafeMutableBufferPointer { dst in
                    if nonInterleaved {
                        // 每个声道一块缓冲
                        for (c, buf) in list.enumerated() where c < channels {
                            guard let p = buf.mData else { continue }
                            for f in 0..<min(frames, Int(buf.mDataByteSize) / width) { dst[f * channels + c] = sample(p, f) }
                        }
                    } else if let buf = list.first, let p = buf.mData {
                        for i in 0..<min(frames * channels, Int(buf.mDataByteSize) / width) { dst[i] = sample(p, i) }
                    }
                }
            }
        } catch {
            return nil
        }
        // arm64 与 x86_64 都是小端，Float 数组的内存就是内核要的格式
        let data = out.withUnsafeBufferPointer { Data(buffer: $0) }
        return (data, UInt32(asbd.mSampleRate.rounded()), UInt32(channels))
    }
}

// MARK: - 隐私屏

/// 隐私屏：每块屏幕盖一个最高层的黑色无边框窗口，本地键鼠由 LocalInputBlocker 拦掉。
/// 黑窗口从截屏里排除（ScreenCapture.filter 排除本进程的窗口），对方照常看到真实画面；
/// 窗口不接收鼠标，远端注入的点击穿过它落到下面的应用。开关可在任何线程调用，窗口只在主线程增删。
final class PrivacyScreen: @unchecked Sendable {
    static let windowIdentifier = "galaxy.privacy-screen"
    private let lock = NSLock()
    /// 开、关整体串行（两个会话同时开关时不交错）
    private let opLock = NSLock()
    private var on = false
    /// 每次重铺窗口加一：旧的「排除完再显示」回调发现编号变了就不再动窗口
    private var serial = 0
    private var displayWindows: [CGDirectDisplayID: CGWindowID] = [:]
    private var refresh: (@Sendable (Set<CGWindowID>) async -> Void)?
    private let blocker = LocalInputBlocker()
    /// 只在主线程读写
    private var windows: [NSWindow] = []

    var isOn: Bool { lock.locked { on } }
    /// 黑窗口的窗口号（任何线程可读，截图时排除）
    var windowIds: Set<CGWindowID> { lock.locked { Set(displayWindows.values) } }
    func windowId(display: CGDirectDisplayID) -> CGWindowID? { lock.locked { displayWindows[display] } }

    /// refresh：黑窗口出现后把它们从正在进行的截屏里排除，排除完才让窗口变黑可见，对方画面不会闪黑
    func turnOn(refresh: @escaping @Sendable (Set<CGWindowID>) async -> Void) -> Bool {
        opLock.lock(); defer { opLock.unlock() }
        if isOn { return true }
        // 先装好本地键鼠拦截，做不到就整个不开
        guard blocker.start() else { return false }
        let s = lock.locked { () -> Int in
            on = true
            self.refresh = refresh
            serial += 1
            return serial
        }
        DispatchQueue.main.async { MainActor.assumeIsolated { self.layout(serial: s) } }
        return true
    }

    func turnOff() {
        opLock.lock(); defer { opLock.unlock() }
        blocker.stop()
        let wasOn = lock.locked { () -> Bool in
            defer { on = false; refresh = nil; serial += 1; displayWindows = [:] }
            return on
        }
        guard wasOn else { return }
        DispatchQueue.main.async { MainActor.assumeIsolated { self.removeWindows() } }
    }

    @MainActor func screensChanged() {
        guard let s = lock.locked({ () -> Int? in
            guard on else { return nil }
            serial += 1
            return serial
        }) else { return }
        layout(serial: s)
    }

    @MainActor private func layout(serial s: Int) {
        guard lock.locked({ on && serial == s }) else { return }
        removeWindows()
        var map: [CGDirectDisplayID: CGWindowID] = [:]
        for screen in NSScreen.screens {
            let w = NSWindow(contentRect: screen.frame, styleMask: [.borderless], backing: .buffered, defer: false)
            w.isReleasedWhenClosed = false
            w.backgroundColor = .black
            w.isOpaque = true
            w.hasShadow = false
            w.animationBehavior = .none
            w.ignoresMouseEvents = true
            w.level = NSWindow.Level(rawValue: Int(CGShieldingWindowLevel()))
            w.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle, .fullScreenAuxiliary]
            w.setAccessibilityIdentifier(Self.windowIdentifier)
            // 先透明铺上，等从截屏里排除后再变黑
            w.alphaValue = 0
            w.setFrame(screen.frame, display: false)
            w.orderFrontRegardless()
            windows.append(w)
            if let n = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber {
                map[n.uint32Value] = CGWindowID(w.windowNumber)
            }
        }
        let ids = Set(windows.map { CGWindowID($0.windowNumber) })
        let refresh = lock.locked { () -> (@Sendable (Set<CGWindowID>) async -> Void)? in
            displayWindows = map
            return self.refresh
        }
        Task.detached {
            await refresh?(ids)
            await MainActor.run { self.reveal(serial: s) }
            // 兜底：万一第一次没等到窗口出现在可共享内容里，变黑之后再排除一次
            if self.lock.locked({ self.on && self.serial == s }) { await refresh?(ids) }
        }
    }

    @MainActor private func reveal(serial s: Int) {
        guard lock.locked({ on && serial == s }) else { return }
        for w in windows { w.alphaValue = 1 }
    }

    @MainActor private func removeWindows() {
        for w in windows {
            w.orderOut(nil)
            w.close()
        }
        windows.removeAll()
    }
}

/// 会话级事件拦截：丢掉本地键盘、鼠标、触控板事件，只放行带 InputInjector.marker 的（远端注入的）事件。
/// 跑在自己的线程上，主线程卡住也不影响；系统因超时停用拦截时立刻重新启用。需要「辅助功能」授权。
final class LocalInputBlocker: @unchecked Sendable {
    private let lock = NSLock()
    private var port: CFMachPort?
    private var running = false

    private static let mask: CGEventMask = {
        let types: [CGEventType] = [.keyDown, .keyUp, .flagsChanged, .leftMouseDown, .leftMouseUp, .rightMouseDown, .rightMouseUp,
                                    .otherMouseDown, .otherMouseUp, .mouseMoved, .leftMouseDragged, .rightMouseDragged, .otherMouseDragged,
                                    .scrollWheel, .tabletPointer, .tabletProximity]
        var m = types.reduce(CGEventMask(0)) { $0 | (CGEventMask(1) << CGEventMask($1.rawValue)) }
        // 系统按键（亮度、音量、媒体键）与触控板手势没有 CGEventType 名字，按 NSEvent.EventType 原值
        let more: [NSEvent.EventType] = [.systemDefined, .gesture, .magnify, .swipe, .rotate, .beginGesture, .endGesture, .smartMagnify, .pressure]
        for t in more { m |= CGEventMask(1) << CGEventMask(t.rawValue) }
        return m
    }()

    /// 装好拦截返回 true；没有「辅助功能」授权等原因装不上返回 false
    func start() -> Bool {
        if lock.locked({ running }) { return true }
        let sem = DispatchSemaphore(value: 0)
        nonisolated(unsafe) var ok = false
        let thread = Thread { [self] in
            let info = Unmanaged.passUnretained(self).toOpaque()
            guard let tap = CGEvent.tapCreate(tap: .cgSessionEventTap, place: .headInsertEventTap, options: .defaultTap,
                                              eventsOfInterest: Self.mask, callback: localInputTapCallback, userInfo: info) else {
                sem.signal()
                return
            }
            let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0)
            let loop = CFRunLoopGetCurrent()
            CFRunLoopAddSource(loop, source, .commonModes)
            CGEvent.tapEnable(tap: tap, enable: true)
            lock.locked { port = tap; running = true }
            ok = true
            sem.signal()
            // stop() 把 running 置 false 后最多 0.25 秒退出并拆掉拦截
            while lock.locked({ running && port === tap }) {
                _ = CFRunLoopRunInMode(.defaultMode, 0.25, false)
            }
            CGEvent.tapEnable(tap: tap, enable: false)
            CFRunLoopRemoveSource(loop, source, .commonModes)
            CFMachPortInvalidate(tap)
        }
        thread.name = "galaxy.privacy.input"
        thread.qualityOfService = .userInteractive
        thread.start()
        sem.wait()
        return ok
    }

    func stop() {
        let tap = lock.locked { () -> CFMachPort? in
            defer { running = false; port = nil }
            return port
        }
        // 立刻停用，线程随后自己清理退出
        if let tap { CGEvent.tapEnable(tap: tap, enable: false) }
    }

    fileprivate func reenable() {
        if let tap = lock.locked({ port }) { CGEvent.tapEnable(tap: tap, enable: true) }
    }
}

private func localInputTapCallback(proxy: CGEventTapProxy, type: CGEventType, event: CGEvent, userInfo: UnsafeMutableRawPointer?) -> Unmanaged<CGEvent>? {
    switch type {
    case .tapDisabledByTimeout, .tapDisabledByUserInput:
        if let userInfo { Unmanaged<LocalInputBlocker>.fromOpaque(userInfo).takeUnretainedValue().reenable() }
        return Unmanaged.passUnretained(event)
    default:
        // 远端注入的事件带标记，放行；本机键盘、鼠标、触控板的一律丢掉
        return event.getIntegerValueField(.eventSourceUserData) == InputInjector.marker ? Unmanaged.passUnretained(event) : nil
    }
}

// MARK: - 输入注入

final class InputInjector {
    /// 注入的事件都带这个标记（kCGEventSourceUserData），隐私屏的事件拦截据此只放行远端操作
    static let marker: Int64 = 0x4C47_414C_5859   // "LGALXY"
    private let source: CGEventSource? = {
        let s = CGEventSource(stateID: .hidSystemState)
        s?.userData = InputInjector.marker
        return s
    }()
    private var buttonsDown: Set<PointerButton> = []
    private var lastPoint = CGPoint.zero
    private var lastClick: (PointerButton, Date, CGPoint, Int64)?
    private let lock = NSLock()
    private var flags: CGEventFlags = []

    /// 被控屏幕像素 → 全局坐标（点）
    private func globalPoint(_ x: Int32, _ y: Int32, display: CGDirectDisplayID) -> CGPoint {
        let b = CGDisplayBounds(display)
        let s = displayScale(display)
        return CGPoint(x: b.minX + CGFloat(x) / s, y: b.minY + CGFloat(y) / s)
    }

    func inject(_ e: InputEvent, display: CGDirectDisplayID) {
        lock.lock(); defer { lock.unlock() }
        injectLocked(e, display: display)
    }

    private func injectLocked(_ e: InputEvent, display: CGDirectDisplayID) {
        switch e {
        case .mouseMove(let x, let y):
            let p = globalPoint(x, y, display: display)
            lastPoint = p
            let type: CGEventType = buttonsDown.contains(.left) ? .leftMouseDragged : buttonsDown.contains(.right) ? .rightMouseDragged
                : buttonsDown.isEmpty ? .mouseMoved : .otherMouseDragged
            post(CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: p, mouseButton: buttonsDown.contains(.right) ? .right : .left))
        case .mouseButton(let b, let down, let x, let y):
            let p = globalPoint(x, y, display: display)
            lastPoint = p
            let (type, cgb): (CGEventType, CGMouseButton) = {
                switch b {
                case .left: return (down ? .leftMouseDown : .leftMouseUp, .left)
                case .right: return (down ? .rightMouseDown : .rightMouseUp, .right)
                case .middle: return (down ? .otherMouseDown : .otherMouseUp, .center)
                case .back: return (down ? .otherMouseDown : .otherMouseUp, CGMouseButton(rawValue: 3)!)
                case .forward: return (down ? .otherMouseDown : .otherMouseUp, CGMouseButton(rawValue: 4)!)
                }
            }()
            if down { buttonsDown.insert(b) } else { buttonsDown.remove(b) }
            var clicks: Int64 = 1
            if down {
                // 双击：同一按键、0.4 秒内、位置接近
                if let (lb, t, lp, n) = lastClick, lb == b, Date().timeIntervalSince(t) < 0.4, hypot(lp.x - p.x, lp.y - p.y) < 5 { clicks = n + 1 }
                lastClick = (b, Date(), p, clicks)
            } else if let (lb, _, _, n) = lastClick, lb == b { clicks = n }
            let ev = CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: p, mouseButton: cgb)
            ev?.setIntegerValueField(.mouseEventClickState, value: clicks)
            ev?.flags = flags
            post(ev)
        case .wheel(let dx, let dy):
            post(CGEvent(scrollWheelEvent2Source: source, units: .pixel, wheelCount: 2, wheel1: dy, wheel2: dx, wheel3: 0))
        case .key(let usage, let down):
            guard let code = HIDKeyMap.macKeyCode(forHID: usage) else { return }
            updateFlags(code: code, down: down)
            let ev = CGEvent(keyboardEventSource: source, virtualKey: CGKeyCode(code), keyDown: down)
            ev?.flags = flags
            post(ev)
        case .text(let text):
            typeText(text)
        case .touch(_, let phase, let x, let y):
            // 触控模式按单指鼠标处理：按下、拖动、抬起
            switch phase {
            case .began: injectLocked(.mouseButton(button: .left, down: true, x: x, y: y), display: display)
            case .moved: injectLocked(.mouseMove(x: x, y: y), display: display)
            case .ended, .cancelled: injectLocked(.mouseButton(button: .left, down: false, x: x, y: y), display: display)
            }
        case .specialCombo(let combo):
            switch combo {
            case .lockScreen: chord([kVK_Control, kVK_Command], kVK_ANSI_Q)
            case .ctrlAltDel, .taskManager: chord([kVK_Command, kVK_Option], kVK_Escape)      // macOS 上对应「强制退出」
            case .altTab: chord([kVK_Command], kVK_Tab)
            case .winKey: chord([kVK_Command], kVK_Space)                                       // 聚焦搜索
            case .showDesktop: chord([kVK_Command], kVK_F3)
            }
        }
    }

    private func updateFlags(code: Int, down: Bool) {
        let f: CGEventFlags?
        switch code {
        case kVK_Shift, kVK_RightShift: f = .maskShift
        case kVK_Control, kVK_RightControl: f = .maskControl
        case kVK_Option, kVK_RightOption: f = .maskAlternate
        case kVK_Command, kVK_RightCommand: f = .maskCommand
        case kVK_CapsLock: f = .maskAlphaShift
        default: f = nil
        }
        guard let f else { return }
        if down { flags.insert(f) } else { flags.remove(f) }
    }

    private func chord(_ mods: [Int], _ key: Int) {
        for m in mods { injectLocked(.key(hidUsage: HIDKeyMap.macToHID[m] ?? 0, down: true), display: CGMainDisplayID()) }
        injectLocked(.key(hidUsage: HIDKeyMap.macToHID[key] ?? 0, down: true), display: CGMainDisplayID())
        injectLocked(.key(hidUsage: HIDKeyMap.macToHID[key] ?? 0, down: false), display: CGMainDisplayID())
        for m in mods.reversed() { injectLocked(.key(hidUsage: HIDKeyMap.macToHID[m] ?? 0, down: false), display: CGMainDisplayID()) }
    }

    /// 输入法提交的文字：直接发 Unicode 字符串事件（每个事件最多 20 个 UTF-16 单元）
    private func typeText(_ text: String) {
        let units = Array(text.utf16)
        var i = 0
        while i < units.count {
            let chunk = Array(units[i..<min(i + 20, units.count)])
            for down in [true, false] {
                let ev = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: down)
                chunk.withUnsafeBufferPointer { ev?.keyboardSetUnicodeString(stringLength: chunk.count, unicodeString: $0.baseAddress) }
                post(ev)
            }
            i += 20
        }
    }

    private func post(_ ev: CGEvent?) { ev?.post(tap: .cghidEventTap) }
}

// MARK: - 辅助功能控件树

/// 遍历屏幕上可见应用的窗口与控件，导出 JSON（ai-agent.md 的 ui_tree 工具）。坐标换算成被控屏幕像素。
struct AXTreeExporter {
    let displayId: UInt32
    let maxNodes: Int

    func export() -> String? {
        var count = 0
        let bounds = CGDisplayBounds(displayId)
        let scale = displayScale(displayId)
        var apps: [[String: Any]] = []
        let running = NSWorkspace.shared.runningApplications.filter { $0.activationPolicy == .regular && !$0.isHidden }
        let frontPid = NSWorkspace.shared.frontmostApplication?.processIdentifier
        // 前台应用优先，节点数有限时先保证它
        for app in running.sorted(by: { a, _ in a.processIdentifier == frontPid }) {
            guard count < maxNodes else { break }
            let el = AXUIElementCreateApplication(app.processIdentifier)
            var windows: [[String: Any]] = []
            for w in children(el, attr: kAXWindowsAttribute) {
                guard count < maxNodes else { break }
                // 隐私屏的黑窗口不算界面内容
                if attr(w, kAXIdentifierAttribute) as? String == PrivacyScreen.windowIdentifier { continue }
                if let node = walk(w, depth: 0, count: &count, bounds: bounds, scale: scale) { windows.append(node) }
            }
            if !windows.isEmpty {
                apps.append(["app": app.localizedName ?? "", "pid": app.processIdentifier, "frontmost": app.processIdentifier == frontPid, "windows": windows])
            }
        }
        let root: [String: Any] = ["display": displayId, "scale": scale, "nodes": count, "truncated": count >= maxNodes, "apps": apps]
        guard let data = try? JSONSerialization.data(withJSONObject: root, options: [.sortedKeys]) else { return nil }
        return String(data: data, encoding: .utf8)
    }

    private func attr(_ el: AXUIElement, _ name: String) -> CFTypeRef? {
        var v: CFTypeRef?
        return AXUIElementCopyAttributeValue(el, name as CFString, &v) == .success ? v : nil
    }

    private func children(_ el: AXUIElement, attr name: String = kAXChildrenAttribute) -> [AXUIElement] {
        (attr(el, name) as? [AXUIElement]) ?? []
    }

    private func walk(_ el: AXUIElement, depth: Int, count: inout Int, bounds: CGRect, scale: CGFloat) -> [String: Any]? {
        guard count < maxNodes, depth < 40 else { return nil }
        count += 1
        var node: [String: Any] = [:]
        if let role = attr(el, kAXRoleAttribute) as? String { node["role"] = role }
        if let sub = attr(el, kAXSubroleAttribute) as? String { node["subrole"] = sub }
        for (key, a) in [("title", kAXTitleAttribute), ("description", kAXDescriptionAttribute), ("help", kAXHelpAttribute)] {
            if let s = attr(el, a) as? String, !s.isEmpty { node[key] = String(s.prefix(200)) }
        }
        if let v = attr(el, kAXValueAttribute) {
            // 密码框不导出内容
            if (node["subrole"] as? String) == (kAXSecureTextFieldSubrole as String) { node["secure"] = true }
            else if let s = v as? String { node["value"] = String(s.prefix(200)) }
            else if let n = v as? NSNumber { node["value"] = n }
        }
        if let e = attr(el, kAXEnabledAttribute) as? Bool, !e { node["enabled"] = false }
        if let f = attr(el, kAXFocusedAttribute) as? Bool, f { node["focused"] = true }
        var pos = CGPoint.zero, size = CGSize.zero
        if let p = attr(el, kAXPositionAttribute), CFGetTypeID(p) == AXValueGetTypeID() { AXValueGetValue(p as! AXValue, .cgPoint, &pos) }
        if let s = attr(el, kAXSizeAttribute), CFGetTypeID(s) == AXValueGetTypeID() { AXValueGetValue(s as! AXValue, .cgSize, &size) }
        if size.width > 0 {
            let r = CGRect(origin: pos, size: size)
            if !r.intersects(bounds) && depth > 0 { return nil }
            node["frame"] = [Int((r.minX - bounds.minX) * scale), Int((r.minY - bounds.minY) * scale), Int(r.width * scale), Int(r.height * scale)]
        }
        var kids: [[String: Any]] = []
        for c in children(el) {
            guard count < maxNodes else { break }
            if let k = walk(c, depth: depth + 1, count: &count, bounds: bounds, scale: scale) { kids.append(k) }
        }
        if !kids.isEmpty { node["children"] = kids }
        return node
    }
}

/// 显示器名称缓存：NSScreen 只能在主线程读，内核线程上 listDisplays 时读缓存
enum DisplayNames {
    private static let lock = NSLock()
    nonisolated(unsafe) private static var names: [CGDirectDisplayID: String] = [:]

    static func refresh() {
        let fresh = Dictionary(NSScreen.screens.compactMap { s -> (CGDirectDisplayID, String)? in
            guard let n = s.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber else { return nil }
            return (n.uint32Value, s.localizedName)
        }, uniquingKeysWith: { a, _ in a })
        lock.locked { names = fresh }
    }

    static func name(_ id: CGDirectDisplayID) -> String {
        lock.locked { names[id] } ?? (CGDisplayIsBuiltin(id) != 0 ? "Built-in Display" : "Display \(id)")
    }
}

/// 显示器的像素 / 点比例（Retina 为 2）
func displayScale(_ id: CGDirectDisplayID) -> CGFloat {
    let b = CGDisplayBounds(id)
    guard let mode = CGDisplayCopyDisplayMode(id), b.width > 0 else { return 1 }
    return max(1, CGFloat(mode.pixelWidth) / b.width)
}

// MARK: - 系统授权

enum SystemPermissions {
    /// selftest 用的假状态（不调用任何系统授权接口）
    static var fake: (screen: Bool, accessibility: Bool)?

    static var screenRecordingGranted: Bool {
        if let f = fake { return f.screen }
        return CGPreflightScreenCaptureAccess()
    }

    static var accessibilityGranted: Bool {
        if let f = fake { return f.accessibility }
        return AXIsProcessTrusted()
    }

    static func requestScreenRecording() {
        guard fake == nil else { return }
        if !CGPreflightScreenCaptureAccess() { CGRequestScreenCaptureAccess() }
        openSettings("Privacy_ScreenCapture")
    }

    static func requestAccessibility() {
        guard fake == nil else { return }
        let opts = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
        if !AXIsProcessTrustedWithOptions(opts) { openSettings("Privacy_Accessibility") }
    }

    static func openSettings(_ anchor: String) {
        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?\(anchor)") { NSWorkspace.shared.open(url) }
    }
}
