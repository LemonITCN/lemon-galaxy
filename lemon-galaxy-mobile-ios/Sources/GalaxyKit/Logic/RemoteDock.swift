import Foundation
import CoreGraphics

// 远程桌面右侧贴边菜单与手势的纯逻辑（与界面无关，可在 macOS 上单元测试）。与安卓端 RemoteDock.kt / RemoteGestures.kt 对应。

/// 显示器循环切换。
public enum DisplayCycle {
    /// 当前显示器在列表里的位置（找不到按第 1 块算）
    public static func index(of current: UInt32?, in ids: [UInt32]) -> Int {
        ids.firstIndex { $0 == current } ?? 0
    }

    /// 往后（step > 0）或往前切换，首尾相接：两块屏时上滑下滑都能换过去。只有一块屏返回 nil。
    public static func next(after current: UInt32?, in ids: [UInt32], step: Int) -> UInt32? {
        guard ids.count > 1 else { return nil }
        let i = index(of: current, in: ids)
        let n = ids.count
        return ids[((i + step) % n + n) % n]
    }
}

/// 把手与菜单的尺寸、位置（视图点）。
public enum DockLayout {
    public static let railWidth: CGFloat = 64
    public static let handleWidth: CGFloat = 22
    public static let handleHeight: CGFloat = 64
    /// 把手的触摸区比看得见的部分高，好按
    public static let handleTouchHeight: CGFloat = 84
    public static let handleTouchWidth: CGFloat = 36
    /// 上下滑动超过这个距离才算切换显示器
    public static let swipeDistance: CGFloat = 28
    public static let slop: CGFloat = 10
    /// 离开上下安全区的余量
    public static let edgeGap: CGFloat = 8
    /// 竖排菜单固定部分：收起键 24 + 连接状态 30 + 分隔线 9 + 上下内边距 12（多块屏再加切换器 54）
    static let railFixed: CGFloat = 24 + 30 + 9 + 12
    static let railSwitcher: CGFloat = 54
    /// 竖排菜单里的功能项个数（显示器、键盘、快捷键、操作方式、工具、断开）
    public static let railItems = 6

    /// 把手可放的竖直范围（顶部、底部避开状态栏 / 刘海 / 主屏指示条）
    public static func handleRange(containerH: CGFloat, topInset: CGFloat, bottomInset: CGFloat) -> ClosedRange<CGFloat> {
        let lo = topInset + edgeGap
        let hi = max(lo, containerH - bottomInset - edgeGap - handleTouchHeight)
        return lo...hi
    }

    /// 按比例（0 顶、1 底）算出把手触摸区的上边
    public static func handleY(frac: CGFloat, containerH: CGFloat, topInset: CGFloat, bottomInset: CGFloat) -> CGFloat {
        let r = handleRange(containerH: containerH, topInset: topInset, bottomInset: bottomInset)
        let f = min(max(frac, 0), 1)
        return r.lowerBound + (r.upperBound - r.lowerBound) * f
    }

    /// 拖动 dy 后的新比例
    public static func movedFrac(_ frac: CGFloat, by dy: CGFloat, containerH: CGFloat, topInset: CGFloat, bottomInset: CGFloat) -> CGFloat {
        let r = handleRange(containerH: containerH, topInset: topInset, bottomInset: bottomInset)
        let span = max(r.upperBound - r.lowerBound, 1)
        return min(max(frac + dy / span, 0), 1)
    }

    /// 竖排菜单每项的高度：按可用高度均分，夹在 36…54 之间（菜单不滚动）
    public static func railItemHeight(available: CGFloat, multiDisplay: Bool) -> CGFloat {
        let fixed = railFixed + (multiDisplay ? railSwitcher : 0)
        return min(max((available - fixed) / CGFloat(railItems), 36), 54)
    }

    /// 二级面板最大宽度
    public static func panelMaxWidth(containerW: CGFloat) -> CGFloat {
        min(max(containerW - railWidth - 24, 200), 384)
    }
}

/// 把手上的状态点：绿 = 局域网 / 直连，黄 = 中转，红 = 录像中。
public enum DockDot: Equatable {
    case none, online, relay, recording

    public static func of(path: PathKind?, recording: Bool) -> DockDot {
        if recording { return .recording }
        switch path {
        case .lan?, .direct?: return .online
        case .relayUdp?, .relayTcp?: return .relay
        default: return .none
        }
    }
}

/// 收起状态把手上的一次触摸：轻点展开、上下滑动切换显示器、长按后拖动改位置。
/// 只有一块屏时，越过阈值的移动直接当作拖动改位置。
public struct DockHandleTracker {
    public enum Mode: Equatable { case undecided, swipe, move }
    public enum Result: Equatable {
        case none
        case tap
        /// +1 下一块（手指上滑），-1 上一块
        case switchDisplay(Int)
    }

    public let canSwitch: Bool
    public let slop: CGFloat
    public let swipeDistance: CGFloat
    public private(set) var mode: Mode = .undecided
    private var lastY: CGFloat = 0
    private var totalY: CGFloat = 0

    public init(canSwitch: Bool, slop: CGFloat = DockLayout.slop, swipeDistance: CGFloat = DockLayout.swipeDistance) {
        self.canSwitch = canSwitch
        self.slop = slop
        self.swipeDistance = swipeDistance
    }

    /// 手指移动（ty 为相对按下点的竖直位移）。返回把手要跟着移动的量（只有拖动改位置时非零）。
    public mutating func moved(to ty: CGFloat) -> CGFloat {
        let delta = ty - lastY
        lastY = ty
        totalY = ty
        switch mode {
        case .undecided:
            guard abs(ty) > slop else { return 0 }
            mode = canSwitch ? .swipe : .move
            return mode == .move ? ty : 0
        case .move:
            return delta
        case .swipe:
            return 0
        }
    }

    /// 长按计时到了：还没动就进入拖动改位置。返回 true 表示刚进入（要震动一下）。
    public mutating func longPressElapsed() -> Bool {
        guard mode == .undecided else { return false }
        mode = .move
        return true
    }

    public func ended() -> Result {
        switch mode {
        case .undecided: return .tap
        case .move: return .none
        case .swipe: return abs(totalY) > swipeDistance ? .switchDisplay(totalY < 0 ? 1 : -1) : .none
        }
    }
}

/// 展开后的竖排菜单上的滑动：竖直方向为主且越过阈值即算滑动（子项的点按随之作废），抬起时够远就切换显示器。
public struct RailSwipeTracker {
    public let slop: CGFloat
    public let swipeDistance: CGFloat
    public private(set) var dragging = false
    private var total = CGSize.zero

    public init(slop: CGFloat = DockLayout.slop, swipeDistance: CGFloat = DockLayout.swipeDistance) {
        self.slop = slop
        self.swipeDistance = swipeDistance
    }

    public mutating func moved(to t: CGSize) {
        total = t
        if !dragging && abs(t.height) > slop && abs(t.height) > abs(t.width) { dragging = true }
    }

    /// 抬起：返回切换方向（+1 下一块，-1 上一块），不切换为 nil
    public func ended() -> Int? {
        guard dragging, abs(total.height) > swipeDistance else { return nil }
        return total.height < 0 ? 1 : -1
    }
}

/// 双指轻点与双指双击的区分：轻点先不发右键，等双击间隔；期间再来一次附近的双指轻点 → 双击（切换缩放），否则 → 右键。
public struct TwoFingerTapResolver {
    public enum Outcome: Equatable {
        /// 记下，等下一次（若之前还有一个待定的轻点，先把它当右键发出去：flushed = true）
        case pending(flushed: Bool)
        case doubleTap(CGPoint)
    }

    public let interval: TimeInterval
    public let maxDistance: CGFloat
    public private(set) var pendingAt: CGPoint?
    public private(set) var pendingTime: TimeInterval = 0

    public init(interval: TimeInterval = 0.3, maxDistance: CGFloat = 60) {
        self.interval = interval
        self.maxDistance = maxDistance
    }

    public var hasPending: Bool { pendingAt != nil }

    /// 一次双指轻点在 time 时抬起，位置 at（按下时的双指中心）
    public mutating func tapped(at: CGPoint, time: TimeInterval) -> Outcome {
        if let p = pendingAt, time - pendingTime <= interval, hypot(p.x - at.x, p.y - at.y) < maxDistance {
            pendingAt = nil
            return .doubleTap(at)
        }
        let flushed = pendingAt != nil
        pendingAt = at
        pendingTime = time
        return .pending(flushed: flushed)
    }

    /// 双击间隔已过、或者开始了别的手势：有待定的轻点就交出去当右键（返回 true）
    public mutating func flush() -> Bool {
        guard pendingAt != nil else { return false }
        pendingAt = nil
        return true
    }

    /// 到 now 为止待定的轻点是否已超过双击间隔
    public func expired(now: TimeInterval) -> Bool {
        pendingAt != nil && now - pendingTime > interval
    }
}
