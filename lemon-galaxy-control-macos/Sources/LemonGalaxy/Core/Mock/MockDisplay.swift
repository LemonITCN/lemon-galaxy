import AppKit
import CoreVideo

/// 假远程桌面：本机用 CoreGraphics 画一张「被控端桌面」，经 VideoToolbox 编码成 H.264 Annex B，
/// 再走 VideoSink（on_config / on_frame / on_cursor）——和真内核推来的数据路径完全一样，界面的解码显示因此能被真实验证。
final class MockDisplayChannel: DisplayChannel, @unchecked Sendable {
    required init(unsafeFromRawPointer pointer: UnsafeMutableRawPointer) { fatalError("模拟对象不能由内核指针构造") }
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

    init(device: DeviceInfo, display: DisplayInfo, options: DisplayOptions, sink: VideoSink, zh: Bool) {
        self.device = device
        self.display = display
        self.sink = sink
        // 模拟端不按原始分辨率出流（CPU 画图太慢），长边限制在 1600，宽高取偶数
        let scale = min(1, 1600 / Double(max(display.width, display.height)))
        streamW = Int(Double(display.width) * scale) & ~1
        streamH = Int(Double(display.height) * scale) & ~1
        renderer = MockDesktopRenderer(device: device, display: display, zh: zh, size: CGSize(width: streamW, height: streamH))
        codec = options.codecs.contains(.h265) && !options.codecs.contains(.h264) ? .h265 : .h264
        super.init(noPointer: NoPointer())
        let fps = max(5, min(Int(options.maxFps), 30))
        queue.async { [weak self] in self?.start(fps: fps, bitrate: Int(options.bitrateKbps)) }
    }

    deinit { timer?.cancel() }

    private func start(fps: Int, bitrate: Int) {
        encoder = VideoEncoder(codec: codec, width: streamW, height: streamH, fps: fps, bitrateKbps: bitrate) { [weak self] data, pts, key in
            self?.queue.async { self?.sink?.onFrame(data: data, ptsUs: pts, keyframe: key) }
        }
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
        encoder.encode(pb, ptsUs: frameIndex * 33_333)
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
            case .text(let t):
                self.renderer.typed += t
            case .key(let usage, let down):
                if down, usage == 0x2A, !self.renderer.typed.isEmpty { self.renderer.typed.removeLast() }
                if down, usage == 0x28 { self.renderer.typed += "\n" }
            case .wheel, .specialCombo, .touch:
                break
            }
        }
    }

    override func requestKeyframe() { encoder?.requestKeyframe() }
    override func setBitrate(kbps: UInt32) { encoder?.setBitrate(kbps: Int(kbps)) }

    override func stop() {
        queue.async { [weak self] in
            self?.stopped = true
            self?.timer?.cancel()
            self?.encoder?.invalidate()
        }
    }
}

/// 画一张假桌面。Windows 风格（底部任务栏）或 macOS 风格（顶部菜单栏 + 程序坞）。
final class MockDesktopRenderer {
    let device: DeviceInfo
    let display: DisplayInfo
    let zh: Bool
    let size: CGSize
    var cursor = CGPoint(x: -100, y: -100)
    var clicks: [(CGPoint, Date)] = []
    var typed = ""
    private var background: CGImage?

    init(device: DeviceInfo, display: DisplayInfo, zh: Bool, size: CGSize? = nil) {
        self.device = device
        self.display = display
        self.zh = zh
        self.size = size ?? CGSize(width: Int(display.width) / 2, height: Int(display.height) / 2)
    }

    private var isWindows: Bool { device.platform == .windows }

    func draw(_ ctx: CGContext, time: Date) {
        if background == nil { background = makeBackground() }
        if let bg = background {
            // 上下文已翻转为 y 向下，画图片要再翻回来
            ctx.saveGState()
            ctx.translateBy(x: 0, y: size.height)
            ctx.scaleBy(x: 1, y: -1)
            ctx.draw(bg, in: CGRect(origin: .zero, size: size))
            ctx.restoreGState()
        }
        let ns = NSGraphicsContext(cgContext: ctx, flipped: true)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = ns
        drawDynamic(time: time)
        NSGraphicsContext.restoreGraphicsState()
    }

    private func makeBackground() -> CGImage? {
        let w = Int(size.width), h = Int(size.height)
        guard let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                                  space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue) else { return nil }
        ctx.translateBy(x: 0, y: CGFloat(h))
        ctx.scaleBy(x: 1, y: -1)
        let ns = NSGraphicsContext(cgContext: ctx, flipped: true)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = ns
        drawStatic()
        NSGraphicsContext.restoreGraphicsState()
        return ctx.makeImage()
    }

    private var u: CGFloat { size.height / 900 }

    private func drawStatic() {
        let rect = CGRect(origin: .zero, size: size)
        let colors = isWindows
            ? [NSColor(srgbRed: 0.05, green: 0.27, blue: 0.62, alpha: 1), NSColor(srgbRed: 0.33, green: 0.62, blue: 0.93, alpha: 1)]
            : [NSColor(srgbRed: 0.18, green: 0.12, blue: 0.38, alpha: 1), NSColor(srgbRed: 0.86, green: 0.45, blue: 0.42, alpha: 1)]
        NSGradient(colors: colors)?.draw(in: rect, angle: isWindows ? -60 : -90)
        // 柔和的光斑
        for (i, c) in [(0.2, 0.3), (0.75, 0.6), (0.5, 0.85)].enumerated() {
            let r = size.height * CGFloat(0.35 + Double(i) * 0.1)
            let center = CGPoint(x: size.width * CGFloat(c.0), y: size.height * CGFloat(c.1))
            NSGradient(colors: [NSColor.white.withAlphaComponent(0.10), NSColor.white.withAlphaComponent(0)])?
                .draw(in: NSBezierPath(ovalIn: CGRect(x: center.x - r, y: center.y - r, width: r * 2, height: r * 2)), relativeCenterPosition: .zero)
        }
        if isWindows {
            // 桌面图标
            let icons = zh ? ["此电脑", "回收站", "文档", "LemonGalaxy"] : ["This PC", "Recycle Bin", "Documents", "LemonGalaxy"]
            for (i, name) in icons.enumerated() {
                let r = CGRect(x: 24 * u, y: (24 + CGFloat(i) * 96) * u, width: 56 * u, height: 48 * u)
                NSColor.white.withAlphaComponent(0.85).setFill()
                NSBezierPath(roundedRect: r.insetBy(dx: 8 * u, dy: 4 * u), xRadius: 6 * u, yRadius: 6 * u).fill()
                text(name, at: CGPoint(x: r.minX - 12 * u, y: r.maxY + 4 * u), size: 12 * u, color: .white, width: 80 * u, center: true)
            }
            // 任务栏
            let bar = CGRect(x: 0, y: size.height - 48 * u, width: size.width, height: 48 * u)
            NSColor(srgbRed: 0.93, green: 0.94, blue: 0.96, alpha: 0.92).setFill()
            bar.fill()
            for i in 0..<6 {
                let r = CGRect(x: size.width / 2 - 150 * u + CGFloat(i) * 50 * u, y: bar.minY + 8 * u, width: 32 * u, height: 32 * u)
                ([NSColor.systemBlue, .systemTeal, .systemOrange, .systemGreen, .systemPurple, .systemYellow][i]).setFill()
                NSBezierPath(roundedRect: r, xRadius: 6 * u, yRadius: 6 * u).fill()
            }
        } else {
            let bar = CGRect(x: 0, y: 0, width: size.width, height: 26 * u)
            NSColor.black.withAlphaComponent(0.25).setFill()
            bar.fill()
            text(zh ? "访达   文件   编辑   显示   前往   窗口   帮助" : "Finder   File   Edit   View   Go   Window   Help",
                 at: CGPoint(x: 40 * u, y: 5 * u), size: 13 * u, color: .white)
            let dock = CGRect(x: size.width / 2 - 240 * u, y: size.height - 74 * u, width: 480 * u, height: 64 * u)
            NSColor.white.withAlphaComponent(0.28).setFill()
            NSBezierPath(roundedRect: dock, xRadius: 18 * u, yRadius: 18 * u).fill()
            for i in 0..<8 {
                let r = CGRect(x: dock.minX + 14 * u + CGFloat(i) * 58 * u, y: dock.minY + 8 * u, width: 48 * u, height: 48 * u)
                ([NSColor.systemBlue, .systemGray, .systemGreen, .systemOrange, .systemRed, .systemPurple, .systemTeal, .systemIndigo][i]).setFill()
                NSBezierPath(roundedRect: r, xRadius: 11 * u, yRadius: 11 * u).fill()
            }
        }
        // 两个窗口：资源监视 + 记事本（记事本里的文字是动态的）
        window(CGRect(x: size.width * 0.52, y: size.height * 0.12, width: size.width * 0.4, height: size.height * 0.42),
               title: zh ? "资源监视器" : "Resource Monitor")
        let m = CGRect(x: size.width * 0.52, y: size.height * 0.12, width: size.width * 0.4, height: size.height * 0.42)
        for (i, v) in [0.42, 0.67, 0.31, 0.55, 0.78, 0.49, 0.62, 0.38, 0.71, 0.44, 0.58, 0.66].enumerated() {
            let bw = (m.width - 40 * u) / 12
            let bh = (m.height - 90 * u) * CGFloat(v)
            NSColor.systemBlue.withAlphaComponent(0.75).setFill()
            NSBezierPath(roundedRect: CGRect(x: m.minX + 20 * u + CGFloat(i) * bw + 2 * u, y: m.maxY - 20 * u - bh, width: bw - 4 * u, height: bh),
                         xRadius: 2 * u, yRadius: 2 * u).fill()
        }
        window(editorRect, title: zh ? "无标题 - 记事本" : "Untitled - Notepad")
    }

    private var editorRect: CGRect {
        CGRect(x: size.width * 0.1, y: size.height * 0.2, width: size.width * 0.38, height: size.height * 0.5)
    }

    private func window(_ r: CGRect, title: String) {
        let shadow = NSShadow()
        shadow.shadowBlurRadius = 18 * u
        shadow.shadowOffset = NSSize(width: 0, height: -6 * u)
        shadow.shadowColor = NSColor.black.withAlphaComponent(0.35)
        NSGraphicsContext.saveGraphicsState()
        shadow.set()
        NSColor.white.setFill()
        NSBezierPath(roundedRect: r, xRadius: 8 * u, yRadius: 8 * u).fill()
        NSGraphicsContext.restoreGraphicsState()
        NSColor(white: 0.95, alpha: 1).setFill()
        let titleBar = NSBezierPath(roundedRect: CGRect(x: r.minX, y: r.minY, width: r.width, height: 30 * u), xRadius: 8 * u, yRadius: 8 * u)
        titleBar.fill()
        CGRect(x: r.minX, y: r.minY + 20 * u, width: r.width, height: 10 * u).fill()
        text(title, at: CGPoint(x: r.minX + 14 * u, y: r.minY + 8 * u), size: 12 * u, color: NSColor(white: 0.2, alpha: 1))
    }

    private func drawDynamic(time: Date) {
        // 记事本内容
        let r = editorRect
        let body = typed.isEmpty
            ? (zh ? "这是模拟的远程桌面画面。\n在这里点击、打字，\n输入会通过 InputEvent 发到被控端。" : "This is a simulated remote desktop.\nClick and type here;\ninput is sent as InputEvent.")
            : typed
        text(body + (Int(time.timeIntervalSince1970 * 2) % 2 == 0 ? "|" : " "), at: CGPoint(x: r.minX + 16 * u, y: r.minY + 44 * u),
             size: 15 * u, color: NSColor(white: 0.15, alpha: 1), width: r.width - 32 * u)
        // 时钟
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss"
        let clock = f.string(from: time)
        if isWindows {
            text(clock, at: CGPoint(x: size.width - 90 * u, y: size.height - 34 * u), size: 13 * u, color: NSColor(white: 0.1, alpha: 1))
        } else {
            text(clock, at: CGPoint(x: size.width - 90 * u, y: 5 * u), size: 13 * u, color: .white)
        }
        // 点击涟漪
        clicks.removeAll { time.timeIntervalSince($0.1) > 0.6 }
        for (p, t) in clicks {
            let k = CGFloat(time.timeIntervalSince(t) / 0.6)
            let rad = (8 + 26 * k) * u
            NSColor.systemYellow.withAlphaComponent(0.9 * (1 - k)).setStroke()
            let path = NSBezierPath(ovalIn: CGRect(x: p.x - rad, y: p.y - rad, width: rad * 2, height: rad * 2))
            path.lineWidth = 3 * u
            path.stroke()
        }
    }

    private func text(_ s: String, at p: CGPoint, size: CGFloat, color: NSColor, width: CGFloat? = nil, center: Bool = false) {
        let para = NSMutableParagraphStyle()
        para.alignment = center ? .center : .left
        let attrs: [NSAttributedString.Key: Any] = [.font: NSFont.systemFont(ofSize: size), .foregroundColor: color, .paragraphStyle: para]
        if let width {
            (s as NSString).draw(in: CGRect(x: p.x, y: p.y, width: width, height: 2000), withAttributes: attrs)
        } else {
            (s as NSString).draw(at: p, withAttributes: attrs)
        }
    }

    /// 截图（AI 的 screenshot 工具、点击标注图用）：PNG。
    func pngSnapshot(maxWidth: Int, mark: CGPoint? = nil) -> ImageData {
        let w = min(maxWidth > 0 ? maxWidth : Int(size.width), Int(size.width))
        let h = Int(CGFloat(w) * size.height / size.width)
        guard let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                                  space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue) else {
            return ImageData(mime: "image/png", data: Data(), width: 0, height: 0)
        }
        ctx.translateBy(x: 0, y: CGFloat(h))
        ctx.scaleBy(x: CGFloat(w) / size.width, y: -CGFloat(h) / size.height)
        draw(ctx, time: Date(timeIntervalSince1970: TimeInterval(MockData.fixedNow / 1000)))
        if let mark {
            let ns = NSGraphicsContext(cgContext: ctx, flipped: true)
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = ns
            NSColor.systemRed.setStroke()
            let rad = 18 * u
            let path = NSBezierPath(ovalIn: CGRect(x: mark.x - rad, y: mark.y - rad, width: rad * 2, height: rad * 2))
            path.lineWidth = 4 * u
            path.stroke()
            NSGraphicsContext.restoreGraphicsState()
        }
        guard let img = ctx.makeImage() else { return ImageData(mime: "image/png", data: Data(), width: 0, height: 0) }
        let rep = NSBitmapImageRep(cgImage: img)
        let data = rep.representation(using: .png, properties: [:]) ?? Data()
        return ImageData(mime: "image/png", data: data, width: UInt32(w), height: UInt32(h))
    }

    /// 记事本「保存」按钮在截图里的位置（AI 点击演示用）
    var saveButtonPoint: CGPoint { CGPoint(x: editorRect.maxX - 40 * u, y: editorRect.minY + 15 * u) }
}
