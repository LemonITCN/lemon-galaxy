import Foundation

/// 悬浮鼠标按键（左键、右键）的状态机，与安卓端 MouseKeys.kt 逐条对应：
/// - 点一下 = 单击。为了和双击区分，松手后等 `doubleInterval` 没有第二下才发出单击；
/// - 按住超过 `holdInterval`（或按着时另一根手指开始移动光标）= 按下不放，松手抬起，期间移动光标就是拖拽；
/// - 双击 = 锁定按下（第二下按下时立即锁定，不再发单击），之后单指移动光标即拖拽；再点一下解锁抬起。
/// 同一时刻只有一个键在用：按另一个键会先结束当前的键。
///
/// 只产生「哪个键按下 / 抬起」，坐标由界面按当前光标位置补上。时间（秒）由调用方传入，便于单元测试。
public struct MouseKeysLogic: Equatable {
    public enum Phase: Equatable { case idle, pending, held, tapped, locked }

    public struct Press: Equatable {
        public let button: PointerButton
        public let down: Bool
        public init(_ button: PointerButton, _ down: Bool) {
            self.button = button
            self.down = down
        }
    }

    public static let holdInterval: TimeInterval = 0.2
    public static let doubleInterval: TimeInterval = 0.26

    public private(set) var phase: Phase = .idle
    public private(set) var button: PointerButton?
    private var at: TimeInterval = 0
    /// 这次按下已经处理完（解锁 / 锁定），对应的松手不再处理
    private var swallowUp = false

    public init() {}

    /// 键处于按下状态（按住或锁定）：界面高亮
    public var isDown: Bool { phase == .held || phase == .locked }
    public var isLocked: Bool { phase == .locked }
    /// 画面上的单击要不要让开：键按着或锁定时，画面轻点不再发单击，免得打断拖拽
    public var busy: Bool { phase == .held || phase == .locked || phase == .pending }

    public mutating func down(_ b: PointerButton, now: TimeInterval) -> [Press] {
        var out: [Press] = []
        switch phase {
        case .locked:
            out.append(Press(button!, false))
            let same = button == b
            reset()
            if same {
                // 再点一下 = 解锁
                swallowUp = true
                return out
            }
        case .tapped:
            if button == b && now - at <= Self.doubleInterval {
                // 双击的第二下：锁定按下
                phase = .locked
                swallowUp = true
                out.append(Press(b, true))
                return out
            }
            out += click(button!)
            reset()
        case .pending, .held:
            out += finish()
        case .idle:
            break
        }
        phase = .pending
        button = b
        at = now
        swallowUp = false
        return out
    }

    public mutating func up(_ b: PointerButton, now: TimeInterval) -> [Press] {
        if swallowUp {
            swallowUp = false
            return []
        }
        guard button == b else { return [] }
        switch phase {
        case .pending:
            phase = .tapped
            at = now
            return []
        case .held:
            reset()
            return [Press(b, false)]
        default:
            return []
        }
    }

    /// 到时间点检查：按住够久变成按下，单击等不到第二下就发出
    public mutating func tick(now: TimeInterval) -> [Press] {
        switch phase {
        case .pending:
            return now - at >= Self.holdInterval ? hold() : []
        case .tapped:
            guard now - at > Self.doubleInterval else { return [] }
            let out = click(button!)
            reset()
            return out
        default:
            return []
        }
    }

    /// 光标要移动了：键还按着没定性时先按下，保证拖拽从起点开始
    public mutating func beforeMove() -> [Press] { phase == .pending ? hold() : [] }

    /// 下一个需要 `tick` 的时刻；没有返回 nil
    public var deadline: TimeInterval? {
        switch phase {
        case .pending: return at + Self.holdInterval
        case .tapped: return at + Self.doubleInterval + 0.001
        default: return nil
        }
    }

    /// 离开画面、切到触控模式、隐藏按键时：按着的键抬起，没发出的单击丢掉
    public mutating func cancel() -> [Press] {
        let out = (phase == .held || phase == .locked) ? [Press(button!, false)] : []
        reset()
        swallowUp = false
        return out
    }

    private mutating func hold() -> [Press] {
        phase = .held
        return [Press(button!, true)]
    }

    private mutating func finish() -> [Press] {
        let out: [Press]
        switch phase {
        case .pending: out = click(button!)
        case .held, .locked: out = [Press(button!, false)]
        default: out = []
        }
        reset()
        return out
    }

    private func click(_ b: PointerButton) -> [Press] { [Press(b, true), Press(b, false)] }

    private mutating func reset() {
        phase = .idle
        button = nil
    }
}
