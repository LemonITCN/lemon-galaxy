import AppKit

/// AI 专属光标（docs/computer-use.md 第 4 节）：一个透明、不接收鼠标、不抢焦点的小面板，画在被操作的窗口之上。
/// 与系统光标明显不同：品牌强调色描边 + 白色内填充的箭头，旁边一个小「AI」标签。
/// 每次动作前沿平滑曲线移到动作位置（约 300 ms），点击时有按下动画；`end` 或 30 秒没有动作后淡出。
///
/// 调用方在后台线程（内核线程）：这里一律 DispatchQueue.main.async，不同步等主线程（主线程可能正等内核，同步等会互锁）。
/// 调用方需要「光标先到位再点」时自己在后台线程睡 travelDuration。
/// 坐标一律是 CoreGraphics 全局坐标（主屏左上角为原点，单位点），与 AX、CGWindowList 一致。
final class AICursorOverlay: @unchecked Sendable {
    static let shared = AICursorOverlay()

    /// 光标移动时长（秒）
    static let travelDuration: Double = 0.3
    /// 没有动作多久后淡出（秒）
    static let idleTimeout: Double = 30

    /// 光标窗口的窗口号（任意线程可读）：远程键鼠注入判断「点在 LemonGalaxy 自己窗口上」时要跳过它
    static var windowNumbers: Set<Int> { numbersLock.locked { numbers } }
    private static let numbersLock = NSLock()
    nonisolated(unsafe) private static var numbers = Set<Int>()

    // 以下只在主线程访问
    private var panel: NSPanel?
    private var view: AICursorView?
    private var position: CGPoint?          // 当前箭头尖端位置（CG 全局坐标）
    private var animTimer: Timer?
    private var idleTimer: Timer?
    private var fadeSerial = 0

    /// 面板大小（点）与箭头尖端在面板里的位置（从左上角起）
    private static let panelSize = CGSize(width: 64, height: 56)
    private static let tipInset = CGPoint(x: 8, y: 8)

    /// 移到 p（在窗口 above 之上），press 为真时到位后播放按下动画
    func move(to p: CGPoint, above window: CGWindowID, press: Bool) {
        DispatchQueue.main.async { MainActor.assumeIsolated { self.doMove(to: p, above: window, press: press) } }
    }

    /// 收起（淡出）
    func hide() {
        DispatchQueue.main.async { MainActor.assumeIsolated { self.fadeOut() } }
    }

    // MARK: 主线程实现

    @MainActor private func ensurePanel() -> (NSPanel, AICursorView) {
        if let p = panel, let v = view { return (p, v) }
        let p = NSPanel(contentRect: NSRect(origin: .zero, size: Self.panelSize),
                        styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        p.isOpaque = false
        p.backgroundColor = .clear
        p.hasShadow = false
        p.ignoresMouseEvents = true
        p.hidesOnDeactivate = false
        p.isReleasedWhenClosed = false
        p.becomesKeyOnlyIfNeeded = true
        p.level = .normal
        p.collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle]
        p.animationBehavior = .none
        p.title = "LemonGalaxy AI Cursor"
        let v = AICursorView(frame: NSRect(origin: .zero, size: Self.panelSize), tip: Self.tipInset)
        p.contentView = v
        panel = p
        view = v
        Self.numbersLock.locked { Self.numbers.insert(p.windowNumber) }
        return (p, v)
    }

    /// CG 全局坐标（左上原点）的箭头尖端 → 面板在 Cocoa 屏幕坐标（主屏左下原点）里的原点
    @MainActor private func panelOrigin(tip p: CGPoint) -> NSPoint {
        let primaryHeight = NSScreen.screens.first?.frame.height ?? CGDisplayBounds(CGMainDisplayID()).height
        return NSPoint(x: p.x - Self.tipInset.x, y: primaryHeight - p.y - (Self.panelSize.height - Self.tipInset.y))
    }

    @MainActor private func doMove(to target: CGPoint, above window: CGWindowID, press: Bool) {
        let (p, v) = ensurePanel()
        fadeSerial += 1
        v.accent = Self.accentColor()
        v.pressed = false
        p.alphaValue = 1
        animTimer?.invalidate()
        animTimer = nil
        // 首次出现：从目标点左上方一点滑入，免得凭空跳出来
        let start = position ?? CGPoint(x: target.x - 60, y: target.y - 40)
        p.setFrameOrigin(panelOrigin(tip: start))
        if window != 0 {
            p.order(.above, relativeTo: Int(window))
        } else {
            p.orderFrontRegardless()
        }
        position = start
        let dx = target.x - start.x, dy = target.y - start.y
        let dist = (dx * dx + dy * dy).squareRoot()
        // 二次贝塞尔：控制点在中点沿法线偏出距离的 20%，走一段柔和的弧线
        let mid = CGPoint(x: (start.x + target.x) / 2, y: (start.y + target.y) / 2)
        let ctrl = dist > 1 ? CGPoint(x: mid.x - dy / dist * dist * 0.2, y: mid.y + dx / dist * dist * 0.2) : mid
        let begin = CACurrentMediaTime()
        let duration = dist < 2 ? 0.01 : Self.travelDuration
        let t = Timer(timeInterval: 1.0 / 60, repeats: true) { [weak self] timer in
            MainActor.assumeIsolated {
                guard let self else { timer.invalidate(); return }
                let raw = min(1, (CACurrentMediaTime() - begin) / duration)
                // 缓入缓出
                let k = raw < 0.5 ? 2 * raw * raw : 1 - pow(-2 * raw + 2, 2) / 2
                let u = 1 - k
                let pt = CGPoint(x: u * u * start.x + 2 * u * k * ctrl.x + k * k * target.x,
                                 y: u * u * start.y + 2 * u * k * ctrl.y + k * k * target.y)
                self.position = pt
                p.setFrameOrigin(self.panelOrigin(tip: pt))
                if raw >= 1 {
                    timer.invalidate()
                    self.animTimer = nil
                    if press { v.playPress() }
                }
            }
        }
        RunLoop.main.add(t, forMode: .common)
        animTimer = t
        restartIdleTimer()
    }

    @MainActor private func restartIdleTimer() {
        idleTimer?.invalidate()
        let t = Timer(timeInterval: Self.idleTimeout, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated { self?.fadeOut() }
        }
        RunLoop.main.add(t, forMode: .common)
        idleTimer = t
    }

    @MainActor private func fadeOut() {
        idleTimer?.invalidate()
        idleTimer = nil
        guard let p = panel, p.isVisible else { position = nil; return }
        fadeSerial += 1
        let serial = fadeSerial
        NSAnimationContext.runAnimationGroup({ ctx in
            ctx.duration = 0.35
            p.animator().alphaValue = 0
        }, completionHandler: { [weak self] in
            MainActor.assumeIsolated {
                guard let self, self.fadeSerial == serial else { return }
                p.orderOut(nil)
                self.position = nil
            }
        })
    }

    /// 品牌强调色：跟主控端界面设置里的配色（读不到就用默认的银河蓝）
    private static func accentColor() -> NSColor {
        let id = UserDefaults.standard.string(forKey: "palette") ?? "galaxy"
        let f = Palettes.families.first { $0.id == id } ?? Palettes.families[0]
        return NSColor(hex: f.accentLight)
    }
}

/// 箭头 + 「AI」标签
final class AICursorView: NSView {
    var accent: NSColor = NSColor(hex: "#2F6BEE") { didSet { needsDisplay = true } }
    var pressed = false { didSet { needsDisplay = true } }
    private let tip: CGPoint
    private var pressSerial = 0

    init(frame: NSRect, tip: CGPoint) {
        self.tip = tip
        super.init(frame: frame)
        wantsLayer = true
    }

    required init?(coder: NSCoder) { fatalError("不支持") }

    override var isFlipped: Bool { true }

    func playPress() {
        pressSerial += 1
        let s = pressSerial
        pressed = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.16) { [weak self] in
            guard let self, self.pressSerial == s else { return }
            self.pressed = false
        }
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let ctx = NSGraphicsContext.current?.cgContext else { return }
        ctx.clear(bounds)
        // 按下时以尖端为中心缩小一点，并在尖端画一圈扩散的强调色圆环
        let scale: CGFloat = pressed ? 0.84 : 1
        if pressed {
            ctx.setStrokeColor(accent.withAlphaComponent(0.55).cgColor)
            ctx.setLineWidth(2)
            ctx.strokeEllipse(in: CGRect(x: tip.x - 7, y: tip.y - 7, width: 14, height: 14))
        }
        ctx.saveGState()
        ctx.translateBy(x: tip.x, y: tip.y)
        ctx.scaleBy(x: scale, y: scale)
        // 箭头（尖端在原点，向右下展开）
        let arrow = CGMutablePath()
        arrow.move(to: .zero)
        arrow.addLine(to: CGPoint(x: 0, y: 22))
        arrow.addLine(to: CGPoint(x: 6, y: 16.5))
        arrow.addLine(to: CGPoint(x: 10.5, y: 25.5))
        arrow.addLine(to: CGPoint(x: 14, y: 24))
        arrow.addLine(to: CGPoint(x: 9.5, y: 15))
        arrow.addLine(to: CGPoint(x: 17, y: 15))
        arrow.closeSubpath()
        ctx.setShadow(offset: CGSize(width: 0, height: 1.5), blur: 3, color: NSColor.black.withAlphaComponent(0.35).cgColor)
        ctx.addPath(arrow)
        ctx.setFillColor(NSColor.white.cgColor)
        ctx.fillPath()
        ctx.setShadow(offset: .zero, blur: 0, color: nil)
        ctx.addPath(arrow)
        ctx.setStrokeColor(accent.cgColor)
        ctx.setLineWidth(2)
        ctx.setLineJoin(.round)
        ctx.strokePath()
        ctx.restoreGState()
        // 「AI」标签：强调色圆角底 + 白字，在箭头右下
        let label = NSAttributedString(string: "AI", attributes: [
            .font: NSFont.systemFont(ofSize: 10, weight: .bold),
            .foregroundColor: NSColor.white,
        ])
        let size = label.size()
        let box = CGRect(x: tip.x + 16, y: tip.y + 22, width: size.width + 10, height: size.height + 4)
        let path = CGPath(roundedRect: box, cornerWidth: box.height / 2, cornerHeight: box.height / 2, transform: nil)
        ctx.addPath(path)
        ctx.setFillColor(accent.cgColor)
        ctx.fillPath()
        label.draw(at: CGPoint(x: box.minX + 5, y: box.minY + 2))
    }
}
