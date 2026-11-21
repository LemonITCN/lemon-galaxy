import SwiftUI
import UIKit
import GalaxyKit

struct RemoteCanvasHost: UIViewRepresentable {
    let session: RemoteSession
    func makeUIView(context: Context) -> UIView {
        let v = UIView()
        v.backgroundColor = .black
        return v
    }
    func updateUIView(_ container: UIView, context: Context) {
        if session.canvas.superview !== container {
            container.subviews.forEach { $0.removeFromSuperview() }
            session.canvas.frame = container.bounds
            session.canvas.autoresizingMask = [.flexibleWidth, .flexibleHeight]
            container.addSubview(session.canvas)
        }
    }
}

/// 远程桌面：画面占满，右侧一个可拖动的悬浮球，点开是工具栏（屏幕、画质、输入方式、键盘、快捷键、声音、录像、隐私屏、剪贴板、断开）。
struct RemoteScreen: View {
    @ObservedObject var session: RemoteSession
    var onMinimize: (() -> Void)?
    @EnvironmentObject var app: AppModel
    @Environment(\.loc) private var loc
    @State private var toolbarOpen = false
    @State private var ball = CGPoint(x: -1, y: -1)
    @State private var dragStart: CGPoint?

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .topLeading) {
                RemoteCanvasHost(session: session)
                    .ignoresSafeArea()
                statusOverlay
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                if toolbarOpen {
                    toolbar
                        .frame(maxWidth: .infinity, alignment: .top)
                        .padding(.top, 8)
                        .padding(.horizontal, 10)
                        .transition(.move(edge: .top).combined(with: .opacity))
                } else if let start = session.recordingStartedAt {
                    // 工具栏收起时也看得到在录像
                    RecordingBadge(start: start)
                        .frame(maxWidth: .infinity, alignment: .top)
                        .padding(.top, 8)
                }
                floatingBall(in: geo.size)
            }
        }
        .background(Color.black.ignoresSafeArea())
        .statusBarHidden(true)
        .persistentSystemOverlays(.hidden)
        .onAppear { session.canvas.onHideKeyboard = { setKeyboard(false) } }
        // 离开画面（收起、切到别的会话）就停止录像
        .onDisappear { session.stopRecording(.left) }
        .modifier(RecordingSavedAlert(session: session))
    }

    private func setKeyboard(_ on: Bool) {
        session.keyboardShown = on
        if on { _ = session.canvas.becomeFirstResponder() } else { _ = session.canvas.resignFirstResponder() }
    }

    @ViewBuilder private var statusOverlay: some View {
        switch session.status {
        case .streaming:
            EmptyView()
        case .connecting:
            VStack(spacing: 12) {
                ProgressView().tint(.white)
                Text(loc("remote.connecting")).font(.subheadline).foregroundStyle(.white.opacity(0.8))
            }
        case .failed(let m):
            VStack(spacing: 14) {
                Image(systemName: "display.trianglebadge.exclamationmark").font(.system(size: 34)).foregroundStyle(.white.opacity(0.8))
                Text(m.isEmpty ? loc("remote.failed") : m).font(.subheadline).foregroundStyle(.white).multilineTextAlignment(.center)
                HStack {
                    Button(loc("common.retry")) { session.retry() }.buttonStyle(.borderedProminent)
                    Button(loc("remote.disconnect")) { app.sessions.close(session.id) }.buttonStyle(.bordered).tint(.white)
                }
            }
            .padding(24)
            .frame(maxWidth: 340)
        }
    }

    private func floatingBall(in size: CGSize) -> some View {
        let d: CGFloat = 48
        let pos = ball.x < 0 ? CGPoint(x: size.width - d / 2 - 14, y: size.height * 0.4) : ball
        return ZStack {
            Circle().fill(Color(hex: "#1B1E2B").opacity(0.82))
            Circle().stroke(Color.white.opacity(0.25), lineWidth: 1)
            Image(systemName: toolbarOpen ? "xmark" : "slider.horizontal.3")
                .font(.system(size: 19, weight: .semibold))
                .foregroundStyle(toolbarOpen ? Color.white : Color(hex: "#C6E35A"))
        }
        .frame(width: d, height: d)
        .contentShape(Circle())
        .position(pos)
        .gesture(
            DragGesture(minimumDistance: 4)
                .onChanged { v in
                    if dragStart == nil { dragStart = pos }
                    let s = dragStart!
                    ball = CGPoint(x: min(max(s.x + v.translation.width, d / 2), size.width - d / 2),
                                   y: min(max(s.y + v.translation.height, d / 2), size.height - d / 2))
                }
                .onEnded { _ in dragStart = nil }
        )
        .onTapGesture { withAnimation(.spring(response: 0.3)) { toolbarOpen.toggle() } }
        .accessibilityLabel(loc("remote.toolbar"))
    }

    // 工具栏拆成小块、字符串先算好再交给视图：大的 ViewBuilder 表达式在较慢的机器上会超出编译器的类型检查时限
    private var toolbar: some View {
        VStack(alignment: .leading, spacing: 6) {
            toolbarHeader
            ScrollView(.horizontal, showsIndicators: false) {
                toolbarButtons
            }
        }
        .padding(.vertical, 10)
        .padding(.horizontal, 6)
        .frame(maxWidth: 720)
        .background(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(Color(hex: "#1B1E2B").opacity(0.94)))
        .shadow(color: .black.opacity(0.4), radius: 14, y: 6)
    }

    private var toolbarHeader: some View {
        HStack(spacing: 10) {
            Text(session.title).font(.subheadline.weight(.semibold)).foregroundStyle(.white).lineLimit(1)
            if let p = session.peer { PathBadge(path: p.path, light: true) }
            if let d = session.display {
                let stats: String = "\(d.width)×\(d.height) · \(session.fps) fps"
                Text(stats)
                    .font(.caption2.monospacedDigit()).foregroundStyle(.white.opacity(0.7))
            }
            Spacer(minLength: 0)
            InfoButton(text: loc("remote.modeInfo"))
                .environment(\.palette, Palettes.make("galaxy", dark: true))
        }
        .padding(.horizontal, 8)
    }

    private var toolbarButtons: some View {
        HStack(spacing: 2) {
            if let onMinimize {
                ToolButton(icon: "chevron.down", label: loc("session.minimize")) { onMinimize() }
            }
            displayMenu
            qualityMenu
            inputModeButton
            ToolButton(icon: "keyboard", label: loc("remote.keyboard"), active: session.keyboardShown) {
                setKeyboard(!session.keyboardShown)
                toolbarOpen = false
            }
            shortcutsMenu
            mediaButtons
            clipboardMenu
            ToolButton(icon: "xmark.circle", label: loc("remote.disconnect"), danger: true) {
                app.sessions.close(session.id)
            }
        }
    }

    private func displayTitle(_ d: DisplayInfo) -> String {
        var t: String = d.name + "  \(d.width)×\(d.height)"
        if d.primary { t += " · " + loc("remote.primary") }
        if d.isVirtual { t += " · " + loc("remote.virtual") }
        return t
    }

    private var displayMenu: some View {
        Menu {
            ForEach(session.displays, id: \.id) { (d: DisplayInfo) in
                let icon: String = d.id == session.displayId ? "checkmark" : "display"
                Button {
                    session.switchDisplay(d.id)
                } label: {
                    Label(displayTitle(d), systemImage: icon)
                }
            }
        } label: { ToolLabel(icon: "display.2", label: loc("remote.display")) }
    }

    private func qualityTitle(_ q: VideoQuality) -> String {
        let name: String = loc("quality." + q.rawValue)
        guard q.bitrateKbps > 0 else { return name }
        return name + " · \(q.bitrateKbps / 1000) Mbps"
    }

    private var qualityMenu: some View {
        Menu {
            ForEach(VideoQuality.allCases) { (q: VideoQuality) in
                let icon: String = q == session.quality ? "checkmark" : "dial.medium"
                Button {
                    session.setQuality(q)
                } label: {
                    Label(qualityTitle(q), systemImage: icon)
                }
            }
        } label: { ToolLabel(icon: "sparkles.tv", label: loc("quality." + session.quality.rawValue)) }
    }

    private var inputModeButton: some View {
        let mouse: Bool = session.mouseMode
        let icon: String = mouse ? "cursorarrow.motionlines" : "hand.tap"
        let label: String = loc(mouse ? "remote.modeMouse" : "remote.modeTouch")
        return ToolButton(icon: icon, label: label, active: mouse) {
            session.mouseMode.toggle()
            app.settings.remoteMouseMode = session.mouseMode
        }
    }

    private var shortcutsMenu: some View {
        let list: [RemoteShortcut] = RemoteShortcuts.list(for: session.platform)
        let common: [RemoteShortcut] = list.filter { $0.group == 0 }
        let system: [RemoteShortcut] = list.filter { $0.group == 1 }
        return Menu {
            Section {
                ForEach(common) { (s: RemoteShortcut) in
                    Button(loc("combo." + s.id)) { session.send(s.events) }
                }
            }
            Section {
                ForEach(system) { (s: RemoteShortcut) in
                    Button(loc("combo." + s.id)) { session.send(s.events) }
                }
            }
        } label: { ToolLabel(icon: "command", label: loc("remote.shortcuts")) }
    }

    /// 声音、录像、隐私屏（单独成组：一个 ViewBuilder 块里的子视图不超过 10 个）
    @ViewBuilder private var mediaButtons: some View {
        if session.supportsAudio { soundButton }
        RecordButton(session: session)
        if session.supportsPrivacy { privacyButton }
    }

    private var soundButton: some View {
        let on: Bool = session.audioOn
        let icon: String = on ? "speaker.wave.2.fill" : "speaker.slash"
        return ToolButton(icon: icon, label: loc("remote.sound"), active: on) {
            session.setAudio(!on)
        }
        .disabled(session.audioBusy)
    }

    private var privacyButton: some View {
        let on: Bool = session.privacyOn
        let icon: String = on ? "eye.slash.fill" : "eye.slash"
        return ToolButton(icon: icon, label: loc("remote.privacy"), active: on) {
            session.setPrivacy(!on)
        }
        .disabled(session.privacyBusy)
    }

    private var clipboardMenu: some View {
        Menu {
            Button { session.sendClipboard() } label: { Label(loc("remote.sendClipboard"), systemImage: "arrow.up.doc.on.clipboard") }
            Button { session.fetchClipboard() } label: { Label(loc("remote.getClipboard"), systemImage: "arrow.down.doc") }
        } label: { ToolLabel(icon: "doc.on.clipboard", label: loc("remote.clipboard")) }
    }
}

private struct ToolLabel: View {
    let icon: String
    let label: String
    var active = false
    var danger = false
    var body: some View {
        let tint: Color = danger ? Color(hex: "#FF7A85") : (active ? Color(hex: "#C6E35A") : .white)
        VStack(spacing: 4) {
            Image(systemName: icon).font(.system(size: 19)).frame(height: 22)
            Text(label).font(.caption2).lineLimit(1)
        }
        .foregroundStyle(tint)
        .frame(minWidth: 58)
        .padding(.horizontal, 6)
        .padding(.vertical, 6)
        .contentShape(Rectangle())
    }
}

/// 录像按钮：录制中变成红色停止键，标签是计时
private struct RecordButton: View {
    @ObservedObject var session: RemoteSession
    @Environment(\.loc) private var loc

    var body: some View {
        let recording: Bool = session.isRecording
        let disabled: Bool = session.recordingBusy || (!recording && session.status != .streaming)
        let a11y: String = loc(recording ? "remote.recordStop" : "remote.record")
        Button { session.toggleRecording() } label: { label }
            .buttonStyle(.plain)
            .disabled(disabled)
            .opacity(disabled ? 0.5 : 1)
            .accessibilityLabel(a11y)
    }

    @ViewBuilder private var label: some View {
        if let start = session.recordingStartedAt {
            TimelineView(.periodic(from: start, by: 1)) { ctx in
                ToolLabel(icon: "stop.circle.fill", label: RecordingClock.text(since: start, now: ctx.date), danger: true)
            }
        } else {
            ToolLabel(icon: "record.circle", label: loc("remote.record"))
        }
    }
}

/// 工具栏收起时画面顶部的红点与计时（不挡触控）
private struct RecordingBadge: View {
    let start: Date

    var body: some View {
        TimelineView(.periodic(from: start, by: 1)) { ctx in
            badge(RecordingClock.text(since: start, now: ctx.date))
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    private func badge(_ text: String) -> some View {
        HStack(spacing: 6) {
            Circle().fill(Color(hex: "#FF4D5E")).frame(width: 8, height: 8)
            Text(text).font(.caption.monospacedDigit().weight(.semibold)).foregroundStyle(.white)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .background(Capsule().fill(Color.black.opacity(0.6)))
    }
}

enum RecordingClock {
    static func text(since start: Date, now: Date) -> String {
        Fmt.elapsed(Int(now.timeIntervalSince(start)))
    }
}

private struct ToolButton: View {
    let icon: String
    let label: String
    var active = false
    var danger = false
    let action: () -> Void
    var body: some View {
        Button(action: action) { ToolLabel(icon: icon, label: label, active: active, danger: danger) }
            .buttonStyle(.plain)
    }
}
