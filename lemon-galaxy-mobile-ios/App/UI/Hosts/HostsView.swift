import SwiftUI
import GalaxyKit

enum HostsPage: String, CaseIterable, Identifiable {
    case hosts, credentials, snippets, knownHosts
    var id: String { rawValue }
}

/// 加密库编辑器
enum EditorTarget: Identifiable {
    case folder(VaultEntry?, parent: String?)
    case ssh(VaultEntry?, folder: String?)
    case rdp(VaultEntry?, folder: String?)
    case credential(VaultEntry?)
    case snippet(VaultEntry?)
    case device(String)
    case moveItem(HostItem)
    case moveFolder(VaultEntry)

    var id: String {
        switch self {
        case .folder(let e, let p): return "folder-\(e?.id ?? "new")-\(p ?? "")"
        case .ssh(let e, _): return "ssh-\(e?.id ?? "new")"
        case .rdp(let e, _): return "rdp-\(e?.id ?? "new")"
        case .credential(let e): return "cred-\(e?.id ?? "new")"
        case .snippet(let e): return "snip-\(e?.id ?? "new")"
        case .device(let d): return "device-" + d
        case .moveItem(let i): return "move-" + i.key
        case .moveFolder(let f): return "movef-" + f.id
        }
    }
}

struct HostsView: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var page: HostsPage = .hosts
    @State private var query = ""
    @State private var favoritesOnly = false
    @State private var tag: String?
    @State private var editor: EditorTarget?
    @State private var chooser: HostItem?
    @State private var confirm: ConfirmAction?

    struct ConfirmAction: Identifiable {
        let id = UUID()
        let title: String
        let message: String
        let button: String
        let action: () -> Void
    }

    var body: some View {
        List {
            Section {
                Picker("", selection: $page) {
                    ForEach(HostsPage.allCases) { Text(loc("hosts.page." + $0.rawValue)).tag($0) }
                }
                .pickerStyle(.segmented)
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets(top: 4, leading: 0, bottom: 4, trailing: 0))
            }
            switch page {
            case .hosts: hostsSection
            case .credentials: CredentialsSection(query: query, editor: $editor, confirm: $confirm)
            case .snippets: SnippetsSection(query: query, editor: $editor, confirm: $confirm)
            case .knownHosts: KnownHostsSection(query: query, confirm: $confirm)
            }
        }
        .listStyle(.insetGrouped)
        .galaxyList()
        .searchable(text: $query, prompt: loc("hosts.search"))
        .refreshable { do { try await account.refreshDevices() } catch { app.showError(error) } }
        .navigationTitle(loc("nav.hosts"))
        .toolbar {
            ToolbarItem(placement: .navigationBarLeading) { connectionState }
            ToolbarItemGroup(placement: .primaryAction) {
                if page == .hosts { filterMenu }
                addMenu
            }
        }
        .onAppear { account.setLanDiscovery(true) }
        .onDisappear { account.setLanDiscovery(false) }
        .sheet(item: $editor) { target in
            EditorSheet(target: target)
                .environmentObject(app)
                .environmentObject(account)
        }
        .confirmationDialog(chooser?.title ?? "", isPresented: Binding(get: { chooser != nil }, set: { if !$0 { chooser = nil } }), titleVisibility: .visible, presenting: chooser) { item in
            ConnectButtons(item: item)
        } message: { item in
            if Self.isOffline(item) { Text(loc("device.offline")) }
        }
        .alert(confirm?.title ?? "", isPresented: Binding(get: { confirm != nil }, set: { if !$0 { confirm = nil } }), presenting: confirm) { c in
            Button(c.button, role: .destructive) { c.action() }
            Button(loc("common.cancel"), role: .cancel) {}
        } message: { c in Text(c.message) }
    }

    private var connectionState: some View {
        HStack(spacing: 6) {
            switch account.clientState {
            case .online: StatusDot(color: p.success, size: 7)
            case .connecting: ProgressView().controlSize(.mini)
            case .offline: StatusDot(color: p.warning, size: 7)
            case .loggedOut: StatusDot(color: p.offline, size: 7)
            }
            if account.clientState != .online {
                Text(loc("state." + stateKey)).font(.caption).foregroundStyle(p.secondaryText)
            }
        }
    }

    private var stateKey: String {
        switch account.clientState {
        case .online: return "online"
        case .connecting: return "connecting"
        case .offline: return "offline"
        case .loggedOut: return "loggedOut"
        }
    }

    private var filterMenu: some View {
        Menu {
            Picker(loc("hosts.filter"), selection: $favoritesOnly) {
                Text(loc("hosts.filter.all")).tag(false)
                Label(loc("hosts.filter.favorites"), systemImage: "star").tag(true)
            }
            let tags = HostTree.allTags(account.hosts)
            if !tags.isEmpty {
                Section(loc("hosts.tags")) {
                    Button { tag = nil } label: { Label(loc("hosts.filter.allTags"), systemImage: tag == nil ? "checkmark" : "tag") }
                    ForEach(tags, id: \.self) { t in
                        Button { tag = t } label: { Label(t, systemImage: tag == t ? "checkmark" : "tag") }
                    }
                }
            }
        } label: {
            Image(systemName: favoritesOnly || tag != nil ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
        }
        .accessibilityLabel(loc("hosts.filter"))
    }

    private var addMenu: some View {
        Menu {
            Button { editor = .ssh(nil, folder: nil) } label: { Label(loc("add.ssh"), systemImage: "terminal") }
            Button { editor = .rdp(nil, folder: nil) } label: { Label(loc("add.rdp"), systemImage: "display") }
            Button { editor = .folder(nil, parent: nil) } label: { Label(loc("add.folder"), systemImage: "folder.badge.plus") }
            Divider()
            Button { editor = .credential(nil) } label: { Label(loc("add.credential"), systemImage: "key") }
            Button { editor = .snippet(nil) } label: { Label(loc("add.snippet"), systemImage: "chevron.left.forwardslash.chevron.right") }
            Divider()
            Button { app.assistShown = true } label: { Label(loc("add.assist"), systemImage: "person.badge.key") }
        } label: { Image(systemName: "plus") }
        .accessibilityLabel(loc("hosts.add"))
    }

    // MARK: 主机树

    @ViewBuilder private var hostsSection: some View {
        let rows = HostTree.rows(account.hosts, query: query, tag: tag, favoritesOnly: favoritesOnly)
        if !account.rdpForwards.isEmpty {
            Section {
                ForEach(account.rdpForwards) { f in
                    RdpForwardRow(forward: f).galaxyRow()
                }
            } header: {
                Text(loc("rdp.forwards"))
            } footer: {
                HStack { Spacer(); InfoButton(text: loc("rdp.suspendInfo"), size: 13) }
            }
        }
        if let tag {
            Section {
                HStack {
                    TagChip(text: tag, selected: true)
                    Spacer()
                    Button(loc("common.clear")) { self.tag = nil }.font(.caption)
                }
                .listRowBackground(Color.clear)
            }
        }
        if rows.isEmpty {
            EmptyState(icon: query.isEmpty ? "server.rack" : "magnifyingglass",
                       title: loc(query.isEmpty && !favoritesOnly && tag == nil ? "hosts.empty" : "search.empty"))
                .listRowBackground(Color.clear)
        } else {
            Section {
                ForEach(rows) { row in
                    if let f = row.folder {
                        FolderRow(row: row)
                            .contentShape(Rectangle())
                            .onTapGesture { withAnimation(.easeInOut(duration: 0.2)) { account.setFolderExpanded(f, !row.expanded) } }
                            .contextMenu { folderMenu(f, name: row.folderName) }
                            .galaxyRow()
                    } else if let item = row.item {
                        HostRow(item: item, depth: row.depth, path: row.path)
                            .contentShape(Rectangle())
                            .onTapGesture { activate(item) }
                            .contextMenu { itemMenu(item) }
                            .swipeActions(edge: .leading) {
                                Button { Task { do { try await account.toggleFavorite(item) } catch { app.showError(error) } } } label: {
                                    Label(loc(item.favorite ? "action.unfavorite" : "action.favorite"), systemImage: item.favorite ? "star.slash" : "star")
                                }
                                .tint(p.warning)
                            }
                            .swipeActions(edge: .trailing) {
                                Button { edit(item) } label: { Label(loc("common.edit"), systemImage: "pencil") }.tint(p.accent)
                            }
                            .galaxyRow()
                    }
                }
            } footer: {
                HStack(spacing: 4) {
                    Spacer()
                    InfoButton(text: loc("status.info"), size: 13)
                }
            }
        }
    }

    private func activate(_ item: HostItem) {
        switch item {
        case .device(let d, _):
            // 离线设备：能远程开机时弹出「远程开机」，否则只提示离线
            guard d.online || d.lanSeen || account.canWake(d.id) else {
                app.showToast(loc("device.offline"), error: true)
                return
            }
            chooser = item
        case .ssh(let e):
            app.openTerminal(.ssh(e.id))
        case .rdp(let e):
            app.openRdp(e.id)
        }
    }

    static func isOffline(_ item: HostItem) -> Bool {
        if case .device(let d, _) = item { return !(d.online || d.lanSeen) }
        return false
    }

    private func edit(_ item: HostItem) {
        switch item {
        case .device(let d, _): editor = .device(d.id)
        case .ssh(let e): editor = .ssh(e, folder: e.folderId)
        case .rdp(let e): editor = .rdp(e, folder: e.folderId)
        }
    }

    @ViewBuilder private func itemMenu(_ item: HostItem) -> some View {
        ConnectButtons(item: item)
        Divider()
        Button { edit(item) } label: { Label(loc("common.edit"), systemImage: "pencil") }
        Button { editor = .moveItem(item) } label: { Label(loc("action.move"), systemImage: "folder") }
        Button { Task { do { try await account.toggleFavorite(item) } catch { app.showError(error) } } } label: {
            Label(loc(item.favorite ? "action.unfavorite" : "action.favorite"), systemImage: item.favorite ? "star.slash" : "star")
        }
        if case .device(let d, _) = item {
            Button(role: .destructive) {
                confirm = ConfirmAction(title: loc("action.removeDevice"), message: loc("device.removeMessage", item.title), button: loc("common.remove")) {
                    Task { do { try await account.removeDevice(d.id) } catch { app.showError(error) } }
                }
            } label: { Label(loc("action.removeDevice"), systemImage: "trash") }
        } else if let e = item.vaultEntry {
            Button(role: .destructive) {
                confirm = ConfirmAction(title: loc("common.delete"), message: loc("vault.deleteMessage", item.title), button: loc("common.delete")) {
                    Task { do { try await account.delete(e.id) } catch { app.showError(error) } }
                }
            } label: { Label(loc("common.delete"), systemImage: "trash") }
        }
    }

    @ViewBuilder private func folderMenu(_ f: VaultEntry, name: String) -> some View {
        Button { editor = .folder(f, parent: f.folderId) } label: { Label(loc("folder.rename"), systemImage: "pencil") }
        Button { editor = .folder(nil, parent: f.id) } label: { Label(loc("folder.newSub"), systemImage: "folder.badge.plus") }
        Button { editor = .ssh(nil, folder: f.id) } label: { Label(loc("add.ssh"), systemImage: "terminal") }
        Button { editor = .moveFolder(f) } label: { Label(loc("action.move"), systemImage: "folder") }
        Button(role: .destructive) {
            confirm = ConfirmAction(title: loc("folder.delete"), message: loc("folder.deleteMessage", name), button: loc("common.delete")) {
                Task { do { try await account.delete(f.id) } catch { app.showError(error) } }
            }
        } label: { Label(loc("folder.delete"), systemImage: "trash") }
    }
}

/// 连接方式（点设备弹出、长按菜单里也有）
struct ConnectButtons: View {
    let item: HostItem
    @EnvironmentObject var app: AppModel
    @Environment(\.loc) private var loc

    var body: some View {
        switch item {
        case .device(let d, _):
            if d.online || d.lanSeen {
                deviceButtons(d)
            } else if app.account.canWake(d.id) {
                // 离线设备只能远程开机（连接类操作连不上）
                Button { Task { await app.wake(d.id) } } label: { Label(loc("wake.action"), systemImage: "power") }
                    .disabled(app.waking.contains(d.id))
            }
        case .ssh(let e):
            Button { app.openTerminal(.ssh(e.id)) } label: { Label(loc("action.terminal"), systemImage: "terminal") }
            Button { app.openFiles(.ssh(e.id)) } label: { Label(loc("action.sftp"), systemImage: "folder") }
            Button { app.openAI([AgentTarget(kind: .sshHost, id: e.id)]) } label: { Label(loc("action.ai"), systemImage: "sparkles") }
        case .rdp(let e):
            Button { app.openRdp(e.id) } label: { Label(loc("rdp.connect"), systemImage: "display") }
        }
    }

    @ViewBuilder private func deviceButtons(_ d: DeviceInfo) -> some View {
        if d.platform == .windows || d.platform == .macos || d.platform == .linux {
            Button { app.openRemoteDesktop(d.id) } label: { Label(loc("action.remoteDesktop"), systemImage: "display") }
        }
        Button { app.openTerminal(.device(d.id)) } label: { Label(loc("action.terminal"), systemImage: "terminal") }
        Button { app.openFiles(.device(d.id)) } label: { Label(loc("action.files"), systemImage: "folder") }
        Button { app.openAI([AgentTarget(kind: .device, id: d.id)]) } label: { Label(loc("action.ai"), systemImage: "sparkles") }
    }
}

struct FolderRow: View {
    let row: TreeRow
    @Environment(\.palette) private var p
    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "chevron.right")
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(p.tertiaryText)
                .rotationEffect(.degrees(row.expanded ? 90 : 0))
                .frame(width: 12)
            Image(systemName: row.expanded ? "folder.fill" : "folder")
                .foregroundStyle(row.folderColor.map { Color(hex: $0) } ?? p.accent)
            Text(row.folderName).font(.body.weight(.semibold)).foregroundStyle(p.text)
            Spacer()
            Text("\(row.count)").font(.footnote.monospacedDigit()).foregroundStyle(p.tertiaryText)
        }
        .padding(.leading, CGFloat(row.depth) * 18)
        .padding(.vertical, 2)
    }
}

struct HostRow: View {
    let item: HostItem
    var depth = 0
    var path: String?
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        HStack(spacing: 12) {
            PlatformBadge(symbol: symbol, tint: tint)
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(item.title).font(.body.weight(.medium)).foregroundStyle(p.text).lineLimit(1)
                    if item.favorite { Image(systemName: "star.fill").font(.caption2).foregroundStyle(p.warning) }
                    if case .device(let d, _) = item { upgradeChip(d) }
                }
                HStack(spacing: 6) {
                    if case .device = item { StatusDot(color: statusColor, size: 6) }
                    Text(subtitle).font(.caption).foregroundStyle(p.secondaryText).lineLimit(1)
                }
                if let path {
                    Label(path, systemImage: "folder").font(.caption2).foregroundStyle(p.tertiaryText).lineLimit(1)
                }
            }
            Spacer(minLength: 4)
            if !item.tags.isEmpty {
                Text(item.tags.prefix(2).joined(separator: " · ")).font(.caption2).foregroundStyle(p.tertiaryText).lineLimit(1)
            }
        }
        .padding(.leading, CGFloat(depth) * 18)
        .padding(.vertical, 2)
        .opacity(dimmed ? 0.55 : 1)
    }

    private var dimmed: Bool {
        if case .device(let d, _) = item { return !(d.online || d.lanSeen) }
        return false
    }

    /// 有新版本时「可升级」，远程升级进行中「升级中」（详情里点「升级被控端」）
    @ViewBuilder private func upgradeChip(_ d: DeviceInfo) -> some View {
        let busy = account.upgrades[d.id]?.inProgress == true
        if busy || account.hasUpgrade(d) {
            Text(loc(busy ? "upgrade.badge.busy" : "upgrade.badge"))
                .font(.caption2.weight(.medium))
                .foregroundStyle(p.accent)
                .padding(.horizontal, 6)
                .padding(.vertical, 1.5)
                .background(Capsule().fill(p.accent.opacity(0.12)))
                .fixedSize()
        }
    }

    private var symbol: String {
        switch item {
        case .device(let d, _): return Icons.platform(d.platform)
        case .ssh: return "terminal"
        case .rdp: return "display"
        }
    }

    private var tint: Color {
        switch item {
        case .device(let d, _): return d.online || d.lanSeen ? p.accent : p.offline
        case .ssh: return p.success
        case .rdp: return p.info
        }
    }

    private var statusColor: Color {
        guard case .device(let d, _) = item else { return p.offline }
        if d.online { return p.success }
        if d.lanSeen { return p.lan }
        return p.offline
    }

    private var subtitle: String {
        switch item {
        case .device(let d, _):
            let state = d.online ? loc("status.online") : (d.lanSeen ? loc("status.lan") : loc("status.lastSeen", Fmt.relative(d.lastSeenAt)))
            return state + " · " + d.osVersion
        default:
            return item.subtitle
        }
    }
}
