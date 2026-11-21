import Foundation
import UIKit
import SwiftTerm
import Combine
import GalaxyKit

/// SwiftTerm 的终端视图，加上键盘上方特殊键栏的粘滞修饰键（Ctrl / Alt）。
final class GalaxyTerminalView: TerminalView {
    var ctrl = false
    var alt = false
    var onModifiersConsumed: (() -> Void)?

    override func insertText(_ text: String) {
        if ctrl || alt {
            let bytes = TerminalKeys.encode(text: text == "\n" ? "\r" : text, ctrl: ctrl, alt: alt)
            ctrl = false
            alt = false
            onModifiersConsumed?()
            send(bytes)
            return
        }
        super.insertText(text)
    }

    /// 特殊键（方向键等）按当前修饰键与应用光标模式编码
    func sendKey(_ key: TerminalKey) {
        let bytes = TerminalKeys.bytes(key, appCursor: getTerminal().applicationCursor, ctrl: ctrl, alt: alt)
        if ctrl || alt {
            ctrl = false
            alt = false
            onModifiersConsumed?()
        }
        send(bytes)
    }
}

/// 一个终端标签：SwiftTerm 仿真器与内核 TerminalChannel 之间的桥。
/// 远端输出 → TerminalSink.onOutput → 主线程 feed；用户输入 → TerminalViewDelegate.send → channel.write（按调用顺序送达）。
@MainActor
final class TerminalSession: NSObject, ObservableObject, Identifiable, TerminalViewDelegate {
    enum Status: Equatable {
        case connecting, connected, detached
        case exited(Int32?)
        case failed(String)
    }

    let id = UUID().uuidString
    let target: RemoteTarget
    let baseTitle: String
    @Published private(set) var title: String
    @Published private(set) var status: Status = .connecting
    @Published private(set) var remoteSessionId: String?
    @Published var fontSize: Double
    @Published var inputBox: Bool
    let view: GalaxyTerminalView
    private(set) var keyBar: KeyBarView!
    /// 通道放在带锁的盒子里：SwiftTerm 的回调直接写，不经过主执行器检查
    private let box = ChannelBox()
    private var channel: TerminalChannel? {
        get { box.channel }
        set { box.channel = newValue }
    }
    private var sink: TerminalSinkAdapter?
    weak var app: AppModel?
    var onShowSnippets: (() -> Void)?
    private var closeWatch: AnyCancellable?

    init(app: AppModel, target: RemoteTarget, title: String) {
        self.app = app
        self.target = target
        self.baseTitle = title
        self.title = title
        self.fontSize = app.settings.terminalFontSize
        self.inputBox = app.settings.terminalInputBox
        view = GalaxyTerminalView(frame: CGRect(x: 0, y: 0, width: 390, height: 600))
        super.init()
        view.terminalDelegate = self
        view.optionAsMetaKey = true
        view.font = UIFont.monospacedSystemFont(ofSize: CGFloat(fontSize), weight: .regular)
        applyTheme()
        keyBar = makeKeyBar()
        view.inputAccessoryView = keyBar
        view.onModifiersConsumed = { [weak self] in self?.keyBar.clearToggles() }
        let pinch = UIPinchGestureRecognizer(target: self, action: #selector(pinched(_:)))
        view.addGestureRecognizer(pinch)
    }

    var isSsh: Bool { target.sshHostId != nil }

    /// 终端配色：SSH 主机可以单独指定，否则用全局设置
    func applyTheme() {
        var themeId = app?.settings.terminalTheme
        if let h = target.sshHostId, let e = app?.account.entry(h), case .sshHost(_, _, _, _, _, _, _, _, _, _, let t?) = e.data { themeId = t }
        let theme = TerminalTheme.named(themeId)
        theme.apply(to: view)
        keyBar?.tint = UIColor(hex: theme.cursor)
    }

    // MARK: 连接

    func start(attach: String?) {
        status = .connecting
        sink?.detach()
        let feedView = view
        let s = TerminalSinkAdapter(onData: { data in
            // 按到达顺序切到主线程喂给仿真器
            DispatchQueue.main.async { feedView.feed(byteArray: ArraySlice([UInt8](data))) }
        }, onExit: { [weak self] code in
            DispatchQueue.main.async { self?.remoteExited(code) }
        })
        sink = s
        let term = view.getTerminal()
        let options = TerminalOptions(attachSession: attach, shell: nil, cols: UInt16(max(term.cols, 20)), rows: UInt16(max(term.rows, 5)),
                                      env: [EnvVar(name: "TERM", value: "xterm-256color"), EnvVar(name: "COLORTERM", value: "truecolor")])
        Task { [weak self] in
            guard let self else { return }
            do {
                guard let app else { return }
                let ch = try await app.account.openTerminal(target, options: options, sink: s)
                channel = ch
                remoteSessionId = isSsh ? nil : ch.sessionId()
                let t = ch.title()
                if !t.isEmpty, title == baseTitle { title = t }
                status = .connected
                // 端到端会话断了（网络切换、被控端下线）：标成已分离，点重连即接续被控端保留的会话
                if let d = target.deviceId, let peer = app.account.existingPeer(d) {
                    closeWatch = peer.$closedReason.compactMap { $0 }.receive(on: DispatchQueue.main).sink { [weak self] reason in
                        guard let self, self.status == .connected || self.status == .connecting else { return }
                        self.channel = nil
                        self.status = reason.isEmpty ? .detached : .failed(reason)
                    }
                }
                // 打开时的尺寸可能已经变了
                let now = view.getTerminal()
                ch.resize(cols: UInt16(now.cols), rows: UInt16(now.rows))
            } catch {
                status = .failed(describeError(error))
            }
        }
    }

    private func remoteExited(_ code: Int32?) {
        guard status != .detached else { return }
        status = .exited(code)
        channel = nil
    }

    /// 断线后重连：设备接续原会话（先清屏，被控端会回放最近输出）；SSH 没有断线继承，新开一个
    func reconnect() {
        let attach = isSsh ? nil : remoteSessionId
        if attach != nil {
            view.feed(byteArray: ArraySlice(Array("\u{1b}[2J\u{1b}[3J\u{1b}[H".utf8)))
        }
        if isSsh, let h = target.sshHostId, app?.account.existingSsh(h) == nil {
            // SSH 连接断了：重新连（可能再问一次密码）
        }
        start(attach: attach)
    }

    /// 分离：被控端保留会话，之后可以从任何设备接续（SSH 主机分离即结束）
    func detach() {
        sink?.detach()
        channel?.detach()
        channel = nil
        if status == .connected || status == .connecting { status = .detached }
    }

    func kill() {
        sink?.detach()
        channel?.kill()
        channel = nil
        status = .exited(nil)
    }

    var isLive: Bool { status == .connected && channel != nil }

    /// 片段、输入框整段发送
    func send(text: String) {
        view.send(txt: text)
    }

    func paste() {
        guard let s = UIPasteboard.general.string, !s.isEmpty else { return }
        // 远端开了括号粘贴模式时包起来，避免多行粘贴被逐行执行
        if view.getTerminal().bracketedPasteMode {
            view.send(txt: "\u{1b}[200~" + s + "\u{1b}[201~")
        } else {
            view.send(txt: s)
        }
    }

    func setFontSize(_ size: Double) {
        let s = min(max(size, AppSettings.fontRange.lowerBound), AppSettings.fontRange.upperBound)
        fontSize = s
        view.font = UIFont.monospacedSystemFont(ofSize: CGFloat(s), weight: .regular)
    }

    @objc private func pinched(_ g: UIPinchGestureRecognizer) {
        guard g.state == .changed else { return }
        if abs(g.scale - 1) > 0.12 {
            setFontSize(fontSize * (g.scale > 1 ? 1.08 : 0.93))
            g.scale = 1
        }
    }

    // MARK: 特殊键栏

    private func makeKeyBar() -> KeyBarView {
        typealias I = KeyBarView.Item
        var items: [I] = [
            .action(id: "hide", symbol: "keyboard.chevron.compact.down"),
            .key(id: "esc", title: "esc", symbol: nil),
            .key(id: "tab", title: "tab", symbol: nil),
            .toggle(id: "ctrl", title: "ctrl"),
            .toggle(id: "alt", title: "alt"),
            .key(id: "left", title: nil, symbol: "arrow.left"),
            .key(id: "up", title: nil, symbol: "arrow.up"),
            .key(id: "down", title: nil, symbol: "arrow.down"),
            .key(id: "right", title: nil, symbol: "arrow.right"),
            .action(id: "snippets", symbol: "chevron.left.forwardslash.chevron.right"),
            .action(id: "paste", symbol: "doc.on.clipboard"),
            .separator,
        ]
        for c in ["|", "/", "-", "~", ":", "_", "*", "&", "$", ">", "<", "\\", "'", "\"", "`", "!", "#", "=", ";"] {
            items.append(.key(id: "char:" + c, title: c, symbol: nil))
        }
        items.append(.separator)
        items += [.key(id: "home", title: "home", symbol: nil), .key(id: "end", title: "end", symbol: nil),
                  .key(id: "pageUp", title: "pgup", symbol: nil), .key(id: "pageDown", title: "pgdn", symbol: nil),
                  .key(id: "delete", title: "del", symbol: nil)]
        for n in 1...12 { items.append(.key(id: "f\(n)", title: "F\(n)", symbol: nil)) }
        let bar = KeyBarView(items: items)
        bar.onToggle = { [weak self] id, on in
            guard let self else { return }
            if id == "ctrl" { self.view.ctrl = on }
            if id == "alt" { self.view.alt = on }
        }
        bar.onKey = { [weak self] id in self?.keyBarPressed(id) }
        return bar
    }

    private func keyBarPressed(_ id: String) {
        switch id {
        case "hide": _ = view.resignFirstResponder()
        case "snippets": onShowSnippets?()
        case "paste": paste()
        default:
            if id.hasPrefix("char:") {
                view.insertText(String(id.dropFirst(5)))
            } else if let k = TerminalKey(rawValue: id) {
                view.sendKey(k)
            }
        }
    }

    // MARK: TerminalViewDelegate

    nonisolated func send(source: TerminalView, data: ArraySlice<UInt8>) {
        box.channel?.write(data: Data(data))
    }

    nonisolated func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
        box.channel?.resize(cols: UInt16(max(newCols, 1)), rows: UInt16(max(newRows, 1)))
    }

    nonisolated func setTerminalTitle(source: TerminalView, title: String) {
        DispatchQueue.main.async { self.title = title.isEmpty ? self.baseTitle : title }
    }

    nonisolated func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}
    nonisolated func scrolled(source: TerminalView, position: Double) {}

    nonisolated func requestOpenLink(source: TerminalView, link: String, params: [String: String]) {
        guard let url = URL(string: link), ["http", "https"].contains(url.scheme?.lowercased() ?? "") else { return }
        DispatchQueue.main.async { UIApplication.shared.open(url) }
    }

    nonisolated func bell(source: TerminalView) {
        DispatchQueue.main.async { UIImpactFeedbackGenerator(style: .light).impactOccurred() }
    }

    nonisolated func clipboardCopy(source: TerminalView, content: Data) {
        guard let s = String(data: content, encoding: .utf8) else { return }
        DispatchQueue.main.async { UIPasteboard.general.string = s }
    }

    nonisolated func iTermContent(source: TerminalView, content: ArraySlice<UInt8>) {}
    nonisolated func rangeChanged(source: TerminalView, startY: Int, endY: Int) {}
}

final class ChannelBox: @unchecked Sendable {
    private let lock = NSLock()
    private var _channel: TerminalChannel?
    var channel: TerminalChannel? {
        get { lock.lock(); defer { lock.unlock() }; return _channel }
        set { lock.lock(); _channel = newValue; lock.unlock() }
    }
}
