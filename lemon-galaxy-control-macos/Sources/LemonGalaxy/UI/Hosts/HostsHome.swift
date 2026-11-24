import SwiftUI

enum EditTarget: Equatable {
    case vault(String)
    case newSsh(folder: String?)
    case newRdp(folder: String?)
    case newCredential
    case device(String)
    case newSnippet
}

/// 「设备与主机」首页：左边是概览 / 详情，右边按需滑出编辑面板（Termius 式）。
struct HostsHome: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p

    var body: some View {
        HStack(spacing: 0) {
            Group {
                if let s = app.selection {
                    detail(for: s)
                } else {
                    HostsOverview()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            if let e = app.editing, isHostEditor(e) {
                Rectangle().fill(p.separator).frame(width: 1)
                EditorPanel(target: e, draft: EntryDraft.from(e, app: app))
                    .frame(width: 360)
                    .background(p.list)
                    .id(editorKey(e))
            }
        }
    }

    private func isHostEditor(_ e: EditTarget) -> Bool {
        if case .newSnippet = e { return false }
        return true
    }

    private func editorKey(_ e: EditTarget) -> String {
        switch e {
        case .vault(let id): return "v" + id
        case .newSsh: return "newssh"
        case .newRdp: return "newrdp"
        case .newCredential: return "newcred"
        case .device(let id): return "d" + id
        case .newSnippet: return "newsnip"
        }
    }

    @ViewBuilder
    private func detail(for id: String) -> some View {
        if id.hasPrefix("dev:"), let d = app.device(String(id.dropFirst(4))) {
            DeviceDetail(device: d)
        } else if let e = app.entry(id) {
            switch e.data {
            case .sshHost: SshDetail(entry: e)
            case .rdpHost: RdpDetail(entry: e)
            case .folder: HostsOverview(folderId: e.id)
            case .credential: HostsOverview()
            default: HostsOverview()
            }
        } else {
            HostsOverview()
        }
    }
}

/// 概览：卡片网格（收藏、各文件夹）
struct HostsOverview: View {
    var folderId: String? = nil
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                header
                if let folderId {
                    let nodes = app.hostNodes().filter { $0.folderId == folderId }.sorted(by: AppModel.order)
                    grid(nodes)
                } else {
                    let favs = app.favoriteNodes()
                    if !favs.isEmpty { group(loc("hosts.favorites"), favs) }
                    let devices = app.hostNodes().filter { $0.kind == .device && $0.device?.isSelf == false && $0.device?.kind != .mobile }
                        .sorted { a, b in
                            let ao = a.device?.online ?? false, bo = b.device?.online ?? false
                            if ao != bo { return ao }
                            return a.title.localizedStandardCompare(b.title) == .orderedAscending
                        }
                    group(loc("hosts.devices"), devices)
                    let hosts = app.hostNodes().filter { $0.kind == .ssh || $0.kind == .rdp }.sorted(by: AppModel.order)
                    group(loc("hosts.directHosts"), hosts)
                }
            }
            .padding(24)
        }
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            if let folderId, let e = app.entry(folderId), case .folder(let name, _, _) = e.data {
                Text(name).font(.system(size: 20, weight: .semibold))
            } else {
                Text(loc("nav.hosts")).font(.system(size: 20, weight: .semibold))
                let online = app.devices.filter { $0.online && !$0.isSelf && $0.kind != .mobile }.count
                Text(loc("hosts.onlineCount", online)).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
            }
            Spacer()
            Button { app.editing = .newSsh(folder: folderId) } label: {
                Label(loc("hosts.new.ssh"), systemImage: "plus")
            }
            .buttonStyle(SecondaryButtonStyle())
        }
    }

    private func group(_ title: String, _ nodes: [HostNode]) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionLabel(text: title)
            grid(nodes)
        }
    }

    private func grid(_ nodes: [HostNode]) -> some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 220, maximum: 320), spacing: 12)], spacing: 12) {
            ForEach(nodes) { n in HostCard(node: n) }
        }
    }
}

struct HostCard: View {
    let node: HostNode
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @State private var hover = false

    var body: some View {
        HStack(spacing: 11) {
            Group {
                switch node.kind {
                case .device:
                    RoundedRectangle(cornerRadius: 8).fill(p.accent.opacity(0.12)).frame(width: 36, height: 36)
                        .overlay(PlatformGlyph(platform: node.device?.platform ?? .linux, size: 17).foregroundStyle(p.accent))
                case .ssh:
                    RoundedRectangle(cornerRadius: 8).fill(p.accent.opacity(0.12)).frame(width: 36, height: 36)
                        .overlay(Text("SSH").font(.system(size: 10, weight: .bold, design: .rounded)).foregroundStyle(p.accent))
                case .rdp:
                    RoundedRectangle(cornerRadius: 8).fill(p.info.opacity(0.12)).frame(width: 36, height: 36)
                        .overlay(Text("RDP").font(.system(size: 10, weight: .bold, design: .rounded)).foregroundStyle(p.info))
                case .folder:
                    RoundedRectangle(cornerRadius: 8).fill(p.elevated).frame(width: 36, height: 36)
                        .overlay(Image(systemName: "folder.fill").foregroundStyle(node.color.map { Color(hex: $0) } ?? p.secondaryText))
                }
            }
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(node.title).font(.system(size: 13, weight: .semibold)).lineLimit(1)
                    if let d = node.device { StatusDot(state: StatusDot.of(d), size: 7) }
                }
                Text(node.subtitle).font(.system(size: 11.5)).foregroundStyle(p.secondaryText).lineLimit(1)
            }
            Spacer(minLength: 0)
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 10).fill(hover ? p.elevated : p.card))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(hover ? p.accent.opacity(0.5) : p.border, lineWidth: hover ? 1 : 0.6))
        .contentShape(Rectangle())
        .onHover { hover = $0 }
        .gesture(TapGesture(count: 2).onEnded { app.open(node) })
        .simultaneousGesture(TapGesture().onEnded { app.selection = node.id })
        .contextMenu { HostContextMenu(node: node) }
    }
}

// MARK: - 详情

struct DetailHeader<Icon: View, Actions: View>: View {
    let title: String
    let subtitle: String
    @ViewBuilder var icon: Icon
    @ViewBuilder var actions: Actions
    @Environment(\.palette) var p
    var body: some View {
        HStack(alignment: .center, spacing: 14) {
            icon
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(.system(size: 20, weight: .semibold)).lineLimit(1)
                Text(subtitle).font(.system(size: 12.5)).foregroundStyle(p.secondaryText).lineLimit(1)
            }
            Spacer()
            actions
        }
    }
}

struct ActionTile: View {
    let icon: String
    let title: String
    var enabled = true
    let action: () -> Void
    @Environment(\.palette) var p
    @State private var hover = false
    var body: some View {
        Button(action: action) {
            VStack(spacing: 8) {
                Image(systemName: icon).font(.system(size: 20, weight: .light)).frame(height: 24)
                Text(title).font(.system(size: 12, weight: .medium))
            }
            .foregroundStyle(enabled ? (hover ? p.accent : p.text) : p.tertiaryText)
            .frame(maxWidth: .infinity).frame(height: 76)
            .background(RoundedRectangle(cornerRadius: 10).fill(p.card))
            .overlay(RoundedRectangle(cornerRadius: 10).stroke(hover && enabled ? p.accent.opacity(0.6) : p.border, lineWidth: 0.7))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .onHover { hover = $0 }
    }
}

struct InfoGrid: View {
    let rows: [(String, String)]
    @Environment(\.palette) var p
    var body: some View {
        Card(padding: 0) {
            VStack(spacing: 0) {
                ForEach(Array(rows.enumerated()), id: \.offset) { i, r in
                    HStack(alignment: .firstTextBaseline) {
                        Text(r.0).font(.system(size: 12.5)).foregroundStyle(p.secondaryText).frame(width: 130, alignment: .leading)
                        Text(r.1).font(.system(size: 12.5)).foregroundStyle(p.text).textSelection(.enabled).lineLimit(2)
                        Spacer()
                    }
                    .padding(.horizontal, 14).padding(.vertical, 9)
                    if i < rows.count - 1 { Rectangle().fill(p.separator).frame(height: 1).padding(.leading, 14) }
                }
            }
        }
    }
}

struct DeviceDetail: View {
    let device: DeviceInfo
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        let id = device.id
        let gui = device.platform == .windows || device.platform == .macos || device.platform == .linux
        let connectable = !device.isSelf && device.kind != .mobile && (device.online || device.lanSeen)
        let meta = app.meta(for: id)
        var tags: [String] = []
        var note = ""
        if let meta, case .deviceMeta(_, _, let t, let n, _) = meta.data { tags = t; note = n }
        return ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                DetailHeader(title: app.deviceName(id), subtitle: "\(device.hostname) · \(device.osVersion) · \(device.arch)") {
                    ZStack(alignment: .bottomTrailing) {
                        RoundedRectangle(cornerRadius: 12).fill(p.accent.opacity(0.12)).frame(width: 52, height: 52)
                            .overlay(PlatformGlyph(platform: device.platform, size: 24).foregroundStyle(p.accent))
                        StatusDot(state: StatusDot.of(device), size: 10).offset(x: 3, y: 3)
                    }
                } actions: {
                    Button { app.editing = .device(id) } label: { Label(loc("hosts.edit"), systemImage: "pencil") }
                        .buttonStyle(SecondaryButtonStyle())
                }
                if !tags.isEmpty {
                    HStack(spacing: 5) { ForEach(tags, id: \.self) { TagChip(text: $0) } }
                }
                if app.canWake(id) {
                    // 离线、能远程开机：醒目放在连接按钮上面
                    HStack(spacing: 12) {
                        RoundedRectangle(cornerRadius: 8).fill(p.elevated).frame(width: 34, height: 34)
                            .overlay(Image(systemName: "powersleep").font(.system(size: 15)).foregroundStyle(p.secondaryText))
                        Text(statusText).font(.system(size: 13, weight: .medium))
                        Spacer()
                        WakeButton(deviceId: id)
                    }
                    .padding(12)
                    .background(RoundedRectangle(cornerRadius: 10).fill(p.card))
                    .overlay(RoundedRectangle(cornerRadius: 10).stroke(p.border, lineWidth: 0.6))
                }
                if device.isSelf {
                    Text(loc("device.selfHint")).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                } else if device.kind == .mobile {
                    Text(loc("device.mobileHint")).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                } else {
                    HStack(spacing: 12) {
                        ActionTile(icon: "display", title: loc("connect.desktop"), enabled: connectable && gui) { app.openRemoteDesktop(deviceId: id) }
                        ActionTile(icon: "terminal", title: loc("connect.terminal"), enabled: connectable) { app.openTerminalAsking(deviceId: id) }
                        ActionTile(icon: "folder", title: loc("connect.files"), enabled: connectable) { app.openSFTP(deviceId: id) }
                        ActionTile(icon: "wand.and.stars", title: loc("connect.ai"), enabled: connectable && (app.user?.aiEnabled ?? false)) {
                            app.openAI(targets: [AgentTarget(kind: .device, id: id)])
                        }
                    }
                }
                InfoGrid(rows: [
                    (loc("device.status"), statusText),
                    (loc("device.kind"), loc("device.kind." + kindKey)),
                    (loc("device.os"), device.osVersion),
                    (loc("device.arch"), device.arch),
                    (loc("device.appVersion"), device.appVersion),
                    (loc("device.lastSeen"), Fmt.date(device.lastSeenAt, zh: loc.isChinese)),
                    (loc("device.id"), device.id),
                ])
                if !note.isEmpty {
                    VStack(alignment: .leading, spacing: 6) {
                        SectionLabel(text: loc("hosts.note"))
                        Text(note).font(.system(size: 12.5)).foregroundStyle(p.text)
                    }
                }
                if !device.isSelf {
                    HStack {
                        Spacer()
                        Button(loc("device.remove")) {
                            app.confirm(loc("device.remove.title"), loc("device.remove.message", app.deviceName(id)), button: loc("device.remove")) {
                                Task {
                                    do { try await app.client.removeDevice(deviceId: id); app.selection = nil } catch { app.showError(error) }
                                }
                            }
                        }
                        .buttonStyle(SecondaryButtonStyle(destructive: true))
                    }
                }
            }
            .padding(24)
        }
    }

    private var kindKey: String {
        switch device.kind { case .control: return "control"; case .worker: return "worker"; case .mobile: return "mobile" }
    }

    private var statusText: String {
        if device.online { return device.lanSeen ? loc("device.onlineLan") : loc("device.online") }
        if device.lanSeen { return loc("device.lanOnly") }
        return loc("device.offlineSince", Fmt.relative(device.lastSeenAt, now: Clock.nowMs, loc: loc))
    }
}

struct SshDetail: View {
    let entry: VaultEntry
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        guard case .sshHost(let label, let host, let port, let user, let auth, _, _, let tags, let note, _, let theme) = entry.data else {
            return AnyView(EmptyView())
        }
        return AnyView(ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                DetailHeader(title: label, subtitle: "\(user)@\(host):\(port)") {
                    ProtocolBadge(text: "SSH", color: p.accent).scaleEffect(1.9).frame(width: 52, height: 52)
                } actions: {
                    Button { app.editing = .vault(entry.id) } label: { Label(loc("hosts.edit"), systemImage: "pencil") }
                        .buttonStyle(SecondaryButtonStyle())
                }
                if !tags.isEmpty { HStack(spacing: 5) { ForEach(tags, id: \.self) { TagChip(text: $0) } } }
                HStack(spacing: 12) {
                    ActionTile(icon: "terminal", title: loc("connect.terminal"), enabled: true) { app.openTerminal(sshHostId: entry.id) }
                    ActionTile(icon: "folder", title: loc("connect.sftp"), enabled: true) { app.openSFTP(sshHostId: entry.id) }
                    ActionTile(icon: "wand.and.stars", title: loc("connect.ai"), enabled: app.user?.aiEnabled ?? false) {
                        app.openAI(targets: [AgentTarget(kind: .sshHost, id: entry.id)])
                    }
                }
                InfoGrid(rows: [
                    (loc("ssh.address"), "\(host):\(port)"),
                    (loc("ssh.username"), user),
                    (loc("ssh.auth"), authText(auth)),
                    (loc("ssh.via"), app.sshViaText(entry.id) + (app.sshVia[entry.id] != nil ? " · " + loc("ssh.via.connected") : "")),
                    (loc("ssh.theme"), theme.map { TerminalTheme.named($0).name } ?? loc("ssh.theme.default")),
                ])
                if !note.isEmpty {
                    VStack(alignment: .leading, spacing: 6) {
                        SectionLabel(text: loc("hosts.note"))
                        Text(note).font(.system(size: 12.5))
                    }
                }
            }
            .padding(24)
        })
    }

    private func authText(_ a: SshAuth) -> String {
        switch a {
        case .password: return loc("ssh.auth.password")
        case .privateKey(let c):
            if let e = app.entry(c), case .credential(let l, _, _, _, _) = e.data { return loc("ssh.auth.key") + " · " + l }
            return loc("ssh.auth.key")
        case .agent: return loc("ssh.auth.agent")
        case .interactive: return loc("ssh.auth.interactive")
        }
    }
}

struct RdpDetail: View {
    let entry: VaultEntry
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        guard case .rdpHost(let label, let host, let port, let user, _, let domain, let jump, let tags, let note, _) = entry.data else {
            return AnyView(EmptyView())
        }
        return AnyView(ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                DetailHeader(title: label, subtitle: "\(host):\(port)") {
                    ProtocolBadge(text: "RDP", color: p.info).scaleEffect(1.9).frame(width: 52, height: 52)
                } actions: {
                    Button { app.editing = .vault(entry.id) } label: { Label(loc("hosts.edit"), systemImage: "pencil") }
                        .buttonStyle(SecondaryButtonStyle())
                }
                if !tags.isEmpty { HStack(spacing: 5) { ForEach(tags, id: \.self) { TagChip(text: $0) } } }
                HStack(spacing: 8) {
                    Button { app.openRdp(entry.id) } label: { Label(loc("connect.rdp"), systemImage: "display") }
                        .buttonStyle(PrimaryButtonStyle())
                    InfoButton(text: loc("rdp.info"))
                }
                InfoGrid(rows: [
                    (loc("ssh.address"), "\(host):\(port)"),
                    (loc("ssh.username"), (domain.map { "\($0)\\" } ?? "") + user),
                    (loc("ssh.jump"), jump.map { app.deviceName($0) } ?? loc("ssh.jump.none")),
                ])
                if !note.isEmpty { Text(note).font(.system(size: 12.5)) }
            }
            .padding(24)
        })
    }
}

/// 「远程开机」按钮 + i 说明（需要在对方主板 / 网卡设置里开启网络唤醒）。发送中显示转圈
struct WakeButton: View {
    let deviceId: String
    var compact = false
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        HStack(spacing: 6) {
            InfoButton(text: loc("wake.info"))
            if app.waking.contains(deviceId) {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.mini)
                    Text(loc("wake.sending")).font(.system(size: 12)).foregroundStyle(p.secondaryText)
                }
                .frame(height: 28)
            } else if compact {
                Button { app.wake(deviceId) } label: { Label(loc("wake.action"), systemImage: "power") }
                    .buttonStyle(SecondaryButtonStyle())
            } else {
                Button { app.wake(deviceId) } label: { Label(loc("wake.action"), systemImage: "power") }
                    .buttonStyle(PrimaryButtonStyle())
            }
        }
    }
}
