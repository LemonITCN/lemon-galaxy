import SwiftUI
import PhotosUI
import UniformTypeIdentifiers
import GalaxyKit

/// 文件页：先选目标（在线设备或 SSH 主机），再浏览远端文件。
struct FilesView: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var account: GalaxyAccount
    @EnvironmentObject var transfers: TransferCenter
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var showTransfers = false
    @State private var browser: FileBrowserModel?

    var body: some View {
        Group {
            if let browser {
                FileBrowserView(model: browser, onChangeTarget: { app.filesTarget = nil })
            } else {
                targetList
            }
        }
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button { showTransfers = true } label: {
                    Image(systemName: transfers.activeCount > 0 ? "arrow.up.arrow.down.circle.fill" : "arrow.up.arrow.down.circle")
                }
                .accessibilityLabel(loc("files.transfers"))
            }
        }
        .sheet(isPresented: $showTransfers) {
            TransfersView().environmentObject(app).environmentObject(transfers)
        }
        .onAppear(perform: sync)
        .onChange(of: app.filesTarget) { _ in sync() }
    }

    private func sync() {
        guard let t = app.filesTarget else {
            if let old = browser?.target { browser = nil; app.releaseIfUnused(old) }
            return
        }
        if browser?.target != t {
            let old = browser?.target
            browser = FileBrowserModel(app: app, target: t)
            if let old { app.releaseIfUnused(old) }
        }
    }

    private var targetList: some View {
        let devices = account.devices.filter { $0.kind != .mobile && !$0.isSelf }
            .sorted { ($0.online || $0.lanSeen ? 0 : 1, account.deviceName($0.id).lowercased()) < ($1.online || $1.lanSeen ? 0 : 1, account.deviceName($1.id).lowercased()) }
        let ssh = account.hosts.sshHosts
        return List {
            if devices.isEmpty && ssh.isEmpty {
                EmptyState(icon: "folder", title: loc("files.targetsEmpty")).listRowBackground(Color.clear)
            }
            if !devices.isEmpty {
                Section(loc("files.devices")) {
                    ForEach(devices, id: \.id) { d in
                        let ok = d.online || d.lanSeen
                        Button { app.filesTarget = .device(d.id) } label: {
                            HStack(spacing: 12) {
                                PlatformBadge(symbol: Icons.platform(d.platform), tint: ok ? p.accent : p.offline, size: 30)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(account.deviceName(d.id)).foregroundStyle(p.text)
                                    Text(ok ? d.osVersion : loc("status.offline")).font(.caption).foregroundStyle(p.secondaryText)
                                }
                                Spacer()
                                Image(systemName: "chevron.right").font(.caption).foregroundStyle(p.tertiaryText)
                            }
                        }
                        .disabled(!ok)
                        .galaxyRow()
                    }
                }
            }
            if !ssh.isEmpty {
                Section(loc("files.sshHosts")) {
                    ForEach(ssh, id: \.id) { e in
                        Button { app.filesTarget = .ssh(e.id) } label: {
                            HStack(spacing: 12) {
                                PlatformBadge(symbol: "terminal", tint: p.success, size: 30)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(account.hosts.sshLabel(e.id)).foregroundStyle(p.text)
                                    Text(HostItem.ssh(e).subtitle).font(.caption).foregroundStyle(p.secondaryText)
                                }
                                Spacer()
                                Image(systemName: "chevron.right").font(.caption).foregroundStyle(p.tertiaryText)
                            }
                        }
                        .galaxyRow()
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
        .galaxyList()
        .navigationTitle(loc("nav.files"))
    }
}

struct FileBrowserView: View {
    @ObservedObject var model: FileBrowserModel
    var onChangeTarget: () -> Void
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var settings: AppSettings
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var selected: FileEntry?
    @State private var prompt: NamePrompt?
    @State private var deleting: FileEntry?
    @State private var chmodTarget: FileEntry?
    @State private var editing: FileEntry?
    @State private var importing = false
    @State private var photos: [PhotosPickerItem] = []
    @State private var photoPicker = false
    @State private var query = ""
    @State private var searchContent = false
    @State private var shareURL: URL?

    struct NamePrompt: Identifiable {
        let id = UUID()
        let title: String
        let initial: String
        let entry: FileEntry?
    }

    var body: some View {
        VStack(spacing: 0) {
            breadcrumb
            List {
                if let e = model.error {
                    Section {
                        Label(e, systemImage: "exclamationmark.triangle").foregroundStyle(p.danger)
                        Button(loc("common.retry")) { Task { await model.refresh() } }
                    }
                    .galaxyRow()
                }
                let rows = model.searchResults ?? model.sorted(settings)
                if rows.isEmpty && !model.loading && model.error == nil {
                    EmptyState(icon: model.searchResults == nil ? "folder" : "magnifyingglass",
                               title: loc(model.searchResults == nil ? "files.empty" : "search.empty"))
                        .listRowBackground(Color.clear)
                }
                ForEach(rows, id: \.path) { e in
                    FileRow(entry: e, showPath: model.searchResults != nil)
                        .contentShape(Rectangle())
                        .onTapGesture { open(e) }
                        .contextMenu { actions(e) }
                        .swipeActions {
                            Button(role: .destructive) { deleting = e } label: { Label(loc("common.delete"), systemImage: "trash") }
                            Button { prompt = NamePrompt(title: loc("common.rename"), initial: e.name, entry: e) } label: { Label(loc("common.rename"), systemImage: "pencil") }
                                .tint(p.accent)
                        }
                        .galaxyRow()
                }
            }
            .listStyle(.plain)
            .galaxyList()
            .overlay { if model.loading && model.entries.isEmpty { ProgressView() } }
            .refreshable { await model.refresh() }
        }
        .background(p.background)
        .navigationTitle(model.name)
        .navigationBarTitleDisplayMode(.inline)
        .searchable(text: $query, prompt: loc("files.search"))
        .onSubmit(of: .search) { Task { await model.search(query, content: searchContent) } }
        .onChange(of: query) { q in if q.isEmpty { model.searchResults = nil } }
        .toolbar {
            ToolbarItem(placement: .navigationBarLeading) {
                Button(action: onChangeTarget) { Image(systemName: "chevron.left") }
                    .accessibilityLabel(loc("files.changeTarget"))
            }
            ToolbarItem(placement: .primaryAction) { moreMenu }
        }
        .task { await model.start() }
        .confirmationDialog(selected?.name ?? "", isPresented: Binding(get: { selected != nil }, set: { if !$0 { selected = nil } }),
                            titleVisibility: .visible, presenting: selected) { e in actions(e) }
        .alert(prompt?.title ?? "", isPresented: Binding(get: { prompt != nil }, set: { if !$0 { prompt = nil } }), presenting: prompt) { pr in
            NamePromptActions(prompt: pr) { name in
                Task {
                    if let e = pr.entry { await model.rename(e, to: name) } else { await model.mkdir(name) }
                }
            }
        }
        .alert(loc("files.deleteTitle", deleting?.name ?? ""), isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), presenting: deleting) { e in
            Button(loc("common.delete"), role: .destructive) { Task { await model.delete(e) } }
            Button(loc("common.cancel"), role: .cancel) {}
        } message: { e in Text(loc(e.kind == .dir ? "files.deleteDirMessage" : "files.deleteFileMessage")) }
        .sheet(item: $chmodTarget) { e in
            ChmodSheet(entry: e) { mode in Task { await model.chmod(e, mode) } }
        }
        .sheet(item: $editing) { e in
            TextFileEditor(model: model, entry: e).environmentObject(app)
        }
        .sheet(item: Binding(get: { shareURL.map { ShareItem(url: $0) } }, set: { if $0 == nil { shareURL = nil } })) { item in
            ShareSheet(items: [item.url])
        }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.item], allowsMultipleSelection: true) { r in
            if case .success(let urls) = r { Task { await model.upload(urls) } }
        }
        .photosPicker(isPresented: $photoPicker, selection: $photos, matching: .any(of: [.images, .videos]))
        .onChange(of: photos) { items in
            guard !items.isEmpty else { return }
            let picked = items
            photos = []
            Task {
                for (i, item) in picked.enumerated() {
                    guard let data = try? await item.loadTransferable(type: Data.self) else { continue }
                    let ext = item.supportedContentTypes.first?.preferredFilenameExtension ?? "jpg"
                    let stamp = Int(Date().timeIntervalSince1970)
                    await model.upload(data: data, name: "IMG_\(stamp)_\(i + 1).\(ext)")
                }
            }
        }
    }

    private var breadcrumb: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 4) {
                    Button { Task { await model.goUp() } } label: { Image(systemName: "arrow.up").font(.caption.weight(.semibold)) }
                        .disabled(!model.canGoUp)
                        .padding(.trailing, 6)
                    ForEach(Array(RemotePath.crumbs(model.path).enumerated()), id: \.offset) { i, c in
                        if i > 1 { Image(systemName: "chevron.compact.right").font(.caption2).foregroundStyle(p.tertiaryText) }
                        Button(c.name) { Task { await model.load(c.path) } }
                            .font(.caption.weight(i == RemotePath.crumbs(model.path).count - 1 ? .semibold : .regular))
                            .foregroundStyle(i == RemotePath.crumbs(model.path).count - 1 ? p.text : p.accent)
                            .id(i)
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
            }
            .onChange(of: model.path) { _ in withAnimation { proxy.scrollTo(RemotePath.crumbs(model.path).count - 1, anchor: .trailing) } }
        }
        .background(p.card)
        .overlay(Divider(), alignment: .bottom)
    }

    private var moreMenu: some View {
        Menu {
            Section {
                Button { importing = true } label: { Label(loc("files.uploadFiles"), systemImage: "doc.badge.plus") }
                Button { photoPicker = true } label: { Label(loc("files.uploadPhotos"), systemImage: "photo.badge.plus") }
                Button { prompt = NamePrompt(title: loc("files.newFolder"), initial: "", entry: nil) } label: { Label(loc("files.newFolder"), systemImage: "folder.badge.plus") }
            }
            Section {
                Picker(loc("files.sort"), selection: $settings.fileSort) {
                    Text(loc("sort.name")).tag(FileSort.name)
                    Text(loc("sort.size")).tag(FileSort.size)
                    Text(loc("sort.modified")).tag(FileSort.modified)
                }
                Toggle(loc("sort.descending"), isOn: $settings.fileSortDescending)
                Toggle(loc("files.showHidden"), isOn: $settings.showHiddenFiles)
                Toggle(loc("files.searchContent"), isOn: $searchContent)
            }
            Section {
                Button { Task { await model.refresh() } } label: { Label(loc("common.refresh"), systemImage: "arrow.clockwise") }
                if case .device(let d) = model.target {
                    Button { app.openTerminal(.device(d)) } label: { Label(loc("action.terminal"), systemImage: "terminal") }
                } else {
                    Button { app.openTerminal(model.target) } label: { Label(loc("action.terminal"), systemImage: "terminal") }
                }
            }
        } label: { Image(systemName: "ellipsis.circle") }
    }

    private func open(_ e: FileEntry) {
        if e.kind == .dir || (e.kind == .symlink && e.size == 0) {
            query = ""
            model.searchResults = nil
            Task { await model.load(e.path) }
        } else {
            selected = e
        }
    }

    @ViewBuilder private func actions(_ e: FileEntry) -> some View {
        if e.kind != .dir {
            Button { Task { await model.download(e) } } label: { Label(loc("files.download"), systemImage: "arrow.down.circle") }
            Button {
                Task {
                    guard let item = await model.download(e) else { return }
                    while item.isActive { try? await Task.sleep(nanoseconds: 200_000_000) }
                    if item.status == .done { shareURL = item.localURL }
                }
            } label: { Label(loc("common.share"), systemImage: "square.and.arrow.up") }
            if e.size <= 2 * 1024 * 1024 {
                Button { editing = e } label: { Label(loc("files.edit"), systemImage: "pencil.line") }
            }
        }
        Button { prompt = NamePrompt(title: loc("common.rename"), initial: e.name, entry: e) } label: { Label(loc("common.rename"), systemImage: "pencil") }
        Button { chmodTarget = e } label: { Label(loc("files.permissions"), systemImage: "lock.shield") }
        Button { UIPasteboard.general.string = e.path; app.showToast(loc("common.copied")) } label: { Label(loc("files.copyPath"), systemImage: "doc.on.doc") }
        Button(role: .destructive) { deleting = e } label: { Label(loc("common.delete"), systemImage: "trash") }
    }
}

private struct NamePromptActions: View {
    let prompt: FileBrowserView.NamePrompt
    let done: (String) -> Void
    @State private var name: String
    @Environment(\.loc) private var loc

    init(prompt: FileBrowserView.NamePrompt, done: @escaping (String) -> Void) {
        self.prompt = prompt
        self.done = done
        _name = State(initialValue: prompt.initial)
    }

    var body: some View {
        TextField(loc("files.name"), text: $name).textInputAutocapitalization(.never).autocorrectionDisabled()
        Button(loc("common.ok")) { if RemotePath.isValidName(name) { done(name.trimmingCharacters(in: .whitespaces)) } }
        Button(loc("common.cancel"), role: .cancel) {}
    }
}

struct FileRow: View {
    let entry: FileEntry
    var showPath = false
    @Environment(\.palette) private var p
    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: Icons.file(entry))
                .font(.system(size: 20))
                .foregroundStyle(entry.kind == .dir ? p.accent : p.secondaryText)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 2) {
                Text(entry.name).foregroundStyle(entry.name.hasPrefix(".") ? p.secondaryText : p.text).lineLimit(1)
                HStack(spacing: 8) {
                    if showPath { Text(RemotePath.parent(entry.path)) } else {
                        if entry.kind != .dir { Text(Fmt.bytes(entry.size)) }
                        Text(Fmt.fileDate(entry.modifiedAt))
                    }
                }
                .font(.caption.monospacedDigit())
                .foregroundStyle(p.tertiaryText)
                .lineLimit(1)
            }
            Spacer()
            Text(Fmt.mode(entry.mode, kind: entry.kind)).font(.caption2.monospaced()).foregroundStyle(p.tertiaryText)
        }
        .padding(.vertical, 2)
    }
}

/// 改权限：九个开关 + 八进制
struct ChmodSheet: View {
    let entry: FileEntry
    let apply: (UInt32) -> Void
    @Environment(\.dismiss) private var dismiss
    @Environment(\.loc) private var loc
    @State private var mode: UInt32 = 0
    @State private var octal = ""

    var body: some View {
        NavigationStack {
            Form {
                ForEach([("chmod.owner", 6), ("chmod.group", 3), ("chmod.others", 0)], id: \.1) { title, shift in
                    Section(loc(title)) {
                        bit(loc("chmod.read"), UInt32(4 << shift))
                        bit(loc("chmod.write"), UInt32(2 << shift))
                        bit(loc("chmod.exec"), UInt32(1 << shift))
                    }
                }
                Section(loc("chmod.octal")) {
                    TextField("755", text: $octal)
                        .keyboardType(.numberPad)
                        .font(.body.monospaced())
                        .onChange(of: octal) { v in if let m = Fmt.parseOctal(v) { mode = (mode & ~0o777) | (m & 0o777) } }
                    Text(Fmt.mode(mode, kind: entry.kind)).font(.body.monospaced())
                }
            }
            .navigationTitle(loc("files.permissions"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(loc("common.cancel")) { dismiss() } }
                ToolbarItem(placement: .confirmationAction) { Button(loc("common.save")) { apply(mode & 0o7777); dismiss() }.fontWeight(.semibold) }
            }
            .onAppear {
                mode = entry.mode & 0o7777
                octal = Fmt.octal(mode)
            }
        }
        .presentationDetents([.large])
    }

    private func bit(_ title: String, _ b: UInt32) -> some View {
        Toggle(title, isOn: Binding(get: { mode & b != 0 }, set: { on in
            mode = on ? mode | b : mode & ~b
            octal = Fmt.octal(mode & 0o777)
        }))
    }
}

/// 在线编辑文本文件（2 MB 以内）
struct TextFileEditor: View {
    @ObservedObject var model: FileBrowserModel
    let entry: FileEntry
    @EnvironmentObject var app: AppModel
    @Environment(\.dismiss) private var dismiss
    @Environment(\.loc) private var loc
    @State private var text = ""
    @State private var original = ""
    @State private var loading = true
    @State private var saving = false
    @State private var confirmDiscard = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Group {
                if loading {
                    ProgressView()
                } else if let error {
                    EmptyState(icon: "exclamationmark.triangle", title: error)
                } else {
                    TextEditor(text: $text)
                        .font(.system(.footnote, design: .monospaced))
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .padding(.horizontal, 6)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .navigationTitle(entry.name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(loc("common.close")) { if text != original { confirmDiscard = true } else { dismiss() } }
                }
                ToolbarItem(placement: .confirmationAction) {
                    if saving { ProgressView() } else {
                        Button(loc("common.save")) {
                            saving = true
                            Task {
                                do {
                                    try await model.writeText(entry, text)
                                    original = text
                                    app.showToast(loc("editor.saved"))
                                    await model.refresh()
                                } catch { app.showError(error) }
                                saving = false
                            }
                        }
                        .disabled(text == original || loading)
                        .fontWeight(.semibold)
                    }
                }
            }
            .confirmationDialog(loc("editor.discardTitle"), isPresented: $confirmDiscard, titleVisibility: .visible) {
                Button(loc("editor.discard"), role: .destructive) { dismiss() }
            }
            .task {
                do {
                    text = try await model.readText(entry)
                    original = text
                } catch { self.error = describeError(error) }
                loading = false
            }
        }
        .interactiveDismissDisabled(text != original)
    }
}

struct ShareItem: Identifiable {
    let url: URL
    var id: String { url.path }
}

struct ShareSheet: UIViewControllerRepresentable {
    let items: [Any]
    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: items, applicationActivities: nil)
    }
    func updateUIViewController(_ vc: UIActivityViewController, context: Context) {}
}
