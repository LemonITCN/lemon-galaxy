import SwiftUI
import AppKit
import UniformTypeIdentifiers

/// 编辑面板的草稿：所有条目类型共用一份字段，保存时按类型组装 VaultData。
@MainActor
final class EntryDraft: ObservableObject {
    enum Kind { case ssh, rdp, credential, folder, device, snippet }
    let kind: Kind
    var id = ""
    var deviceId = ""
    @Published var folderId: String?
    @Published var label = ""
    @Published var host = ""
    @Published var port = ""
    @Published var username = ""
    @Published var authKind = "password"      // password | key | agent | interactive
    @Published var password = ""
    @Published var credentialId = ""
    @Published var jumpDeviceId = ""
    @Published var jumpHostId = ""
    @Published var domain = ""
    @Published var tags = ""
    @Published var note = ""
    @Published var favorite = false
    @Published var terminalTheme = ""
    @Published var privateKey = ""
    @Published var passphrase = ""
    @Published var color = ""
    @Published var expanded = true
    @Published var command = ""
    var sort: Int64 = 0

    init(kind: Kind) { self.kind = kind }

    static func from(_ target: EditTarget, app: AppModel) -> EntryDraft {
        switch target {
        case .newSsh(let folder):
            let d = EntryDraft(kind: .ssh); d.folderId = folder; d.port = "22"; return d
        case .newRdp(let folder):
            let d = EntryDraft(kind: .rdp); d.folderId = folder; d.port = "3389"; return d
        case .newCredential:
            return EntryDraft(kind: .credential)
        case .newSnippet:
            return EntryDraft(kind: .snippet)
        case .device(let did):
            let d = EntryDraft(kind: .device)
            d.deviceId = did
            if let m = app.meta(for: did), case .deviceMeta(_, let alias, let tags, let note, let fav) = m.data {
                d.id = m.id; d.folderId = m.folderId; d.sort = m.sort
                d.label = alias ?? ""; d.tags = tags.joined(separator: ", "); d.note = note; d.favorite = fav
            }
            return d
        case .vault(let id):
            guard let e = app.entry(id) else { return EntryDraft(kind: .ssh) }
            var d: EntryDraft
            switch e.data {
            case .sshHost(let label, let host, let port, let user, let auth, let jd, let jh, let tags, let note, let fav, let theme):
                d = EntryDraft(kind: .ssh)
                d.label = label; d.host = host; d.port = "\(port)"; d.username = user
                switch auth {
                case .password(let pw): d.authKind = "password"; d.password = pw ?? ""
                case .privateKey(let c): d.authKind = "key"; d.credentialId = c
                case .agent: d.authKind = "agent"
                case .interactive: d.authKind = "interactive"
                }
                d.jumpDeviceId = jd ?? ""; d.jumpHostId = jh ?? ""; d.tags = tags.joined(separator: ", "); d.note = note
                d.favorite = fav; d.terminalTheme = theme ?? ""
            case .rdpHost(let label, let host, let port, let user, let pw, let domain, let jump, let tags, let note, let fav):
                d = EntryDraft(kind: .rdp)
                d.label = label; d.host = host; d.port = "\(port)"; d.username = user; d.password = pw ?? ""
                d.domain = domain ?? ""; d.jumpDeviceId = jump ?? ""; d.tags = tags.joined(separator: ", "); d.note = note; d.favorite = fav
            case .credential(let label, let user, let pw, let key, let pass):
                d = EntryDraft(kind: .credential)
                d.label = label; d.username = user; d.password = pw ?? ""; d.privateKey = key ?? ""; d.passphrase = pass ?? ""
            case .folder(let name, let color, let expanded):
                d = EntryDraft(kind: .folder)
                d.label = name; d.color = color ?? ""; d.expanded = expanded
            case .snippet(let label, let command, let tags):
                d = EntryDraft(kind: .snippet)
                d.label = label; d.command = command; d.tags = tags.joined(separator: ", ")
            default:
                d = EntryDraft(kind: .ssh)
            }
            d.id = e.id; d.folderId = e.folderId; d.sort = e.sort
            return d
        }
    }

    var tagList: [String] {
        tags.split(whereSeparator: { $0 == "," || $0 == "，" || $0 == " " }).map { String($0).trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
    }

    var portValue: UInt16? { UInt16(port) }

    var isValid: Bool {
        switch kind {
        case .ssh, .rdp: return !label.isEmpty && !host.isEmpty && portValue != nil && portValue != 0 && !username.isEmpty && (kind == .rdp || authKind != "key" || !credentialId.isEmpty)
        case .credential: return !label.isEmpty
        case .folder: return !label.isEmpty
        case .device: return true
        case .snippet: return !label.isEmpty && !command.isEmpty
        }
    }

    func build() -> VaultEntry? {
        let opt: (String) -> String? = { $0.isEmpty ? nil : $0 }
        let data: VaultData
        switch kind {
        case .ssh:
            let auth: SshAuth
            switch authKind {
            case "key": auth = .privateKey(credentialId: credentialId)
            case "agent": auth = .agent
            case "interactive": auth = .interactive
            default: auth = .password(password: opt(password))
            }
            data = .sshHost(label: label, host: host, port: portValue ?? 22, username: username, auth: auth, jumpDeviceId: opt(jumpDeviceId),
                            jumpHostId: opt(jumpHostId), tags: tagList, note: note, favorite: favorite, terminalTheme: opt(terminalTheme))
        case .rdp:
            data = .rdpHost(label: label, host: host, port: portValue ?? 3389, username: username, password: opt(password), domain: opt(domain),
                            jumpDeviceId: opt(jumpDeviceId), tags: tagList, note: note, favorite: favorite)
        case .credential:
            data = .credential(label: label, username: username, password: opt(password), privateKey: opt(privateKey), passphrase: opt(passphrase))
        case .folder:
            data = .folder(name: label, color: opt(color), expanded: expanded)
        case .device:
            data = .deviceMeta(deviceId: deviceId, alias: opt(label), tags: tagList, note: note, favorite: favorite)
        case .snippet:
            data = .snippet(label: label, command: command, tags: tagList)
        }
        return VaultEntry(id: id, folderId: folderId, sort: sort, data: data, updatedAt: 0)
    }
}

struct EditorPanel: View {
    let target: EditTarget
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @StateObject private var draft: EntryDraft
    @State private var saving = false

    /// draft 由父视图按 target 生成；父视图用 .id(target) 保证换目标时重建
    init(target: EditTarget, draft: @autoclosure @escaping () -> EntryDraft) {
        self.target = target
        _draft = StateObject(wrappedValue: draft())
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text(title).font(.system(size: 14, weight: .semibold))
                Spacer()
                IconButton(icon: "xmark", help: loc("common.close"), size: 11) { app.editing = nil }
            }
            .padding(.horizontal, 16).frame(height: 44)
            Rectangle().fill(p.separator).frame(height: 1)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    switch draft.kind {
                    case .ssh: sshFields
                    case .rdp: rdpFields
                    case .credential: credentialFields
                    case .folder: folderFields
                    case .device: deviceFields
                    case .snippet: snippetFields
                    }
                }
                .padding(16)
            }
            Rectangle().fill(p.separator).frame(height: 1)
            HStack(spacing: 8) {
                if !draft.id.isEmpty && draft.kind != .device {
                    Button(loc("common.delete")) {
                        let id = draft.id
                        app.confirm(loc("hosts.delete.title"), loc("hosts.delete.message", draft.label), button: loc("common.delete")) {
                            Task { await app.delete(id); app.editing = nil }
                        }
                    }
                    .buttonStyle(SecondaryButtonStyle(destructive: true))
                }
                Spacer()
                Button(loc("common.cancel")) { app.editing = nil }.buttonStyle(SecondaryButtonStyle())
                Button(loc("common.save")) { save() }
                    .buttonStyle(PrimaryButtonStyle())
                    .disabled(!draft.isValid || saving)
                    .keyboardShortcut("s", modifiers: .command)
            }
            .padding(12)
        }
    }

    private var title: String {
        switch target {
        case .newSsh: return loc("editor.newSsh")
        case .newRdp: return loc("editor.newRdp")
        case .newCredential: return loc("editor.newCredential")
        case .newSnippet: return loc("editor.newSnippet")
        case .device: return loc("editor.device")
        case .vault:
            switch draft.kind {
            case .ssh: return loc("editor.ssh")
            case .rdp: return loc("editor.rdp")
            case .credential: return loc("editor.credential")
            case .folder: return loc("editor.folder")
            case .snippet: return loc("editor.snippet")
            case .device: return loc("editor.device")
            }
        }
    }

    private func save() {
        guard let e = draft.build() else { return }
        saving = true
        Task {
            if let saved = await app.save(e) {
                if draft.kind == .snippet { app.selectedSnippet = saved.id } else if draft.kind != .device && draft.kind != .credential { app.selection = saved.id }
                app.editing = nil
                app.showToast(loc("common.saved"))
            }
            saving = false
        }
    }

    // MARK: 字段

    private func labeled<C: View>(_ label: String, info: String? = nil, @ViewBuilder _ c: () -> C) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 4) {
                Text(label).font(.system(size: 11.5, weight: .medium)).foregroundStyle(p.secondaryText)
                if let info { InfoButton(text: info) }
            }
            c()
        }
    }

    private var folderPicker: some View {
        labeled(loc("editor.folder")) {
            Picker("", selection: $draft.folderId) {
                Text(loc("hosts.root")).tag(String?.none)
                ForEach(app.hostNodes().filter { $0.kind == .folder && $0.id != draft.id }) { f in
                    Text(f.title).tag(Optional(f.id))
                }
            }
            .labelsHidden()
        }
    }

    private var jumpDevices: [DeviceInfo] { app.devices.filter { !$0.isSelf && $0.kind != .mobile } }
    /// 可作跳板的其他 SSH 主机（不含正在编辑的这台）
    private var jumpHosts: [VaultEntry] {
        app.vault.filter { if case .sshHost = $0.data { return $0.id != draft.id }; return false }
    }

    /// SSH 主机的跳转：经设备（d:<设备 id>）或经另一台 SSH 主机（s:<条目 id>），二选一
    private var sshJump: Binding<String> {
        Binding(
            get: {
                if !draft.jumpDeviceId.isEmpty { return "d:" + draft.jumpDeviceId }
                if !draft.jumpHostId.isEmpty { return "s:" + draft.jumpHostId }
                return ""
            },
            set: { v in
                draft.jumpDeviceId = v.hasPrefix("d:") ? String(v.dropFirst(2)) : ""
                draft.jumpHostId = v.hasPrefix("s:") ? String(v.dropFirst(2)) : ""
            }
        )
    }

    @ViewBuilder
    private var sshFields: some View {
        labeled(loc("editor.label")) { TextField("web-01", text: $draft.label).fieldStyle() }
        HStack(spacing: 8) {
            labeled(loc("ssh.host")) { TextField("10.0.0.5", text: $draft.host).fieldStyle() }
            labeled(loc("ssh.port")) { TextField("22", text: $draft.port).fieldStyle(invalid: draft.portValue == nil && !draft.port.isEmpty).frame(width: 70) }
        }
        labeled(loc("ssh.username")) { TextField("root", text: $draft.username).fieldStyle() }
        labeled(loc("ssh.auth"), info: loc("ssh.auth.info")) {
            Picker("", selection: $draft.authKind) {
                Text(loc("ssh.auth.password")).tag("password")
                Text(loc("ssh.auth.key")).tag("key")
                Text(loc("ssh.auth.interactive")).tag("interactive")
                Text(loc("ssh.auth.agent")).tag("agent")
            }
            .labelsHidden().pickerStyle(.menu)
        }
        if draft.authKind == "password" {
            labeled(loc("ssh.password"), info: loc("ssh.password.info")) { SecureField(loc("ssh.password.ask"), text: $draft.password).fieldStyle() }
        } else if draft.authKind == "key" {
            labeled(loc("ssh.credential")) {
                HStack {
                    Picker("", selection: $draft.credentialId) {
                        Text(loc("ssh.credential.choose")).tag("")
                        ForEach(app.credentials, id: \.id) { c in
                            if case .credential(let l, _, _, _, _) = c.data { Text(l).tag(c.id) }
                        }
                    }
                    .labelsHidden()
                    Button(loc("ssh.credential.new")) { app.hostsTab = .credentials; app.editing = .newCredential }
                        .buttonStyle(SecondaryButtonStyle())
                }
            }
        }
        labeled(loc("ssh.jumpVia"), info: loc("ssh.jumpVia.info")) {
            Picker("", selection: sshJump) {
                Text(loc("ssh.jump.none")).tag("")
                Section(loc("hosts.devices")) {
                    ForEach(jumpDevices, id: \.id) { d in Text(app.deviceName(d.id)).tag("d:" + d.id) }
                }
                Section(loc("hosts.sshHosts")) {
                    ForEach(jumpHosts, id: \.id) { e in Text(app.sshLabel(e.id)).tag("s:" + e.id) }
                }
            }
            .labelsHidden()
        }
        labeled(loc("ssh.theme")) {
            Picker("", selection: $draft.terminalTheme) {
                Text(loc("ssh.theme.default")).tag("")
                ForEach(TerminalTheme.all) { t in Text(t.name).tag(t.id) }
            }
            .labelsHidden()
        }
        commonFields
    }

    @ViewBuilder
    private var rdpFields: some View {
        labeled(loc("editor.label")) { TextField("WIN-SRV", text: $draft.label).fieldStyle() }
        HStack(spacing: 8) {
            labeled(loc("ssh.host")) { TextField("10.0.0.5", text: $draft.host).fieldStyle() }
            labeled(loc("ssh.port")) { TextField("3389", text: $draft.port).fieldStyle().frame(width: 70) }
        }
        HStack(spacing: 8) {
            labeled(loc("rdp.domain")) { TextField("CORP", text: $draft.domain).fieldStyle().frame(width: 100) }
            labeled(loc("ssh.username")) { TextField("Administrator", text: $draft.username).fieldStyle() }
        }
        labeled(loc("ssh.password"), info: loc("ssh.password.info")) { SecureField(loc("ssh.password.ask"), text: $draft.password).fieldStyle() }
        labeled(loc("ssh.jump"), info: loc("rdp.jump.info")) {
            Picker("", selection: $draft.jumpDeviceId) {
                Text(loc("ssh.jump.none")).tag("")
                ForEach(jumpDevices, id: \.id) { d in Text(app.deviceName(d.id)).tag(d.id) }
            }
            .labelsHidden()
        }
        commonFields
    }

    @ViewBuilder
    private var commonFields: some View {
        folderPicker
        labeled(loc("hosts.tags"), info: loc("hosts.tags.info")) { TextField("prod, web", text: $draft.tags).fieldStyle() }
        labeled(loc("hosts.note")) {
            TextEditor(text: $draft.note).font(.system(size: 12.5)).frame(height: 60)
                .scrollContentBackground(.hidden).padding(4)
                .background(RoundedRectangle(cornerRadius: 7).fill(p.elevated))
                .overlay(RoundedRectangle(cornerRadius: 7).stroke(p.border, lineWidth: 0.7))
        }
        ToggleRow(label: loc("hosts.favorite"), isOn: $draft.favorite)
    }

    @ViewBuilder
    private var credentialFields: some View {
        labeled(loc("editor.label")) { TextField(loc("cred.label.placeholder"), text: $draft.label).fieldStyle() }
        labeled(loc("ssh.username")) { TextField("deploy", text: $draft.username).fieldStyle() }
        labeled(loc("ssh.password")) { SecureField("", text: $draft.password).fieldStyle() }
        labeled(loc("cred.privateKey"), info: loc("cred.privateKey.info")) {
            VStack(alignment: .trailing, spacing: 6) {
                TextEditor(text: $draft.privateKey)
                    .font(.system(size: 10.5, design: .monospaced))
                    .frame(height: 120)
                    .scrollContentBackground(.hidden).padding(4)
                    .background(RoundedRectangle(cornerRadius: 7).fill(p.elevated))
                    .overlay(RoundedRectangle(cornerRadius: 7).stroke(p.border, lineWidth: 0.7))
                HStack {
                    if !draft.privateKey.isEmpty {
                        Text(keySummary).font(.system(size: 11)).foregroundStyle(p.tertiaryText)
                    }
                    Spacer()
                    Button(loc("cred.import")) { importKey() }.buttonStyle(SecondaryButtonStyle())
                }
            }
        }
        labeled(loc("cred.passphrase")) { SecureField("", text: $draft.passphrase).fieldStyle() }
    }

    private var keySummary: String {
        let k = draft.privateKey
        if k.contains("OPENSSH PRIVATE KEY") { return "OpenSSH" }
        if k.contains("RSA PRIVATE KEY") { return "RSA (PEM)" }
        if k.contains("EC PRIVATE KEY") { return "ECDSA (PEM)" }
        if k.contains("PRIVATE KEY") { return "PKCS#8" }
        return loc("cred.unknownFormat")
    }

    private func importKey() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.showsHiddenFiles = true
        panel.directoryURL = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".ssh")
        if panel.runModal() == .OK, let url = panel.url, let text = try? String(contentsOf: url, encoding: .utf8) {
            draft.privateKey = text
            if draft.label.isEmpty { draft.label = url.lastPathComponent }
        }
    }

    @ViewBuilder
    private var folderFields: some View {
        labeled(loc("editor.label")) { TextField("", text: $draft.label).fieldStyle() }
        labeled(loc("folder.color")) {
            HStack(spacing: 8) {
                ForEach(["", "#3B82F6", "#10B981", "#F59E0B", "#EF4444", "#8B5CF6", "#EC4899", "#64748B"], id: \.self) { c in
                    Circle()
                        .fill(c.isEmpty ? p.secondaryText.opacity(0.3) : Color(hex: c))
                        .frame(width: 20, height: 20)
                        .overlay(Circle().stroke(p.text, lineWidth: draft.color == c ? 2 : 0).padding(-3))
                        .onTapGesture { draft.color = c }
                }
            }
            .padding(.vertical, 4)
        }
        folderPicker
    }

    @ViewBuilder
    private var deviceFields: some View {
        if let d = app.device(draft.deviceId) {
            HStack(spacing: 8) {
                PlatformGlyph(platform: d.platform, size: 16).foregroundStyle(p.secondaryText)
                Text(d.hostname).font(.system(size: 12.5, weight: .medium))
                StatusDot(state: StatusDot.of(d), size: 7)
            }
        }
        labeled(loc("device.alias")) { TextField(app.device(draft.deviceId)?.hostname ?? "", text: $draft.label).fieldStyle() }
        commonFields
    }

    @ViewBuilder
    private var snippetFields: some View {
        labeled(loc("editor.label")) { TextField("", text: $draft.label).fieldStyle() }
        labeled(loc("snippet.command")) {
            TextEditor(text: $draft.command)
                .font(.system(size: 12, design: .monospaced))
                .frame(height: 140)
                .scrollContentBackground(.hidden).padding(4)
                .background(RoundedRectangle(cornerRadius: 7).fill(p.elevated))
                .overlay(RoundedRectangle(cornerRadius: 7).stroke(p.border, lineWidth: 0.7))
        }
        labeled(loc("hosts.tags")) { TextField("linux, docker", text: $draft.tags).fieldStyle() }
    }
}
