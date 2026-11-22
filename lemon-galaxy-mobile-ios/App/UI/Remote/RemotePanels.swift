import SwiftUI
import GalaxyKit

/// 二级面板：从菜单左侧弹出。内容按网格排，常规屏幕上一屏放得下；极矮的屏幕才滚动（面板不承担切换显示器手势）。
struct DockPanelView: View {
    let panel: RemotePanel
    @ObservedObject var session: RemoteSession
    let actions: DockActions
    let portrait: Bool
    let onClose: () -> Void
    @Environment(\.loc) private var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            ViewThatFits(in: .vertical) {
                content
                ScrollView(.vertical, showsIndicators: false) { content }
            }
        }
        .background(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(DockColors.bg))
        .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).stroke(DockColors.line, lineWidth: 1))
        // 吃掉面板内空白处的点按，不穿透到下面的收起层
        .contentShape(Rectangle())
        .onTapGesture {}
    }

    private var title: String {
        switch panel {
        case .display: return loc("remote.display")
        case .shortcuts: return loc("remote.shortcuts")
        case .tools: return loc("remote.tools")
        }
    }

    private var header: some View {
        HStack(spacing: 4) {
            Text(title).font(.subheadline.weight(.semibold)).foregroundStyle(.white)
            Spacer(minLength: 4)
            if panel == .tools { InfoButton(text: loc("remote.modeInfo")) }
            Button(action: onClose) {
                Image(systemName: "xmark")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(DockColors.dim)
                    .frame(width: 40, height: 40)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(loc("common.close"))
        }
        .padding(.leading, 16)
        .padding(.trailing, 4)
        .padding(.top, 4)
    }

    @ViewBuilder private var content: some View {
        VStack(alignment: .leading, spacing: 0) {
            switch panel {
            case .display: DisplayPanel(session: session, actions: actions, portrait: portrait)
            case .shortcuts: ShortcutsPanel(session: session)
            case .tools: ToolsPanel(session: session, actions: actions)
            }
        }
        .padding(.horizontal, 14)
        .padding(.bottom, 14)
    }
}

struct PanelSectionTitle: View {
    let text: String
    var body: some View {
        Text(text).font(.caption.weight(.medium)).foregroundStyle(DockColors.dim).padding(.top, 10).padding(.bottom, 6)
    }
}

/// 面板里的小按钮（画质、视图、快捷键）
struct DockChip: View {
    let label: String
    var icon: String?
    var selected = false
    var enabled = true
    let action: () -> Void

    private var fg: Color {
        if !enabled { return DockColors.disabled }
        return selected ? DockColors.onAccent : .white
    }

    var body: some View {
        Button(action: action) { face }
            .buttonStyle(.plain)
            .disabled(!enabled)
            .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private var face: some View {
        HStack(spacing: 4) {
            if let icon { Image(systemName: icon).font(.system(size: 12, weight: .medium)) }
            Text(label).font(.system(size: 12)).lineLimit(1).minimumScaleFactor(0.75)
        }
        .foregroundStyle(fg)
        .padding(.horizontal, 4)
        .frame(maxWidth: .infinity)
        .frame(height: 36)
        .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(selected && enabled ? DockColors.accent : DockColors.chip))
        .contentShape(Rectangle())
    }
}

/// 固定列数的网格（不足一行的格子留空，保证每格等宽）
struct DockGrid<Item, Cell: View>: View {
    let items: [Item]
    let columns: Int
    let cell: (Item) -> Cell

    var body: some View {
        VStack(spacing: 6) {
            ForEach(0..<rowCount, id: \.self) { r in
                HStack(spacing: 6) {
                    ForEach(0..<columns, id: \.self) { c in
                        slot(r * columns + c)
                    }
                }
            }
        }
    }

    private var rowCount: Int { (items.count + columns - 1) / columns }

    @ViewBuilder private func slot(_ i: Int) -> some View {
        if i < items.count {
            cell(items[i]).frame(maxWidth: .infinity)
        } else {
            Color.clear.frame(maxWidth: .infinity, maxHeight: 0)
        }
    }
}

// MARK: 显示器

/// 显示器：屏幕卡片、画质五档、视图（适应屏幕 / 原始像素 / 横竖屏）、连接路径与分辨率
struct DisplayPanel: View {
    @ObservedObject var session: RemoteSession
    let actions: DockActions
    let portrait: Bool
    @Environment(\.loc) private var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if !session.displays.isEmpty { displays }
            PanelSectionTitle(text: loc("remote.quality"))
            qualityRow
            PanelSectionTitle(text: loc("remote.view"))
            viewRow
            if let peer = session.peer {
                DisplayPathLine(peer: peer, display: session.display, fps: session.fps)
                    .padding(.top, 10)
            }
        }
    }

    private var displays: some View {
        VStack(alignment: .leading, spacing: 6) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(Array(session.displays.enumerated()), id: \.element.id) { i, d in
                        DisplayCard(index: i, display: d, selected: d.id == session.displayId) { actions.pickDisplay(d.id) }
                    }
                }
                .padding(.top, 4)
            }
            if session.displays.count > 1 {
                Text(loc("remote.swipeHint")).font(.system(size: 11)).foregroundStyle(DockColors.dim)
            }
        }
    }

    private var qualityRow: some View {
        HStack(spacing: 6) {
            ForEach(VideoQuality.allCases) { q in
                DockChip(label: loc("quality." + q.rawValue), selected: q == session.quality) { session.setQuality(q) }
            }
        }
    }

    private var viewRow: some View {
        let flags = session.viewFlags
        return HStack(spacing: 6) {
            DockChip(label: loc("remote.fit"), icon: "arrow.down.right.and.arrow.up.left", selected: !flags.zoomed, action: actions.fit)
            DockChip(label: loc("remote.actualSize"), icon: "1.magnifyingglass", selected: flags.atActualSize,
                     enabled: flags.canActualSize, action: actions.actualSize)
            if let rotate = actions.rotate {
                DockChip(label: loc(portrait ? "remote.toLandscape" : "remote.toPortrait"), icon: "rotate.right") { rotate(portrait) }
            }
        }
    }
}

private struct DisplayCard: View {
    let index: Int
    let display: DisplayInfo
    let selected: Bool
    let action: () -> Void
    @Environment(\.loc) private var loc

    private var detail: String {
        var s: String = "\(display.width)×\(display.height)"
        if display.primary { s += " · " + loc("remote.primary") }
        if display.isVirtual { s += " · " + loc("remote.virtual") }
        return s
    }

    var body: some View {
        Button(action: action) { face }
            .buttonStyle(.plain)
            .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private var face: some View {
        HStack(spacing: 8) {
            Text("\(index + 1)")
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(selected ? DockColors.onAccent : .white)
                .frame(width: 24, height: 24)
                .background(RoundedRectangle(cornerRadius: 6, style: .continuous).fill(selected ? DockColors.accent : Color.white.opacity(0.14)))
            VStack(alignment: .leading, spacing: 1) {
                Text(display.name).font(.caption).foregroundStyle(.white).lineLimit(1)
                Text(detail).font(.system(size: 11).monospacedDigit()).foregroundStyle(DockColors.dim).lineLimit(1)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .frame(minWidth: 128, maxWidth: 180, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(selected ? DockColors.accent.opacity(0.14) : DockColors.chip))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(selected ? DockColors.accent : .clear, lineWidth: 1))
    }
}

/// 连接路径 · 往返延迟 · 分辨率 · 帧率，旁边是路径说明
private struct DisplayPathLine: View {
    @ObservedObject var peer: PeerConnection
    let display: DisplayInfo?
    let fps: Int
    @Environment(\.loc) private var loc
    @Environment(\.palette) private var p

    private var text: String {
        let path = peer.path
        var s = PathStyle.label(path.kind, loc)
        if path.kind != .connecting && path.kind != .closed { s += " · " + loc("path.rtt", path.rttMs) }
        if let d = display { s += " · \(d.width)×\(d.height)" }
        s += " · \(fps) fps"
        return s
    }

    var body: some View {
        HStack(spacing: 6) {
            Circle().fill(PathStyle.color(peer.path.kind, p)).frame(width: 7, height: 7)
            Text(text).font(.caption.monospacedDigit()).foregroundStyle(DockColors.dim).lineLimit(1).minimumScaleFactor(0.8)
            InfoButton(text: loc("path.info"))
        }
    }
}

// MARK: 快捷键

/// 快捷键：系统组合键一组、编辑常用键一组，三列网格
struct ShortcutsPanel: View {
    @ObservedObject var session: RemoteSession
    @Environment(\.loc) private var loc

    var body: some View {
        let list = RemoteShortcuts.list(for: session.platform)
        VStack(spacing: 0) {
            grid(list.filter { $0.group == 0 })
                .padding(.top, 4)
            Rectangle().fill(DockColors.line).frame(height: 1).padding(.vertical, 10)
            grid(list.filter { $0.group == 1 })
        }
    }

    private func grid(_ items: [RemoteShortcut]) -> some View {
        DockGrid(items: items, columns: 3) { s in
            DockChip(label: shortLabel(s)) { session.send(s.events) }
        }
    }

    /// 面板格子窄：去掉文字里括号中的组合键说明（如「强制退出（Cmd+Opt+Esc）」只留「强制退出」）
    private func shortLabel(_ s: RemoteShortcut) -> String {
        let full = loc("combo." + s.id)
        guard let cut = full.firstIndex(where: { $0 == "（" || $0 == "(" }), cut != full.startIndex else { return full }
        return full[..<cut].trimmingCharacters(in: .whitespaces)
    }
}

// MARK: 工具

private struct ToolItem: Identifiable {
    let id: String
    let icon: String
    let label: String
    var active = false
    var enabled = true
    var recordingSince: Date?
    var a11y: String?
    let action: () -> Void
}

/// 工具：声音、隐私屏、录像、剪贴板、文件（iPhone 上还有收起画面），三列方块，开启时高亮
struct ToolsPanel: View {
    @ObservedObject var session: RemoteSession
    let actions: DockActions
    @Environment(\.loc) private var loc

    var body: some View {
        DockGrid(items: tools, columns: 3) { t in ToolTile(tool: t) }
            .padding(.top, 4)
    }

    private var tools: [ToolItem] {
        var list: [ToolItem] = []
        if session.supportsAudio { list.append(sound) }
        if session.supportsPrivacy { list.append(privacy) }
        list.append(record)
        list.append(ToolItem(id: "send", icon: "arrow.up.doc.on.clipboard", label: loc("remote.sendClipboard")) { session.sendClipboard() })
        list.append(ToolItem(id: "get", icon: "arrow.down.doc", label: loc("remote.getClipboard")) { session.fetchClipboard() })
        list.append(ToolItem(id: "files", icon: "folder", label: loc("remote.files"), action: actions.files))
        if let minimize = actions.minimize {
            list.append(ToolItem(id: "minimize", icon: "rectangle.bottomhalf.inset.filled", label: loc("remote.minimize"), action: minimize))
        }
        return list
    }

    private var sound: ToolItem {
        let on = session.audioOn
        return ToolItem(id: "sound", icon: on ? "speaker.wave.2.fill" : "speaker.slash", label: loc("remote.sound"),
                        active: on, enabled: !session.audioBusy) { session.setAudio(!on) }
    }

    private var privacy: ToolItem {
        let on = session.privacyOn
        return ToolItem(id: "privacy", icon: on ? "eye.slash.fill" : "eye.slash", label: loc("remote.privacy"),
                        active: on, enabled: !session.privacyBusy) { session.setPrivacy(!on) }
    }

    private var record: ToolItem {
        let recording = session.isRecording
        let enabled = !session.recordingBusy && (recording || session.status == .streaming)
        return ToolItem(id: "record", icon: recording ? "stop.fill" : "record.circle", label: loc("remote.record"),
                        enabled: enabled, recordingSince: session.recordingStartedAt,
                        a11y: loc(recording ? "remote.recordStop" : "remote.record")) { session.toggleRecording() }
    }
}

private struct ToolTile: View {
    let tool: ToolItem

    private var danger: Bool { tool.recordingSince != nil }

    private var fg: Color {
        if !tool.enabled { return DockColors.disabled }
        if danger { return .white }
        return tool.active ? DockColors.onAccent : .white
    }

    private var bg: Color {
        if danger { return DockColors.record.opacity(0.85) }
        return tool.active && tool.enabled ? DockColors.accent : DockColors.chip
    }

    var body: some View {
        Button(action: tool.action) { face }
            .buttonStyle(.plain)
            .disabled(!tool.enabled)
            .accessibilityLabel(tool.a11y ?? tool.label)
            .accessibilityAddTraits(tool.active ? .isSelected : [])
    }

    private var face: some View {
        VStack(spacing: 4) {
            Image(systemName: tool.icon).font(.system(size: 19)).frame(height: 22)
            label
        }
        .foregroundStyle(fg)
        .padding(.horizontal, 4)
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity)
        .frame(height: 68)
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(bg))
        .contentShape(Rectangle())
    }

    @ViewBuilder private var label: some View {
        if let start = tool.recordingSince {
            TimelineView(.periodic(from: start, by: 1)) { ctx in
                Text(RecordingClock.text(since: start, now: ctx.date)).font(.system(size: 11).monospacedDigit())
            }
        } else {
            Text(tool.label)
                .font(.system(size: 11))
                .lineLimit(2)
                .multilineTextAlignment(.center)
                .minimumScaleFactor(0.85)
        }
    }
}
