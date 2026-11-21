import Foundation
import CoreGraphics

/// 远程画面在视图里的摆放：等比适配（留黑边）后再乘缩放、加平移。坐标换算全部在这里，输入事件统一换成被控屏幕像素。
public struct Viewport: Equatable {
    public var containerW: CGFloat
    public var containerH: CGFloat
    public var remoteW: Int
    public var remoteH: Int
    public var zoom: CGFloat
    public var panX: CGFloat
    public var panY: CGFloat

    public static let maxZoom: CGFloat = 5

    public init(containerW: CGFloat, containerH: CGFloat, remoteW: Int, remoteH: Int, zoom: CGFloat = 1, panX: CGFloat = 0, panY: CGFloat = 0) {
        self.containerW = containerW
        self.containerH = containerH
        self.remoteW = remoteW
        self.remoteH = remoteH
        self.zoom = zoom
        self.panX = panX
        self.panY = panY
    }

    var base: CGFloat {
        guard remoteW > 0, remoteH > 0 else { return 1 }
        return min(containerW / CGFloat(remoteW), containerH / CGFloat(remoteH))
    }

    public var contentW: CGFloat { CGFloat(remoteW) * base * zoom }
    public var contentH: CGFloat { CGFloat(remoteH) * base * zoom }
    public var left: CGFloat { (containerW - contentW) / 2 + panX }
    public var top: CGFloat { (containerH - contentH) / 2 + panY }
    public var contentRect: CGRect { CGRect(x: left, y: top, width: contentW, height: contentH) }

    /// 视图坐标 → 被控屏幕像素（夹在屏幕范围内）
    public func toRemote(_ x: CGFloat, _ y: CGFloat) -> (Int32, Int32) {
        guard contentW > 0, contentH > 0 else { return (0, 0) }
        let rx = Int(((x - left) / contentW * CGFloat(remoteW)).rounded())
        let ry = Int(((y - top) / contentH * CGFloat(remoteH)).rounded())
        return (Int32(min(max(rx, 0), max(0, remoteW - 1))), Int32(min(max(ry, 0), max(0, remoteH - 1))))
    }

    public func toView(_ rx: Int32, _ ry: Int32) -> CGPoint {
        guard remoteW > 0, remoteH > 0 else { return .zero }
        return CGPoint(x: left + CGFloat(rx) / CGFloat(remoteW) * contentW, y: top + CGFloat(ry) / CGFloat(remoteH) * contentH)
    }

    /// 视图上的一个点对应多少被控像素（鼠标模式相对移动用）
    public var remotePerViewPoint: CGFloat { contentW <= 0 ? 1 : CGFloat(remoteW) / contentW }

    /// 以 (fx, fy) 为中心缩放，再加平移，最后夹住：放大后不露出黑边
    public func zoomed(_ factor: CGFloat, fx: CGFloat, fy: CGFloat, dx: CGFloat = 0, dy: CGFloat = 0) -> Viewport {
        let nz = min(max(zoom * factor, 1), Self.maxZoom)
        let f = nz / zoom
        let nl = fx - (fx - left) * f + dx
        let nt = fy - (fy - top) * f + dy
        let ncw = CGFloat(remoteW) * base * nz
        let nch = CGFloat(remoteH) * base * nz
        var v = self
        v.zoom = nz
        v.panX = nl - (containerW - ncw) / 2
        v.panY = nt - (containerH - nch) / 2
        return v.clamped()
    }

    public func panned(_ dx: CGFloat, _ dy: CGFloat) -> Viewport {
        var v = self
        v.panX += dx
        v.panY += dy
        return v.clamped()
    }

    public func clamped() -> Viewport {
        let maxX = max(0, (contentW - containerW) / 2)
        let maxY = max(0, (contentH - containerH) / 2)
        var v = self
        v.panX = min(max(panX, -maxX), maxX)
        v.panY = min(max(panY, -maxY), maxY)
        return v
    }

    /// 容器尺寸或画面分辨率变化：保留缩放，重新夹住
    public func resized(containerW: CGFloat, containerH: CGFloat, remoteW: Int, remoteH: Int) -> Viewport {
        var v = self
        v.containerW = containerW
        v.containerH = containerH
        if v.remoteW != remoteW || v.remoteH != remoteH {
            v.remoteW = remoteW
            v.remoteH = remoteH
            v.zoom = 1
            v.panX = 0
            v.panY = 0
        }
        return v.clamped()
    }

    /// 让被控坐标保持在可见区域内（鼠标模式放大后画面跟随光标）
    public func follow(_ rx: Int32, _ ry: Int32, margin: CGFloat = 48) -> Viewport {
        guard zoom > 1 else { return self }
        let p = toView(rx, ry)
        var dx: CGFloat = 0, dy: CGFloat = 0
        if p.x < margin { dx = margin - p.x } else if p.x > containerW - margin { dx = containerW - margin - p.x }
        if p.y < margin { dy = margin - p.y } else if p.y > containerH - margin { dy = containerH - margin - p.y }
        return dx == 0 && dy == 0 ? self : panned(dx, dy)
    }
}
