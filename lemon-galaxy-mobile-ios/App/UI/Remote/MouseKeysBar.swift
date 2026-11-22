import SwiftUI
import UIKit
import GalaxyKit

/// 悬浮鼠标按键的界面状态：包一层状态机（MouseKeysLogic），到时间点自己推进，按下 / 抬起交给画面按当前光标发出。
@MainActor
final class MouseKeysModel: ObservableObject {
    @Published private(set) var logic = MouseKeysLogic()
    /// 发出按键（画面补上当前光标坐标）
    var send: (([MouseKeysLogic.Press]) -> Void)?
    private var timer: Timer?

    var busy: Bool { logic.busy }

    func down(_ b: PointerButton) { apply { $0.down(b, now: CACurrentMediaTime()) } }
    func up(_ b: PointerButton) { apply { $0.up(b, now: CACurrentMediaTime()) } }
    func beforeMove() { apply { $0.beforeMove() } }
    func cancel() { apply { $0.cancel() } }

    private func apply(_ f: (inout MouseKeysLogic) -> [MouseKeysLogic.Press]) {
        let out = f(&logic)
        if !out.isEmpty { send?(out) }
        schedule()
    }

    private func schedule() {
        timer?.invalidate()
        timer = nil
        guard let d = logic.deadline else { return }
        let wait = max(0, d - CACurrentMediaTime())
        timer = Timer.scheduledTimer(withTimeInterval: wait, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.apply { $0.tick(now: CACurrentMediaTime()) }
            }
        }
    }
}

/// 画面左侧的悬浮鼠标按键条（与安卓端一致）：顶部把手（只有拖把手才挪位置）、左键、右键。
/// 位置是条在可用区域里的比例，拖完把手记到本机设置。
struct MouseKeysBar: View {
    @ObservedObject var keys: MouseKeysModel
    @EnvironmentObject var app: AppModel
    @Environment(\.loc) private var loc
    @State private var dragStart: CGPoint?
    @State private var pos: CGPoint?

    static let width: CGFloat = 52
    static let keyHeight: CGFloat = 56
    static let handleHeight: CGFloat = 26
    static var height: CGFloat { handleHeight + (keyHeight + 8) * 2 + 1 }

    var body: some View {
        GeometryReader { geo in
            let maxX = max(geo.size.width - Self.width, 0)
            let maxY = max(geo.size.height - Self.height, 0)
            let p = pos ?? CGPoint(x: app.settings.mouseKeysX, y: app.settings.mouseKeysY)
            bar(maxX: maxX, maxY: maxY, p: p)
                .offset(x: p.x * maxX, y: p.y * maxY)
        }
    }

    private func bar(maxX: CGFloat, maxY: CGFloat, p: CGPoint) -> some View {
        VStack(spacing: 0) {
            Image(systemName: "line.3.horizontal")
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(DockColors.dim)
                .frame(maxWidth: .infinity)
                .frame(height: Self.handleHeight)
                .contentShape(Rectangle())
                .gesture(
                    DragGesture(minimumDistance: 2, coordinateSpace: .global)
                        .onChanged { g in
                            let start = dragStart ?? p
                            if dragStart == nil { dragStart = p }
                            let nx = maxX > 0 ? min(max(start.x * maxX + g.translation.width, 0), maxX) / maxX : 0
                            let ny = maxY > 0 ? min(max(start.y * maxY + g.translation.height, 0), maxY) / maxY : 0
                            pos = CGPoint(x: nx, y: ny)
                        }
                        .onEnded { _ in
                            dragStart = nil
                            if let pos {
                                app.settings.mouseKeysX = pos.x
                                app.settings.mouseKeysY = pos.y
                            }
                        }
                )
                .accessibilityLabel(loc("remote.mouseKeysMove"))
            key(.left, loc("remote.mouseLeft"))
            Rectangle().fill(DockColors.line).frame(height: 1).padding(.horizontal, 8)
            key(.right, loc("remote.mouseRight"))
        }
        .frame(width: Self.width, height: Self.height)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Color(hex: "#1B1E2B").opacity(0.8)))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).stroke(DockColors.line, lineWidth: 1))
        .accessibilityIdentifier("mouse-keys")
    }

    private func key(_ b: PointerButton, _ label: String) -> some View {
        let l = keys.logic
        let active = l.button == b && l.isDown
        return KeyFace(label: label, active: active, locked: active && l.isLocked,
                       onDown: { keys.down(b) }, onUp: { keys.up(b) })
            .padding(4)
            .frame(height: Self.keyHeight + 8)
            .accessibilityLabel(label)
    }
}

/// 一个键：手指按下立即报按下，抬起报抬起（期间的移动不当成别的手势）
private struct KeyFace: View {
    let label: String
    let active: Bool
    let locked: Bool
    let onDown: () -> Void
    let onUp: () -> Void
    @State private var pressing = false

    var body: some View {
        let fg = active ? DockColors.onAccent : Color.white
        ZStack(alignment: .topTrailing) {
            Text(label).font(.system(size: 13)).foregroundStyle(fg)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            if locked {
                Image(systemName: "lock.fill").font(.system(size: 9)).foregroundStyle(fg).padding(4)
            }
        }
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(active ? DockColors.accent : Color.clear))
        .contentShape(Rectangle())
        .gesture(
            DragGesture(minimumDistance: 0)
                .onChanged { _ in
                    if !pressing {
                        pressing = true
                        onDown()
                    }
                }
                .onEnded { _ in
                    pressing = false
                    onUp()
                }
        )
    }
}
