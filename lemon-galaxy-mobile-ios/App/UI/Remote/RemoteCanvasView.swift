import UIKit
import AVFoundation
import CoreMedia
import GalaxyKit

/// 远程画面 + 手势 + 键盘。
/// 触控模式：点哪里点哪里，长按右键，拖动即按住左键拖。
/// 鼠标（触控板）模式：单指移动光标，单指轻点左键，双指轻点右键，双指拖动滚轮，长按后移动是拖拽。
/// 两种模式都能双指捏合缩放（以双指中心为准，画面始终保持被控屏幕比例），双指双击在适应屏幕与放大之间切换；
/// 放大后触控模式双指拖动平移，鼠标模式画面跟随光标。单指双击不拦截，照常发两次单击（被控端收到的就是双击）。
/// 光标位置由本机记（LocalPointer，手指一动立即更新）。鼠标模式且被控端支持 "client_cursor" 时，
/// 被控端画面里不画光标，箭头在这里本地画（跟手）；其余情况光标在画面里，这里不再叠加。
final class RemoteCanvasView: UIView, UIKeyInput {
    /// 画面摆放变化的原因：捏合 / 双指双击要提示倍数，其余（布局、菜单里的适应屏幕等）不提示
    enum ViewportChange { case gesture, other }

    let displayLayer = AVSampleBufferDisplayLayer()
    var send: (([InputEvent]) -> Void)?
    /// 悬浮鼠标按键按着或锁定（正在拖拽）：画面轻点不再单击
    var keysBusy: (() -> Bool)?
    /// 鼠标模式下光标要移动了（悬浮鼠标按键刚按下还没定性时先按下）
    var beforePointerMove: (() -> Void)?
    var onViewportChange: ((Viewport, ViewportChange) -> Void)?
    var mouseMode = false { didSet { updatePointerLayer() } }
    /// 被控端支持 "client_cursor"（鼠标模式下画面里不画光标，由这里画）
    var clientCursor = false { didSet { updatePointerLayer() } }
    /// 画面在出图（连接中、断开时不画箭头）
    var streaming = false { didSet { updatePointerLayer() } }
    var viewOnly = false
    var platform: Platform = .windows
    private(set) var viewport = Viewport(containerW: 1, containerH: 1, remoteW: 1, remoteH: 1)
    private var pointer = LocalPointer()
    private var cursor: (Int32, Int32) { (pointer.x, pointer.y) }
    /// 本地画的箭头（尖端 = 热点）。大小固定、不随画面缩放
    private let pointerLayer = CAShapeLayer()

    // 键盘上方特殊键的粘滞修饰键
    private var mods: [String: Bool] = [:]
    lazy var keyBar: KeyBarView = makeKeyBar()

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .black
        clipsToBounds = true
        isMultipleTouchEnabled = true
        // 图层大小始终按被控屏幕比例摆放，拉伸填满即不变形
        displayLayer.videoGravity = .resize
        displayLayer.backgroundColor = UIColor.black.cgColor
        layer.addSublayer(displayLayer)
        pointerLayer.path = PointerArrow.path()
        pointerLayer.fillColor = UIColor.white.cgColor
        pointerLayer.strokeColor = UIColor.black.cgColor
        pointerLayer.lineWidth = PointerArrow.strokeWidth * PointerArrow.unit
        pointerLayer.lineJoin = .round
        pointerLayer.bounds = .zero
        pointerLayer.isHidden = true
        layer.addSublayer(pointerLayer)
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

    /// 被控屏幕分辨率（换屏、对方改分辨率时回到适应屏幕；同一分辨率重开画面保持缩放）
    func setRemoteSize(_ w: Int, _ h: Int) {
        let changed = w != viewport.remoteW || h != viewport.remoteH
        if changed { pointer.reset(Int32(w / 2), Int32(h / 2)) }
        setViewport(viewport.resized(containerW: bounds.width, containerH: bounds.height, remoteW: w, remoteH: h), .other)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        // 旋转、弹出键盘：保持倍数，原来在中心的画面点仍在中心
        setViewport(viewport.resized(containerW: bounds.width, containerH: bounds.height, remoteW: viewport.remoteW, remoteH: viewport.remoteH), .other)
    }

    /// 回到适应屏幕（菜单「视图」）
    func fitToScreen() { setViewport(viewport.fit(), .other) }

    /// 原始像素：1 个被控像素占 1 个视图点，以视图中心为准
    func actualSize() {
        setViewport(viewport.zoomedTo(viewport.actualSizeZoom, fx: bounds.midX, fy: bounds.midY), .other)
    }

    private func setViewport(_ v: Viewport, _ reason: ViewportChange) {
        let old = viewport
        viewport = v
        if v.contentRect != displayLayer.frame {
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            displayLayer.frame = v.contentRect
            CATransaction.commit()
        }
        if v != old { updatePointerLayer() }
        if v != old || reason == .gesture { onViewportChange?(v, reason) }
    }

    /// 换屏幕：光标先放屏幕中心（被控端有回报后以回报为准）
    func resetPointer() {
        pointer.reset(Int32(viewport.remoteW / 2), Int32(viewport.remoteH / 2))
        updatePointerLayer()
    }

    /// 被控端报来的光标位置（被控屏幕像素）：本机刚动过时是自己移动的回声，不采用（见 LocalPointer）
    func remoteCursor(_ x: Int32, _ y: Int32, visible: Bool) {
        pointer.hostReport(x, y, visible: visible, now: CACurrentMediaTime())
        updatePointerLayer()
    }

    /// 摆放本地箭头：尖端对准光标所在被控像素的中心，随画面缩放平移换算
    private func updatePointerLayer() {
        let show = mouseMode && clientCursor && streaming && pointer.visible
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        pointerLayer.isHidden = !show
        if show { pointerLayer.position = viewport.toView(pointer.x, pointer.y) }
        CATransaction.commit()
    }

    // MARK: 输入事件

    private func remote(_ pt: CGPoint) -> (Int32, Int32) { viewport.toRemote(pt.x, pt.y) }

    private func click(_ b: PointerButton, _ x: Int32, _ y: Int32) -> [InputEvent] {
        [.mouseMove(x: x, y: y), .mouseButton(button: b, down: true, x: x, y: y), .mouseButton(button: b, down: false, x: x, y: y)]
    }

    /// 悬浮鼠标按键：在当前光标位置按下 / 抬起
    func mouseKeys(_ presses: [MouseKeysLogic.Press]) {
        let (x, y) = cursor
        emit(presses.map { .mouseButton(button: $0.button, down: $0.down, x: x, y: y) })
    }

    /// 本机先记下光标位置（不等被控端回报）并立即挪箭头，再发出去
    private func emit(_ events: [InputEvent]) {
        let now = CACurrentMediaTime()
        var moved = false
        for e in events {
            switch e {
            case .mouseMove(let x, let y), .mouseButton(_, _, let x, let y):
                pointer.moved(x, y, now: now)
                moved = true
            default: break
            }
        }
        if moved { updatePointerLayer() }
        send?(events)
    }

    // 手势状态机（与安卓端 RemoteGestures.kt 一致）
    private var touches = Set<UITouch>()
    private var downAt = CGPoint.zero
    private var downTime: TimeInterval = 0
    private var lastPos = CGPoint.zero
    private var maxPointers = 0
    private var moved = false
    private var longPressed = false
    private var dragging = false
    private var dragButtonDown = false
    private var twoMode = 0          // 0 未定 1 缩放 2 滚动或平移
    private var twoDownCentroid: CGPoint?
    private var twoStartCenter = CGPoint.zero
    private var twoStartSpan: CGFloat = 0
    private var prevCenter = CGPoint.zero
    private var prevSpan: CGFloat = 0
    private var longPressTimer: Timer?
    private var scrollRemainder = CGPoint.zero
    private let slop: CGFloat = 10
    /// 双指轻点先挂起，等双击间隔：期间再来一次 → 双指双击（切换缩放），否则 → 右键
    private var twoTaps = TwoFingerTapResolver(interval: 0.3, maxDistance: 60)
    private var twoTapTimer: Timer?

    private func centroid(_ ps: [CGPoint]) -> CGPoint {
        CGPoint(x: ps.map(\.x).reduce(0, +) / CGFloat(ps.count), y: ps.map(\.y).reduce(0, +) / CGFloat(ps.count))
    }

    private func span(_ ps: [CGPoint], _ c: CGPoint) -> CGFloat {
        ps.map { hypot($0.x - c.x, $0.y - c.y) }.reduce(0, +) / CGFloat(ps.count)
    }

    /// 挂起的双指轻点交出去当右键（光标所在处）
    private func flushTwoTap() {
        twoTapTimer?.invalidate()
        twoTapTimer = nil
        if twoTaps.flush() {
            let (x, y) = cursor
            emit(click(.right, x, y))
        }
    }

    override func touchesBegan(_ set: Set<UITouch>, with event: UIEvent?) {
        let wasEmpty = touches.isEmpty
        touches.formUnion(set)
        if wasEmpty {
            let t = set.first!
            downAt = t.location(in: self)
            downTime = t.timestamp
            lastPos = downAt
            maxPointers = touches.count
            moved = false
            longPressed = false
            dragging = false
            twoMode = 0
            twoStartSpan = 0
            twoDownCentroid = nil
            longPressTimer?.invalidate()
            longPressTimer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: false) { [weak self] _ in
                DispatchQueue.main.async { self?.longPressFired() }
            }
        } else {
            maxPointers = max(maxPointers, touches.count)
            longPressTimer?.invalidate()
        }
        if touches.count == 2 && maxPointers == 2 && twoDownCentroid == nil {
            twoDownCentroid = centroid(touches.map { $0.location(in: self) })
        }
    }

    private func longPressFired() {
        guard touches.count == 1, !moved, maxPointers == 1 else { return }
        flushTwoTap()
        longPressed = true
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        if !mouseMode {
            let (x, y) = remote(downAt)
            emit(click(.right, x, y))
        }
    }

    override func touchesMoved(_ set: Set<UITouch>, with event: UIEvent?) {
        if touches.count >= 2 {
            twoFingerMoved(touches.map { $0.location(in: self) })
            return
        }
        // 双指手势结束后剩一根手指：忽略直到全部抬起
        guard maxPointers < 2, let t = touches.first else { return }
        let pos = t.location(in: self)
        if !moved, hypot(pos.x - downAt.x, pos.y - downAt.y) > slop {
            flushTwoTap()
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

    /// 双指：越过阈值时判定一次，间距变化为主 → 缩放（同时随质心平移），否则 → 滚动或平移
    private func twoFingerMoved(_ pts: [CGPoint]) {
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
            if twoMode != 0 { flushTwoTap() }
        }
        switch twoMode {
        case 1:
            setViewport(viewport.pinched(prevSpan > 0 ? s / prevSpan : 1, from: prevCenter, to: c), .gesture)
        case 2:
            if !mouseMode && viewport.isZoomed {
                setViewport(viewport.panned(c.x - prevCenter.x, c.y - prevCenter.y), .other)
            } else {
                scroll(dx: c.x - prevCenter.x, dy: c.y - prevCenter.y)
            }
        default: break
        }
        prevCenter = c
        prevSpan = s
        if twoMode != 0 { moved = true }
    }

    override func touchesEnded(_ set: Set<UITouch>, with event: UIEvent?) { finish(set, cancelled: false) }
    override func touchesCancelled(_ set: Set<UITouch>, with event: UIEvent?) { finish(set, cancelled: true) }

    private func finish(_ set: Set<UITouch>, cancelled: Bool) {
        touches.subtract(set)
        guard touches.isEmpty else { return }
        longPressTimer?.invalidate()
        if dragging {
            flushTwoTap()
            dragEnd()
        } else if cancelled {
            // 被系统手势打断：不发点击
            flushTwoTap()
        } else if maxPointers >= 2 && twoMode == 0 {
            twoFingerTapped(at: twoDownCentroid ?? downAt)
        } else if maxPointers == 1 && !moved && !longPressed {
            flushTwoTap()
            if mouseMode && keysBusy?() == true { dragging = false; return }
            let (x, y) = mouseMode ? cursor : remote(downAt)
            emit(click(.left, x, y))
        } else {
            flushTwoTap()
        }
        dragging = false
    }

    private func twoFingerTapped(at p: CGPoint) {
        switch twoTaps.tapped(at: p, time: downTime) {
        case .doubleTap(let at):
            twoTapTimer?.invalidate()
            twoTapTimer = nil
            setViewport(viewport.toggled(fx: at.x, fy: at.y), .gesture)
        case .pending(let flushed):
            if flushed {
                let (x, y) = cursor
                emit(click(.right, x, y))
            }
            twoTapTimer?.invalidate()
            twoTapTimer = Timer.scheduledTimer(withTimeInterval: twoTaps.interval + 0.02, repeats: false) { [weak self] _ in
                DispatchQueue.main.async {
                    // 手指还按着（可能是第二次轻点）：等这次手势结束时再判定
                    guard let self, self.touches.isEmpty else { return }
                    self.flushTwoTap()
                }
            }
        }
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
            // 悬浮鼠标按键刚按下还没定性：先按下，拖拽从起点开始
            beforePointerMove?()
            // 触控板：相对移动，带一点加速；放大后画面跟着光标走，光标始终看得见
            let k = viewport.remotePerViewPoint * 1.6
            let maxX = Int32(max(viewport.remoteW - 1, 0)), maxY = Int32(max(viewport.remoteH - 1, 0))
            let nx = min(max(Int32((CGFloat(cursor.0) + delta.x * k).rounded()), 0), maxX)
            let ny = min(max(Int32((CGFloat(cursor.1) + delta.y * k).rounded()), 0), maxY)
            emit([.mouseMove(x: nx, y: ny)])
            setViewport(viewport.follow(nx, ny), .other)
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

    /// 像触控板「自然滚动」：手指上推 = 内容跟着上移。协议里 dy 为正是滚轮往上推（内容下移），
    /// 所以手指位移直接作为滚动量（上推 dy<0 → 内容上移）；按视图点数换算成被控像素
    private func scroll(dx: CGFloat, dy: CGFloat) {
        let k = viewport.remotePerViewPoint
        scrollRemainder.x += dx * k
        scrollRemainder.y += dy * k
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

    /// 发过「按下」还没抬起的外接键盘按键（HID 用法码）
    private var heldKeys = Set<UInt32>()

    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        var handled = false
        for p in presses {
            guard let key = p.key else { continue }
            let usage = UInt32(key.keyCode.rawValue)
            let isModifier = (0xE0...0xE7).contains(usage)
            if !isModifier, key.modifierFlags.contains(.command) {
                // 按着 Command 的组合键：系统可能不再送这个键的「抬起」，按下即抬起，免得远端一直当它按着
                // （GNOME Wayland 上卡住的键会在每个新输入框里自动重复，比如一串 a）
                emit([.key(hidUsage: usage, down: true), .key(hidUsage: usage, down: false)])
            } else {
                heldKeys.insert(usage)
                emit([.key(hidUsage: usage, down: true)])
            }
            handled = true
        }
        if !handled { super.pressesBegan(presses, with: event) }
    }

    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        var handled = false
        for p in presses {
            guard let key = p.key else { continue }
            let usage = UInt32(key.keyCode.rawValue)
            // 按下时已经抬起过的组合键不再发
            if heldKeys.remove(usage) != nil { emit([.key(hidUsage: usage, down: false)]) }
            handled = true
        }
        if !handled { super.pressesEnded(presses, with: event) }
    }

    /// 失去焦点时把还按着的键全部抬起（之后的抬起不会再交给我们）
    func releaseHeldKeys() {
        guard !heldKeys.isEmpty else { return }
        emit(heldKeys.map { .key(hidUsage: $0, down: false) })
        heldKeys.removeAll()
    }

    override func resignFirstResponder() -> Bool {
        releaseHeldKeys()
        return super.resignFirstResponder()
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
