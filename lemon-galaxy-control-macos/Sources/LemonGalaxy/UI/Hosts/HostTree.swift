import SwiftUI
import UniformTypeIdentifiers

/// 设备、SSH 主机、RDP 主机与文件夹组成的一棵树（文件夹结构存在端到端加密库里）。
struct HostNode: Identifiable {
    enum Kind { case folder, device, ssh, rdp }
    let id: String            // 文件夹 / SSH / RDP：加密库条目 id；设备："dev:" + 设备 id
    let kind: Kind
    var title: String
    var subtitle: String
    var tags: [String] = []
    var favorite = false
    var folderId: String?
    var sort: Int64 = 0
    var children: [HostNode] = []
    var device: DeviceInfo?
    var color: String?
    var expanded = true

    var deviceId: String? { kind == .device ? String(id.dropFirst(4)) : nil }
    var searchable: String { ([title, subtitle] + tags).joined(separator: " ").lowercased() }
}

extension AppModel {
    func hostNodes() -> [HostNode] {
        var nodes: [HostNode] = []
        var metaByDevice: [String: VaultEntry] = [:]
        for e in vault {
            switch e.data {
            case .folder(let name, let color, let expanded):
                nodes.append(HostNode(id: e.id, kind: .folder, title: name, subtitle: "", folderId: e.folderId, sort: e.sort, color: color, expanded: expanded))
            case .deviceMeta(let did, _, _, _, _):
                metaByDevice[did] = e
            case .sshHost(let label, let host, let port, let user, _, let jump, _, let tags, _, let fav, _):
                var sub = "\(user)@\(host)" + (port == 22 ? "" : ":\(port)")
                if let jump { sub += " · " + L("hosts.via", deviceName(jump)) }
                nodes.append(HostNode(id: e.id, kind: .ssh, title: label, subtitle: sub, tags: tags, favorite: fav, folderId: e.folderId, sort: e.sort))
            case .rdpHost(let label, let host, let port, let user, _, _, _, let tags, _, let fav):
                nodes.append(HostNode(id: e.id, kind: .rdp, title: label, subtitle: "\(user)@\(host)" + (port == 3389 ? "" : ":\(port)"),
                                      tags: tags, favorite: fav, folderId: e.folderId, sort: e.sort))
            default: break
            }
        }
        for d in devices {
            var n = HostNode(id: "dev:" + d.id, kind: .device, title: d.hostname, subtitle: d.osVersion, sort: 100, device: d)
            if let m = metaByDevice[d.id], case .deviceMeta(_, let alias, let tags, _, let fav) = m.data {
                if let alias, !alias.isEmpty { n.title = alias; n.subtitle = d.hostname + " · " + d.osVersion }
                n.tags = tags
                n.favorite = fav
                n.folderId = m.folderId
                n.sort = m.sort
            }
            nodes.append(n)
        }
        return nodes
    }

    /// 组装成树；文件夹在前，其余按排序值、名称
    func hostTree() -> [HostNode] {
        let all = hostNodes()
        let folderIds = Set(all.filter { $0.kind == .folder }.map(\.id))
        func children(of parent: String?) -> [HostNode] {
            all.filter { n in
                let f = n.folderId.flatMap { folderIds.contains($0) ? $0 : nil }
                return f == parent
            }
            .map { n -> HostNode in
                var n = n
                if n.kind == .folder { n.children = children(of: n.id) }
                return n
            }
            .sorted(by: Self.order)
        }
        return children(of: nil)
    }

    nonisolated static func order(_ a: HostNode, _ b: HostNode) -> Bool {
        if (a.kind == .folder) != (b.kind == .folder) { return a.kind == .folder }
        if a.sort != b.sort { return a.sort < b.sort }
        return a.title.localizedStandardCompare(b.title) == .orderedAscending
    }

    func favoriteNodes() -> [HostNode] {
        hostNodes().filter { $0.favorite && $0.kind != .folder }.sorted { $0.title.localizedStandardCompare($1.title) == .orderedAscending }
    }

    func filteredNodes() -> [HostNode] {
        let q = searchText.trimmingCharacters(in: .whitespaces).lowercased()
        return hostNodes().filter { n in
            guard n.kind != .folder else { return false }
            if let t = tagFilter, !n.tags.contains(t) { return false }
            if !q.isEmpty, !n.searchable.contains(q) { return false }
            return true
        }
        .sorted { a, b in
            if a.favorite != b.favorite { return a.favorite }
            return a.title.localizedStandardCompare(b.title) == .orderedAscending
        }
    }

    // MARK: 树操作

    func toggleFolder(_ id: String) {
        guard var e = entry(id), case .folder(let name, let color, let expanded) = e.data else { return }
        e.data = .folder(name: name, color: color, expanded: !expanded)
        Task { _ = await save(e) }
    }

    func isAncestor(_ folder: String, of target: String?) -> Bool {
        var cur = target
        while let c = cur {
            if c == folder { return true }
            cur = entry(c)?.folderId
        }
        return false
    }

    func move(_ itemId: String, to folderId: String?) {
        if itemId == folderId { return }
        if itemId.hasPrefix("dev:") {
            let did = String(itemId.dropFirst(4))
            Task { await updateDeviceMeta(did) { _, _, _, _, folder in folder = folderId } }
            return
        }
        guard var e = entry(itemId) else { return }
        if case .folder = e.data, let folderId, isAncestor(itemId, of: folderId) { return }
        e.folderId = folderId
        e.sort = (vault.filter { $0.folderId == folderId }.map(\.sort).max() ?? 0) + 1
        Task { _ = await save(e) }
    }

    func rename(_ itemId: String, to name: String) {
        let name = name.trimmingCharacters(in: .whitespaces)
        if itemId.hasPrefix("dev:") {
            let did = String(itemId.dropFirst(4))
            Task { await updateDeviceMeta(did) { alias, _, _, _, _ in alias = name.isEmpty ? nil : name } }
            return
        }
        guard !name.isEmpty, var e = entry(itemId) else { return }
        switch e.data {
        case .folder(_, let c, let x): e.data = .folder(name: name, color: c, expanded: x)
        case .sshHost(_, let h, let p, let u, let a, let jd, let jh, let t, let n, let f, let th):
            e.data = .sshHost(label: name, host: h, port: p, username: u, auth: a, jumpDeviceId: jd, jumpHostId: jh, tags: t, note: n, favorite: f, terminalTheme: th)
        case .rdpHost(_, let h, let p, let u, let pw, let d, let j, let t, let n, let f):
            e.data = .rdpHost(label: name, host: h, port: p, username: u, password: pw, domain: d, jumpDeviceId: j, tags: t, note: n, favorite: f)
        default: return
        }
        Task { _ = await save(e) }
    }

    func toggleFavorite(_ itemId: String) {
        if itemId.hasPrefix("dev:") {
            let did = String(itemId.dropFirst(4))
            Task { await updateDeviceMeta(did) { _, _, _, fav, _ in fav.toggle() } }
            return
        }
        guard var e = entry(itemId) else { return }
        switch e.data {
        case .sshHost(let l, let h, let p, let u, let a, let jd, let jh, let t, let n, let f, let th):
            e.data = .sshHost(label: l, host: h, port: p, username: u, auth: a, jumpDeviceId: jd, jumpHostId: jh, tags: t, note: n, favorite: !f, terminalTheme: th)
        case .rdpHost(let l, let h, let p, let u, let pw, let d, let j, let t, let n, let f):
            e.data = .rdpHost(label: l, host: h, port: p, username: u, password: pw, domain: d, jumpDeviceId: j, tags: t, note: n, favorite: !f)
        default: return
        }
        Task { _ = await save(e) }
    }

    func newFolder(in parent: String?) {
        let e = VaultEntry(id: "", folderId: parent, sort: 0, data: .folder(name: L("hosts.newFolderName"), color: nil, expanded: true), updatedAt: 0)
        Task {
            if let saved = await save(e) {
                selection = saved.id
                renamingId = saved.id
            }
        }
    }

    func open(_ n: HostNode) {
        switch n.kind {
        case .device: if let d = n.deviceId { activateDevice(d) }
        case .ssh: if sshSupported { openTerminal(sshHostId: n.id) } else { showToast(L("ssh.unsupported"), error: true) }
        case .rdp: openRdp(n.id)
        case .folder: toggleFolder(n.id)
        }
    }
}

// MARK: - 中间列：设备与主机

struct HostsList: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var rootTargeted = false

    var body: some View {
        VStack(spacing: 0) {
            ColumnHeader(title: loc("nav.hosts")) {
                IconButton(icon: "arrow.clockwise", help: loc("common.refresh")) { Task { try? await app.client.refreshDevices() } }
                Menu {
                    Button(loc("hosts.new.folder")) { app.hostsTab = .hosts; app.newFolder(in: currentFolder) }
                    Button(loc("hosts.new.ssh")) { app.hostsTab = .hosts; app.selectHome(.hosts); app.editing = .newSsh(folder: currentFolder) }
                    Button(loc("hosts.new.rdp")) { app.hostsTab = .hosts; app.selectHome(.hosts); app.editing = .newRdp(folder: currentFolder) }
                    Button(loc("hosts.new.credential")) { app.hostsTab = .credentials; app.selectHome(.hosts); app.editing = .newCredential }
                    Divider()
                    Button(loc("assist.connect.title")) { app.modal = .assistConnect }
                } label: {
                    Image(systemName: "plus").font(.system(size: 13, weight: .medium)).frame(width: 26, height: 24)
                }
                .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
                .help(loc("common.new"))
            }
            Picker("", selection: $app.hostsTab) {
                Text(loc("hosts.tab.hosts")).tag(HostsListTab.hosts)
                Text(loc("hosts.tab.credentials")).tag(HostsListTab.credentials)
                Text(loc("hosts.tab.known")).tag(HostsListTab.knownHosts)
            }
            .pickerStyle(.segmented).labelsHidden()
            .padding(.horizontal, 12).padding(.bottom, 8)
            switch app.hostsTab {
            case .hosts: hostsTab
            case .credentials: CredentialsList()
            case .knownHosts: KnownHostsList()
            }
        }
        .onAppear { app.setLanDiscovery(true) }
        .onDisappear { app.setLanDiscovery(false) }
    }

    private var currentFolder: String? {
        guard let s = app.selection, let e = app.entry(s) else { return nil }
        if case .folder = e.data { return e.id }
        return e.folderId
    }

    private var hostsTab: some View {
        VStack(spacing: 0) {
            SearchField(placeholder: loc("hosts.search"), text: $app.searchText)
                .padding(.horizontal, 12).padding(.bottom, 6)
            let tags = app.allTags
            if !tags.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 5) {
                        ForEach(tags, id: \.self) { t in
                            TagChip(text: t, selected: app.tagFilter == t)
                                .onTapGesture { app.tagFilter = app.tagFilter == t ? nil : t }
                        }
                    }
                    .padding(.horizontal, 12)
                }
                .padding(.bottom, 6)
            }
            Rectangle().fill(p.separator).frame(height: 1)
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 1) {
                    if app.searchText.isEmpty && app.tagFilter == nil {
                        let favs = app.favoriteNodes()
                        if !favs.isEmpty {
                            SectionLabel(text: loc("hosts.favorites")).padding(.horizontal, 14).padding(.top, 10).padding(.bottom, 4)
                            ForEach(favs) { n in HostRow(node: n, depth: 0, rowId: "fav-" + n.id) }
                            SectionLabel(text: loc("hosts.all")).padding(.horizontal, 14).padding(.top, 12).padding(.bottom, 4)
                        }
                        ForEach(app.hostTree()) { n in HostTreeBranch(node: n, depth: 0) }
                    } else {
                        let list = app.filteredNodes()
                        if list.isEmpty {
                            Text(loc("hosts.noMatch")).font(.system(size: 12)).foregroundStyle(p.tertiaryText).padding(16)
                        }
                        ForEach(list) { n in HostRow(node: n, depth: 0, rowId: n.id) }
                    }
                }
                .padding(.vertical, 6)
                .padding(.horizontal, 6)
            }
            .background(rootTargeted ? p.accent.opacity(0.06) : Color.clear)
            .onDrop(of: [UTType.plainText], isTargeted: $rootTargeted) { providers in
                handleDrop(providers, to: nil)
            }
        }
    }

    func handleDrop(_ providers: [NSItemProvider], to folder: String?) -> Bool {
        guard let item = providers.first else { return false }
        _ = item.loadObject(ofClass: NSString.self) { obj, _ in
            guard let id = obj as? String else { return }
            DispatchQueue.main.async { app.move(id, to: folder) }
        }
        return true
    }
}

struct HostTreeBranch: View {
    let node: HostNode
    let depth: Int
    var body: some View {
        HostRow(node: node, depth: depth, rowId: node.id)
        if node.kind == .folder && node.expanded {
            ForEach(node.children) { c in HostTreeBranch(node: c, depth: depth + 1) }
        }
    }
}

struct HostRow: View {
    let node: HostNode
    let depth: Int
    let rowId: String
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var hover = false
    @State private var targeted = false
    @State private var draftName = ""
    @FocusState private var renameFocus: Bool

    var selected: Bool { app.selection == node.id && app.activeTabId == nil }

    var body: some View {
        HStack(spacing: 7) {
            if node.kind == .folder {
                Image(systemName: "chevron.right")
                    .font(.system(size: 9, weight: .bold))
                    .foregroundStyle(p.tertiaryText)
                    .rotationEffect(.degrees(node.expanded ? 90 : 0))
                    .frame(width: 12)
                    .contentShape(Rectangle())
                    .onTapGesture { app.toggleFolder(node.id) }
            } else {
                Spacer().frame(width: 12)
            }
            icon
            if app.renamingId == node.id {
                TextField("", text: $draftName)
                    .textFieldStyle(.plain)
                    .font(.system(size: 12.5))
                    .focused($renameFocus)
                    .onAppear { draftName = node.title; renameFocus = true }
                    .onSubmit { app.rename(node.id, to: draftName); app.renamingId = nil }
                    .onExitCommand { app.renamingId = nil }
            } else {
                VStack(alignment: .leading, spacing: 1) {
                    Text(node.title).font(.system(size: 12.5, weight: node.kind == .folder ? .semibold : .medium))
                        .foregroundStyle(p.text).lineLimit(1)
                    if node.kind != .folder {
                        Text(node.subtitle).font(.system(size: 11)).foregroundStyle(p.tertiaryText).lineLimit(1)
                    }
                }
            }
            Spacer(minLength: 4)
            if node.kind == .folder {
                Text("\(node.children.count)").font(.system(size: 11).monospacedDigit()).foregroundStyle(p.tertiaryText)
            } else {
                if let t = node.tags.first, hover == false { TagChip(text: t) }
                if node.favorite { Image(systemName: "star.fill").font(.system(size: 9.5)).foregroundStyle(p.warning) }
                if node.device?.isSelf == true { TagChip(text: loc("device.thisMac")) }
            }
        }
        .padding(.leading, 6 + CGFloat(depth) * 14).padding(.trailing, 8)
        .frame(height: node.kind == .folder ? 28 : 38)
        .background(RoundedRectangle(cornerRadius: 6).fill(selected ? p.selection : (targeted ? p.accent.opacity(0.12) : (hover ? p.hover : .clear))))
        .contentShape(Rectangle())
        .onHover { hover = $0 }
        .gesture(TapGesture(count: 2).onEnded { app.open(node) })
        .simultaneousGesture(TapGesture().onEnded { select() })
        .onDrag { NSItemProvider(object: node.id as NSString) }
        .modifier(FolderDrop(node: node, targeted: $targeted))
        .contextMenu { HostContextMenu(node: node) }
    }

    private func select() {
        app.selection = node.id
        app.activeTabId = nil
        if node.kind == .folder { app.editing = nil }
    }

    @ViewBuilder
    private var icon: some View {
        switch node.kind {
        case .folder:
            Image(systemName: node.expanded ? "folder" : "folder.fill")
                .font(.system(size: 13))
                .foregroundStyle(node.color.map { Color(hex: $0) } ?? p.secondaryText)
                .frame(width: 22)
        case .device:
            ZStack(alignment: .bottomTrailing) {
                RoundedRectangle(cornerRadius: 6).fill(p.elevated).frame(width: 26, height: 26)
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(p.border, lineWidth: 0.5))
                    .overlay(PlatformGlyph(platform: node.device?.platform ?? .linux, size: 13).foregroundStyle(p.secondaryText))
                if let d = node.device { StatusDot(state: StatusDot.of(d), size: 7).offset(x: 2, y: 2) }
            }
        case .ssh: ProtocolBadge(text: "SSH", color: p.accent)
        case .rdp: ProtocolBadge(text: "RDP", color: p.info)
        }
    }
}

struct ProtocolBadge: View {
    let text: String
    let color: Color
    @Environment(\.palette) var p
    var body: some View {
        RoundedRectangle(cornerRadius: 6).fill(color.opacity(0.13)).frame(width: 26, height: 26)
            .overlay(Text(text).font(.system(size: 7.5, weight: .bold, design: .rounded)).foregroundStyle(color))
    }
}

private struct FolderDrop: ViewModifier {
    let node: HostNode
    @Binding var targeted: Bool
    @EnvironmentObject var app: AppModel
    func body(content: Content) -> some View {
        if node.kind == .folder {
            content.onDrop(of: [UTType.plainText], isTargeted: $targeted) { providers in
                guard let item = providers.first else { return false }
                _ = item.loadObject(ofClass: NSString.self) { obj, _ in
                    guard let id = obj as? String else { return }
                    DispatchQueue.main.async { app.move(id, to: node.id) }
                }
                return true
            }
        } else {
            content
        }
    }
}

struct HostContextMenu: View {
    let node: HostNode
    @EnvironmentObject var app: AppModel
    @Environment(\.loc) var loc

    var body: some View {
        switch node.kind {
        case .device:
            if let d = node.device, !d.isSelf, d.kind != .mobile, let id = node.deviceId {
                if d.platform == .windows || d.platform == .macos {
                    Button(loc("connect.desktop")) { app.openRemoteDesktop(deviceId: id) }
                }
                Button(loc("connect.terminal")) { app.openTerminalAsking(deviceId: id) }
                Button(loc("connect.files")) { app.openSFTP(deviceId: id) }
                Button(loc("connect.ai")) { app.openAI(targets: [AgentTarget(kind: .device, id: id)]) }
                Divider()
            }
            Button(loc("hosts.edit")) { app.selection = node.id; app.editing = .device(node.deviceId ?? "") }
            Button(loc("hosts.rename")) { app.renamingId = node.id }
            Button(node.favorite ? loc("hosts.unfavorite") : loc("hosts.favorite")) { app.toggleFavorite(node.id) }
        case .ssh:
            Button(loc("connect.terminal")) { app.openTerminal(sshHostId: node.id) }.disabled(!app.sshSupported)
            Button(loc("connect.sftp")) { app.openSFTP(sshHostId: node.id) }.disabled(!app.sshSupported)
            Button(loc("connect.ai")) { app.openAI(targets: [AgentTarget(kind: .sshHost, id: node.id)]) }.disabled(!app.sshSupported)
            Divider()
            Button(loc("hosts.edit")) { app.selection = node.id; app.editing = .vault(node.id) }
            Button(loc("hosts.rename")) { app.renamingId = node.id }
            Button(node.favorite ? loc("hosts.unfavorite") : loc("hosts.favorite")) { app.toggleFavorite(node.id) }
            Divider()
            Button(loc("common.delete"), role: .destructive) { askDelete() }
        case .rdp:
            Button(loc("connect.rdp")) { app.openRdp(node.id) }
            Divider()
            Button(loc("hosts.edit")) { app.selection = node.id; app.editing = .vault(node.id) }
            Button(loc("hosts.rename")) { app.renamingId = node.id }
            Button(node.favorite ? loc("hosts.unfavorite") : loc("hosts.favorite")) { app.toggleFavorite(node.id) }
            Divider()
            Button(loc("common.delete"), role: .destructive) { askDelete() }
        case .folder:
            Button(loc("hosts.new.folder")) { app.newFolder(in: node.id) }
            Button(loc("hosts.new.ssh")) { app.editing = .newSsh(folder: node.id) }
            Divider()
            Button(loc("hosts.rename")) { app.renamingId = node.id }
            Button(loc("hosts.edit")) { app.selection = node.id; app.editing = .vault(node.id) }
            Divider()
            Button(loc("common.delete"), role: .destructive) { askDelete() }
        }
        if node.kind != .folder {
            Menu(loc("hosts.moveTo")) {
                Button(loc("hosts.root")) { app.move(node.id, to: nil) }
                ForEach(app.hostNodes().filter { $0.kind == .folder }) { f in
                    Button(f.title) { app.move(node.id, to: f.id) }
                }
            }
        }
    }

    private func askDelete() {
        let msg = node.kind == .folder ? loc("hosts.delete.folderMessage", node.title) : loc("hosts.delete.message", node.title)
        app.confirm(loc("hosts.delete.title"), msg, button: loc("common.delete")) {
            Task { await app.delete(node.id) }
        }
    }
}

// MARK: - 凭据、已知主机

struct CredentialsList: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    var body: some View {
        ScrollView {
            LazyVStack(spacing: 1) {
                ForEach(app.credentials, id: \.id) { e in
                    if case .credential(let label, let user, _, let key, _) = e.data {
                        let selected = app.selection == e.id
                        HStack(spacing: 9) {
                            Image(systemName: key != nil ? "key.horizontal" : "lock")
                                .font(.system(size: 13)).foregroundStyle(p.secondaryText).frame(width: 26, height: 26)
                                .background(RoundedRectangle(cornerRadius: 6).fill(p.elevated))
                            VStack(alignment: .leading, spacing: 1) {
                                Text(label).font(.system(size: 12.5, weight: .medium)).lineLimit(1)
                                Text(user + " · " + (key != nil ? loc("cred.kind.key") : loc("cred.kind.password")))
                                    .font(.system(size: 11)).foregroundStyle(p.tertiaryText)
                            }
                            Spacer()
                        }
                        .padding(.horizontal, 8).frame(height: 40)
                        .background(RoundedRectangle(cornerRadius: 6).fill(selected ? p.selection : .clear))
                        .contentShape(Rectangle())
                        .onTapGesture { app.selection = e.id; app.editing = .vault(e.id); app.activeTabId = nil }
                    }
                }
                if app.credentials.isEmpty {
                    Text(loc("cred.empty")).font(.system(size: 12)).foregroundStyle(p.tertiaryText).padding(16)
                }
            }
            .padding(6)
        }
    }
}

struct KnownHostsList: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 1) {
                ForEach(app.knownHosts, id: \.id) { e in
                    if case .knownHost(let host, let port, let type, let fp) = e.data {
                        VStack(alignment: .leading, spacing: 3) {
                            HStack {
                                Text(port == 22 ? host : "\(host):\(port)").font(.system(size: 12.5, weight: .medium))
                                Spacer()
                                Text(type).font(.system(size: 10.5)).foregroundStyle(p.tertiaryText)
                            }
                            Text(fp).font(.system(size: 10.5, design: .monospaced)).foregroundStyle(p.secondaryText).lineLimit(1).truncationMode(.middle)
                        }
                        .padding(.horizontal, 10).padding(.vertical, 7)
                        .contextMenu {
                            Button(loc("known.copy")) {
                                NSPasteboard.general.clearContents(); NSPasteboard.general.setString(fp, forType: .string)
                            }
                            Button(loc("common.delete"), role: .destructive) {
                                app.confirm(loc("known.delete.title"), loc("known.delete.message", host), button: loc("common.delete")) {
                                    Task { await app.delete(e.id) }
                                }
                            }
                        }
                        Divider().padding(.horizontal, 10)
                    }
                }
            }
            .padding(6)
        }
    }
}
