import SwiftUI
import AppKit
import Combine
import SwiftTerm

/// 终端标签：SwiftTerm 负责仿真与渲染，原始字节经 TerminalChannel / TerminalSink 与内核往来。
@MainActor
final class TerminalTabModel: TabModel, TerminalViewDelegate {
    enum Target: Equatable { case device(String), ssh(String) }
    enum Phase: Equatable { case connecting, running, ended(String) }

    let target: Target
    let terminalView: TerminalView
    @Published var phase: Phase = .connecting
    @Published var peer: PeerConnection?
    @Published var sessionTitle: String = ""
    /// 只给 selftest 用：离屏截图时把终端单独渲染成图片（共享上下文里 SwiftTerm 的文字矩阵、背景混合模式会出错）
    @Published var snapshot: NSImage?
    private var channel: TerminalChannel?
    private var sink: TerminalSinkAdapter?
    private var attach: TerminalSessionInfo?
    private var cancellables: Set<AnyCancellable> = []
    private var themeOverride: String?

    init(app: AppModel, target: Target, title: String, attach: TerminalSessionInfo?) {
        self.target = target
        self.attach = attach
        let settings = app.settings
        terminalView = TerminalView(frame: NSRect(x: 0, y: 0, width: 800, height: 500),
                                    font: TerminalFonts.font(family: settings.terminalFont, size: settings.terminalFontSize))
        super.init(app: app, kind: .terminal, title: title)
        if case .device(let d) = target { deviceId = d }
        if case .ssh(let h) = target, let e = app.entry(h), case .sshHost(_, _, _, _, _, _, _, _, _, _, let theme) = e.data { themeOverride = theme }
        terminalView.terminalDelegate = self
        terminalView.optionAsMetaKey = settings.terminalOptionAsMeta
        applyAppearance()
        // 设置变化即时生效
        settings.objectWillChange.receive(on: DispatchQueue.main).sink { [weak self] _ in
            DispatchQueue.main.async { self?.applyAppearance() }
        }.store(in: &cancellables)
    }

    var theme: TerminalTheme { TerminalTheme.named(themeOverride ?? app?.settings.terminalTheme) }

    func applyAppearance() {
        guard let s = app?.settings else { return }
        let f = TerminalFonts.font(family: s.terminalFont, size: s.terminalFontSize)
        if terminalView.font != f { terminalView.font = f }
        theme.apply(to: terminalView)
        terminalView.optionAsMetaKey = s.terminalOptionAsMeta
        terminalView.getTerminal().setCursorStyle(s.terminalCursorBlink ? .blinkBlock : .steadyBlock)
        terminalView.needsDisplay = true
    }

    func start() {
        phase = .connecting
        live = nil
        let t = terminalView.getTerminal()
        let cols = UInt16(max(t.cols, 20)), rows = UInt16(max(t.rows, 5))
        let adapter = TerminalSinkAdapter(owner: self)
        sink = adapter
        Task {
            guard let app else { return }
            do {
                switch target {
                case .device(let id):
                    let p = try await app.peer(id)
                    peer = p
                    let ch = try await p.session.openTerminal(options: TerminalOptions(attachSession: attach?.id, shell: nil, cols: cols, rows: rows), sink: adapter)
                    channel = ch
                    sessionTitle = attach?.title ?? ""
                case .ssh(let id):
                    let s = try await app.ssh(id)
                    channel = try await s.session.openTerminal(cols: cols, rows: rows, sink: adapter)
                }
                phase = .running
                live = true
            } catch {
                phase = .ended(AppModel.describe(error))
                live = false
            }
        }
    }

    func reconnect(attachTo info: TerminalSessionInfo? = nil) {
        channel?.detach()
        channel = nil
        attach = info
        terminalView.feed(text: "\r\n")
        start()
    }

    func received(_ data: Data) {
        terminalView.feed(byteArray: ArraySlice(data))
    }

    func exited(_ code: Int32?) {
        phase = .ended(code.map { L("terminal.exited.code", $0) } ?? L("terminal.exited"))
        live = false
        channel = nil
    }

    func makeSnapshot() {
        let v = terminalView
        guard let rep = v.bitmapImageRepForCachingDisplay(in: v.bounds) else { return }
        v.cacheDisplay(in: v.bounds, to: rep)
        let img = NSImage(size: v.bounds.size)
        img.addRepresentation(rep)
        snapshot = img
    }

    func sendText(_ s: String) {
        channel?.write(data: Data(s.utf8))
    }

    override func close() {
        // 关闭标签只断开，被控端保留会话，下次可以接续
        channel?.detach()
        channel = nil
        cancellables.removeAll()
    }

    func killSession() {
        channel?.kill()
        channel = nil
        phase = .ended(L("terminal.killed"))
        live = false
    }

    // MARK: TerminalViewDelegate

    nonisolated func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
        Task { @MainActor in self.channel?.resize(cols: UInt16(newCols), rows: UInt16(newRows)) }
    }

    nonisolated func setTerminalTitle(source: TerminalView, title: String) {
        Task { @MainActor in self.sessionTitle = title }
    }

    nonisolated func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}

    nonisolated func send(source: TerminalView, data: ArraySlice<UInt8>) {
        let d = Data(data)
        Task { @MainActor in self.channel?.write(data: d) }
    }

    nonisolated func scrolled(source: TerminalView, position: Double) {}

    nonisolated func requestOpenLink(source: TerminalView, link: String, params: [String: String]) {
        if let url = URL(string: link), ["http", "https"].contains(url.scheme ?? "") {
            DispatchQueue.main.async { NSWorkspace.shared.open(url) }
        }
    }

    nonisolated func bell(source: TerminalView) { DispatchQueue.main.async { NSSound.beep() } }

    nonisolated func clipboardCopy(source: TerminalView, content: Data) {
        guard let s = String(data: content, encoding: .utf8) else { return }
        DispatchQueue.main.async {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(s, forType: .string)
        }
    }

    nonisolated func iTermContent(source: TerminalView, content: ArraySlice<UInt8>) {}
    nonisolated func rangeChanged(source: TerminalView, startY: Int, endY: Int) {}
}

/// 内核线程 → 主线程；连续到来的输出在主线程合并后一次喂给终端。
final class TerminalSinkAdapter: TerminalSink {
    private weak var owner: TerminalTabModel?
    private var pending = Data()
    private var scheduled = false
    private let lock = NSLock()

    init(owner: TerminalTabModel) { self.owner = owner }

    func onOutput(data: Data) {
        lock.lock()
        pending.append(data)
        let schedule = !scheduled
        scheduled = true
        lock.unlock()
        guard schedule else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.lock.lock()
            let d = self.pending
            self.pending = Data()
            self.scheduled = false
            self.lock.unlock()
            MainActor.assumeIsolated { self.owner?.received(d) }
        }
    }

    func onExit(code: Int32?) {
        DispatchQueue.main.async { [weak self] in MainActor.assumeIsolated { self?.owner?.exited(code) } }
    }
}

// MARK: - 视图

struct TerminalTabView: View {
    @ObservedObject var tab: TerminalTabModel
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(spacing: 0) {
            TerminalHeader(tab: tab)
            ZStack {
                Rectangle().fill(tab.theme.bgColor)
                TerminalHost(view: tab.terminalView)
                    .padding(.leading, 10).padding(.top, 6).padding(.bottom, 4)
                    .opacity(tab.snapshot == nil ? 1 : 0)
                if let img = tab.snapshot {
                    Image(nsImage: img).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                        .padding(.leading, 10).padding(.top, 6).padding(.bottom, 4)
                }
                if case .connecting = tab.phase {
                    ProgressView().controlSize(.small)
                }
            }
            if case .ended(let reason) = tab.phase {
                endedBar(reason)
            }
        }
    }

    private func endedBar(_ reason: String) -> some View {
        HStack(spacing: 10) {
            Image(systemName: "bolt.horizontal.circle").foregroundStyle(p.warning)
            Text(reason).foregroundStyle(p.text).lineLimit(1)
            Spacer()
            if case .device(let id) = tab.target {
                Button(loc("terminal.reattach")) {
                    Task {
                        guard let peer = try? await app.peer(id), let list = try? await peer.session.listTerminals() else { return }
                        app.modal = .terminalSessions(deviceId: id, sessions: list.filter { !$0.attached })
                    }
                }
            }
            Button(loc("terminal.reconnect")) { tab.reconnect() }.buttonStyle(.borderedProminent)
        }
        .padding(.horizontal, 14).padding(.vertical, 8)
        .background(p.card)
        .overlay(Rectangle().fill(p.border).frame(height: 1), alignment: .top)
    }
}

/// 终端顶栏：目标、连接路径（局域网 / 直连 / 中转）与延迟、片段、会话菜单。
struct TerminalHeader: View {
    @ObservedObject var tab: TerminalTabModel
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        HStack(spacing: 10) {
            PlatformGlyph(platform: platform, size: 13).foregroundStyle(p.secondaryText)
            Text(tab.title).font(.system(size: 12.5, weight: .semibold)).foregroundStyle(p.text).lineLimit(1)
            if !tab.sessionTitle.isEmpty {
                Text(tab.sessionTitle).font(.system(size: 12)).foregroundStyle(p.tertiaryText).lineLimit(1)
            }
            Spacer(minLength: 8)
            if let peer = tab.peer {
                PathBadge(peer: peer)
            } else if case .ssh(let id) = tab.target {
                SshPathBadge(hostId: id)
            }
            Menu {
                ForEach(app.snippets, id: \.id) { s in
                    if case .snippet(let label, let command, _) = s.data {
                        Button(label) { tab.sendText(command + "\r") }
                    }
                }
            } label: {
                Image(systemName: "chevron.left.forwardslash.chevron.right")
            }
            .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
            .help(loc("terminal.snippets"))
            Menu {
                Button(loc("terminal.copy")) { tab.terminalView.copy(self) }
                Button(loc("terminal.paste")) { tab.terminalView.paste(self) }
                Divider()
                if case .device(let id) = tab.target {
                    Button(loc("terminal.newSession")) { app.openTerminal(deviceId: id) }
                    Button(loc("terminal.reattach")) {
                        Task {
                            guard let peer = try? await app.peer(id), let list = try? await peer.session.listTerminals() else { return }
                            app.modal = .terminalSessions(deviceId: id, sessions: list.filter { !$0.attached })
                        }
                    }
                    Divider()
                    Button(loc("terminal.kill"), role: .destructive) { tab.killSession() }
                } else if case .ssh(let id) = tab.target {
                    Button(loc("terminal.newSession")) { app.openTerminal(sshHostId: id) }
                }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
        }
        .padding(.horizontal, 12)
        .frame(height: 34)
        .background(p.content)
        .overlay(Rectangle().fill(p.separator).frame(height: 1), alignment: .bottom)
    }

    var platform: Platform {
        if case .device(let id) = tab.target { return app.device(id)?.platform ?? .linux }
        return .linux
    }
}

struct PathBadge: View {
    @ObservedObject var peer: PeerConnection
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        let (label, color, icon) = Self.describe(peer.path.kind, p: p, loc: loc)
        HStack(spacing: 5) {
            Image(systemName: icon).font(.system(size: 10, weight: .semibold))
            Text(label).font(.system(size: 11, weight: .medium))
            Text("\(peer.path.rttMs) ms").font(.system(size: 11).monospacedDigit()).foregroundStyle(p.secondaryText)
        }
        .foregroundStyle(color)
        .padding(.horizontal, 8).padding(.vertical, 3)
        .background(Capsule().fill(color.opacity(0.12)))
        .help(peer.path.remoteAddr)
    }

    static func describe(_ k: PathKind, p: Palette, loc: Strings) -> (String, SwiftUI.Color, String) {
        switch k {
        case .lan: return (loc("path.lan"), p.success, "wifi")
        case .direct: return (loc("path.direct"), p.info, "arrow.left.and.right")
        case .relayUdp: return (loc("path.relayUdp"), p.warning, "arrow.triangle.branch")
        case .relayTcp: return (loc("path.relayTcp"), p.warning, "arrow.triangle.branch")
        }
    }
}

struct SshPathBadge: View {
    let hostId: String
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        var text = loc("path.ssh")
        if let e = app.entry(hostId), case .sshHost(_, _, _, _, _, let jump?, _, _, _, _, _) = e.data {
            text = loc("path.sshVia", app.deviceName(jump))
        }
        return HStack(spacing: 5) {
            Image(systemName: "lock").font(.system(size: 10, weight: .semibold))
            Text(text).font(.system(size: 11, weight: .medium))
        }
        .foregroundStyle(p.info)
        .padding(.horizontal, 8).padding(.vertical, 3)
        .background(Capsule().fill(p.info.opacity(0.12)))
    }
}

/// 把模型里常驻的 SwiftTerm 视图放进 SwiftUI；切换标签时视图不重建，内容不丢。
struct TerminalHost: NSViewRepresentable {
    let view: TerminalView
    func makeNSView(context: Context) -> NSView {
        let container = NSView()
        attach(container)
        return container
    }
    func updateNSView(_ nsView: NSView, context: Context) {
        if view.superview !== nsView { attach(nsView) }
    }
    private func attach(_ container: NSView) {
        view.removeFromSuperview()
        view.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(view)
        NSLayoutConstraint.activate([
            view.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            view.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            view.topAnchor.constraint(equalTo: container.topAnchor),
            view.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])
        DispatchQueue.main.async { view.window?.makeFirstResponder(view) }
    }
}
