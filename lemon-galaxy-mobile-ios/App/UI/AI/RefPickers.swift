import SwiftUI
import GalaxyKit

/// 「+」→ 设备或主机：同 @ 候选，可多选
struct DeviceRefPicker: View {
    let add: ([ContextRef]) -> Void
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.dismiss) private var dismiss
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var selected: [String: ContextRef] = [:]
    @State private var search = ""

    var body: some View {
        NavigationStack {
            let list = TargetCandidate.all(account: account, query: search)
            List {
                if list.isEmpty {
                    EmptyState(icon: "desktopcomputer", title: loc(search.isEmpty ? "files.targetsEmpty" : "ai.noMatch")).listRowBackground(Color.clear)
                }
                ForEach(list) { c in
                    Button {
                        if selected[c.id] == nil { selected[c.id] = c.ref } else { selected[c.id] = nil }
                    } label: { TargetCandidateRow(c: c, selected: selected[c.id] != nil) }
                        .listRowInsets(EdgeInsets(top: 0, leading: 4, bottom: 0, trailing: 4))
                        .galaxyRow()
                }
            }
            .galaxyList()
            .searchable(text: $search, placement: .navigationBarDrawer(displayMode: .always))
            .navigationTitle(loc("ai.ref.devices"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(loc("common.cancel")) { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button(loc("ai.addCount", selected.count)) {
                        add(Array(selected.values).sorted { $0.label < $1.label })
                        dismiss()
                    }
                    .fontWeight(.semibold)
                    .disabled(selected.isEmpty)
                }
            }
        }
        .presentationDetents([.medium, .large])
    }
}

/// 「+」→ 命令片段 / 端口映射：从加密库条目里选，可多选
struct VaultRefPicker: View {
    enum Kind { case snippets, forwards }
    let kind: Kind
    let add: ([ContextRef]) -> Void
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.dismiss) private var dismiss
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var selected: Set<String> = []

    private struct Row: Identifiable {
        let id: String
        let label: String
        let detail: String
    }

    private var rows: [Row] {
        let h = account.hosts
        switch kind {
        case .snippets:
            return h.snippets.compactMap { e in
                if case .snippet(let label, let command, _) = e.data { return Row(id: e.id, label: label, detail: command) }
                return nil
            }
        case .forwards:
            return h.vault.compactMap { e in
                guard case .portForward(let label, let viaDevice, let viaSsh, let localPort, let remoteHost, let remotePort, _, _) = e.data else { return nil }
                let via = viaDevice.map { h.deviceName($0) } ?? viaSsh.map { h.sshLabel($0) } ?? ""
                let local = localPort == 0 ? "auto" : "\(localPort)"
                return Row(id: e.id, label: label.isEmpty ? "\(remoteHost):\(remotePort)" : label,
                           detail: "\(local) → \(remoteHost):\(remotePort)" + (via.isEmpty ? "" : " · \(via)"))
            }
        }
    }

    var body: some View {
        NavigationStack {
            let list = rows
            List {
                if list.isEmpty {
                    EmptyState(icon: kind == .snippets ? "chevron.left.forwardslash.chevron.right" : "arrow.left.arrow.right",
                               title: loc(kind == .snippets ? "ai.ref.snippetsEmpty" : "ai.ref.forwardsEmpty"))
                        .listRowBackground(Color.clear)
                }
                ForEach(list) { r in
                    Button {
                        if selected.contains(r.id) { selected.remove(r.id) } else { selected.insert(r.id) }
                    } label: {
                        HStack(spacing: 12) {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(r.label).foregroundStyle(p.text).lineLimit(1)
                                Text(r.detail).font(.caption.monospaced()).foregroundStyle(p.secondaryText).lineLimit(1)
                            }
                            Spacer()
                            Image(systemName: selected.contains(r.id) ? "checkmark.circle.fill" : "circle")
                                .font(.title3)
                                .foregroundStyle(selected.contains(r.id) ? p.accent : p.tertiaryText)
                        }
                    }
                    .galaxyRow()
                }
            }
            .galaxyList()
            .navigationTitle(loc(kind == .snippets ? "ai.ref.snippets" : "ai.ref.forwards"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(loc("common.cancel")) { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button(loc("ai.addCount", selected.count)) {
                        add(list.filter { selected.contains($0.id) }.map {
                            kind == .snippets ? ContextRef.snippet($0.id, label: $0.label) : ContextRef.portForward($0.id, label: $0.label)
                        })
                        dismiss()
                    }
                    .fontWeight(.semibold)
                    .disabled(selected.isEmpty)
                }
            }
        }
        .presentationDetents([.medium, .large])
    }
}

/// 「+」→ 文件或文件夹：先选设备或主机，再浏览目录（复用文件页的远端浏览：设备走端到端会话，SSH 主机走 SFTP）
struct FileRefPicker: View {
    let add: (ContextRef) -> Void
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.dismiss) private var dismiss
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var path: [RemoteTarget] = []

    var body: some View {
        NavigationStack(path: $path) {
            let devices = account.devices.filter { $0.kind != .mobile && !$0.isSelf }
                .sorted { ($0.online || $0.lanSeen ? 0 : 1, account.deviceName($0.id).lowercased()) < ($1.online || $1.lanSeen ? 0 : 1, account.deviceName($1.id).lowercased()) }
            let ssh = account.hosts.sshHosts
            List {
                if devices.isEmpty && ssh.isEmpty {
                    EmptyState(icon: "folder", title: loc("files.targetsEmpty")).listRowBackground(Color.clear)
                }
                if !devices.isEmpty {
                    Section(loc("files.devices")) {
                        ForEach(devices, id: \.id) { d in
                            let on = d.online || d.lanSeen
                            Button { path = [.device(d.id)] } label: {
                                targetRow(symbol: Icons.platform(d.platform), title: account.deviceName(d.id),
                                          subtitle: on ? d.osVersion : loc("status.offline"), enabled: on)
                            }
                            .disabled(!on)
                        }
                    }
                    .galaxyRow()
                }
                if !ssh.isEmpty {
                    Section(loc("ai.sshHosts")) {
                        ForEach(ssh, id: \.id) { e in
                            Button { path = [.ssh(e.id)] } label: {
                                targetRow(symbol: "terminal", title: account.hosts.sshLabel(e.id), subtitle: HostItem.ssh(e).subtitle, enabled: true)
                            }
                        }
                    }
                    .galaxyRow()
                }
            }
            .galaxyList()
            .navigationTitle(loc("ai.ref.files"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button(loc("common.cancel")) { dismiss() } } }
            .navigationDestination(for: RemoteTarget.self) { t in
                RefFileBrowser(model: FileBrowserModel(app: app, target: t)) { r in
                    add(r)
                    dismiss()
                }
            }
        }
        .presentationDetents([.large])
    }

    private func targetRow(symbol: String, title: String, subtitle: String, enabled: Bool) -> some View {
        HStack(spacing: 12) {
            PlatformBadge(symbol: symbol, tint: enabled ? p.accent : p.offline, size: 30)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).foregroundStyle(p.text)
                Text(subtitle).font(.caption).foregroundStyle(p.secondaryText).lineLimit(1)
            }
            Spacer()
            Image(systemName: "chevron.right").font(.caption.weight(.semibold)).foregroundStyle(p.tertiaryText)
        }
    }
}

private struct RefFileBrowser: View {
    @StateObject var model: FileBrowserModel
    let pick: (ContextRef) -> Void
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var settings: AppSettings
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var selectedFile: FileEntry?

    init(model: FileBrowserModel, pick: @escaping (ContextRef) -> Void) {
        _model = StateObject(wrappedValue: model)
        self.pick = pick
    }

    private var targetId: String { model.target.deviceId ?? model.target.sshHostId ?? "" }

    var body: some View {
        List {
            if let e = model.error {
                VStack(spacing: 10) {
                    Label(e, systemImage: "exclamationmark.triangle").foregroundStyle(p.danger).font(.callout)
                    Button(loc("common.retry")) { Task { await model.refresh() } }.buttonStyle(.bordered)
                }
                .frame(maxWidth: .infinity)
                .listRowBackground(Color.clear)
            }
            if model.canGoUp {
                Button { selectedFile = nil; Task { await model.goUp() } } label: {
                    Label("..", systemImage: "arrow.turn.left.up").foregroundStyle(p.secondaryText)
                }
                .galaxyRow()
            }
            ForEach(model.sorted(settings), id: \.path) { e in
                Button {
                    if e.kind == .dir {
                        selectedFile = nil
                        Task { await model.load(e.path) }
                    } else {
                        selectedFile = selectedFile?.path == e.path ? nil : e
                    }
                } label: {
                    HStack {
                        FileRow(entry: e)
                        if selectedFile?.path == e.path {
                            Image(systemName: "checkmark.circle.fill").foregroundStyle(p.accent)
                        }
                    }
                }
                .galaxyRow()
            }
        }
        .galaxyList()
        .overlay { if model.loading && model.entries.isEmpty { ProgressView() } }
        .navigationTitle(model.path.isEmpty ? model.name : RemotePath.name(model.path).isEmpty ? model.path : RemotePath.name(model.path))
        .navigationBarTitleDisplayMode(.inline)
        .safeAreaInset(edge: .bottom) {
            HStack(spacing: 10) {
                Button {
                    pick(.folder(target: targetId, path: model.path))
                } label: { Text(loc("ai.pickFolder")).frame(maxWidth: .infinity) }
                    .buttonStyle(.bordered)
                    .disabled(model.path.isEmpty)
                Button {
                    if let f = selectedFile { pick(.file(target: targetId, path: f.path)) }
                } label: { Text(loc("ai.pickFile")).frame(maxWidth: .infinity) }
                    .buttonStyle(.borderedProminent)
                    .disabled(selectedFile == nil)
            }
            .controlSize(.large)
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            .background(.bar)
        }
        .task { await model.start() }
        .onDisappear { app.releaseIfUnused(model.target) }
    }
}
