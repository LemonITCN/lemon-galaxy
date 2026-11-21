import Foundation
import CoreGraphics
import CoreText
import CoreVideo
import ImageIO
import QuartzCore

/// 假远程桌面：本机用 CoreGraphics 画一张「被控端桌面」，经 VideoToolbox 编码成 H.264 Annex B，
/// 再走 VideoSink（on_config / on_frame / on_cursor）——和真内核推来的数据路径完全一样，界面的解码显示因此能被真实验证。
final class MockDisplayChannel: DisplayChannel, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("模拟对象不能由内核指针构造") }
    private let device: DeviceInfo
    private let display: DisplayInfo
    private weak var sink: VideoSink?
    private let renderer: MockDesktopRenderer
    private var encoder: VideoEncoder?
    private let queue = DispatchQueue(label: "mock.display")
    private var timer: DispatchSourceTimer?
    private var frameIndex: UInt64 = 0
    private var stopped = false
    private let streamW: Int
    private let streamH: Int
    private let codec: VideoCodec
    private let zh: Bool
    /// 录像中（只在 queue 上读写）
    private var recorder: MockRecorder?

    init(device: DeviceInfo, display: DisplayInfo, options: DisplayOptions, sink: VideoSink, zh: Bool) {
        self.device = device
        self.display = display
        self.sink = sink
        // 模拟端不按原始分辨率出流（CPU 画图太慢），长边限制在 1280，宽高取偶数
        let scale = min(1, 1280 / Double(max(display.width, display.height)))
        streamW = Int(Double(display.width) * scale) & ~1
        streamH = Int(Double(display.height) * scale) & ~1
        renderer = MockDesktopRenderer(device: device, display: display, zh: zh, size: CGSize(width: streamW, height: streamH))
        self.zh = zh
        codec = options.codecs.first == .h265 ? .h265 : .h264
        super.init(noHandle: NoHandle())
        let fps = max(5, min(Int(options.maxFps), 20))
        queue.async { [weak self] in self?.start(fps: fps, bitrate: Int(options.bitrateKbps)) }
    }

    deinit { timer?.cancel() }

    private func start(fps: Int, bitrate: Int) {
        encoder = VideoEncoder(codec: codec, width: streamW, height: streamH, fps: fps, bitrateKbps: bitrate == 0 ? 4000 : bitrate) { [weak self] data, pts, key in
            self?.queue.async { self?.sink?.onFrame(data: data, ptsUs: pts, keyframe: key) }
        }
        if encoder == nil {
            sink?.onClosed(reason: "")
            return
        }
        // 被控端按原始分辨率报告，坐标都是被控屏幕像素
        sink?.onConfig(codec: codec, width: UInt32(streamW), height: UInt32(streamH))
        renderFrame()
        let t = DispatchSource.makeTimerSource(queue: queue)
        t.schedule(deadline: .now() + 1.0 / Double(fps), repeating: 1.0 / Double(fps))
        t.setEventHandler { [weak self] in self?.renderFrame() }
        t.resume()
        timer = t
    }

    private func renderFrame() {
        guard !stopped, let encoder, let pb = PixelBuffers.make(width: streamW, height: streamH) else { return }
        PixelBuffers.draw(pb) { ctx in renderer.draw(ctx, time: Date()) }
        frameIndex += 1
        encoder.encode(pb, ptsUs: frameIndex * 50_000)
        recorder?.append(pb, at: CACurrentMediaTime())
    }

    private func toStream(_ x: Int32, _ y: Int32) -> CGPoint {
        CGPoint(x: Double(x) * Double(streamW) / Double(display.width), y: Double(y) * Double(streamH) / Double(display.height))
    }

    override func sendInput(event: InputEvent) {
        queue.async { [weak self] in
            guard let self else { return }
            switch event {
            case .mouseMove(let x, let y):
                self.renderer.cursor = self.toStream(x, y)
                self.sink?.onCursor(x: x, y: y, visible: true)
            case .mouseButton(_, let down, let x, let y):
                if down { self.renderer.clicks.append((self.toStream(x, y), Date())) }
            case .touch(_, let phase, let x, let y):
                if phase == .began { self.renderer.clicks.append((self.toStream(x, y), Date())) }
            case .text(let t):
                self.renderer.typed += t
            case .key(let usage, let down):
                if down, usage == Hid.backspace, !self.renderer.typed.isEmpty { self.renderer.typed.removeLast() }
                if down, usage == Hid.enter { self.renderer.typed += "\n" }
            case .wheel, .specialCombo:
                break
            }
        }
    }

    override func requestKeyframe() { encoder?.requestKeyframe() }
    override func setBitrate(kbps: UInt32) { encoder?.setBitrate(kbps: Int(kbps == 0 ? 4000 : kbps)) }

    // 录像：和真内核一样，关画面时自动结束（不返回结果）

    override func isRecording() -> Bool { queue.sync { recorder != nil } }

    override func startRecording(path: String, withAudio: Bool) async throws {
        // 先判断再建文件：重复开始不能碰正在写的文件
        let busy: Bool = queue.sync { recorder != nil || stopped }
        if busy { throw GalaxyError.Other(detail: zh ? "已经在录像了" : "Already recording") }
        let r = try MockRecorder(url: URL(fileURLWithPath: path), width: streamW, height: streamH)
        let ok: Bool = queue.sync {
            guard recorder == nil, !stopped else { return false }
            recorder = r
            return true
        }
        guard ok else {
            _ = r.finish()
            throw GalaxyError.Other(detail: zh ? "已经在录像了" : "Already recording")
        }
    }

    override func stopRecording() throws -> RecordingInfo {
        let r: MockRecorder? = queue.sync {
            let r = recorder
            recorder = nil
            return r
        }
        guard let r else { throw GalaxyError.Other(detail: zh ? "没有在录像" : "Not recording") }
        return r.finish()
    }

    override func stop() {
        queue.async { [weak self] in
            self?.stopped = true
            self?.timer?.cancel()
            self?.encoder?.invalidate()
            if let r = self?.recorder {
                self?.recorder = nil
                _ = r.finish()
            }
        }
    }
}

/// 画一张假桌面：Windows 风格（底部任务栏）或 macOS 风格（顶部菜单栏 + 程序坞）。只用 CoreGraphics / CoreText，iOS 与 macOS 通用。
final class MockDesktopRenderer {
    let device: DeviceInfo
    let display: DisplayInfo
    let zh: Bool
    let size: CGSize
    var cursor = CGPoint(x: -100, y: -100)
    var clicks: [(CGPoint, Date)] = []
    var typed = ""
    private var background: CGImage?
    private let rgb = CGColorSpace(name: CGColorSpace.sRGB)!

    init(device: DeviceInfo, display: DisplayInfo, zh: Bool, size: CGSize? = nil) {
        self.device = device
        self.display = display
        self.zh = zh
        self.size = size ?? CGSize(width: Int(display.width) / 2, height: Int(display.height) / 2)
    }

    private var isWindows: Bool { device.platform == .windows }
    private var u: CGFloat { size.height / 900 }

    private func color(_ r: CGFloat, _ g: CGFloat, _ b: CGFloat, _ a: CGFloat = 1) -> CGColor {
        CGColor(colorSpace: rgb, components: [r, g, b, a])!
    }

    private var palette: [CGColor] {
        [color(0.04, 0.52, 1), color(0.35, 0.78, 0.98), color(1, 0.62, 0.04), color(0.2, 0.78, 0.35),
         color(0.69, 0.32, 0.87), color(1, 0.8, 0), color(1, 0.27, 0.23), color(0.35, 0.34, 0.84)]
    }

    /// 上下文已经翻转成 y 向下
    func draw(_ ctx: CGContext, time: Date) {
        if background == nil { background = makeBackground() }
        if let bg = background {
            ctx.saveGState()
            ctx.translateBy(x: 0, y: size.height)
            ctx.scaleBy(x: 1, y: -1)
            ctx.draw(bg, in: CGRect(origin: .zero, size: size))
            ctx.restoreGState()
        }
        drawDynamic(ctx, time: time)
    }

    private func makeBackground() -> CGImage? {
        let w = Int(size.width), h = Int(size.height)
        guard let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                                  space: rgb, bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue) else { return nil }
        ctx.translateBy(x: 0, y: CGFloat(h))
        ctx.scaleBy(x: 1, y: -1)
        drawStatic(ctx)
        return ctx.makeImage()
    }

    private func fillRound(_ ctx: CGContext, _ r: CGRect, _ radius: CGFloat, _ c: CGColor) {
        ctx.setFillColor(c)
        ctx.addPath(CGPath(roundedRect: r, cornerWidth: radius, cornerHeight: radius, transform: nil))
        ctx.fillPath()
    }

    private func drawStatic(_ ctx: CGContext) {
        let colors = isWindows ? [color(0.05, 0.27, 0.62), color(0.33, 0.62, 0.93)] : [color(0.18, 0.12, 0.38), color(0.86, 0.45, 0.42)]
        if let g = CGGradient(colorsSpace: rgb, colors: colors as CFArray, locations: [0, 1]) {
            ctx.drawLinearGradient(g, start: .zero, end: CGPoint(x: isWindows ? size.width : 0, y: size.height), options: [])
        }
        if isWindows {
            let icons = zh ? ["此电脑", "回收站", "文档", "LemonGalaxy"] : ["This PC", "Recycle Bin", "Documents", "LemonGalaxy"]
            for (i, name) in icons.enumerated() {
                let r = CGRect(x: 24 * u, y: (24 + CGFloat(i) * 96) * u, width: 56 * u, height: 48 * u)
                fillRound(ctx, r.insetBy(dx: 8 * u, dy: 4 * u), 6 * u, color(1, 1, 1, 0.85))
                text(ctx, name, at: CGPoint(x: r.minX - 12 * u, y: r.maxY + 4 * u), size: 12 * u, color: color(1, 1, 1), width: 80 * u, center: true)
            }
            let bar = CGRect(x: 0, y: size.height - 48 * u, width: size.width, height: 48 * u)
            ctx.setFillColor(color(0.93, 0.94, 0.96, 0.92))
            ctx.fill(bar)
            for i in 0..<6 {
                let r = CGRect(x: size.width / 2 - 150 * u + CGFloat(i) * 50 * u, y: bar.minY + 8 * u, width: 32 * u, height: 32 * u)
                fillRound(ctx, r, 6 * u, palette[i])
            }
        } else {
            ctx.setFillColor(color(0, 0, 0, 0.25))
            ctx.fill(CGRect(x: 0, y: 0, width: size.width, height: 26 * u))
            text(ctx, zh ? "访达   文件   编辑   显示   前往   窗口   帮助" : "Finder   File   Edit   View   Go   Window   Help",
                 at: CGPoint(x: 40 * u, y: 5 * u), size: 13 * u, color: color(1, 1, 1))
            let dock = CGRect(x: size.width / 2 - 240 * u, y: size.height - 74 * u, width: 480 * u, height: 64 * u)
            fillRound(ctx, dock, 18 * u, color(1, 1, 1, 0.28))
            for i in 0..<8 {
                let r = CGRect(x: dock.minX + 14 * u + CGFloat(i) * 58 * u, y: dock.minY + 8 * u, width: 48 * u, height: 48 * u)
                fillRound(ctx, r, 11 * u, palette[i])
            }
        }
        let m = CGRect(x: size.width * 0.52, y: size.height * 0.12, width: size.width * 0.4, height: size.height * 0.42)
        window(ctx, m, title: zh ? "资源监视器" : "Resource Monitor")
        for (i, v) in [0.42, 0.67, 0.31, 0.55, 0.78, 0.49, 0.62, 0.38, 0.71, 0.44, 0.58, 0.66].enumerated() {
            let bw = (m.width - 40 * u) / 12
            let bh = (m.height - 90 * u) * CGFloat(v)
            fillRound(ctx, CGRect(x: m.minX + 20 * u + CGFloat(i) * bw + 2 * u, y: m.maxY - 20 * u - bh, width: bw - 4 * u, height: bh),
                      2 * u, color(0.04, 0.52, 1, 0.75))
        }
        window(ctx, editorRect, title: zh ? "无标题 - 记事本" : "Untitled - Notepad")
        // 保存按钮
        let save = CGRect(x: editorRect.maxX - 64 * u, y: editorRect.minY + 5 * u, width: 50 * u, height: 20 * u)
        fillRound(ctx, save, 4 * u, color(0.04, 0.52, 1))
        text(ctx, zh ? "保存" : "Save", at: CGPoint(x: save.minX, y: save.minY + 3 * u), size: 11 * u, color: color(1, 1, 1), width: save.width, center: true)
    }

    private var editorRect: CGRect {
        CGRect(x: size.width * 0.1, y: size.height * 0.2, width: size.width * 0.38, height: size.height * 0.5)
    }

    private func window(_ ctx: CGContext, _ r: CGRect, title: String) {
        ctx.saveGState()
        ctx.setShadow(offset: CGSize(width: 0, height: 6 * u), blur: 18 * u, color: color(0, 0, 0, 0.35))
        fillRound(ctx, r, 8 * u, color(1, 1, 1))
        ctx.restoreGState()
        fillRound(ctx, CGRect(x: r.minX, y: r.minY, width: r.width, height: 30 * u), 8 * u, color(0.95, 0.95, 0.95))
        ctx.setFillColor(color(0.95, 0.95, 0.95))
        ctx.fill(CGRect(x: r.minX, y: r.minY + 20 * u, width: r.width, height: 10 * u))
        text(ctx, title, at: CGPoint(x: r.minX + 14 * u, y: r.minY + 8 * u), size: 12 * u, color: color(0.2, 0.2, 0.2))
    }

    private func drawDynamic(_ ctx: CGContext, time: Date) {
        let r = editorRect
        let body = typed.isEmpty
            ? (zh ? "这是模拟的远程桌面画面。\n在这里点击、打字，\n输入会通过 InputEvent 发到被控端。" : "This is a simulated remote desktop.\nClick and type here;\ninput is sent as InputEvent.")
            : typed
        text(ctx, body + (Int(time.timeIntervalSince1970 * 2) % 2 == 0 ? "|" : " "), at: CGPoint(x: r.minX + 16 * u, y: r.minY + 44 * u),
             size: 15 * u, color: color(0.15, 0.15, 0.15), width: r.width - 32 * u)
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss"
        let clock = f.string(from: time)
        if isWindows {
            text(ctx, clock, at: CGPoint(x: size.width - 90 * u, y: size.height - 34 * u), size: 13 * u, color: color(0.1, 0.1, 0.1))
        } else {
            text(ctx, clock, at: CGPoint(x: size.width - 90 * u, y: 5 * u), size: 13 * u, color: color(1, 1, 1))
        }
        clicks.removeAll { time.timeIntervalSince($0.1) > 0.6 }
        for (p, t) in clicks {
            let k = CGFloat(time.timeIntervalSince(t) / 0.6)
            let rad = (8 + 26 * k) * u
            ctx.setStrokeColor(color(1, 0.8, 0, 0.9 * (1 - k)))
            ctx.setLineWidth(3 * u)
            ctx.strokeEllipse(in: CGRect(x: p.x - rad, y: p.y - rad, width: rad * 2, height: rad * 2))
        }
    }

    /// 在 y 向下的上下文里画文字：左上角对齐；给了 width 就按宽度折行
    private func text(_ ctx: CGContext, _ s: String, at p: CGPoint, size fontSize: CGFloat, color c: CGColor, width: CGFloat? = nil, center: Bool = false) {
        let font = CTFontCreateUIFontForLanguage(.system, fontSize, nil) ?? CTFontCreateWithName("Helvetica" as CFString, fontSize, nil)
        let attrs: [NSAttributedString.Key: Any] = [NSAttributedString.Key(kCTFontAttributeName as String): font,
                                                    NSAttributedString.Key(kCTForegroundColorAttributeName as String): c]
        let ascent = CTFontGetAscent(font), descent = CTFontGetDescent(font)
        let lineH = (ascent + descent) * 1.25
        var y = p.y
        for para in s.components(separatedBy: "\n") {
            let attr = NSAttributedString(string: para.isEmpty ? " " : para, attributes: attrs)
            let typesetter = CTTypesetterCreateWithAttributedString(attr)
            var start = 0
            let length = attr.length
            repeat {
                let count = width.map { CTTypesetterSuggestLineBreak(typesetter, start, Double($0)) } ?? (length - start)
                let n = max(1, count)
                let line = CTTypesetterCreateLine(typesetter, CFRange(location: start, length: n))
                var x = p.x
                if center, let w = width {
                    let lw = CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil))
                    x = p.x + (w - lw) / 2
                }
                ctx.saveGState()
                ctx.textMatrix = .identity
                ctx.translateBy(x: x, y: y + ascent)
                ctx.scaleBy(x: 1, y: -1)
                ctx.textPosition = .zero
                CTLineDraw(line, ctx)
                ctx.restoreGState()
                y += lineH
                start += n
            } while start < length
        }
    }

    /// 截图（AI 的 screenshot 工具、点击标注图用）：PNG
    func pngSnapshot(maxWidth: Int, mark: CGPoint? = nil) -> ImageData {
        let w = min(maxWidth > 0 ? maxWidth : Int(size.width), Int(size.width))
        let h = Int(CGFloat(w) * size.height / size.width)
        let empty = ImageData(mime: "image/png", data: Data(), width: 0, height: 0)
        guard let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                                  space: rgb, bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue) else { return empty }
        ctx.translateBy(x: 0, y: CGFloat(h))
        ctx.scaleBy(x: CGFloat(w) / size.width, y: -CGFloat(h) / size.height)
        draw(ctx, time: Date(timeIntervalSince1970: TimeInterval(MockData.fixedNow / 1000)))
        if let mark {
            let rad = 18 * u
            ctx.setStrokeColor(color(1, 0.23, 0.19))
            ctx.setLineWidth(4 * u)
            ctx.strokeEllipse(in: CGRect(x: mark.x - rad, y: mark.y - rad, width: rad * 2, height: rad * 2))
        }
        guard let img = ctx.makeImage() else { return empty }
        let out = NSMutableData()
        guard let dest = CGImageDestinationCreateWithData(out as CFMutableData, "public.png" as CFString, 1, nil) else { return empty }
        CGImageDestinationAddImage(dest, img, nil)
        guard CGImageDestinationFinalize(dest) else { return empty }
        return ImageData(mime: "image/png", data: out as Data, width: UInt32(w), height: UInt32(h))
    }

    /// 记事本「保存」按钮在截图里的位置（AI 点击演示用）
    var saveButtonPoint: CGPoint { CGPoint(x: editorRect.maxX - 39 * u, y: editorRect.minY + 15 * u) }
}

/// 模拟远端声音：不出声，只记下音量、静音与是否在播（界面开关与单元测试用）。
final class MockAudioPlayback: AudioPlayback, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("模拟对象不能由内核指针构造") }
    private let lock = NSLock()
    private var active = true
    private var _volume: Float
    private var _muted = false

    init(volume: Float) {
        _volume = min(max(volume, 0), 1)
        super.init(noHandle: NoHandle())
    }

    var volume: Float { lock.locked { _volume } }
    var muted: Bool { lock.locked { _muted } }
    override func isActive() -> Bool { lock.locked { active } }
    override func setVolume(volume: Float) { lock.locked { _volume = min(max(volume, 0), 1) } }
    override func setMuted(muted: Bool) { lock.locked { _muted = muted } }
    override func stop() { lock.locked { active = false } }
}
