import Foundation
import CoreGraphics

/// 远程画面在视图里的摆放：先等比适配（留黑边），再乘缩放、加平移。
/// 宽高始终按同一个比例缩放，画面比例永远等于被控屏幕比例。坐标换算全部在这里，输入事件统一换成被控屏幕像素。
/// 与安卓端 RemoteViewport.kt 逐条对应。
///
/// - zoom：相对「适应屏幕」的倍数，1 = 适应屏幕。
/// - panX / panY：画面中心相对视图中心的偏移（视图点）。
public struct Viewport: Equatable {
    public var containerW: CGFloat
    public var containerH: CGFloat
    public var remoteW: Int
    public var remoteH: Int
    public var zoom: CGFloat
    public var panX: CGFloat
    public var panY: CGFloat

    /// 最大倍数的下限（被控分辨率很高时允许放得更大）
    public static let minMaxZoom: CGFloat = 5

    public init(containerW: CGFloat, containerH: CGFloat, remoteW: Int, remoteH: Int, zoom: CGFloat = 1, panX: CGFloat = 0, panY: CGFloat = 0) {
        self.containerW = containerW
        self.containerH = containerH
        self.remoteW = remoteW
        self.remoteH = remoteH
        self.zoom = zoom
        self.panX = panX
        self.panY = panY
    }

    /// 适应屏幕时 1 个被控像素占多少视图点
    public var base: CGFloat {
        guard remoteW > 0, remoteH > 0 else { return 1 }
        return min(containerW / CGFloat(remoteW), containerH / CGFloat(remoteH))
    }

    public var scale: CGFloat { base * zoom }
    public var contentW: CGFloat { CGFloat(remoteW) * scale }
    public var contentH: CGFloat { CGFloat(remoteH) * scale }
    public var left: CGFloat { (containerW - contentW) / 2 + panX }
    public var top: CGFloat { (containerH - contentH) / 2 + panY }
    public var contentRect: CGRect { CGRect(x: left, y: top, width: contentW, height: contentH) }

    /// 最大倍数：至少 5 倍，且能放大到 1 个被控像素占 1.5 个视图点
    public var maxZoom: CGFloat { max(Self.minMaxZoom, 1.5 / base) }

    /// 「原始像素」对应的倍数（1 个被控像素 = 1 个视图点），不小于适应屏幕
    public var actualSizeZoom: CGFloat { min(max(1 / base, 1), maxZoom) }

    public var isZoomed: Bool { zoom > 1.01 }

    /// 视图坐标 → 被控坐标（浮点、不夹）
    public func toRemoteF(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
        CGPoint(x: (x - left) / scale, y: (y - top) / scale)
    }

    /// 视图坐标 → 被控屏幕像素（取所在像素，夹在屏幕范围内）
    public func toRemote(_ x: CGFloat, _ y: CGFloat) -> (Int32, Int32) {
        let f = toRemoteF(x, y)
        guard f.x.isFinite, f.y.isFinite else { return (0, 0) }
        let rx = min(max(Int(f.x.rounded(.down)), 0), max(0, remoteW - 1))
        let ry = min(max(Int(f.y.rounded(.down)), 0), max(0, remoteH - 1))
        return (Int32(rx), Int32(ry))
    }

    /// 被控像素 → 视图坐标（像素中心）
    public func toView(_ rx: Int32, _ ry: Int32) -> CGPoint {
        CGPoint(x: left + (CGFloat(rx) + 0.5) * scale, y: top + (CGFloat(ry) + 0.5) * scale)
    }

    /// 视图上的一个点对应多少被控像素（鼠标模式相对移动、滚轮换算用）
    public var remotePerViewPoint: CGFloat { scale <= 0 ? 1 : 1 / scale }

    /// 按左上角位置与倍数重建（换算回中心偏移）
    private func placed(_ nl: CGFloat, _ nt: CGFloat, _ nz: CGFloat) -> Viewport {
        let cw = CGFloat(remoteW) * base * nz
        let ch = CGFloat(remoteH) * base * nz
        var v = self
        v.zoom = nz
        v.panX = nl - (containerW - cw) / 2
        v.panY = nt - (containerH - ch) / 2
        return v
    }

    /// 捏合一步：缩放 factor 倍，同时让原来在 from 下的画面点移到 to（双指质心随手指移动）。
    /// 最后夹住：放大后不露出多余黑边，缩小不低于适应屏幕。
    public func pinched(_ factor: CGFloat, from: CGPoint, to: CGPoint) -> Viewport {
        let nz = min(max(zoom * factor, 1), maxZoom)
        let f = nz / zoom
        return placed(to.x - (from.x - left) * f, to.y - (from.y - top) * f, nz).clamped()
    }

    /// 以 (fx, fy) 为中心缩放到 z 倍
    public func zoomedTo(_ z: CGFloat, fx: CGFloat, fy: CGFloat) -> Viewport {
        let p = CGPoint(x: fx, y: fy)
        return pinched(z / zoom, from: p, to: p)
    }

    public func panned(_ dx: CGFloat, _ dy: CGFloat) -> Viewport {
        var v = self
        v.panX += dx
        v.panY += dy
        return v.clamped()
    }

    /// 回到适应屏幕
    public func fit() -> Viewport {
        var v = self
        v.zoom = 1
        v.panX = 0
        v.panY = 0
        return v
    }

    /// 双指双击：已放大 → 适应屏幕；否则以点按处为中心放大（至少 2 倍，被控分辨率高时放到原始像素）
    public func toggled(fx: CGFloat, fy: CGFloat) -> Viewport {
        isZoomed ? fit() : zoomedTo(min(max(2, actualSizeZoom), maxZoom), fx: fx, fy: fy)
    }

    /// 夹住平移：
    /// - 画面比视图大的方向：不许露出黑边；
    /// - 画面比视图小的方向（留黑边那一维）：画面不出视图，且偏离居中的量不超过 (倍数-1)×适应尺寸/2。
    ///   这样从适应屏幕开始捏合时双指中心下的点不会被强行拉回中间，缩回 1 倍时又正好回到居中。
    public func clamped() -> Viewport {
        func limit(_ content: CGFloat, _ container: CGFloat, _ fitSize: CGFloat) -> CGFloat {
            if content >= container { return (content - container) / 2 }
            return max(min((container - content) / 2, (zoom - 1) * fitSize / 2), 0)
        }
        let maxX = limit(contentW, containerW, CGFloat(remoteW) * base)
        let maxY = limit(contentH, containerH, CGFloat(remoteH) * base)
        var v = self
        v.panX = min(max(panX, -maxX), maxX)
        v.panY = min(max(panY, -maxY), maxY)
        return v
    }

    /// 让被控坐标保持在可见区域内（鼠标模式放大后画面跟随光标）
    public func follow(_ rx: Int32, _ ry: Int32, margin: CGFloat = 48) -> Viewport {
        guard isZoomed else { return self }
        let p = toView(rx, ry)
        var dx: CGFloat = 0, dy: CGFloat = 0
        if p.x < margin { dx = margin - p.x } else if p.x > containerW - margin { dx = containerW - margin - p.x }
        if p.y < margin { dy = margin - p.y } else if p.y > containerH - margin { dy = containerH - margin - p.y }
        return dx == 0 && dy == 0 ? self : panned(dx, dy)
    }

    /// 视图或被控屏幕尺寸变化：
    /// - 被控分辨率变了（换屏、对方改分辨率）→ 回到适应屏幕；
    /// - 只是视图变了（旋转、弹出键盘）→ 保持倍数，原来在视图中心的画面点仍放在中心。
    public func resized(containerW cw: CGFloat, containerH ch: CGFloat, remoteW rw: Int, remoteH rh: Int) -> Viewport {
        let w = max(cw, 1), h = max(ch, 1)
        let nrw = max(rw, 1), nrh = max(rh, 1)
        if nrw != remoteW || nrh != remoteH { return Viewport(containerW: w, containerH: h, remoteW: nrw, remoteH: nrh) }
        if abs(w - containerW) < 0.5 && abs(h - containerH) < 0.5 { return self }
        let c = toRemoteF(containerW / 2, containerH / 2)
        let next = Viewport(containerW: w, containerH: h, remoteW: nrw, remoteH: nrh, zoom: zoom)
        // 新视图里把 c 放到中心
        return next.placed(w / 2 - c.x * next.scale, h / 2 - c.y * next.scale, zoom).clamped()
    }
}

/// 画面状态里菜单要用到的部分（只在这几项变化时刷新界面，捏合过程中不跟着每帧刷新）
public struct ViewportFlags: Equatable {
    public var zoomed: Bool
    public var canActualSize: Bool
    public var atActualSize: Bool

    public init(zoomed: Bool = false, canActualSize: Bool = false, atActualSize: Bool = false) {
        self.zoomed = zoomed
        self.canActualSize = canActualSize
        self.atActualSize = atActualSize
    }

    public init(_ v: Viewport) {
        let can = v.actualSizeZoom > 1.01
        self.init(zoomed: v.isZoomed, canActualSize: can, atActualSize: can && abs(v.zoom - v.actualSizeZoom) < 0.01)
    }
}
