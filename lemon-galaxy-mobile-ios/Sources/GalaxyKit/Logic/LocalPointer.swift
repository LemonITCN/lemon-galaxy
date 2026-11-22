import Foundation
import CoreGraphics

/// 本机记的光标位置（被控屏幕像素）：鼠标模式相对移动、画面跟随、本地画箭头都用它。与安卓端 LocalPointer.kt 逐条对应。
///
/// 位置以本机为准，手指一动立即更新（跟手）。被控端回报的位置只在本机 `quietInterval` 内没发过移动时才采用：
/// 那是被控端那边有人动了鼠标；本机刚动过时收到的回报多半是自己发出去的移动隔着网络绕回来的旧位置，
/// 采用了光标会往回跳。是否显示一律听被控端的（对面隐藏了光标，这里也不画）。
public struct LocalPointer: Equatable {
    /// 本机停手多久之后才采用被控端回报的位置（秒）
    public static let quietInterval: TimeInterval = 0.5

    public private(set) var x: Int32
    public private(set) var y: Int32
    public private(set) var visible = true
    /// 本机最后一次发出移动 / 点击的时刻（秒，单调时钟）
    private var lastLocalAt: TimeInterval = -.greatestFiniteMagnitude

    public init(x: Int32 = 0, y: Int32 = 0) {
        self.x = x
        self.y = y
    }

    /// 本机发出了移动或点击（坐标就是发出去的坐标）
    public mutating func moved(_ nx: Int32, _ ny: Int32, now: TimeInterval) {
        x = nx
        y = ny
        lastLocalAt = now
    }

    /// 被控端回报光标；返回是否采用了位置
    @discardableResult
    public mutating func hostReport(_ hx: Int32, _ hy: Int32, visible shown: Bool, now: TimeInterval) -> Bool {
        visible = shown
        guard now - lastLocalAt >= Self.quietInterval else { return false }
        x = hx
        y = hy
        return true
    }

    /// 换屏幕 / 重开画面：放到给定位置，忘掉本机移动的时刻
    public mutating func reset(_ nx: Int32, _ ny: Int32) {
        x = nx
        y = ny
        visible = true
        lastLocalAt = -.greatestFiniteMagnitude
    }

    /// 被控端回报的是画面像素：画面被缩放过时换算成被控屏幕像素（尺寸相等或未知时原样）
    public static func videoToRemote(_ x: Int32, _ y: Int32, video: CGSize, remoteW: Int, remoteH: Int) -> (Int32, Int32) {
        guard video.width > 0, video.height > 0, remoteW > 0, remoteH > 0 else { return (x, y) }
        let sx = Int64(x) * Int64(remoteW) / Int64(video.width)
        let sy = Int64(y) * Int64(remoteH) / Int64(video.height)
        return (Int32(clamping: sx), Int32(clamping: sy))
    }
}

/// 标准箭头光标的外形（尖端在原点，单位约等于 1 视图点的 1/1.3）：白色填充、黑色描边。与安卓端 LocalPointer.kt 同一组顶点。
public enum PointerArrow {
    public static let points: [CGPoint] = [
        CGPoint(x: 0, y: 0), CGPoint(x: 0, y: 16.5), CGPoint(x: 4, y: 12.8), CGPoint(x: 6.6, y: 18.8),
        CGPoint(x: 9.4, y: 17.6), CGPoint(x: 6.8, y: 11.8), CGPoint(x: 12, y: 11.8),
    ]
    /// 每个单位多少视图点（安卓端是 1.3dp）
    public static let unit: CGFloat = 1.3
    /// 描边宽度（单位同上）
    public static let strokeWidth: CGFloat = 1.1

    /// 尖端在原点、已按 `unit` 放大的路径
    public static func path() -> CGPath {
        let p = CGMutablePath()
        p.addLines(between: points.map { CGPoint(x: $0.x * unit, y: $0.y * unit) })
        p.closeSubpath()
        return p
    }
}
