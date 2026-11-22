import SwiftUI
import AppKit

/// 「+」菜单打开的选择面板：设备或主机、文件或文件夹、命令片段、端口映射（docs/ai-chat-ui.md 第 2.2 节）
struct RefPickerPanel: View {
    let kind: RefPickerKind
    @ObservedObject var conv: AIConversationModel
    let close: () -> Void
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var ai: AIHubModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var filter = ""
    @State private var chosen: [String: ContextRef] = [:]
    @StateObject private var browser = RefFileBrowser()

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                if kind == .files, browser.target != nil {
                    IconButton(icon: "chevron.left", help: loc("common.back")) { browser.reset() }
                }
                Text(title).font(.system(size: 14, weight: .semibold))
                Spacer()
                IconButton(icon: "xmark", help: loc("common.close")) { close() }
            }
            .padding(.horizontal, 16).frame(height: 46)
            Rectangle().fill(p.separator).frame(height: 1)
            if kind == .files, browser.target != nil {
                FileBrowserBody(browser: browser)
            } else {
                SearchField(placeholder: loc("ai.picker.search"), text: $filter).padding(.horizontal, 14).padding(.vertical, 10)
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 1) {
                        let list = entries
                        ForEach(list, id: \.ref.key) { e in row(e) }
                        if list.isEmpty {
                            Text(loc("ai.picker.empty")).font(.system(size: 12)).foregroundStyle(p.tertiaryText)
                                .frame(maxWidth: .infinity).padding(.vertical, 30)
                        }
                    }
                    .padding(.horizontal, 8).padding(.bottom, 8)
                }
            }
            Rectangle().fill(p.separator).frame(height: 1)
            footer.padding(.horizontal, 14).frame(height: 52)
        }
        .frame(width: 480, height: 500)
        .background(RoundedRectangle(cornerRadius: 12).fill(p.card).shadow(color: .black.opacity(0.25), radius: 20, y: 6))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(p.border, lineWidth: 0.5))
        .onAppear { browser.app = app }
    }

    private var title: String {
        switch kind {
        case .targets: return loc("ai.ref.targets")
        case .files: return browser.target.map { ai.targetName($0.id) } ?? loc("ai.picker.chooseTarget")
        case .snippets: return loc("ai.ref.snippets")
        case .forwards: return loc("ai.ref.forwards")
        }
    }

    struct Entry {
        let ref: ContextRef
        let platform: Platform?
        let online: Bool
        let detail: String
        let enabled: Bool
    }

    private var entries: [Entry] {
        let q = filter.trimmingCharacters(in: .whitespaces)
        switch kind {
        case .targets, .files:
            return ai.candidates(q).map { c in
                // 浏览文件要先连上设备：离线设备不能选
                Entry(ref: c.ref, platform: c.platform, online: c.online, detail: c.ref.kind == .sshHost ? "SSH" : "",
                      enabled: kind == .targets || c.online)
            }
        case .snippets:
            return app.snippets.compactMap { e -> Entry? in
                guard case .snippet(let label, let command, _) = e.data else { return nil }
                guard q.isEmpty || label.localizedCaseInsensitiveContains(q) || command.localizedCaseInsensitiveContains(q) else { return nil }
                return Entry(ref: ContextRef(kind: .snippet, targetId: "", id: e.id, label: label), platform: nil, online: true, detail: command, enabled: true)
            }
        case .forwards:
            return app.forwardRules.compactMap { r -> Entry? in
                guard q.isEmpty || r.label.localizedCaseInsensitiveContains(q) || r.target.localizedCaseInsensitiveContains(q) else { return nil }
                return Entry(ref: ContextRef(kind: .portForward, targetId: "", id: r.id, label: r.label), platform: nil, online: true, detail: r.target, enabled: true)
            }
        }
    }

    private func row(_ e: Entry) -> some View {
        let sel = chosen[e.ref.key] != nil || conv.refs.contains { $0.key == e.ref.key }
        return Button {
            if kind == .files {
                browser.open(e.ref)
            } else if chosen[e.ref.key] != nil {
                chosen[e.ref.key] = nil
            } else if !conv.refs.contains(where: { $0.key == e.ref.key }) {
                chosen[e.ref.key] = e.ref
            }
        } label: {
            HStack(spacing: 10) {
                Group {
                    if let pf = e.platform { PlatformGlyph(platform: pf, size: 12) } else { Image(systemName: e.ref.kind.icon).font(.system(size: 12)) }
                }
                .foregroundStyle(p.secondaryText).frame(width: 18)
                VStack(alignment: .leading, spacing: 2) {
                    Text(e.ref.label).font(.system(size: 12.5)).lineLimit(1)
                    if !e.detail.isEmpty && e.ref.kind != .sshHost {
                        Text(e.detail).font(.system(size: 11, design: .monospaced)).foregroundStyle(p.tertiaryText).lineLimit(1)
                    }
                }
                if e.ref.kind == .sshHost { TagChip(text: "SSH") }
                Spacer(minLength: 6)
                if e.ref.kind == .device { StatusDot(state: e.online ? .online : .offline, size: 7) }
                if kind == .files {
                    Image(systemName: "chevron.right").font(.system(size: 9, weight: .semibold)).foregroundStyle(p.tertiaryText)
                } else {
                    Image(systemName: sel ? "checkmark.circle.fill" : "circle").font(.system(size: 14))
                        .foregroundStyle(sel ? p.accent : p.tertiaryText)
                }
            }
            .padding(.horizontal, 8).frame(minHeight: 36)
            .contentShape(Rectangle())
            .opacity(e.enabled ? 1 : 0.45)
        }
        .buttonStyle(.plain)
        .disabled(!e.enabled)
    }

    @ViewBuilder private var footer: some View {
        HStack(spacing: 8) {
            if kind == .files, browser.target != nil {
                if let e = browser.error { Text(e).font(.system(size: 11.5)).foregroundStyle(p.danger).lineLimit(2) }
                Spacer()
                Button(loc("ai.picker.pickFolder")) { pick(browser.folderRef()) }
                    .buttonStyle(SecondaryButtonStyle())
                    .disabled(browser.path.isEmpty || browser.loading)
                Button(loc("ai.picker.pickFile")) { if let r = browser.fileRef() { pick(r) } }
                    .buttonStyle(PrimaryButtonStyle())
                    .disabled(browser.selected == nil)
            } else if kind == .files {
                Spacer()
                Button(loc("common.cancel"), action: close).buttonStyle(SecondaryButtonStyle()).keyboardShortcut(.cancelAction)
            } else {
                Spacer()
                Button(loc("common.cancel"), action: close).buttonStyle(SecondaryButtonStyle()).keyboardShortcut(.cancelAction)
                Button(chosen.isEmpty ? loc("ai.picker.add") : loc("ai.picker.addN", chosen.count)) {
                    // 按列表顺序加入
                    for e in entries where chosen[e.ref.key] != nil { conv.addRef(e.ref) }
                    conv.focusToken += 1
                    close()
                }
                .buttonStyle(PrimaryButtonStyle())
                .disabled(chosen.isEmpty)
                .keyboardShortcut(.defaultAction)
            }
        }
    }

    private func pick(_ r: ContextRef?) {
        guard let r else { return }
        conv.addRef(r)
        conv.focusToken += 1
        close()
    }
}

/// 文件或文件夹选择：复用文件管理的数据来源（设备文件通道 / SSH 的 SFTP）
@MainActor
final class RefFileBrowser: ObservableObject {
    weak var app: AppModel?
    @Published var target: ContextRef?
    @Published var path = ""
    @Published var pathField = ""
    @Published var items: [FileItem] = []
    @Published var loading = false
    @Published var error: String?
    @Published var selected: FileItem?
    @Published var showHidden = false
    private var source: FileSource?

    func reset() {
        target = nil
        source = nil
        path = ""
        items = []
        selected = nil
        error = nil
    }

    func open(_ t: ContextRef) {
        guard let app else { return }
        target = t
        loading = true
        error = nil
        Task {
            do {
                let s: FileSource
                if t.id == AIHubModel.localId {
                    // 本机：直接浏览这台电脑的文件，路径就是内核要的 POSIX 路径
                    s = LocalFileSource()
                } else if t.kind == .device {
                    let peer = try await app.peer(t.id)
                    s = RemoteFileSource(endpoint: .device(t.id), client: peer.session.files())
                } else {
                    let ssh = try await app.ssh(t.id)
                    s = RemoteFileSource(endpoint: .ssh(t.id), client: ssh.session.files())
                }
                source = s
                let home = try await s.home()
                await load(home)
            } catch {
                self.error = AppModel.describe(error)
                loading = false
            }
        }
    }

    func load(_ p: String) async {
        guard let source else { return }
        loading = true
        do {
            let list = try await source.list(p)
            path = p
            pathField = p
            items = list.sorted { $0.sortName < $1.sortName }
            selected = nil
            error = nil
        } catch {
            self.error = AppModel.describe(error)
            pathField = path
        }
        loading = false
    }

    func go(_ p: String) { Task { await load(p) } }

    func up() {
        guard let source, !path.isEmpty, path != "/" else { return }
        go(source.parent(path))
    }

    var visible: [FileItem] { showHidden ? items : items.filter { !$0.name.hasPrefix(".") } }

    func fileRef() -> ContextRef? {
        guard let t = target, let f = selected, !f.isDir else { return nil }
        return ContextRef(kind: .file, targetId: t.id, id: f.path, label: f.name)
    }

    func folderRef() -> ContextRef? {
        guard let t = target, !path.isEmpty else { return nil }
        let name = (path as NSString).lastPathComponent
        return ContextRef(kind: .folder, targetId: t.id, id: path, label: name.isEmpty ? path : name)
    }
}

struct FileBrowserBody: View {
    @ObservedObject var browser: RefFileBrowser
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                IconButton(icon: "arrow.up", help: loc("ai.picker.up")) { browser.up() }
                    .disabled(browser.path.isEmpty || browser.path == "/")
                TextField("", text: $browser.pathField).fieldStyle().font(.system(size: 12, design: .monospaced))
                    .onSubmit { browser.go(browser.pathField) }
                IconButton(icon: browser.showHidden ? "eye" : "eye.slash", help: loc("sftp.hidden"), active: browser.showHidden) { browser.showHidden.toggle() }
            }
            .padding(.horizontal, 12).padding(.vertical, 8)
            ZStack {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 1) {
                        ForEach(browser.visible) { f in
                            let sel = browser.selected?.path == f.path
                            Button {
                                if f.isDir { browser.go(f.path) } else { browser.selected = f }
                            } label: {
                                HStack(spacing: 9) {
                                    Image(systemName: f.isDir ? "folder.fill" : "doc").font(.system(size: 12))
                                        .foregroundStyle(f.isDir ? p.accent : p.secondaryText).frame(width: 18)
                                    Text(f.name).font(.system(size: 12.5)).lineLimit(1).truncationMode(.middle)
                                    Spacer(minLength: 6)
                                    if !f.isDir { Text(Fmt.bytes(f.size)).font(.system(size: 11).monospacedDigit()).foregroundStyle(p.tertiaryText) }
                                    if f.isDir { Image(systemName: "chevron.right").font(.system(size: 9, weight: .semibold)).foregroundStyle(p.tertiaryText) }
                                }
                                .padding(.horizontal, 8).frame(height: 28)
                                .background(RoundedRectangle(cornerRadius: 6).fill(sel ? p.selection : .clear))
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .padding(.horizontal, 8).padding(.bottom, 8)
                }
                if browser.loading { ProgressView().controlSize(.small) }
                else if browser.visible.isEmpty && browser.error == nil {
                    Text(loc("ai.picker.emptyFolder")).font(.system(size: 12)).foregroundStyle(p.tertiaryText)
                }
            }
        }
    }
}
