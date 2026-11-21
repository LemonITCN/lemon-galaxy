import SwiftUI
import UIKit
import SwiftTerm
import GalaxyKit

/// 把会话里常驻的终端视图嵌进 SwiftUI（切换会话时换子视图，缓冲区不丢）。
struct TerminalHost: UIViewRepresentable {
    let session: TerminalSession

    func makeUIView(context: Context) -> UIView {
        let v = UIView()
        v.backgroundColor = session.view.backgroundColor
        return v
    }

    func updateUIView(_ container: UIView, context: Context) {
        if session.view.superview !== container {
            container.subviews.forEach { $0.removeFromSuperview() }
            session.view.frame = container.bounds
            session.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
            container.addSubview(session.view)
        }
        container.backgroundColor = session.view.backgroundColor
    }
}

struct TerminalScreen: View {
    @ObservedObject var session: TerminalSession
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var showSnippets = false
    @State private var line = ""

    var body: some View {
        let theme = TerminalTheme.named(app.settings.terminalTheme)
        VStack(spacing: 0) {
            ZStack {
                TerminalHost(session: session)
                    .padding(.horizontal, 4)
                    .background(Color(uiColor: session.view.backgroundColor ?? theme.uiBackground))
                statusOverlay
            }
            if session.inputBox { inputBox }
        }
        .background(Color(uiColor: session.view.backgroundColor ?? theme.uiBackground).ignoresSafeArea())
        .sheet(isPresented: $showSnippets) {
            SnippetPicker { text, run in
                showSnippets = false
                session.send(text: run ? text + "\r" : text)
            }
            .environmentObject(app)
            .environmentObject(app.account)
        }
        .onAppear {
            session.onShowSnippets = { showSnippets = true }
            if session.status == .connected { _ = session.view.becomeFirstResponder() }
        }
        .onChange(of: session.status) { s in
            if s == .connected { _ = session.view.becomeFirstResponder() }
        }
        .onChange(of: app.settings.terminalTheme) { _ in session.applyTheme() }
    }

    @ViewBuilder private var statusOverlay: some View {
        switch session.status {
        case .connected:
            EmptyView()
        case .connecting:
            VStack(spacing: 12) {
                ProgressView().tint(.white)
                Text(loc("terminal.connecting")).font(.subheadline).foregroundStyle(.white.opacity(0.8))
            }
            .padding(20)
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 14))
        case .detached, .exited, .failed:
            VStack(spacing: 12) {
                Image(systemName: icon).font(.system(size: 28)).foregroundStyle(.white.opacity(0.85))
                Text(message).font(.subheadline).foregroundStyle(.white).multilineTextAlignment(.center)
                HStack(spacing: 10) {
                    Button(loc("terminal.reconnect")) { session.reconnect() }
                        .buttonStyle(.borderedProminent)
                    if !session.isSsh {
                        Button(loc("terminal.newSession")) { session.start(attach: nil) }
                            .buttonStyle(.bordered)
                            .tint(.white)
                    }
                }
            }
            .padding(22)
            .frame(maxWidth: 320)
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 16))
            .environment(\.colorScheme, .dark)
        }
    }

    private var icon: String {
        if case .failed = session.status { return "exclamationmark.triangle" }
        return "bolt.horizontal.circle"
    }

    private var message: String {
        switch session.status {
        case .detached: return loc("terminal.detached")
        case .exited(let c): return c.map { loc("terminal.exitedCode", $0) } ?? loc("terminal.exited")
        case .failed(let m): return m
        default: return ""
        }
    }

    private var inputBox: some View {
        HStack(spacing: 8) {
            TextField(loc("terminal.inputHint"), text: $line)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .font(.system(.body, design: .monospaced))
                .submitLabel(.send)
                .onSubmit(sendLine)
                .padding(.horizontal, 12)
                .padding(.vertical, 9)
                .background(RoundedRectangle(cornerRadius: 10).fill(p.elevated))
            Button(action: sendLine) {
                Image(systemName: "arrow.up.circle.fill").font(.system(size: 30))
            }
            .disabled(!session.isLive)
            .accessibilityLabel(loc("terminal.send"))
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(p.card)
    }

    private func sendLine() {
        guard session.isLive else { return }
        session.send(text: line + "\r")
        line = ""
    }
}

/// 代码片段：点一下直接执行，「插入」只放进命令行不回车
struct SnippetPicker: View {
    let onPick: (String, Bool) -> Void
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.dismiss) private var dismiss
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var query = ""

    var body: some View {
        NavigationStack {
            List {
                let items = account.hosts.snippets.filter { e in
                    guard !query.isEmpty, case .snippet(let label, let cmd, let tags) = e.data else { return true }
                    let q = query.lowercased()
                    return label.lowercased().contains(q) || cmd.lowercased().contains(q) || tags.contains { $0.lowercased().contains(q) }
                }
                if items.isEmpty {
                    EmptyState(icon: "chevron.left.forwardslash.chevron.right", title: loc("snippets.empty"))
                        .listRowBackground(Color.clear)
                }
                ForEach(items, id: \.id) { e in
                    if case .snippet(let label, let cmd, _) = e.data {
                        HStack {
                            VStack(alignment: .leading, spacing: 3) {
                                Text(label).font(.body.weight(.medium)).foregroundStyle(p.text)
                                Text(cmd).font(.caption.monospaced()).foregroundStyle(p.secondaryText).lineLimit(2)
                            }
                            Spacer()
                            Button(loc("snippets.insert")) { onPick(cmd, false) }
                                .buttonStyle(.bordered)
                                .controlSize(.small)
                            Button { onPick(cmd, true) } label: { Image(systemName: "play.fill") }
                                .buttonStyle(.borderedProminent)
                                .controlSize(.small)
                                .accessibilityLabel(loc("snippets.run"))
                        }
                        .galaxyRow()
                    }
                }
            }
            .galaxyList()
            .searchable(text: $query)
            .navigationTitle(loc("snippets.title"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button(loc("common.close")) { dismiss() } } }
        }
        .presentationDetents([.medium, .large])
    }
}
