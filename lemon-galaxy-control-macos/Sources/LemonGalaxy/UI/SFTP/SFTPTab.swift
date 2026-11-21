import SwiftUI
import AppKit
import UniformTypeIdentifiers

/// 双栏中的一栏
@MainActor
final class FilePane: ObservableObject {
    @Published var endpoint: FileEndpoint
    @Published var path = ""
    @Published var items: [FileItem] = []
    @Published var selection = Set<String>()
    @Published var sortOrder = [KeyPathComparator(\FileItem.sortName)]
    @Published var loading = false
    @Published var error: String?
    @Published var showHidden = false
    @Published var filter = ""
    var source: FileSource?
    var homePath = ""
    var back: [String] = []
    weak var tab: SFTPTabModel?

    init(endpoint: FileEndpoint) { self.endpoint = endpoint }

    var visible: [FileItem] {
        var list = items
        if !showHidden { list = list.filter { !$0.name.hasPrefix(".") } }
        if !filter.isEmpty { list = list.filter { $0.name.localizedCaseInsensitiveContains(filter) } }
        let dirs = list.filter(\.isDir).sorted(using: sortOrder)
        let files = list.filter { !$0.isDir }.sorted(using: sortOrder)
        return dirs + files
    }

    func connect(_ ep: FileEndpoint) async {
        guard let app = tab?.app else { return }
        endpoint = ep
        loading = true
        error = nil
        items = []
        do {
            switch ep {
            case .local: source = LocalFileSource()
            case .device(let id):
                let p = try await app.peer(id)
                source = RemoteFileSource(endpoint: ep, client: p.session.files())
            case .ssh(let id):
                let s = try await app.ssh(id)
                source = RemoteFileSource(endpoint: ep, client: s.session.files())
            }
            let home = try await source!.home()
            homePath = home
            back = []
            await load(home)
        } catch {
            self.error = AppModel.describe(error)
            loading = false
        }
    }

    func load(_ p: String, pushHistory: Bool = false) async {
        guard let source else { return }
        loading = true
        do {
            let list = try await source.list(p)
            if pushHistory, !path.isEmpty { back.append(path) }
            path = p
            items = list
            selection = []
            error = nil
        } catch {
            self.error = AppModel.describe(error)
        }
        loading = false
    }

    func refresh() { Task { await load(path) } }
    func goUp() { guard let s = source else { return }; Task { await load(s.parent(path), pushHistory: true) } }
    func goBack() { guard let prev = back.popLast() else { return }; Task { await load(prev) } }
    func goHome() { guard let s = source else { return }; Task { if let h = try? await s.home() { await load(h, pushHistory: true) } } }
    func open(_ item: FileItem) {
        if item.isDir { Task { await load(item.path, pushHistory: true) } }
        else if item.looksLikeText, let s = source { TextEditorWindow.open(source: s, path: item.path, title: item.name, app: tab?.app) }
    }

    var endpointName: String {
        switch endpoint {
        case .local: return L("sftp.local")
        case .device(let id): return tab?.app?.deviceName(id) ?? id
        case .ssh(let id): return tab?.app?.sshLabel(id) ?? id
        }
    }

    var isLocal: Bool { endpoint == .local }
}

enum SFTPDialog: Identifiable {
    case newFolder(FilePane)
    case rename(FilePane, FileItem)
    case chmod(FilePane, FileItem)
    case delete(FilePane, [FileItem])
    var id: String {
        switch self {
        case .newFolder: return "new"
        case .rename(_, let i): return "rename" + i.path
        case .chmod(_, let i): return "chmod" + i.path
        case .delete(_, let l): return "del" + l.map(\.path).joined()
        }
    }
}

@MainActor
final class SFTPTabModel: TabModel {
    let left = FilePane(endpoint: .local)
    let right = FilePane(endpoint: .local)
    @Published var dialog: SFTPDialog?
    @Published var queueOpen = true

    init(app: AppModel) {
        super.init(app: app, kind: .sftp, title: L("nav.sftp"))
        left.tab = self
        right.tab = self
    }

    func start(remote: FileEndpoint?) {
        Task { await left.connect(.local) }
        if let remote {
            Task { await right.connect(remote); live = right.error == nil }
        } else {
            Task { await right.connect(.local) }
        }
    }

    func other(_ pane: FilePane) -> FilePane { pane === left ? right : left }

    /// 把 pane 里选中的条目传到另一栏的当前目录
    func transfer(_ items: [FileItem], from: FilePane) {
        let to = other(from)
        guard let app, let src = from.source, let dst = to.source else { return }
        for item in items {
            let target = dst.join(to.path, item.name)
            switch (from.source, to.source) {
            case (is LocalFileSource, let r as RemoteFileSource):
                if item.isDir { uploadDir(item.path, target, r, to) } else {
                    _ = app.transfers.start(name: item.name, direction: .upload, client: r.client, local: item.path, remote: target,
                                            endpointName: to.endpointName) { to.refresh() }
                }
            case (let r as RemoteFileSource, is LocalFileSource):
                if item.isDir { downloadDir(item.path, target, r, to) } else {
                    _ = app.transfers.start(name: item.name, direction: .download, client: r.client, local: target, remote: item.path,
                                            endpointName: from.endpointName) { to.refresh() }
                }
            case (let a as RemoteFileSource, let b as RemoteFileSource):
                // 远端到远端：先下到本机临时目录，再上传到另一端
                guard !item.isDir else { app.showToast(L("sftp.remoteDirUnsupported"), error: true); continue }
                let tmp = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + "-" + item.name).path
                _ = app.transfers.start(name: item.name, direction: .download, client: a.client, local: tmp, remote: item.path,
                                        endpointName: from.endpointName) { [weak self] in
                    guard let self, let app = self.app else { return }
                    _ = app.transfers.start(name: item.name, direction: .upload, client: b.client, local: tmp, remote: target,
                                            endpointName: to.endpointName) {
                        try? FileManager.default.removeItem(atPath: tmp)
                        to.refresh()
                    }
                }
            default:
                // 本机到本机：直接复制
                do { try FileManager.default.copyItem(atPath: item.path, toPath: target); to.refresh() } catch { app.showError(error) }
            }
        }
        _ = src
        queueOpen = true
    }

    private func uploadDir(_ local: String, _ remote: String, _ r: RemoteFileSource, _ to: FilePane) {
        guard let app else { return }
        Task {
            try? await r.mkdir(remote)
            let fm = FileManager.default
            guard let e = fm.enumerator(atPath: local) else { return }
            while let rel = e.nextObject() as? String {
                let full = (local as NSString).appendingPathComponent(rel)
                var isDir: ObjCBool = false
                fm.fileExists(atPath: full, isDirectory: &isDir)
                let dest = remote + "/" + rel
                if isDir.boolValue { try? await r.mkdir(dest) } else {
                    _ = app.transfers.start(name: rel, direction: .upload, client: r.client, local: full, remote: dest, endpointName: to.endpointName) { to.refresh() }
                }
            }
            to.refresh()
        }
    }

    private func downloadDir(_ remote: String, _ local: String, _ r: RemoteFileSource, _ to: FilePane) {
        guard let app else { return }
        Task {
            @MainActor func walk(_ rdir: String, _ ldir: String) async {
                try? FileManager.default.createDirectory(atPath: ldir, withIntermediateDirectories: true)
                guard let list = try? await r.list(rdir) else { return }
                for i in list {
                    let lpath = (ldir as NSString).appendingPathComponent(i.name)
                    if i.kind == .dir { await walk(i.path, lpath) } else {
                        _ = app.transfers.start(name: i.name, direction: .download, client: r.client, local: lpath, remote: i.path,
                                                endpointName: r.endpoint == .local ? "" : (to.tab?.other(to).endpointName ?? "")) { to.refresh() }
                    }
                }
            }
            await walk(remote, local)
            to.refresh()
        }
    }

    /// 从访达拖进来的本机文件
    func uploadFiles(_ urls: [URL], to pane: FilePane) {
        guard let app, let r = pane.source as? RemoteFileSource else {
            if let dst = pane.source as? LocalFileSource {
                for u in urls { try? FileManager.default.copyItem(atPath: u.path, toPath: dst.join(pane.path, u.lastPathComponent)) }
                pane.refresh()
            }
            return
        }
        for u in urls {
            var isDir: ObjCBool = false
            FileManager.default.fileExists(atPath: u.path, isDirectory: &isDir)
            let target = r.join(pane.path, u.lastPathComponent)
            if isDir.boolValue { uploadDir(u.path, target, r, pane) } else {
                _ = app.transfers.start(name: u.lastPathComponent, direction: .upload, client: r.client, local: u.path, remote: target,
                                        endpointName: pane.endpointName) { pane.refresh() }
            }
        }
        queueOpen = true
    }
}

// MARK: - 视图

struct SFTPTabView: View {
    @ObservedObject var tab: SFTPTabModel
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 0) {
                FilePaneView(pane: tab.left, tab: tab)
                Rectangle().fill(p.separator).frame(width: 1)
                FilePaneView(pane: tab.right, tab: tab)
            }
            TransferQueueView(open: $tab.queueOpen)
        }
        .overlay {
            if let d = tab.dialog {
                ZStack {
                    Color.black.opacity(p.isDark ? 0.4 : 0.2).onTapGesture { tab.dialog = nil }
                    SFTPDialogView(dialog: d, tab: tab)
                        .frame(width: 360)
                        .background(RoundedRectangle(cornerRadius: 12).fill(p.card).shadow(color: .black.opacity(0.25), radius: 20, y: 6))
                        .overlay(RoundedRectangle(cornerRadius: 12).stroke(p.border, lineWidth: 0.5))
                }
            }
        }
    }
}

struct FilePaneView: View {
    @ObservedObject var pane: FilePane
    @ObservedObject var tab: SFTPTabModel
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var dropTarget = false

    var body: some View {
        VStack(spacing: 0) {
            header
            Rectangle().fill(p.separator).frame(height: 1)
            ZStack {
                table
                if pane.loading && pane.items.isEmpty { ProgressView().controlSize(.small) }
                if let e = pane.error, pane.items.isEmpty {
                    VStack(spacing: 8) {
                        Image(systemName: "exclamationmark.triangle").font(.system(size: 22, weight: .light)).foregroundStyle(p.warning)
                        Text(e).font(.system(size: 12.5)).foregroundStyle(p.secondaryText).multilineTextAlignment(.center)
                        Button(loc("common.retry")) { Task { await pane.connect(pane.endpoint) } }.buttonStyle(SecondaryButtonStyle())
                    }
                    .padding(30)
                }
            }
            .overlay(RoundedRectangle(cornerRadius: 4).stroke(p.accent, lineWidth: dropTarget ? 2 : 0).padding(2))
            .onDrop(of: [UTType.plainText, UTType.fileURL], isTargeted: $dropTarget) { providers in handleDrop(providers) }
            footer
        }
        .background(p.content)
    }

    private var header: some View {
        VStack(spacing: 6) {
            HStack(spacing: 6) {
                endpointMenu
                Spacer()
                IconButton(icon: "eye", help: loc("sftp.hidden"), active: pane.showHidden, size: 11.5) { pane.showHidden.toggle() }
                IconButton(icon: "folder.badge.plus", help: loc("sftp.newFolder"), size: 12) { tab.dialog = .newFolder(pane) }
                IconButton(icon: "arrow.clockwise", help: loc("common.refresh"), size: 11.5) { pane.refresh() }
            }
            HStack(spacing: 4) {
                IconButton(icon: "chevron.left", help: loc("sftp.back"), size: 11) { pane.goBack() }.disabled(pane.back.isEmpty)
                IconButton(icon: "arrow.up", help: loc("sftp.up"), size: 11) { pane.goUp() }
                IconButton(icon: "house", help: loc("sftp.home"), size: 11) { pane.goHome() }
                PathField(pane: pane)
            }
        }
        .padding(.horizontal, 10).padding(.vertical, 8)
    }

    private var endpointMenu: some View {
        Menu {
            Button(loc("sftp.local")) { Task { await pane.connect(.local) } }
            Section(loc("hosts.devices")) {
                ForEach(app.devices.filter { !$0.isSelf && $0.kind != .mobile && ($0.online || $0.lanSeen) }, id: \.id) { d in
                    Button(app.deviceName(d.id)) { Task { await pane.connect(.device(d.id)) } }
                }
            }
            if app.sshSupported {
                Section(loc("hosts.sshHosts")) {
                    ForEach(app.vault.filter { if case .sshHost = $0.data { return true }; return false }, id: \.id) { e in
                        Button(app.sshLabel(e.id)) { Task { await pane.connect(.ssh(e.id)) } }
                    }
                }
            }
        } label: {
            HStack(spacing: 6) {
                Image(systemName: pane.isLocal ? "laptopcomputer" : (pane.endpoint == .local ? "server.rack" : "server.rack"))
                    .font(.system(size: 12))
                Text(pane.endpointName).font(.system(size: 12.5, weight: .semibold)).lineLimit(1)
                Image(systemName: "chevron.down").font(.system(size: 8.5, weight: .bold)).foregroundStyle(p.tertiaryText)
            }
        }
        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
    }

    private var table: some View {
        Table(of: FileItem.self, selection: $pane.selection, sortOrder: $pane.sortOrder) {
            TableColumn(loc("sftp.name"), value: \.sortName) { item in
                HStack(spacing: 7) {
                    Image(systemName: icon(item)).font(.system(size: 12)).foregroundStyle(item.isDir ? p.accent : p.secondaryText).frame(width: 16)
                    Text(item.name).font(.system(size: 12.5)).lineLimit(1)
                    if let t = item.linkTarget { Text("→ " + t).font(.system(size: 11)).foregroundStyle(p.tertiaryText).lineLimit(1) }
                }
            }
            TableColumn(loc("sftp.size"), value: \.sortSize) { item in
                Text(item.isDir ? "—" : Fmt.bytes(item.size)).font(.system(size: 12).monospacedDigit()).foregroundStyle(p.secondaryText)
            }
            .width(min: 56, ideal: 70, max: 110)
            TableColumn(loc("sftp.modified"), value: \.modified) { item in
                Text(Fmt.shortDate(item.modified, zh: loc.isChinese)).font(.system(size: 12).monospacedDigit()).foregroundStyle(p.secondaryText)
            }
            .width(min: 76, ideal: 92, max: 140)
            TableColumn(loc("sftp.perms"), value: \.mode) { item in
                Text(item.modeString).font(.system(size: 11.5, design: .monospaced)).foregroundStyle(p.secondaryText)
            }
            .width(min: 70, ideal: 82, max: 110)
        } rows: {
            ForEach(pane.visible) { item in
                TableRow(item).itemProvider { dragProvider(item) }
            }
        }
        .tableStyle(.inset(alternatesRowBackgrounds: false))
        .scrollContentBackground(.hidden)
        .contextMenu(forSelectionType: String.self) { ids in
            contextMenu(ids)
        } primaryAction: { ids in
            if let id = ids.first, let item = pane.items.first(where: { $0.path == id }) { pane.open(item) }
        }
    }

    private func icon(_ i: FileItem) -> String {
        if i.isDir { return "folder.fill" }
        if i.kind == .symlink { return "arrow.turn.up.right" }
        let ext = (i.name as NSString).pathExtension.lowercased()
        switch ext {
        case "png", "jpg", "jpeg", "gif", "webp", "heic": return "photo"
        case "zip", "gz", "tgz", "xz", "bz2", "7z", "tar": return "doc.zipper"
        case "sh", "py", "js", "ts", "rs", "go", "swift", "c", "cpp": return "chevron.left.forwardslash.chevron.right"
        case "exe", "bin", "app", "dmg": return "gearshape"
        default: return i.looksLikeText ? "doc.text" : "doc"
        }
    }

    private func dragProvider(_ item: FileItem) -> NSItemProvider {
        let ids = pane.selection.contains(item.path) ? Array(pane.selection) : [item.path]
        let side = pane === tab.left ? "L" : "R"
        return NSItemProvider(object: ("lgpane:" + side + "\n" + ids.joined(separator: "\n")) as NSString)
    }

    private func handleDrop(_ providers: [NSItemProvider]) -> Bool {
        var handled = false
        for prov in providers {
            if prov.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) {
                handled = true
                _ = prov.loadObject(ofClass: URL.self) { url, _ in
                    guard let url else { return }
                    DispatchQueue.main.async { tab.uploadFiles([url], to: pane) }
                }
            } else if prov.canLoadObject(ofClass: NSString.self) {
                handled = true
                _ = prov.loadObject(ofClass: NSString.self) { obj, _ in
                    guard let s = obj as? String, s.hasPrefix("lgpane:") else { return }
                    let lines = s.split(separator: "\n").map(String.init)
                    let side = lines[0].dropFirst("lgpane:".count)
                    DispatchQueue.main.async {
                        let from = side == "L" ? tab.left : tab.right
                        guard from !== pane else { return }
                        let items = from.items.filter { lines.dropFirst().contains($0.path) }
                        tab.transfer(items, from: from)
                    }
                }
            }
        }
        return handled
    }

    @ViewBuilder
    private func contextMenu(_ ids: Set<String>) -> some View {
        let items = pane.items.filter { ids.contains($0.path) }
        if let one = items.first, items.count == 1 {
            Button(one.isDir ? loc("sftp.open") : loc("sftp.edit")) { pane.open(one) }.disabled(!one.isDir && !one.looksLikeText)
        }
        if !items.isEmpty {
            Button(pane.isLocal ? loc("sftp.uploadTo", tab.other(pane).endpointName) : loc("sftp.transferTo", tab.other(pane).endpointName)) {
                tab.transfer(items, from: pane)
            }
            Divider()
        }
        if let one = items.first, items.count == 1 {
            Button(loc("sftp.rename")) { tab.dialog = .rename(pane, one) }
            Button(loc("sftp.chmod")) { tab.dialog = .chmod(pane, one) }
            Button(loc("sftp.copyPath")) {
                NSPasteboard.general.clearContents(); NSPasteboard.general.setString(one.path, forType: .string)
            }
        }
        if !items.isEmpty {
            Button(loc("common.delete"), role: .destructive) { tab.dialog = .delete(pane, items) }
            Divider()
        }
        Button(loc("sftp.newFolder")) { tab.dialog = .newFolder(pane) }
        Button(loc("common.refresh")) { pane.refresh() }
    }

    private var footer: some View {
        HStack {
            Text(loc("sftp.count", pane.visible.count)).font(.system(size: 11)).foregroundStyle(p.tertiaryText)
            if !pane.selection.isEmpty {
                let size = pane.items.filter { pane.selection.contains($0.path) && !$0.isDir }.reduce(UInt64(0)) { $0 + $1.size }
                Text(loc("sftp.selected", pane.selection.count, Fmt.bytes(size))).font(.system(size: 11)).foregroundStyle(p.tertiaryText)
            }
            Spacer()
            if pane.loading { ProgressView().controlSize(.mini) }
        }
        .padding(.horizontal, 10).frame(height: 24)
        .overlay(Rectangle().fill(p.separator).frame(height: 1), alignment: .top)
    }
}

struct PathField: View {
    @ObservedObject var pane: FilePane
    @State private var editing = false
    @State private var text = ""
    @Environment(\.palette) var p

    /// 本机主目录显示成 ~
    private var displayPath: String {
        guard pane.isLocal, !pane.homePath.isEmpty, pane.path.hasPrefix(pane.homePath) else { return pane.path }
        return "~" + pane.path.dropFirst(pane.homePath.count)
    }
    var body: some View {
        Group {
            if editing {
                TextField("", text: $text)
                    .textFieldStyle(.plain)
                    .font(.system(size: 12, design: .monospaced))
                    .onSubmit { editing = false; Task { await pane.load(text, pushHistory: true) } }
                    .onExitCommand { editing = false }
            } else {
                Text(displayPath).font(.system(size: 12, design: .monospaced)).foregroundStyle(p.secondaryText)
                    .lineLimit(1).truncationMode(.head)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
                    .onTapGesture { text = pane.path; editing = true }
            }
        }
        .padding(.horizontal, 8).frame(height: 24)
        .background(RoundedRectangle(cornerRadius: 6).fill(p.elevated))
        .overlay(RoundedRectangle(cornerRadius: 6).stroke(p.border, lineWidth: 0.5))
    }
}

struct SFTPDialogView: View {
    let dialog: SFTPDialog
    @ObservedObject var tab: SFTPTabModel
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var text = ""
    @State private var mode: UInt32 = 0o644

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            switch dialog {
            case .newFolder(let pane):
                Text(loc("sftp.newFolder")).font(.system(size: 14, weight: .semibold))
                TextField(loc("sftp.folderName"), text: $text).fieldStyle().onSubmit { run(pane) }
                buttons(loc("common.create")) { run(pane) }
            case .rename(let pane, let item):
                Text(loc("sftp.rename")).font(.system(size: 14, weight: .semibold))
                TextField("", text: $text).fieldStyle().onAppear { text = item.name }.onSubmit { run(pane) }
                buttons(loc("sftp.rename")) { run(pane) }
            case .chmod(let pane, let item):
                Text(loc("sftp.chmod.title", item.name)).font(.system(size: 14, weight: .semibold)).lineLimit(1)
                ChmodGrid(mode: $mode).onAppear { mode = item.mode & 0o777 }
                HStack(spacing: 8) {
                    Text(loc("sftp.chmod.octal")).font(.system(size: 12)).foregroundStyle(p.secondaryText)
                    TextField("", text: Binding(get: { String(mode, radix: 8) },
                                                set: { if let v = UInt32($0, radix: 8), v <= 0o777 { mode = v } }))
                        .fieldStyle().frame(width: 70).font(.system(size: 13, design: .monospaced))
                    Text(Fmt.mode(mode)).font(.system(size: 12.5, design: .monospaced)).foregroundStyle(p.secondaryText)
                }
                buttons(loc("common.apply")) { run(pane) }
            case .delete(let pane, let items):
                Text(loc("sftp.delete.title")).font(.system(size: 14, weight: .semibold))
                Text(items.count == 1 ? loc("sftp.delete.one", items[0].name) : loc("sftp.delete.many", items.count))
                    .font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                if !pane.isLocal {
                    Text(loc("sftp.delete.irreversible")).font(.system(size: 12)).foregroundStyle(p.danger)
                }
                buttons(loc("common.delete"), destructive: true) { run(pane) }
            }
        }
        .padding(18)
    }

    private func buttons(_ title: String, destructive: Bool = false, _ action: @escaping () -> Void) -> some View {
        HStack {
            Spacer()
            Button(loc("common.cancel")) { tab.dialog = nil }.buttonStyle(SecondaryButtonStyle()).keyboardShortcut(.cancelAction)
            Button(title, action: action).buttonStyle(PrimaryButtonStyle(destructive: destructive)).keyboardShortcut(.defaultAction)
        }
    }

    private func run(_ pane: FilePane) {
        guard let s = pane.source else { return }
        let d = dialog
        tab.dialog = nil
        Task {
            do {
                switch d {
                case .newFolder: try await s.mkdir(s.join(pane.path, text))
                case .rename(_, let item): try await s.rename(item.path, s.join(s.parent(item.path), text))
                case .chmod(_, let item): try await s.chmod(item.path, mode: mode)
                case .delete(_, let items): for i in items { try await s.remove(i.path, recursive: true) }
                }
                pane.refresh()
            } catch {
                app.showError(error)
            }
        }
    }
}

/// 权限勾选：所有者 / 组 / 其他 × 读 / 写 / 执行
struct ChmodGrid: View {
    @Binding var mode: UInt32
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    var body: some View {
        Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 8) {
            GridRow {
                Text("")
                Text(loc("sftp.chmod.read")).font(.system(size: 11.5)).foregroundStyle(p.secondaryText)
                Text(loc("sftp.chmod.write")).font(.system(size: 11.5)).foregroundStyle(p.secondaryText)
                Text(loc("sftp.chmod.exec")).font(.system(size: 11.5)).foregroundStyle(p.secondaryText)
            }
            ForEach(0..<3) { who in
                GridRow {
                    Text(loc(["sftp.chmod.owner", "sftp.chmod.group", "sftp.chmod.other"][who])).font(.system(size: 12.5))
                    ForEach(0..<3) { bit in
                        let mask: UInt32 = 1 << UInt32((2 - who) * 3 + (2 - bit))
                        Toggle("", isOn: Binding(get: { mode & mask != 0 }, set: { on in mode = on ? (mode | mask) : (mode & ~mask) }))
                            .toggleStyle(.checkbox).labelsHidden()
                    }
                }
            }
        }
    }
}

struct TransferQueueView: View {
    @Binding var open: Bool
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        TransferQueueBody(queue: app.transfers, open: $open)
    }
}

struct TransferQueueBody: View {
    @ObservedObject var queue: TransferQueue
    @Binding var open: Bool
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Button { open.toggle() } label: {
                    HStack(spacing: 6) {
                        Image(systemName: "chevron.right").font(.system(size: 9, weight: .bold)).rotationEffect(.degrees(open ? 90 : 0))
                        Text(loc("sftp.queue")).font(.system(size: 12, weight: .semibold))
                        if queue.activeCount > 0 {
                            Text("\(queue.activeCount)").font(.system(size: 10.5, weight: .semibold)).foregroundStyle(p.accentText)
                                .padding(.horizontal, 5).background(Capsule().fill(p.accent))
                        }
                    }
                }
                .buttonStyle(.plain)
                Spacer()
                if open && !queue.items.isEmpty {
                    Button(loc("sftp.queue.clear")) { queue.clearFinished() }.buttonStyle(.plain).font(.system(size: 11.5)).foregroundStyle(p.secondaryText)
                }
            }
            .padding(.horizontal, 12).frame(height: 30)
            if open {
                Rectangle().fill(p.separator).frame(height: 1)
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(queue.items) { item in TransferRow(item: item, queue: queue) }
                        if queue.items.isEmpty {
                            Text(loc("sftp.queue.empty")).font(.system(size: 12)).foregroundStyle(p.tertiaryText).padding(14)
                        }
                    }
                }
                .frame(height: 132)
            }
        }
        .background(p.list)
        .overlay(Rectangle().fill(p.separator).frame(height: 1), alignment: .top)
    }
}

struct TransferRow: View {
    @ObservedObject var item: TransferItem
    let queue: TransferQueue
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: item.direction == .upload ? "arrow.up.circle" : "arrow.down.circle")
                .font(.system(size: 14)).foregroundStyle(stateColor)
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text(item.name).font(.system(size: 12, weight: .medium)).lineLimit(1)
                    Text(item.direction == .upload ? loc("sftp.to", item.endpointName) : loc("sftp.from", item.endpointName))
                        .font(.system(size: 11)).foregroundStyle(p.tertiaryText).lineLimit(1)
                    Spacer()
                    Text(statusText).font(.system(size: 11).monospacedDigit()).foregroundStyle(stateColor).lineLimit(1)
                }
                GeometryReader { g in
                    ZStack(alignment: .leading) {
                        Capsule().fill(p.border.opacity(0.6))
                        Capsule().fill(stateColor).frame(width: max(0, g.size.width * item.fraction))
                    }
                }
                .frame(height: 4)
            }
            HStack(spacing: 2) {
                switch item.state {
                case .running, .queued:
                    IconButton(icon: "pause.fill", help: loc("sftp.pause"), size: 10) { queue.pause(item) }
                    IconButton(icon: "xmark", help: loc("common.cancel"), size: 10) { queue.cancel(item) }
                case .paused:
                    IconButton(icon: "play.fill", help: loc("sftp.resume"), size: 10) { queue.resume(item) }
                    IconButton(icon: "xmark", help: loc("common.cancel"), size: 10) { queue.cancel(item) }
                case .failed, .cancelled:
                    IconButton(icon: "arrow.clockwise", help: loc("common.retry"), size: 10) { queue.retry(item) }
                case .done:
                    EmptyView()
                }
            }
            .frame(width: 56, alignment: .trailing)
        }
        .padding(.horizontal, 12).padding(.vertical, 6)
    }

    private var stateColor: Color {
        switch item.state {
        case .failed: return p.danger
        case .done: return p.success
        case .paused, .cancelled: return p.tertiaryText
        default: return p.accent
        }
    }

    private var statusText: String {
        switch item.state {
        case .queued: return loc("sftp.state.queued")
        case .running: return "\(Fmt.bytes(item.done)) / \(Fmt.bytes(item.total))" + (item.speed > 0 ? " · \(Fmt.bytes(UInt64(item.speed)))/s" : "")
        case .paused: return loc("sftp.state.paused")
        case .done: return loc("sftp.state.done")
        case .failed(let e): return e
        case .cancelled: return loc("sftp.state.cancelled")
        }
    }
}
