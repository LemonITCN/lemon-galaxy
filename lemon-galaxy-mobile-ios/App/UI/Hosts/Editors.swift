import SwiftUI
import GalaxyKit

struct EditorSheet: View {
    let target: EditorTarget
    var body: some View {
        NavigationStack {
            switch target {
            case .folder(let e, let parent): FolderEditor(entry: e, parent: parent)
            case .ssh(let e, let folder): SshEditor(entry: e, folder: folder)
            case .rdp(let e, let folder): RdpEditor(entry: e, folder: folder)
            case .credential(let e): CredentialEditor(entry: e)
            case .snippet(let e): SnippetEditor(entry: e)
            case .device(let d): DeviceEditor(deviceId: d)
            case .moveItem(let item): MovePicker(title: item.title, current: item.folderId, excluded: []) { f, account in try await account.move(item, to: f) }
            case .moveFolder(let f):
                MovePicker(title: { if case .folder(let n, _, _) = f.data { return n }; return "" }(), current: f.folderId, excludedFolder: f.id) { parent, account in
                    try await account.moveFolder(f, to: parent)
                }
            }
        }
    }
}

/// 编辑器的公共外壳：取消 / 保存 + 出错提示
struct EditorForm<Content: View>: View {
    let title: String
    let canSave: Bool
    let save: () async throws -> Void
    @ViewBuilder let content: () -> Content
    @EnvironmentObject var app: AppModel
    @Environment(\.dismiss) private var dismiss
    @Environment(\.loc) private var loc
    @State private var saving = false
    @State private var error: String?

    var body: some View {
        Form {
            content()
            if let error {
                Section { Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(.red).font(.callout) }
            }
        }
        .galaxyList()
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) { Button(loc("common.cancel")) { dismiss() } }
            ToolbarItem(placement: .confirmationAction) {
                if saving { ProgressView() } else {
                    Button(loc("common.save")) {
                        saving = true
                        Task {
                            do {
                                try await save()
                                dismiss()
                            } catch {
                                self.error = describeError(error)
                            }
                            saving = false
                        }
                    }
                    .disabled(!canSave)
                    .fontWeight(.semibold)
                }
            }
        }
    }
}

private func parseTags(_ s: String) -> [String] {
    s.split(whereSeparator: { $0 == "," || $0 == "，" }).map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
}

struct FolderPickerRow: View {
    @Binding var folder: String?
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.loc) private var loc
    var body: some View {
        Picker(loc("field.folder"), selection: $folder) {
            Text(loc("move.root")).tag(String?.none)
            ForEach(HostTree.moveTargets(account.hosts), id: \.id) { t in Text(t.path).tag(String?.some(t.id)) }
        }
    }
}

struct FolderEditor: View {
    let entry: VaultEntry?
    let parent: String?
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.loc) private var loc
    @State private var name = ""
    @State private var color: String?
    private let colors = ["#3B82F6", "#10B981", "#F59E0B", "#EF4444", "#8B5CF6", "#EC4899", "#6B7280"]

    var body: some View {
        EditorForm(title: loc(entry == nil ? "folder.new" : "folder.rename"), canSave: !name.trimmingCharacters(in: .whitespaces).isEmpty, save: save) {
            Section {
                TextField(loc("folder.name"), text: $name)
            }
            Section(loc("folder.color")) {
                HStack(spacing: 12) {
                    Button { color = nil } label: {
                        Circle().stroke(Color.secondary, lineWidth: 1.5).frame(width: 26, height: 26)
                            .overlay(color == nil ? Image(systemName: "checkmark").font(.caption.bold()) : nil)
                    }
                    ForEach(colors, id: \.self) { c in
                        Button { color = c } label: {
                            Circle().fill(Color(hex: c)).frame(width: 26, height: 26)
                                .overlay(color == c ? Image(systemName: "checkmark").font(.caption.bold()).foregroundStyle(.white) : nil)
                        }
                    }
                }
                .buttonStyle(.plain)
            }
        }
        .onAppear {
            if let entry, case .folder(let n, let c, _) = entry.data { name = n; color = c }
        }
    }

    private func save() async throws {
        let n = name.trimmingCharacters(in: .whitespaces)
        var expanded = true
        if let entry, case .folder(_, _, let e) = entry.data { expanded = e }
        try await account.saveData(id: entry?.id, folderId: entry?.folderId ?? parent, data: .folder(name: n, color: color, expanded: expanded))
    }
}

struct SshEditor: View {
    let entry: VaultEntry?
    @State var folder: String?
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.loc) private var loc
    @State private var label = ""
    @State private var host = ""
    @State private var port = "22"
    @State private var username = ""
    @State private var authKind = "password"
    @State private var password = ""
    @State private var credentialId: String?
    @State private var jump = "none"
    @State private var tags = ""
    @State private var note = ""
    @State private var theme: String?
    @State private var favorite = false

    var body: some View {
        EditorForm(title: loc(entry == nil ? "editor.newSsh" : "editor.editSsh"), canSave: valid, save: save) {
            Section {
                TextField(loc("field.label"), text: $label)
                TextField(loc("field.host"), text: $host).textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(.URL)
                TextField(loc("field.port"), text: $port).keyboardType(.numberPad)
                TextField(loc("field.username"), text: $username).textInputAutocapitalization(.never).autocorrectionDisabled()
            } footer: {
                if !port.isEmpty, UInt16(port) == nil || UInt16(port) == 0 { Text(loc("field.portInvalid")).foregroundStyle(.red) }
            }
            Section {
                Picker(loc("field.auth"), selection: $authKind) {
                    Text(loc("auth.password")).tag("password")
                    Text(loc("auth.key")).tag("key")
                    Text(loc("auth.interactive")).tag("interactive")
                    Text(loc("auth.agent")).tag("agent")
                }
                switch authKind {
                case "password":
                    SecretField(title: loc("field.passwordOptional"), text: $password)
                case "key":
                    Picker(loc("field.credential"), selection: $credentialId) {
                        Text(loc("common.none")).tag(String?.none)
                        ForEach(account.hosts.credentials, id: \.id) { c in
                            if case .credential(let l, _, _, _, _) = c.data { Text(l).tag(String?.some(c.id)) }
                        }
                    }
                default:
                    EmptyView()
                }
            } footer: {
                HStack { Spacer(); InfoButton(text: loc(authKind == "agent" ? "auth.agentInfo" : "auth.info"), size: 13) }
            }
            Section {
                Picker(loc("field.jump"), selection: $jump) {
                    Text(loc("jump.none")).tag("none")
                    ForEach(account.devices.filter { $0.kind != .mobile && !$0.isSelf }, id: \.id) { d in
                        Text(loc("jump.device", account.deviceName(d.id))).tag("device:" + d.id)
                    }
                    ForEach(account.hosts.sshHosts.filter { $0.id != entry?.id }, id: \.id) { h in
                        Text(loc("jump.host", account.hosts.sshLabel(h.id))).tag("ssh:" + h.id)
                    }
                }
            } footer: {
                HStack { Spacer(); InfoButton(text: loc("jump.info"), size: 13) }
            }
            Section {
                FolderPickerRow(folder: $folder)
                TextField(loc("field.tags"), text: $tags).textInputAutocapitalization(.never)
                Picker(loc("field.terminalTheme"), selection: $theme) {
                    Text(loc("common.useDefault")).tag(String?.none)
                    ForEach(TerminalTheme.all) { t in Text(t.name).tag(String?.some(t.id)) }
                }
                Toggle(loc("action.favorite"), isOn: $favorite)
                TextField(loc("field.note"), text: $note, axis: .vertical).lineLimit(2...5)
            }
        }
        .onAppear(perform: load)
    }

    private var valid: Bool {
        !host.trimmingCharacters(in: .whitespaces).isEmpty && !username.trimmingCharacters(in: .whitespaces).isEmpty
            && (UInt16(port) ?? 0) > 0
    }

    private func load() {
        guard let entry, case let .sshHost(l, h, pt, u, auth, jd, jh, tg, n, fav, th) = entry.data else { return }
        label = l; host = h; port = String(pt); username = u; tags = tg.joined(separator: ", "); note = n; favorite = fav; theme = th
        switch auth {
        case .password(let pw): authKind = "password"; password = pw ?? ""
        case .privateKey(let c): authKind = "key"; credentialId = c
        case .agent: authKind = "agent"
        case .interactive: authKind = "interactive"
        }
        if let jd { jump = "device:" + jd } else if let jh { jump = "ssh:" + jh }
    }

    private func save() async throws {
        let auth: SshAuth
        switch authKind {
        case "key": auth = .privateKey(credentialId: credentialId ?? "")
        case "agent": auth = .agent
        case "interactive": auth = .interactive
        default: auth = .password(password: password.isEmpty ? nil : password)
        }
        let h = host.trimmingCharacters(in: .whitespaces)
        let data = VaultData.sshHost(label: label.trimmingCharacters(in: .whitespaces).isEmpty ? h : label, host: h, port: UInt16(port) ?? 22,
                                     username: username.trimmingCharacters(in: .whitespaces), auth: auth,
                                     jumpDeviceId: jump.hasPrefix("device:") ? String(jump.dropFirst(7)) : nil,
                                     jumpHostId: jump.hasPrefix("ssh:") ? String(jump.dropFirst(4)) : nil,
                                     tags: parseTags(tags), note: note, favorite: favorite, terminalTheme: theme)
        try await account.saveData(id: entry?.id, folderId: folder, data: data)
    }
}

struct RdpEditor: View {
    let entry: VaultEntry?
    @State var folder: String?
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.loc) private var loc
    @State private var label = ""
    @State private var host = ""
    @State private var port = "3389"
    @State private var username = ""
    @State private var password = ""
    @State private var domain = ""
    @State private var jump: String?
    @State private var tags = ""
    @State private var note = ""
    @State private var favorite = false

    var body: some View {
        EditorForm(title: loc(entry == nil ? "editor.newRdp" : "editor.editRdp"), canSave: !host.isEmpty && (UInt16(port) ?? 0) > 0, save: save) {
            Section {
                TextField(loc("field.label"), text: $label)
                TextField(loc("field.host"), text: $host).textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(.URL)
                TextField(loc("field.port"), text: $port).keyboardType(.numberPad)
            } footer: {
                HStack { Spacer(); InfoButton(text: loc("rdp.unsupported"), size: 13) }
            }
            Section {
                TextField(loc("field.username"), text: $username).textInputAutocapitalization(.never).autocorrectionDisabled()
                SecretField(title: loc("field.passwordOptional"), text: $password)
                TextField(loc("field.domain"), text: $domain).textInputAutocapitalization(.never).autocorrectionDisabled()
            }
            Section {
                Picker(loc("field.jump"), selection: $jump) {
                    Text(loc("jump.none")).tag(String?.none)
                    ForEach(account.devices.filter { $0.kind != .mobile && !$0.isSelf }, id: \.id) { d in
                        Text(loc("jump.device", account.deviceName(d.id))).tag(String?.some(d.id))
                    }
                }
                FolderPickerRow(folder: $folder)
                TextField(loc("field.tags"), text: $tags).textInputAutocapitalization(.never)
                Toggle(loc("action.favorite"), isOn: $favorite)
                TextField(loc("field.note"), text: $note, axis: .vertical).lineLimit(2...5)
            }
        }
        .onAppear {
            guard let entry, case let .rdpHost(l, h, pt, u, pw, dm, jd, tg, n, fav) = entry.data else { return }
            label = l; host = h; port = String(pt); username = u; password = pw ?? ""; domain = dm ?? ""; jump = jd
            tags = tg.joined(separator: ", "); note = n; favorite = fav
        }
    }

    private func save() async throws {
        let h = host.trimmingCharacters(in: .whitespaces)
        try await account.saveData(id: entry?.id, folderId: folder, data: .rdpHost(
            label: label.isEmpty ? h : label, host: h, port: UInt16(port) ?? 3389, username: username,
            password: password.isEmpty ? nil : password, domain: domain.isEmpty ? nil : domain, jumpDeviceId: jump,
            tags: parseTags(tags), note: note, favorite: favorite))
    }
}

struct CredentialEditor: View {
    let entry: VaultEntry?
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.loc) private var loc
    @State private var label = ""
    @State private var username = ""
    @State private var password = ""
    @State private var privateKey = ""
    @State private var passphrase = ""
    @State private var importing = false

    var body: some View {
        EditorForm(title: loc(entry == nil ? "editor.newCredential" : "editor.editCredential"), canSave: !label.trimmingCharacters(in: .whitespaces).isEmpty, save: save) {
            Section {
                TextField(loc("field.label"), text: $label)
                TextField(loc("field.username"), text: $username).textInputAutocapitalization(.never).autocorrectionDisabled()
                SecretField(title: loc("field.password"), text: $password)
            }
            Section {
                TextEditor(text: $privateKey)
                    .font(.system(.caption, design: .monospaced))
                    .frame(minHeight: 120)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Button { importing = true } label: { Label(loc("credential.importKey"), systemImage: "square.and.arrow.down") }
                SecretField(title: loc("field.passphrase"), text: $passphrase)
            } header: {
                Text(loc("field.privateKey"))
            } footer: {
                HStack { Spacer(); InfoButton(text: loc("credentials.info"), size: 13) }
            }
        }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.data, .text, .item]) { r in
            guard case .success(let url) = r else { return }
            let ok = url.startAccessingSecurityScopedResource()
            defer { if ok { url.stopAccessingSecurityScopedResource() } }
            if let s = try? String(contentsOf: url, encoding: .utf8), s.count < 64 * 1024 { privateKey = s }
        }
        .onAppear {
            guard let entry, case let .credential(l, u, pw, k, pp) = entry.data else { return }
            label = l; username = u; password = pw ?? ""; privateKey = k ?? ""; passphrase = pp ?? ""
        }
    }

    private func save() async throws {
        let key = privateKey.trimmingCharacters(in: .whitespacesAndNewlines)
        try await account.saveData(id: entry?.id, folderId: nil, data: .credential(
            label: label.trimmingCharacters(in: .whitespaces), username: username, password: password.isEmpty ? nil : password,
            privateKey: key.isEmpty ? nil : key + "\n", passphrase: passphrase.isEmpty ? nil : passphrase))
    }
}

struct SnippetEditor: View {
    let entry: VaultEntry?
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.loc) private var loc
    @State private var label = ""
    @State private var command = ""
    @State private var tags = ""

    var body: some View {
        EditorForm(title: loc(entry == nil ? "editor.newSnippet" : "editor.editSnippet"),
                   canSave: !label.trimmingCharacters(in: .whitespaces).isEmpty && !command.isEmpty, save: save) {
            Section {
                TextField(loc("field.label"), text: $label)
                TextField(loc("field.tags"), text: $tags).textInputAutocapitalization(.never)
            }
            Section(loc("field.command")) {
                TextEditor(text: $command)
                    .font(.system(.body, design: .monospaced))
                    .frame(minHeight: 120)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
            }
        }
        .onAppear {
            guard let entry, case let .snippet(l, c, t) = entry.data else { return }
            label = l; command = c; tags = t.joined(separator: ", ")
        }
    }

    private func save() async throws {
        try await account.saveData(id: entry?.id, folderId: nil, data: .snippet(label: label.trimmingCharacters(in: .whitespaces),
                                                                                command: command, tags: parseTags(tags)))
    }
}

struct DeviceEditor: View {
    let deviceId: String
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var alias = ""
    @State private var tags = ""
    @State private var note = ""
    @State private var favorite = false
    @State private var folder: String?

    var body: some View {
        EditorForm(title: loc("editor.editDevice"), canSave: true, save: save) {
            if let d = account.device(deviceId) {
                Section {
                    LabeledContent(loc("device.hostname"), value: d.hostname)
                    LabeledContent(loc("device.system"), value: d.osVersion)
                    LabeledContent(loc("device.arch"), value: d.arch)
                    LabeledContent(loc("device.version"), value: d.appVersion)
                    LabeledContent(loc("device.kind"), value: loc("kind." + kindKey(d.kind)))
                }
            }
            Section {
                TextField(loc("field.alias"), text: $alias)
                FolderPickerRow(folder: $folder)
                TextField(loc("field.tags"), text: $tags).textInputAutocapitalization(.never)
                Toggle(loc("action.favorite"), isOn: $favorite)
                TextField(loc("field.note"), text: $note, axis: .vertical).lineLimit(2...5)
            }
        }
        .onAppear {
            guard let m = account.hosts.metaFor(deviceId), case let .deviceMeta(_, a, t, n, f) = m.data else { return }
            alias = a ?? ""; tags = t.joined(separator: ", "); note = n; favorite = f; folder = m.folderId
        }
    }

    private func kindKey(_ k: DeviceKind) -> String {
        switch k {
        case .control: return "control"
        case .worker: return "worker"
        case .mobile: return "mobile"
        }
    }

    private func save() async throws {
        let a = alias.trimmingCharacters(in: .whitespaces)
        let t = parseTags(tags)
        try await account.updateDeviceMeta(deviceId, folder: .some(folder)) { al, tg, nt, fav in
            al = a.isEmpty ? nil : a
            tg = t
            nt = note
            fav = favorite
        }
    }
}

struct MovePicker: View {
    let title: String
    let current: String?
    var excluded: Set<String> = []
    var excludedFolder: String?
    let move: (String?, GalaxyAccount) async throws -> Void
    @EnvironmentObject var account: GalaxyAccount
    @EnvironmentObject var app: AppModel
    @Environment(\.dismiss) private var dismiss
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    init(title: String, current: String?, excluded: Set<String> = [], excludedFolder: String? = nil, move: @escaping (String?, GalaxyAccount) async throws -> Void) {
        self.title = title
        self.current = current
        self.excluded = excluded
        self.excludedFolder = excludedFolder
        self.move = move
    }

    var body: some View {
        let skip = excludedFolder.map { HostTree.descendants(account.hosts, of: $0) } ?? excluded
        List {
            row(nil, name: loc("move.root"))
            ForEach(HostTree.moveTargets(account.hosts, excluding: skip), id: \.id) { t in row(t.id, name: t.path) }
        }
        .galaxyList()
        .navigationTitle(loc("move.title", title))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { ToolbarItem(placement: .cancellationAction) { Button(loc("common.cancel")) { dismiss() } } }
    }

    private func row(_ id: String?, name: String) -> some View {
        Button {
            Task {
                do { try await move(id, account); dismiss() } catch { app.showError(error) }
            }
        } label: {
            HStack {
                Image(systemName: id == nil ? "tray" : "folder").foregroundStyle(p.accent)
                Text(name).foregroundStyle(p.text)
                Spacer()
                if id == current { Image(systemName: "checkmark").foregroundStyle(p.accent) }
            }
        }
        .galaxyRow()
    }
}
