import SwiftUI
import AppKit

/// 文本文件在线编辑：独立小窗口，读入 → 编辑 → 保存回原处（本机或远端）。
@MainActor
final class TextEditorModel: ObservableObject {
    let source: FileSource
    let path: String
    let title: String
    @Published var text = ""
    @Published var original = ""
    @Published var loading = true
    @Published var saving = false
    @Published var error: String?

    init(source: FileSource, path: String, title: String) {
        self.source = source; self.path = path; self.title = title
    }

    var dirty: Bool { text != original }

    func load() {
        Task {
            do {
                text = try await source.readText(path)
                original = text
            } catch { self.error = AppModel.describe(error) }
            loading = false
        }
    }

    func save() {
        saving = true
        Task {
            do {
                try await source.writeText(path, text)
                original = text
                error = nil
            } catch { self.error = AppModel.describe(error) }
            saving = false
        }
    }
}

struct TextEditorView: View {
    @ObservedObject var model: TextEditorModel
    @ObservedObject var settings: AppSettings
    @ObservedObject var localization = Localization.shared
    @Environment(\.colorScheme) var scheme

    var body: some View {
        let p = Palettes.make(settings.palette, dark: scheme == .dark)
        let loc = localization.strings
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Image(systemName: "doc.text").foregroundStyle(p.secondaryText)
                Text(model.path).font(.system(size: 12, design: .monospaced)).foregroundStyle(p.secondaryText).lineLimit(1).truncationMode(.head)
                Spacer()
                if model.dirty { Text(loc("editor.unsaved")).font(.system(size: 11.5)).foregroundStyle(p.warning) }
                if let e = model.error { Text(e).font(.system(size: 11.5)).foregroundStyle(p.danger).lineLimit(1) }
                Button(loc("common.save")) { model.save() }
                    .buttonStyle(PrimaryButtonStyle())
                    .disabled(!model.dirty || model.saving)
                    .keyboardShortcut("s", modifiers: .command)
            }
            .padding(.horizontal, 12).frame(height: 40)
            .background(p.list)
            Rectangle().fill(p.separator).frame(height: 1)
            ZStack {
                TextEditor(text: $model.text)
                    .font(.system(size: CGFloat(settings.terminalFontSize), design: .monospaced))
                    .scrollContentBackground(.hidden)
                    .padding(8)
                    .background(p.content)
                if model.loading { ProgressView().controlSize(.small) }
            }
        }
        .environment(\.palette, p)
        .environment(\.loc, loc)
        .tint(p.accent)
        .preferredColorScheme(settings.appearance == "light" ? .light : settings.appearance == "dark" ? .dark : nil)
    }
}

@MainActor
enum TextEditorWindow {
    private static var windows: [NSWindow] = []

    static func open(source: FileSource, path: String, title: String, app: AppModel?) {
        let model = TextEditorModel(source: source, path: path, title: title)
        model.load()
        let view = TextEditorView(model: model, settings: app?.settings ?? AppSettings.shared)
        let win = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 560),
                           styleMask: [.titled, .closable, .resizable, .miniaturizable], backing: .buffered, defer: false)
        win.title = title
        win.contentView = NSHostingView(rootView: view)
        win.isReleasedWhenClosed = false
        win.center()
        win.makeKeyAndOrderFront(nil)
        windows.append(win)
        NotificationCenter.default.addObserver(forName: NSWindow.willCloseNotification, object: win, queue: .main) { _ in
            MainActor.assumeIsolated { windows.removeAll { $0 === win } }
        }
    }
}
