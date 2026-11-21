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
/// 内核在自己的线程上调用这些方法；需要「屏幕录制」「辅助功能」授权，见 Permissions。
final class MacHostPlatform: HostPlatform, @unchecked Sendable {
    private let lock = NSLock()
    private var captures: [UInt32: ScreenCapture] = [:]
    private var lastDisplay: UInt32 = CGMainDisplayID()
    private let injector = InputInjector()

    init() {
        DisplayNames.refresh()
        NotificationCenter.default.addObserver(forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main) { _ in
            DisplayNames.refresh()
        }
    }

    // MARK: 能力（内核据此告诉主控端能用什么）

    func capabilities() -> [String] {
        var caps = ["clipboard"]
        if SystemPermissions.screenRecordingGranted { caps += ["display", "screenshot"] }
        if SystemPermissions.accessibilityGranted { caps += ["input", "ui_tree"] }
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
        if started == nil { lock.locked { captures[displayId] = nil } }
        return started
    }

    func stopCapture(displayId: UInt32) {
        let cap = lock.locked { captures.removeValue(forKey: displayId) }
        cap?.stop()
    }

    func requestKeyframe(displayId: UInt32) { lock.locked { captures[displayId] }?.requestKeyframe() }
    func setBitrate(displayId: UInt32, kbps: UInt32) { lock.locked { captures[displayId] }?.setBitrate(Int(kbps)) }

    // MARK: 输入

    func inject(event: InputEvent) {
        let display = lock.locked { lastDisplay }
        injector.inject(event, display: display)
    }

    // MARK: 截图

    func screenshot(displayId: UInt32, maxWidth: UInt32, maxHeight: UInt32) -> ImageData? {
        guard SystemPermissions.screenRecordingGranted else { return nil }
        var image: CGImage?
        if #available(macOS 14.0, *) {
            let sem = DispatchSemaphore(value: 0)
            Task.detached {
                do {
                    let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
                    if let d = content.displays.first(where: { $0.displayID == displayId }) ?? content.displays.first {
                        let filter = SCContentFilter(display: d, excludingWindows: [])
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
            // 不把自己的提示条录进去
            let own = content.windows.filter { $0.owningApplication?.processID == ProcessInfo.processInfo.processIdentifier }
            let filter = SCContentFilter(display: d, excludingWindows: own)
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

// MARK: - 输入注入

final class InputInjector {
    private let source = CGEventSource(stateID: .hidSystemState)
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
