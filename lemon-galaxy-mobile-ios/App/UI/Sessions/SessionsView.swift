import SwiftUI
import GalaxyKit

/// 打开中的会话（终端 / 远程桌面）。iPhone 上是全屏层（可以收起，会话照常运行）；iPad 上放在分栏的详情区。
struct SessionsView: View {
    @EnvironmentObject var app: AppModel
    @ObservedObject var store: SessionStore
    /// iPhone：收起全屏层
    var onMinimize: (() -> Void)?
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        if let item = store.active {
            switch item {
            case .terminal(let t):
                VStack(spacing: 0) {
                    TerminalTopBar(session: t, store: store, onMinimize: onMinimize)
                    TerminalScreen(session: t)
                }
                .id(t.id)
            case .remote(let r):
                RemoteScreen(session: r, onMinimize: onMinimize)
                    .id(r.id)
            }
        } else {
            EmptyState(icon: "rectangle.stack", title: loc("session.none"))
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(p.background)
        }
    }
}

/// 终端顶栏：收起、会话切换、路径状态、操作菜单。
struct TerminalTopBar: View {
    @ObservedObject var session: TerminalSession
    @ObservedObject var store: SessionStore
    var onMinimize: (() -> Void)?
    @EnvironmentObject var app: AppModel
    @Environment(\.loc) private var loc
    @State private var remoteBox: RemoteSessionsBox?
    @State private var confirmKill = false

    var body: some View {
        let theme = TerminalTheme.named(app.settings.terminalTheme)
        let fg = theme.fgColor
        HStack(spacing: 6) {
            if let onMinimize {
                Button(action: onMinimize) {
                    Image(systemName: "chevron.down").font(.system(size: 17, weight: .semibold)).frame(width: 36, height: 36)
                }
                .accessibilityLabel(loc("session.minimize"))
            }
            Menu {
                ForEach(store.items) { item in
                    Button {
                        store.activeId = item.id
                    } label: {
                        Label(store.title(item), systemImage: item.id == store.active?.id ? "checkmark" : item.icon)
                    }
                }
            } label: {
                HStack(spacing: 6) {
                    statusDot
                    Text(session.title).font(.subheadline.weight(.semibold)).lineLimit(1)
                    if store.items.count > 1 {
                        Text("\(store.items.count)").font(.caption2.weight(.bold))
                            .padding(.horizontal, 6).padding(.vertical, 2)
                            .background(Capsule().fill(fg.opacity(0.15)))
                    }
                    Image(systemName: "chevron.up.chevron.down").font(.caption2)
                }
            }
            Spacer(minLength: 4)
            if let d = session.target.deviceId, let peer = app.account.existingPeer(d) {
                PeerPathView(peer: peer)
            }
            actionsMenu
            Button { store.close(session.id, kill: session.isSsh) } label: {
                Image(systemName: "xmark").font(.system(size: 15, weight: .semibold)).frame(width: 36, height: 36)
            }
            .accessibilityLabel(loc("common.close"))
        }
        .foregroundStyle(fg)
        .padding(.horizontal, 8)
        .frame(height: 46)
        .background(theme.bgColor.overlay(Color.white.opacity(theme.dark ? 0.04 : 0)).ignoresSafeArea(edges: .top))
        .sheet(item: $remoteBox) { box in
            RemoteSessionsSheet(target: session.target, sessions: box.list)
                .environmentObject(app)
        }
        .confirmationDialog(loc("terminal.killConfirm"), isPresented: $confirmKill, titleVisibility: .visible) {
            Button(loc("terminal.kill"), role: .destructive) { store.close(session.id, kill: true) }
        }
    }

    private var statusDot: some View {
        let color: Color
        switch session.status {
        case .connected: color = Color(hex: "#34C77B")
        case .connecting: color = Color(hex: "#F0A43A")
        default: color = Color(hex: "#8A93A2")
        }
        return StatusDot(color: color, size: 7)
    }

    private var actionsMenu: some View {
        Menu {
            Section {
                Button { app.openTerminal(session.target, ask: false) } label: { Label(loc("terminal.newSession"), systemImage: "plus.rectangle.on.rectangle") }
                if let d = session.target.deviceId {
                    Button {
                        Task {
                            do { remoteBox = RemoteSessionsBox(list: try await app.account.peer(d).session.listTerminals()) } catch { app.showError(error) }
                        }
                    } label: { Label(loc("terminal.remoteSessions"), systemImage: "rectangle.stack") }
                }
            }
            Section {
                Button { session.paste() } label: { Label(loc("common.paste"), systemImage: "doc.on.clipboard") }
                Button { session.onShowSnippets?() } label: { Label(loc("snippets.title"), systemImage: "chevron.left.forwardslash.chevron.right") }
                Toggle(isOn: $session.inputBox) { Label(loc("terminal.inputBox"), systemImage: "text.cursor") }
                ControlGroup {
                    Button { session.setFontSize(session.fontSize - 1) } label: { Label(loc("terminal.smaller"), systemImage: "textformat.size.smaller") }
                    Button { session.setFontSize(session.fontSize + 1) } label: { Label(loc("terminal.larger"), systemImage: "textformat.size.larger") }
                }
            }
            Section {
                Button { app.openFiles(session.target) } label: { Label(loc("action.files"), systemImage: "folder") }
                Button { app.openAI([session.target.agentTarget]) } label: { Label(loc("action.ai"), systemImage: "sparkles") }
            }
            Section {
                Button { session.reconnect() } label: { Label(loc("terminal.reconnect"), systemImage: "arrow.clockwise") }
                if !session.isSsh {
                    Button { store.close(session.id, kill: false) } label: { Label(loc("terminal.detach"), systemImage: "rectangle.portrait.and.arrow.right") }
                }
                Button(role: .destructive) { confirmKill = true } label: { Label(loc("terminal.kill"), systemImage: "xmark.octagon") }
            }
        } label: {
            Image(systemName: "ellipsis.circle").font(.system(size: 18)).frame(width: 36, height: 36)
        }
    }
}

private struct RemoteSessionsBox: Identifiable {
    let id = UUID()
    let list: [TerminalSessionInfo]
}

/// 显示一条端到端会话的路径（随路径切换刷新）
struct PeerPathView: View {
    @ObservedObject var peer: PeerConnection
    var light = false
    var body: some View { PathBadge(path: peer.path, light: light) }
}

/// 被控端保留的终端会话：接续或结束。
struct RemoteSessionsSheet: View {
    let target: RemoteTarget
    @State var sessions: [TerminalSessionInfo]
    @EnvironmentObject var app: AppModel
    @Environment(\.dismiss) private var dismiss
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        NavigationStack {
            List {
                if sessions.isEmpty {
                    EmptyState(icon: "rectangle.stack", title: loc("terminal.noRemoteSessions")).listRowBackground(Color.clear)
                }
                ForEach(sessions, id: \.id) { s in
                    HStack {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(s.title.isEmpty ? s.id : s.title).font(.body.weight(.medium))
                            Text((s.attached ? loc("terminal.attachedElsewhere") : loc("terminal.detachedState")) + " · " + Fmt.relative(s.createdAt))
                                .font(.caption).foregroundStyle(p.secondaryText)
                        }
                        Spacer()
                        Button(loc("terminal.attach")) {
                            dismiss()
                            app.openTerminal(target, attach: s)
                        }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.small)
                    }
                    .galaxyRow()
                    .swipeActions {
                        Button(role: .destructive) {
                            Task {
                                do {
                                    if let d = target.deviceId { try await app.account.peer(d).session.killTerminal(sessionId: s.id) }
                                    sessions.removeAll { $0.id == s.id }
                                } catch { app.showError(error) }
                            }
                        } label: { Label(loc("terminal.kill"), systemImage: "xmark.octagon") }
                    }
                }
            }
            .galaxyList()
            .navigationTitle(loc("terminal.remoteSessionsOn", app.targetName(target)))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(loc("common.close")) { dismiss() } }
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        dismiss()
                        app.openTerminal(target, ask: false)
                    } label: { Label(loc("terminal.newSession"), systemImage: "plus") }
                }
                ToolbarItem(placement: .bottomBar) { InfoButton(text: loc("terminal.sessionsInfo")) }
            }
        }
        .presentationDetents([.medium, .large])
    }
}
