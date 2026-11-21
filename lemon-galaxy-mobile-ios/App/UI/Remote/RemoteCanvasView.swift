import UIKit
import AVFoundation
import CoreMedia
import GalaxyKit

/// 远程画面 + 手势 + 键盘。
/// 触控模式：点哪里点哪里，长按右键，拖动即按住左键拖。
/// 鼠标（触控板）模式：单指移动光标，单指轻点左键，双指轻点右键，双指拖动滚轮，长按后移动是拖拽。
/// 两种模式都能双指捏合缩放；放大后触控模式双指拖动平移，鼠标模式画面跟随光标。
final class RemoteCanvasView: UIView, UIKeyInput {
    let displayLayer = AVSampleBufferDisplayLayer()
    private let cursorLayer = CAShapeLayer()
    var send: (([InputEvent]) -> Void)?
    var mouseMode = false { didSet { updateCursor() } }
    var viewOnly = false
    var platform: Platform = .windows
    private(set) var viewport = Viewport(containerW: 1, containerH: 1, remoteW: 1, remoteH: 1)
    private var cursor: (Int32, Int32) = (0, 0)
    private var cursorVisible = false

    // 键盘上方特殊键的粘滞修饰键
    private var mods: [String: Bool] = [:]
    lazy var keyBar: KeyBarView = makeKeyBar()

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .black
        clipsToBounds = true
        isMultipleTouchEnabled = true
        displayLayer.videoGravity = .resize
        displayLayer.backgroundColor = UIColor.black.cgColor
        layer.addSublayer(displayLayer)
        cursorLayer.path = Self.arrowPath()
        cursorLayer.fillColor = UIColor.white.cgColor
        cursorLayer.strokeColor = UIColor.black.cgColor
        cursorLayer.lineWidth = 1.2
        cursorLayer.isHidden = true
        layer.addSublayer(cursorLayer)
        // iPad 外接鼠标 / 触控板：悬停移动光标、右键、双指滚动
        addGestureRecognizer(UIHoverGestureRecognizer(target: self, action: #selector(hovered(_:))))
        let right = UITapGestureRecognizer(target: self, action: #selector(secondaryClick(_:)))
        right.buttonMaskRequired = .secondary
        addGestureRecognizer(right)
        let scroll = UIPanGestureRecognizer(target: self, action: #selector(pointerScroll(_:)))
        scroll.allowedScrollTypesMask = .continuous
        scroll.maximumNumberOfTouches = 0
        addGestureRecognizer(scroll)
    }

    required init?(coder: NSCoder) { fatalError() }

    static func enqueue(_ pb: CVPixelBuffer, on layer: AVSampleBufferDisplayLayer) {
        var format: CMVideoFormatDescription?
        guard CMVideoFormatDescriptionCreateForImageBuffer(allocator: nil, imageBuffer: pb, formatDescriptionOut: &format) == noErr, let format else { return }
        var timing = CMSampleTimingInfo(duration: .invalid, presentationTimeStamp: CMClockGetTime(CMClockGetHostTimeClock()), decodeTimeStamp: .invalid)
        var sample: CMSampleBuffer?
        guard CMSampleBufferCreateReadyWithImageBuffer(allocator: nil, imageBuffer: pb, formatDescription: format, sampleTiming: &timing,
                                                       sampleBufferOut: &sample) == noErr, let sample else { return }
        if let atts = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: true), CFArrayGetCount(atts) > 0 {
            let dict = unsafeBitCast(CFArrayGetValueAtIndex(atts, 0), to: CFMutableDictionary.self)
            CFDictionarySetValue(dict, Unmanaged.passUnretained(kCMSampleAttachmentKey_DisplayImmediately).toOpaque(),
                                 Unmanaged.passUnretained(kCFBooleanTrue).toOpaque())
        }
        if layer.status == .failed { layer.flush() }
        layer.enqueue(sample)
    }

    // MARK: 布局

    func setRemoteSize(_ w: Int, _ h: Int) {
        viewport = viewport.resized(containerW: bounds.width, containerH: bounds.height, remoteW: max(w, 1), remoteH: max(h, 1))
        cursor = (Int32(w / 2), Int32(h / 2))
        applyViewport()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        viewport = viewport.resized(containerW: bounds.width, containerH: bounds.height, remoteW: viewport.remoteW, remoteH: viewport.remoteH)
        applyViewport()
    }

    private func applyViewport() {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        displayLayer.frame = viewport.contentRect
        CATransaction.commit()
        updateCursor()
    }

    func remoteCursor(_ x: Int32, _ y: Int32, visible: Bool) {
        cursor = (x, y)
        cursorVisible = visible
        updateCursor()
    }

    private func updateCursor() {
        let p = viewport.toView(cursor.0, cursor.1)
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        cursorLayer.position = p
        cursorLayer.isHidden = !(mouseMode || cursorVisible)
        CATransaction.commit()
    }

    private static func arrowPath() -> CGPath {
        let s: CGFloat = 1
        let p = UIBezierPath()
        p.move(to: .zero)
        p.addLine(to: CGPoint(x: 0, y: 19 * s))
        p.addLine(to: CGPoint(x: 4.5 * s, y: 14.8 * s))
        p.addLine(to: CGPoint(x: 7.6 * s, y: 21.6 * s))
        p.addLine(to: CGPoint(x: 10.6 * s, y: 20.3 * s))
        p.addLine(to: CGPoint(x: 7.6 * s, y: 13.6 * s))
        p.addLine(to: CGPoint(x: 13.6 * s, y: 13.6 * s))
        p.close()
        return p.cgPath
    }

    // MARK: 输入事件

    private func remote(_ pt: CGPoint) -> (Int32, Int32) { viewport.toRemote(pt.x, pt.y) }

    private func click(_ b: PointerButton, _ x: Int32, _ y: Int32) -> [InputEvent] {
        [.mouseMove(x: x, y: y), .mouseButton(button: b, down: true, x: x, y: y), .mouseButton(button: b, down: false, x: x, y: y)]
    }

    private func emit(_ events: [InputEvent]) {
        for e in events {
            switch e {
            case .mouseMove(let x, let y), .mouseButton(_, _, let x, let y): cursor = (x, y)
            default: break
            }
        }
        updateCursor()
        send?(events)
    }

    // 手势状态机（与安卓端一致）
    private var touches = Set<UITouch>()
    private var downAt = CGPoint.zero
    private var lastPos = CGPoint.zero
    private var maxPointers = 0
    private var moved = false
    private var longPressed = false
    private var dragging = false
    private var dragButtonDown = false
    private var twoMode = 0          // 0 未定 1 缩放 2 滚动或平移
    private var twoStartCenter = CGPoint.zero
    private var twoStartSpan: CGFloat = 0
    private var prevCenter = CGPoint.zero
    private var prevSpan: CGFloat = 0
    private var longPressTimer: Timer?
    private var scrollRemainder = CGPoint.zero
    private let slop: CGFloat = 10

    private func centroid(_ ps: [CGPoint]) -> CGPoint {
        CGPoint(x: ps.map(\.x).reduce(0, +) / CGFloat(ps.count), y: ps.map(\.y).reduce(0, +) / CGFloat(ps.count))
    }

    private func span(_ ps: [CGPoint], _ c: CGPoint) -> CGFloat {
        ps.map { hypot($0.x - c.x, $0.y - c.y) }.reduce(0, +) / CGFloat(ps.count)
    }

    override func touchesBegan(_ set: Set<UITouch>, with event: UIEvent?) {
        let wasEmpty = touches.isEmpty
        touches.formUnion(set)
        if wasEmpty {
            downAt = set.first!.location(in: self)
            lastPos = downAt
            maxPointers = touches.count
            moved = false
            longPressed = false
            dragging = false
            twoMode = 0
            twoStartSpan = 0
            longPressTimer?.invalidate()
            longPressTimer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: false) { [weak self] _ in
                DispatchQueue.main.async { self?.longPressFired() }
            }
        } else {
            maxPointers = max(maxPointers, touches.count)
            longPressTimer?.invalidate()
        }
    }

    private func longPressFired() {
        guard touches.count == 1, !moved, maxPointers == 1 else { return }
        longPressed = true
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        if !mouseMode {
            let (x, y) = remote(downAt)
            emit(click(.right, x, y))
        }
    }

    override func touchesMoved(_ set: Set<UITouch>, with event: UIEvent?) {
        let pts = touches.map { $0.location(in: self) }
        if touches.count >= 2 {
            longPressTimer?.invalidate()
            if dragging { dragEnd() }
            let c = centroid(pts), s = span(pts, c)
            if twoStartSpan == 0 {
                twoStartCenter = c; twoStartSpan = s; prevCenter = c; prevSpan = s
                return
            }
            if twoMode == 0 {
                let ds = abs(s - twoStartSpan), dc = hypot(c.x - twoStartCenter.x, c.y - twoStartCenter.y)
                if ds > slop * 1.5 && ds > dc * 0.8 { twoMode = 1 } else if dc > slop { twoMode = 2 }
            }
            switch twoMode {
            case 1:
                viewport = viewport.zoomed(prevSpan > 0 ? s / prevSpan : 1, fx: c.x, fy: c.y, dx: c.x - prevCenter.x, dy: c.y - prevCenter.y)
                applyViewport()
            case 2:
                if !mouseMode && viewport.zoom > 1.01 {
                    viewport = viewport.panned(c.x - prevCenter.x, c.y - prevCenter.y)
                    applyViewport()
                } else {
                    scroll(dx: c.x - prevCenter.x, dy: c.y - prevCenter.y)
                }
            default: break
            }
            prevCenter = c
            prevSpan = s
            if twoMode != 0 { moved = true }
            return
        }
        guard maxPointers < 2, let t = touches.first else { return }
        let pos = t.location(in: self)
        if !moved, hypot(pos.x - downAt.x, pos.y - downAt.y) > slop {
            moved = true
            dragging = true
            longPressTimer?.invalidate()
            dragStart(afterLongPress: longPressed)
            lastPos = downAt
        }
        if dragging {
            dragMove(to: pos, delta: CGPoint(x: pos.x - lastPos.x, y: pos.y - lastPos.y))
            lastPos = pos
        }
    }

    override func touchesEnded(_ set: Set<UITouch>, with event: UIEvent?) { finish(set) }
    override func touchesCancelled(_ set: Set<UITouch>, with event: UIEvent?) { finish(set) }

    private func finish(_ set: Set<UITouch>) {
        touches.subtract(set)
        guard touches.isEmpty else { return }
        longPressTimer?.invalidate()
        if dragging {
            dragEnd()
        } else if maxPointers >= 2 && twoMode == 0 {
            let (x, y) = cursor
            emit(click(.right, x, y))
        } else if maxPointers == 1 && !moved && !longPressed {
            if mouseMode {
                let (x, y) = cursor
                emit(click(.left, x, y))
            } else {
                let (x, y) = remote(downAt)
                emit(click(.left, x, y))
            }
        }
        dragging = false
    }

    private func dragStart(afterLongPress: Bool) {
        if mouseMode {
            if afterLongPress {
                let (x, y) = cursor
                emit([.mouseButton(button: .left, down: true, x: x, y: y)])
                dragButtonDown = true
            }
        } else if !afterLongPress {
            let (x, y) = remote(downAt)
            emit([.mouseMove(x: x, y: y), .mouseButton(button: .left, down: true, x: x, y: y)])
            dragButtonDown = true
        }
    }

    private func dragMove(to pos: CGPoint, delta: CGPoint) {
        if mouseMode {
            // 触控板：相对移动，带一点加速
            let k = viewport.remotePerViewPoint * 1.6
            let nx = min(max(Int32((CGFloat(cursor.0) + delta.x * k).rounded()), 0), Int32(viewport.remoteW - 1))
            let ny = min(max(Int32((CGFloat(cursor.1) + delta.y * k).rounded()), 0), Int32(viewport.remoteH - 1))
            emit([.mouseMove(x: nx, y: ny)])
            let followed = viewport.follow(nx, ny)
            if followed != viewport { viewport = followed; applyViewport() }
        } else if dragButtonDown {
            let (x, y) = remote(pos)
            emit([.mouseMove(x: x, y: y)])
        }
    }

    private func dragEnd() {
        if dragButtonDown {
            let (x, y) = cursor
            emit([.mouseButton(button: .left, down: false, x: x, y: y)])
            dragButtonDown = false
        }
        dragging = false
    }

    /// 手指上推 = 内容上移 = 滚轮向下；按视图点数换算成被控像素
    private func scroll(dx: CGFloat, dy: CGFloat) {
        let k = viewport.remotePerViewPoint
        scrollRemainder.x += -dx * k
        scrollRemainder.y += -dy * k
        let ix = Int32(scrollRemainder.x.rounded(.towardZero)), iy = Int32(scrollRemainder.y.rounded(.towardZero))
        guard ix != 0 || iy != 0 else { return }
        scrollRemainder.x -= CGFloat(ix)
        scrollRemainder.y -= CGFloat(iy)
        emit([.wheel(dx: ix, dy: iy)])
    }

    @objc private func hovered(_ g: UIHoverGestureRecognizer) {
        guard g.state == .changed || g.state == .began else { return }
        let (x, y) = remote(g.location(in: self))
        emit([.mouseMove(x: x, y: y)])
    }

    @objc private func secondaryClick(_ g: UITapGestureRecognizer) {
        let (x, y) = remote(g.location(in: self))
        emit(click(.right, x, y))
    }

    @objc private func pointerScroll(_ g: UIPanGestureRecognizer) {
        let t = g.translation(in: self)
        g.setTranslation(.zero, in: self)
        scroll(dx: t.x, dy: t.y)
    }

    // MARK: 键盘（软键盘文字 → Text 事件；外接键盘按键 → HID 用法码）

    override var canBecomeFirstResponder: Bool { true }
    override var inputAccessoryView: UIView? { keyBar }
    var hasText: Bool { true }
    var autocorrectionType: UITextAutocorrectionType { get { .no } set {} }
    var autocapitalizationType: UITextAutocapitalizationType { get { .none } set {} }
    var smartQuotesType: UITextSmartQuotesType { get { .no } set {} }
    var smartDashesType: UITextSmartDashesType { get { .no } set {} }
    var spellCheckingType: UITextSpellCheckingType { get { .no } set {} }
    var keyboardAppearance: UIKeyboardAppearance { get { .dark } set {} }

    private var activeMods: [UInt32] {
        var m: [UInt32] = []
        if mods["ctrl"] == true { m.append(Hid.ctrl) }
        if mods["alt"] == true { m.append(Hid.alt) }
        if mods["shift"] == true { m.append(Hid.shift) }
        if mods["gui"] == true { m.append(Hid.gui) }
        return m
    }

    private func consumeMods() {
        mods.removeAll()
        keyBar.clearToggles()
    }

    func insertText(_ text: String) {
        let m = activeMods
        if !m.isEmpty {
            var events: [InputEvent] = []
            for ch in text {
                guard let (usage, shift) = Hid.usage(for: ch) else { continue }
                var keys: [UInt32] = m
                if shift && !m.contains(Hid.shift) { keys.append(Hid.shift) }
                keys.append(usage)
                events += Hid.combo(keys)
            }
            consumeMods()
            emit(events)
            return
        }
        if text == "\n" {
            emit(Hid.tap(Hid.enter))
        } else {
            emit([.text(text: text)])
        }
    }

    func deleteBackward() {
        let m = activeMods
        if !m.isEmpty { consumeMods() }
        emit(Hid.combo(m + [Hid.backspace]))
    }

    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        var handled = false
        for p in presses {
            guard let key = p.key else { continue }
            emit([.key(hidUsage: UInt32(key.keyCode.rawValue), down: true)])
            handled = true
        }
        if !handled { super.pressesBegan(presses, with: event) }
    }

    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        var handled = false
        for p in presses {
            guard let key = p.key else { continue }
            emit([.key(hidUsage: UInt32(key.keyCode.rawValue), down: false)])
            handled = true
        }
        if !handled { super.pressesEnded(presses, with: event) }
    }

    override func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        pressesEnded(presses, with: event)
    }

    var onHideKeyboard: (() -> Void)?

    private func makeKeyBar() -> KeyBarView {
        typealias I = KeyBarView.Item
        let mac = platform == .macos
        var items: [I] = [
            .action(id: "hide", symbol: "keyboard.chevron.compact.down"),
            .key(id: "esc", title: "esc", symbol: nil),
            .key(id: "tab", title: "tab", symbol: nil),
            .toggle(id: "ctrl", title: mac ? "⌃" : "ctrl"),
            .toggle(id: "alt", title: mac ? "⌥" : "alt"),
            .toggle(id: "gui", title: mac ? "⌘" : "win"),
            .toggle(id: "shift", title: "⇧"),
            .key(id: "left", title: nil, symbol: "arrow.left"),
            .key(id: "up", title: nil, symbol: "arrow.up"),
            .key(id: "down", title: nil, symbol: "arrow.down"),
            .key(id: "right", title: nil, symbol: "arrow.right"),
            .key(id: "delete", title: "del", symbol: nil),
            .separator,
            .key(id: "home", title: "home", symbol: nil), .key(id: "end", title: "end", symbol: nil),
            .key(id: "pageUp", title: "pgup", symbol: nil), .key(id: "pageDown", title: "pgdn", symbol: nil),
        ]
        for n in 1...12 { items.append(.key(id: "f\(n)", title: "F\(n)", symbol: nil)) }
        let bar = KeyBarView(items: items)
        bar.onToggle = { [weak self] id, on in self?.mods[id] = on }
        bar.onKey = { [weak self] id in self?.barKey(id) }
        return bar
    }

    private func barKey(_ id: String) {
        let usage: UInt32?
        switch id {
        case "hide":
            onHideKeyboard?()
            return
        case "esc": usage = Hid.esc
        case "tab": usage = Hid.tab
        case "left": usage = Hid.left
        case "right": usage = Hid.right
        case "up": usage = Hid.up
        case "down": usage = Hid.down
        case "delete": usage = Hid.delete
        case "home": usage = Hid.home
        case "end": usage = Hid.end
        case "pageUp": usage = Hid.pageUp
        case "pageDown": usage = Hid.pageDown
        default:
            if id.hasPrefix("f"), let n = Int(id.dropFirst()) { usage = Hid.f(n) } else { usage = nil }
        }
        guard let usage else { return }
        let m = activeMods
        consumeMods()
        emit(Hid.combo(m + [usage]))
    }
}
