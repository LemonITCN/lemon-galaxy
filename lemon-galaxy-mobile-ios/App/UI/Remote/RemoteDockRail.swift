import SwiftUI
import GalaxyKit

/// 展开后的右侧竖排菜单：固定几项，按可用高度自适应，不需要滚动。上下滑动切换显示器。
struct DockRail: View {
    @ObservedObject var session: RemoteSession
    let actions: DockActions
    let panel: RemotePanel?
    let availableHeight: CGFloat
    let rightInset: CGFloat
    let onPanel: (RemotePanel) -> Void
    let onCollapse: () -> Void
    @Environment(\.loc) private var loc
    @State private var swipe = RailSwipeTracker()
    /// 刚滑过一次：这次抬起不再算点按子项
    @State private var swipeEndedAt = Date.distantPast

    private var multi: Bool { session.displays.count > 1 }
    private var itemH: CGFloat { DockLayout.railItemHeight(available: availableHeight, multiDisplay: multi) }

    var body: some View {
        VStack(spacing: 0) {
            collapseRow
            RailStatus(peer: session.peer)
            if multi { switcher }
            Rectangle().fill(DockColors.line).frame(width: 36, height: 1).padding(.vertical, 4)
            items
        }
        .padding(.vertical, 6)
        .frame(width: DockLayout.railWidth)
        .padding(.trailing, rightInset)
        .background(LeadingRoundedRect(radius: 16).fill(DockColors.bg))
        .overlay(LeadingRoundedRect(radius: 16).stroke(DockColors.line, lineWidth: 1))
        .contentShape(Rectangle())
        .simultaneousGesture(swipeGesture)
    }

    /// 菜单上上下滑动：切换显示器（越过阈值后子项的点按作废）
    private var swipeGesture: some Gesture {
        DragGesture(minimumDistance: DockLayout.slop, coordinateSpace: .global)
            .onChanged { v in swipe.moved(to: v.translation) }
            .onEnded { _ in
                let step = swipe.ended()
                if swipe.dragging { swipeEndedAt = Date() }
                swipe = RailSwipeTracker()
                if let step { actions.switchDisplay(step) }
            }
    }

    /// 子项点按：正在滑动或刚滑完时不算
    private func tap(_ action: @escaping () -> Void) -> () -> Void {
        {
            guard !swipe.dragging, Date().timeIntervalSince(swipeEndedAt) > 0.3 else { return }
            action()
        }
    }

    private var collapseRow: some View {
        Button(action: tap(onCollapse)) {
            Image(systemName: "chevron.right")
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(DockColors.dim)
                .frame(maxWidth: .infinity)
                .frame(height: 24)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(loc("remote.menuCollapse"))
    }

    private var switcher: some View {
        let ids = session.displays.map(\.id)
        let idx = DisplayCycle.index(of: session.displayId, in: ids)
        return VStack(spacing: 0) {
            arrowButton("chevron.up", label: loc("remote.prevDisplay"), step: -1)
            Text("\(idx + 1) / \(ids.count)")
                .font(.system(size: 12, weight: .semibold).monospacedDigit())
                .foregroundStyle(DockColors.accent)
                .frame(height: 20)
            arrowButton("chevron.down", label: loc("remote.nextDisplay"), step: 1)
        }
    }

    private func arrowButton(_ icon: String, label: String, step: Int) -> some View {
        Button(action: tap { actions.switchDisplay(step) }) {
            Image(systemName: icon)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(.white)
                .frame(maxWidth: .infinity)
                .frame(height: 17)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }

    @ViewBuilder private var items: some View {
        RailItem(icon: "display", label: loc("remote.display"), height: itemH, selected: panel == .display,
                 action: tap { onPanel(.display) })
        RailItem(icon: "keyboard", label: loc("remote.keyboard"), height: itemH, active: session.keyboardShown,
                 action: tap(actions.keyboard))
        RailItem(icon: "command", label: loc("remote.shortcuts"), height: itemH, selected: panel == .shortcuts,
                 action: tap { onPanel(.shortcuts) })
        modeItem
        ToolsRailItem(start: session.recordingStartedAt, label: loc("remote.tools"), height: itemH, selected: panel == .tools,
                      action: tap { onPanel(.tools) })
        RailItem(icon: "xmark.circle", label: loc("remote.disconnect"), height: itemH, danger: true,
                 action: tap(actions.disconnect))
    }

    private var modeItem: some View {
        let mouse = session.mouseMode
        return RailItem(icon: mouse ? "cursorarrow" : "hand.tap", label: loc(mouse ? "remote.modeMouse" : "remote.modeTouch"),
                        height: itemH, active: mouse, action: tap(actions.mouseMode))
    }
}

/// 菜单顶部的连接状态：路径（局域网 / 直连 / 中转）+ 往返延迟
private struct RailStatus: View {
    let peer: PeerConnection?

    var body: some View {
        Group {
            if let peer {
                RailPath(peer: peer)
            } else {
                Color.clear
            }
        }
        .frame(height: 30)
    }
}

private struct RailPath: View {
    @ObservedObject var peer: PeerConnection
    @Environment(\.loc) private var loc
    @Environment(\.palette) private var p

    var body: some View {
        let path = peer.path
        let show = path.kind != .connecting && path.kind != .closed
        VStack(spacing: 1) {
            HStack(spacing: 4) {
                Circle().fill(PathStyle.color(path.kind, p)).frame(width: 6, height: 6)
                Text(PathStyle.label(path.kind, loc)).font(.system(size: 10)).foregroundStyle(.white).lineLimit(1)
            }
            if show {
                Text(loc("path.rtt", path.rttMs)).font(.system(size: 10).monospacedDigit()).foregroundStyle(DockColors.dim).lineLimit(1)
            }
        }
        .minimumScaleFactor(0.8)
        .padding(.horizontal, 4)
    }
}

/// 竖排菜单的一项：图标 + 小字。iPhone 横屏一格只有 38 左右，比安卓端（低于 40 只留图标）多留文字：图标收紧一点，文字始终显示
struct RailItem: View {
    let icon: String
    let label: String
    let height: CGFloat
    var selected = false
    var active = false
    var danger = false
    var badge: Color?
    var monospaced = false
    let action: () -> Void

    private var tint: Color {
        if danger { return DockColors.danger }
        return active || selected ? DockColors.accent : .white
    }

    var body: some View {
        Button(action: action) { content }
            .buttonStyle(.plain)
            .accessibilityLabel(label)
            .accessibilityAddTraits(selected || active ? .isSelected : [])
    }

    private var content: some View {
        VStack(spacing: height >= 46 ? 3 : 0) {
            iconView
            labelView
        }
        .frame(maxWidth: .infinity)
        .frame(height: height)
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(selected ? Color.white.opacity(0.1) : .clear))
        .padding(.horizontal, 4)
        .contentShape(Rectangle())
    }

    private var iconView: some View {
        Image(systemName: icon)
            .font(.system(size: height >= 46 ? 19 : 16))
            .foregroundStyle(tint)
            .frame(height: height >= 46 ? 22 : 20)
            .overlay(alignment: .topTrailing) {
                if let badge {
                    Circle().fill(badge).frame(width: 7, height: 7).offset(x: 4, y: -1)
                }
            }
    }

    private var labelView: some View {
        Text(label)
            .font(monospaced ? .system(size: 10).monospacedDigit() : .system(size: 10))
            .foregroundStyle(tint.opacity(0.92))
            .lineLimit(1)
            .minimumScaleFactor(0.8)
            .padding(.horizontal, 2)
    }
}

/// 「工具」项：录像中显示红点与计时
private struct ToolsRailItem: View {
    let start: Date?
    let label: String
    let height: CGFloat
    let selected: Bool
    let action: () -> Void

    var body: some View {
        if let start {
            TimelineView(.periodic(from: start, by: 1)) { ctx in
                item(RecordingClock.text(since: start, now: ctx.date), badge: DockColors.record)
            }
        } else {
            item(label, badge: nil)
        }
    }

    private func item(_ text: String, badge: Color?) -> some View {
        RailItem(icon: "square.grid.2x2", label: text, height: height, selected: selected, badge: badge, monospaced: badge != nil, action: action)
    }
}
