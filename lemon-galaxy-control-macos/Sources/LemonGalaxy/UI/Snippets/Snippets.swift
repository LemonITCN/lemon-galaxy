import SwiftUI
import AppKit

// MARK: - 片段

struct SnippetsList: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var query = ""

    var body: some View {
        VStack(spacing: 0) {
            ColumnHeader(title: loc("nav.snippets")) {
                IconButton(icon: "plus", help: loc("snippet.new")) { app.selectedSnippet = nil; app.editing = .newSnippet; app.selectHome(.snippets) }
            }
            SearchField(placeholder: loc("snippet.search"), text: $query).padding(.horizontal, 12).padding(.bottom, 8)
            Rectangle().fill(p.separator).frame(height: 1)
            ScrollView {
                LazyVStack(spacing: 1) {
                    ForEach(filtered, id: \.id) { e in
                        if case .snippet(let label, let command, let tags) = e.data {
                            let sel = app.selectedSnippet == e.id && app.activeTabId == nil
                            VStack(alignment: .leading, spacing: 3) {
                                HStack {
                                    Text(label).font(.system(size: 12.5, weight: .medium)).lineLimit(1)
                                    Spacer()
                                    if let t = tags.first { TagChip(text: t) }
                                }
                                Text(command).font(.system(size: 11, design: .monospaced)).foregroundStyle(p.tertiaryText).lineLimit(1)
                            }
                            .padding(.horizontal, 10).padding(.vertical, 7)
                            .background(RoundedRectangle(cornerRadius: 7).fill(sel ? p.selection : .clear))
                            .contentShape(Rectangle())
                            .onTapGesture { app.selectedSnippet = e.id; app.editing = nil; app.activeTabId = nil }
                            .onDrag { NSItemProvider(object: command as NSString) }
                        }
                    }
                }
                .padding(6)
            }
        }
    }

    private var filtered: [VaultEntry] {
        let q = query.lowercased()
        return app.snippets.filter { e in
            guard !q.isEmpty, case .snippet(let l, let c, let t) = e.data else { return true }
            return l.lowercased().contains(q) || c.lowercased().contains(q) || t.contains { $0.lowercased().contains(q) }
        }
    }
}

struct SnippetHome: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        if case .newSnippet? = app.editing {
            EditorPanel(target: .newSnippet, draft: EntryDraft.from(.newSnippet, app: app)).id("new-snippet").frame(maxWidth: 560).frame(maxWidth: .infinity)
        } else if let id = app.selectedSnippet, let e = app.entry(id), case .snippet(let label, let command, let tags) = e.data {
            if case .vault(let eid)? = app.editing, eid == id {
                EditorPanel(target: .vault(id), draft: EntryDraft.from(.vault(id), app: app)).id("edit-" + id).frame(maxWidth: 560).frame(maxWidth: .infinity)
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        HStack {
                            Text(label).font(.system(size: 20, weight: .semibold))
                            Spacer()
                            Button { app.editing = .vault(id) } label: { Label(loc("hosts.edit"), systemImage: "pencil") }.buttonStyle(SecondaryButtonStyle())
                        }
                        if !tags.isEmpty { HStack(spacing: 5) { ForEach(tags, id: \.self) { TagChip(text: $0) } } }
                        Text(command)
                            .font(.system(size: 13, design: .monospaced))
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(14)
                            .background(RoundedRectangle(cornerRadius: 9).fill(p.isDark ? Color.black.opacity(0.3) : Color.black.opacity(0.04)))
                        HStack(spacing: 8) {
                            let terms = app.tabs.compactMap { $0 as? TerminalTabModel }.filter { $0.phase == .running }
                            Menu {
                                if terms.isEmpty { Text(loc("snippet.noTerminal")) }
                                ForEach(terms) { t in Button(t.title) { t.sendText(command + "\r"); app.activeTabId = t.id } }
                            } label: {
                                Label(loc("snippet.sendTo"), systemImage: "terminal")
                            }
                            .fixedSize()
                            Button {
                                NSPasteboard.general.clearContents()
                                NSPasteboard.general.setString(command, forType: .string)
                                app.showToast(loc("common.copied"))
                            } label: { Label(loc("common.copy"), systemImage: "doc.on.doc") }
                            .buttonStyle(SecondaryButtonStyle())
                            InfoButton(text: loc("snippet.info"))
                        }
                    }
                    .padding(24)
                }
            }
        } else {
            EmptyState(icon: "chevron.left.forwardslash.chevron.right", title: loc("snippet.empty"),
                       action: (loc("snippet.new"), { app.editing = .newSnippet }))
        }
    }
}

// MARK: - SFTP 分区

struct SFTPTargetsList: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(spacing: 0) {
            ColumnHeader(title: loc("nav.sftp")) {
                IconButton(icon: "plus.rectangle.on.rectangle", help: loc("sftp.newLocal")) { app.openSFTP() }
            }
            Rectangle().fill(p.separator).frame(height: 1)
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 1) {
                    SectionLabel(text: loc("hosts.devices")).padding(.horizontal, 10).padding(.top, 8).padding(.bottom, 2)
                    ForEach(app.devices.filter { !$0.isSelf && $0.kind != .mobile }, id: \.id) { d in
                        row(icon: AnyView(PlatformGlyph(platform: d.platform, size: 13).foregroundStyle(p.secondaryText)),
                            title: app.deviceName(d.id), subtitle: d.hostname, state: StatusDot.of(d), enabled: d.online || d.lanSeen) {
                            app.openSFTP(deviceId: d.id)
                        }
                    }
                    SectionLabel(text: loc("hosts.sshHosts")).padding(.horizontal, 10).padding(.top, 12).padding(.bottom, 2)
                    ForEach(app.vault.filter { if case .sshHost = $0.data { return true }; return false }, id: \.id) { e in
                        if case .sshHost(let label, let host, _, let user, _, _, _, _, _, _, _) = e.data {
                            row(icon: AnyView(ProtocolBadge(text: "SSH", color: p.accent).scaleEffect(0.8)), title: label, subtitle: "\(user)@\(host)",
                                state: nil, enabled: true) { app.openSFTP(sshHostId: e.id) }
                        }
                    }
                }
                .padding(6)
            }
        }
    }

    private func row(icon: AnyView, title: String, subtitle: String, state: StatusDot.State?, enabled: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 9) {
                icon.frame(width: 22)
                VStack(alignment: .leading, spacing: 1) {
                    Text(title).font(.system(size: 12.5, weight: .medium)).lineLimit(1)
                    Text(subtitle).font(.system(size: 11)).foregroundStyle(p.tertiaryText).lineLimit(1)
                }
                Spacer()
                if let state { StatusDot(state: state, size: 7) }
            }
            .padding(.horizontal, 10).frame(height: 38)
            .contentShape(Rectangle())
            .opacity(enabled ? 1 : 0.5)
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
    }
}

struct SFTPHome: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    var body: some View {
        VStack(spacing: 0) {
            EmptyState(icon: "rectangle.split.2x1", title: loc("sftp.home.title"), action: (loc("sftp.newLocal"), { app.openSFTP() }))
            TransferQueueView(open: .constant(true))
        }
    }
}
