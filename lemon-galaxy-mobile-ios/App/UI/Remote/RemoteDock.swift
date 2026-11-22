import SwiftUI
import UIKit
import GalaxyKit

/// 右侧贴边菜单（参照网易 UU 远程手机端的侧边工具栏，与安卓端 RemoteDock.kt 一致）：
/// - 收起：贴右边缘的把手，轻点展开，上下滑动切换显示器，长按后拖动改变位置；
/// - 展开：右侧竖排菜单（不滚动，高度自适应），菜单上上下滑动同样切换显示器，点菜单外任意处收起；
/// - 二级面板从菜单左侧弹出，放显示器 / 画质 / 快捷键 / 工具等详细功能。
/// 整层铺满（含安全区），按安全区摆放：把手与菜单的底色延伸到屏幕右边缘，内容避开刘海、灵动岛与主屏指示条。
struct RemoteDock: View {
    @ObservedObject var session: RemoteSession
    let actions: DockActions
    @Binding var expanded: Bool
    @Binding var panel: RemotePanel?
    /// 把手在可放范围里的位置（0 顶、1 底）
    @State private var handleFrac: CGFloat = 0.4

    var body: some View {
        GeometryReader { geo in
            layer(size: geo.size, insets: geo.safeAreaInsets)
        }
        .ignoresSafeArea(.container)
    }

    private func layer(size: CGSize, insets: EdgeInsets) -> some View {
        ZStack(alignment: .topTrailing) {
            if expanded {
                scrim
                menu(size: size, insets: insets)
                    .transition(.move(edge: .trailing).combined(with: .opacity))
            } else {
                handle(size: size, insets: insets)
                    .transition(.opacity)
            }
        }
        .frame(width: size.width, height: size.height, alignment: .topTrailing)
        .animation(.spring(response: 0.3, dampingFraction: 0.9), value: expanded)
        .animation(.easeOut(duration: 0.2), value: panel)
    }

    /// 点菜单外任意处收起（这层吃掉点按，不会误点到远端）
    private var scrim: some View {
        Color.black.opacity(0.18)
            .contentShape(Rectangle())
            .onTapGesture { collapse() }
            .transition(.opacity)
    }

    private func collapse() {
        expanded = false
        panel = nil
    }

    private func handle(size: CGSize, insets: EdgeInsets) -> some View {
        let y = DockLayout.handleY(frac: handleFrac, containerH: size.height, topInset: insets.top, bottomInset: insets.bottom)
        return DockHandle(
            session: session,
            rightInset: insets.trailing,
            onTap: { expanded = true },
            onSwipe: actions.switchDisplay,
            onMove: { dy in
                handleFrac = DockLayout.movedFrac(handleFrac, by: dy, containerH: size.height, topInset: insets.top, bottomInset: insets.bottom)
            }
        )
        .offset(y: y)
    }

    private func menu(size: CGSize, insets: EdgeInsets) -> some View {
        let availH = max(size.height - insets.top - insets.bottom - 12, 0)
        let panelW = DockLayout.panelMaxWidth(containerW: size.width - insets.leading - insets.trailing)
        return HStack(spacing: 8) {
            if let p = panel {
                DockPanelView(panel: p, session: session, actions: actions, portrait: size.height > size.width, onClose: { panel = nil })
                    .frame(width: panelW)
                    .frame(maxHeight: availH - 4)
                    .transition(.opacity.combined(with: .offset(x: 24)))
            }
            DockRail(
                session: session,
                actions: actions,
                panel: panel,
                availableHeight: availH,
                rightInset: insets.trailing,
                onPanel: { p in panel = panel == p ? nil : p },
                onCollapse: collapse
            )
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .trailing)
        .padding(.top, insets.top)
        .padding(.bottom, insets.bottom)
    }
}

/// 只有左侧两个角是圆角的矩形（贴右边缘的把手与菜单）
struct LeadingRoundedRect: Shape {
    var radius: CGFloat

    func path(in rect: CGRect) -> Path {
        let r = min(radius, rect.height / 2, rect.width)
        var p = Path()
        p.move(to: CGPoint(x: rect.maxX, y: rect.minY))
        p.addLine(to: CGPoint(x: rect.minX + r, y: rect.minY))
        p.addArc(center: CGPoint(x: rect.minX + r, y: rect.minY + r), radius: r, startAngle: .degrees(-90), endAngle: .degrees(180), clockwise: true)
        p.addLine(to: CGPoint(x: rect.minX, y: rect.maxY - r))
        p.addArc(center: CGPoint(x: rect.minX + r, y: rect.maxY - r), radius: r, startAngle: .degrees(180), endAngle: .degrees(90), clockwise: true)
        p.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY))
        p.closeSubpath()
        return p
    }
}

// MARK: 收起状态的把手

/// 贴右边缘的把手：轻点展开、上下滑动切换显示器（只有一块屏时直接拖动改位置）、长按后拖动改位置（震动一下）
struct DockHandle: View {
    @ObservedObject var session: RemoteSession
    let rightInset: CGFloat
    let onTap: () -> Void
    let onSwipe: (Int) -> Void
    let onMove: (CGFloat) -> Void
    @Environment(\.loc) private var loc
    @State private var tracker: DockHandleTracker?
    @State private var longPress: Task<Void, Never>?

    var body: some View {
        HStack(spacing: 0) {
            Spacer(minLength: 0)
            face
        }
        .frame(width: DockLayout.handleTouchWidth + rightInset, height: DockLayout.handleTouchHeight)
        .contentShape(Rectangle())
        // 全局坐标：拖动改位置时把手本身在动，局部坐标的位移会跟着变
        .gesture(DragGesture(minimumDistance: 0, coordinateSpace: .global).onChanged(changed).onEnded(ended))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(loc("remote.menuExpand"))
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { onTap() }
        .accessibilityAdjustableAction { dir in
            switch dir {
            case .increment: onSwipe(1)
            case .decrement: onSwipe(-1)
            @unknown default: break
            }
        }
    }

    private var face: some View {
        VStack(spacing: 4) {
            DockStatusDot(peer: session.peer, recording: session.isRecording, size: 6)
            Image(systemName: "chevron.left")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(.white)
            if session.displays.count > 1 {
                DisplayDots(ids: session.displays.map(\.id), current: session.displayId)
            }
        }
        .frame(width: DockLayout.handleWidth, height: DockLayout.handleHeight)
        .padding(.trailing, rightInset)
        .background(LeadingRoundedRect(radius: 12).fill(DockColors.handle))
        .overlay(LeadingRoundedRect(radius: 12).stroke(DockColors.line, lineWidth: 1))
    }

    private func changed(_ v: DragGesture.Value) {
        var t = tracker ?? DockHandleTracker(canSwitch: session.displays.count > 1)
        if tracker == nil { startLongPressTimer() }
        let dy = t.moved(to: v.translation.height)
        tracker = t
        if t.mode == .swipe { longPress?.cancel() }
        if dy != 0 { onMove(dy) }
    }

    private func startLongPressTimer() {
        longPress?.cancel()
        longPress = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 500_000_000)
            guard !Task.isCancelled, var t = tracker else { return }
            let entered = t.longPressElapsed()
            tracker = t
            if entered { UIImpactFeedbackGenerator(style: .medium).impactOccurred() }
        }
    }

    private func ended(_ v: DragGesture.Value) {
        longPress?.cancel()
        longPress = nil
        guard let t = tracker else { return }
        tracker = nil
        switch t.ended() {
        case .tap: onTap()
        case .switchDisplay(let step): onSwipe(step)
        case .none: break
        }
    }
}

/// 连接状态点：绿 = 局域网 / 直连，黄 = 中转，红 = 录像中；没有路径信息时不画
struct DockStatusDot: View {
    let peer: PeerConnection?
    let recording: Bool
    var size: CGFloat = 6

    var body: some View {
        if let peer {
            PeerDot(peer: peer, recording: recording, size: size)
        } else if recording {
            Circle().fill(DockColors.record).frame(width: size, height: size)
        }
    }
}

private struct PeerDot: View {
    @ObservedObject var peer: PeerConnection
    let recording: Bool
    let size: CGFloat
    @Environment(\.palette) private var p

    var body: some View {
        if let color = dotColor {
            Circle().fill(color).frame(width: size, height: size)
        }
    }

    private var dotColor: Color? {
        switch DockDot.of(path: peer.path.kind, recording: recording) {
        case .online: return p.success
        case .relay: return p.warning
        case .recording: return DockColors.record
        case .none: return nil
        }
    }
}

/// 竖排小圆点：几块屏、当前第几块（最多画 5 个）
struct DisplayDots: View {
    let ids: [UInt32]
    let current: UInt32?

    var body: some View {
        VStack(spacing: 3) {
            ForEach(Array(ids.prefix(5)), id: \.self) { id in
                dot(on: id == current)
            }
        }
    }

    private func dot(on: Bool) -> some View {
        Circle()
            .fill(on ? DockColors.accent : Color.white.opacity(0.4))
            .frame(width: on ? 5 : 4, height: on ? 5 : 4)
    }
}
