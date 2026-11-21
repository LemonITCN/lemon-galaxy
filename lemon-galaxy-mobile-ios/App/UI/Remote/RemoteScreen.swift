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

/// 远程桌面：画面占满，右侧一个可拖动的悬浮球，点开是工具栏（屏幕、画质、输入方式、键盘、快捷键、剪贴板、断开）。
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
                }
                floatingBall(in: geo.size)
            }
        }
        .background(Color.black.ignoresSafeArea())
        .statusBarHidden(true)
        .persistentSystemOverlays(.hidden)
        .onAppear { session.canvas.onHideKeyboard = { setKeyboard(false) } }
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

    private var toolbar: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                Text(session.title).font(.subheadline.weight(.semibold)).foregroundStyle(.white).lineLimit(1)
                if let p = session.peer { PathBadge(path: p.path, light: true) }
                if let d = session.display {
                    Text("\(d.width)×\(d.height) · \(session.fps) fps")
                        .font(.caption2.monospacedDigit()).foregroundStyle(.white.opacity(0.7))
                }
                Spacer(minLength: 0)
                InfoButton(text: loc("remote.modeInfo"))
                    .environment(\.palette, Palettes.make("galaxy", dark: true))
            }
            .padding(.horizontal, 8)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 2) {
                    if let onMinimize {
                        ToolButton(icon: "chevron.down", label: loc("session.minimize")) { onMinimize() }
                    }
                    Menu {
                        ForEach(session.displays, id: \.id) { d in
                            Button {
                                session.switchDisplay(d.id)
                            } label: {
                                Label(d.name + "  \(d.width)×\(d.height)" + (d.primary ? " · " + loc("remote.primary") : "") + (d.isVirtual ? " · " + loc("remote.virtual") : ""),
                                      systemImage: d.id == session.displayId ? "checkmark" : "display")
                            }
                        }
                    } label: { ToolLabel(icon: "display.2", label: loc("remote.display")) }
                    Menu {
                        ForEach(VideoQuality.allCases) { q in
                            Button {
                                session.setQuality(q)
                            } label: {
                                Label(loc("quality." + q.rawValue) + (q.bitrateKbps > 0 ? " · \(q.bitrateKbps / 1000) Mbps" : ""),
                                      systemImage: q == session.quality ? "checkmark" : "dial.medium")
                            }
                        }
                    } label: { ToolLabel(icon: "sparkles.tv", label: loc("quality." + session.quality.rawValue)) }
                    ToolButton(icon: session.mouseMode ? "cursorarrow.motionlines" : "hand.tap",
                               label: loc(session.mouseMode ? "remote.modeMouse" : "remote.modeTouch"), active: session.mouseMode) {
                        session.mouseMode.toggle()
                        app.settings.remoteMouseMode = session.mouseMode
                    }
                    ToolButton(icon: "keyboard", label: loc("remote.keyboard"), active: session.keyboardShown) {
                        setKeyboard(!session.keyboardShown)
                        toolbarOpen = false
                    }
                    Menu {
                        let list = RemoteShortcuts.list(for: session.platform)
                        Section {
                            ForEach(list.filter { $0.group == 0 }) { s in
                                Button(loc("combo." + s.id)) { session.send(s.events) }
                            }
                        }
                        Section {
                            ForEach(list.filter { $0.group == 1 }) { s in
                                Button(loc("combo." + s.id)) { session.send(s.events) }
                            }
                        }
                    } label: { ToolLabel(icon: "command", label: loc("remote.shortcuts")) }
                    if session.supportsPrivacy {
                        ToolButton(icon: session.privacyOn ? "eye.slash.fill" : "eye.slash", label: loc("remote.privacy"), active: session.privacyOn) {
                            session.setPrivacy(!session.privacyOn)
                        }
                        .disabled(session.privacyBusy)
                    }
                    Menu {
                        Button { session.sendClipboard() } label: { Label(loc("remote.sendClipboard"), systemImage: "arrow.up.doc.on.clipboard") }
                        Button { session.fetchClipboard() } label: { Label(loc("remote.getClipboard"), systemImage: "arrow.down.doc") }
                    } label: { ToolLabel(icon: "doc.on.clipboard", label: loc("remote.clipboard")) }
                    ToolButton(icon: "xmark.circle", label: loc("remote.disconnect"), danger: true) {
                        app.sessions.close(session.id)
                    }
                }
            }
        }
        .padding(.vertical, 10)
        .padding(.horizontal, 6)
        .frame(maxWidth: 720)
        .background(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(Color(hex: "#1B1E2B").opacity(0.94)))
        .shadow(color: .black.opacity(0.4), radius: 14, y: 6)
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
