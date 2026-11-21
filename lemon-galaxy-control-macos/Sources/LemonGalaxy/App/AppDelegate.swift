import AppKit
import SwiftUI
import Combine

/// 应用入口：主窗口 + 菜单栏常驻图标。关掉主窗口不退出（本机被控要一直在线）。
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    var app: AppModel!
    var window: NSWindow?
    private var cancellables: Set<AnyCancellable> = []

    func applicationDidFinishLaunching(_ notification: Notification) {
        let settings = AppSettings.shared
        settings.load()
        Localization.shared.apply(setting: settings.language)
        settings.applyAppearance()
        app = AppModel(settings: settings, selfTest: false)
        buildMainMenu()
        StatusMenu.shared.install(app: app)
        showMainWindow()
        // 语言变化时重建菜单（菜单文字不是 SwiftUI，不会自动刷新）
        Localization.shared.$strings.dropFirst().receive(on: DispatchQueue.main).sink { [weak self] _ in
            self?.buildMainMenu()
            StatusMenu.shared.update(self?.app.host)
        }.store(in: &cancellables)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        showMainWindow()
        return true
    }

    func showMainWindow() {
        if window == nil {
            let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1320, height: 820),
                             styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
                             backing: .buffered, defer: false)
            w.titlebarAppearsTransparent = true
            w.titleVisibility = .hidden
            w.title = "LemonGalaxy"
            w.minSize = NSSize(width: 980, height: 620)
            w.isReleasedWhenClosed = false
            w.contentView = NSHostingView(rootView: RootView(app: app, settings: app.settings))
            w.setFrameAutosaveName("LemonGalaxyMain")
            if !w.setFrameUsingName("LemonGalaxyMain") { w.center() }
            w.delegate = self
            w.tabbingMode = .disallowed
            window = w
        }
        NSApp.setActivationPolicy(.regular)
        window?.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    func windowWillClose(_ notification: Notification) {
        // 主窗口关了只留菜单栏图标
        NSApp.setActivationPolicy(.accessory)
    }

    // MARK: 菜单

    private func buildMainMenu() {
        let main = NSMenu()
        func item(_ title: String, _ action: Selector?, _ key: String = "", mods: NSEvent.ModifierFlags = .command, target: AnyObject? = nil) -> NSMenuItem {
            let i = NSMenuItem(title: title, action: action, keyEquivalent: key)
            i.keyEquivalentModifierMask = mods
            i.target = target
            return i
        }
        let appItem = NSMenuItem()
        let appMenu = NSMenu()
        appMenu.addItem(item(L("menu.about"), #selector(showAbout), target: self))
        appMenu.addItem(.separator())
        appMenu.addItem(item(L("menu.settings"), #selector(showSettings), ",", target: self))
        appMenu.addItem(.separator())
        appMenu.addItem(item(L("menu.hide"), #selector(NSApplication.hide(_:)), "h"))
        appMenu.addItem(item(L("menu.hideOthers"), #selector(NSApplication.hideOtherApplications(_:)), "h", mods: [.command, .option]))
        appMenu.addItem(.separator())
        appMenu.addItem(item(L("menu.quit"), #selector(NSApplication.terminate(_:)), "q"))
        appItem.submenu = appMenu
        main.addItem(appItem)

        let fileItem = NSMenuItem()
        let fileMenu = NSMenu(title: L("menu.file"))
        fileMenu.addItem(item(L("menu.newLocalSftp"), #selector(newSftp), "n", mods: [.command, .shift], target: self))
        fileMenu.addItem(item(L("menu.closeTab"), #selector(closeTab), "w", target: self))
        fileItem.submenu = fileMenu
        main.addItem(fileItem)

        let editItem = NSMenuItem()
        let editMenu = NSMenu(title: L("menu.edit"))
        editMenu.addItem(item(L("menu.undo"), Selector(("undo:")), "z"))
        editMenu.addItem(item(L("menu.redo"), Selector(("redo:")), "z", mods: [.command, .shift]))
        editMenu.addItem(.separator())
        editMenu.addItem(item(L("menu.cut"), #selector(NSText.cut(_:)), "x"))
        editMenu.addItem(item(L("menu.copy"), #selector(NSText.copy(_:)), "c"))
        editMenu.addItem(item(L("menu.paste"), #selector(NSText.paste(_:)), "v"))
        editMenu.addItem(item(L("menu.selectAll"), #selector(NSText.selectAll(_:)), "a"))
        editItem.submenu = editMenu
        main.addItem(editItem)

        let viewItem = NSMenuItem()
        let viewMenu = NSMenu(title: L("menu.view"))
        for (i, s) in NavSection.allCases.enumerated() {
            let it = item(L(s.titleKey), #selector(selectSection(_:)), "\(i + 1)", target: self)
            it.tag = i
            viewMenu.addItem(it)
        }
        viewMenu.addItem(.separator())
        viewMenu.addItem(item(L("nav.toggleList"), #selector(toggleList), "0", target: self))
        viewMenu.addItem(item(L("menu.fullscreen"), #selector(NSWindow.toggleFullScreen(_:)), "f", mods: [.command, .control]))
        viewItem.submenu = viewMenu
        main.addItem(viewItem)

        let winItem = NSMenuItem()
        let winMenu = NSMenu(title: L("menu.window"))
        winMenu.addItem(item(L("menu.minimize"), #selector(NSWindow.performMiniaturize(_:)), "m"))
        winMenu.addItem(item(L("menu.mainWindow"), #selector(openMain), "", target: self))
        winItem.submenu = winMenu
        main.addItem(winItem)
        NSApp.mainMenu = main
        NSApp.windowsMenu = winMenu
    }

    @objc func showAbout() { app.settingsPage = .about; app.selectHome(.settings); showMainWindow() }
    @objc func showSettings() { app.selectHome(.settings); showMainWindow() }
    @objc func openMain() { showMainWindow() }
    @objc func newSftp() { app.openSFTP() }
    @objc func closeTab() {
        if let id = app.activeTabId { app.closeTab(id) } else { window?.performClose(nil) }
    }
    @objc func toggleList() { app.settings.listHidden.toggle() }
    @objc func selectSection(_ sender: NSMenuItem) {
        let all = NavSection.allCases
        guard sender.tag < all.count else { return }
        app.selectHome(all[sender.tag])
    }
}

// MARK: - 菜单栏图标

@MainActor
final class StatusMenu: NSObject, NSMenuDelegate {
    static let shared = StatusMenu()
    private var item: NSStatusItem?
    private weak var app: AppModel?

    func install(app: AppModel) {
        self.app = app
        let it = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        it.button?.image = Self.icon(state: .idle)
        it.button?.toolTip = "LemonGalaxy"
        let menu = NSMenu()
        menu.delegate = self
        it.menu = menu
        item = it
        update(app.host)
    }

    enum IconState { case idle, online, controlled, ai, disabled }

    func update(_ host: LocalHostModel?) {
        guard let item, let host else { return }
        let s = host.status
        let state: IconState = !s.remoteEnabled ? .disabled : s.aiActive ? .ai : !s.sessions.isEmpty ? .controlled : s.online ? .online : .idle
        item.button?.image = Self.icon(state: state)
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        guard let app else { return }
        let host = app.host
        let s = host.status
        let title: String
        if !s.remoteEnabled { title = L("status.disabled") }
        else if s.aiActive { title = L("status.ai") }
        else if !s.sessions.isEmpty { title = L("status.controlled", s.sessions.map(\.displayName).joined(separator: ", ")) }
        else if s.online { title = L("status.online") }
        else { title = L("status.offline") }
        let head = NSMenuItem(title: "LemonGalaxy · " + title, action: nil, keyEquivalent: "")
        head.isEnabled = false
        menu.addItem(head)
        if let act = host.aiActivity.values.first {
            let a = NSMenuItem(title: act, action: nil, keyEquivalent: "")
            a.isEnabled = false
            menu.addItem(a)
        }
        menu.addItem(.separator())
        if !s.sessions.isEmpty {
            menu.addItem(make(L("banner.disconnect"), #selector(disconnect)))
        }
        if s.aiActive { menu.addItem(make(L("banner.stopAi"), #selector(stopAi))) }
        let toggle = make(L("status.allowRemote"), #selector(toggleRemote))
        toggle.state = s.remoteEnabled ? .on : .off
        menu.addItem(toggle)
        if let a = s.assist {
            let i = NSMenuItem(title: L("status.assistCode", a.deviceCode, a.verificationCode), action: nil, keyEquivalent: "")
            i.isEnabled = false
            menu.addItem(i)
        }
        menu.addItem(.separator())
        menu.addItem(make(L("status.openMain"), #selector(openMain)))
        if app.core.isMock, let mock = app.client.host() as? MockHostService {
            let sim = NSMenuItem(title: L("status.simulate"), action: nil, keyEquivalent: "")
            let sub = NSMenu()
            sub.addItem(SimItem(L("status.sim.assist")) { mock.simulateAssistRequest() })
            sub.addItem(SimItem(L("status.sim.session")) { mock.simulateSession(ai: false) })
            sub.addItem(SimItem(L("status.sim.ai")) { mock.simulateSession(ai: true) })
            sim.submenu = sub
            menu.addItem(sim)
        }
        menu.addItem(.separator())
        menu.addItem(NSMenuItem(title: L("menu.quit"), action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q"))
    }

    private func make(_ title: String, _ sel: Selector) -> NSMenuItem {
        let i = NSMenuItem(title: title, action: sel, keyEquivalent: "")
        i.target = self
        return i
    }

    @objc private func disconnect() { app?.host.disconnectAll() }
    @objc private func stopAi() { app?.host.stopAi() }
    @objc private func toggleRemote() { guard let h = app?.host else { return }; h.setRemoteEnabled(!h.status.remoteEnabled) }
    @objc private func openMain() { (NSApp.delegate as? AppDelegate)?.showMainWindow() }

    /// 菜单栏图标：模板图（跟随菜单栏深浅），被控 / AI 操作时右下角加彩色圆点
    static func icon(state: IconState) -> NSImage {
        let size = NSSize(width: 18, height: 18)
        let template = !(state == .controlled || state == .ai)
        let img = NSImage(size: size, flipped: false) { r in
            // 模板图用黑色由系统着色；带彩色圆点的图不能是模板，线条用 labelColor 跟随菜单栏深浅
            let ink: NSColor = template ? .black : .labelColor
            ink.setStroke()
            let ring = NSBezierPath()
            ring.appendArc(withCenter: NSPoint(x: 9, y: 9), radius: 6.5, startAngle: 20, endAngle: 160)
            ring.move(to: NSPoint(x: 9 + 6.5 * cos(.pi * 200 / 180), y: 9 + 6.5 * sin(.pi * 200 / 180)))
            ring.appendArc(withCenter: NSPoint(x: 9, y: 9), radius: 6.5, startAngle: 200, endAngle: 340)
            ring.lineWidth = 1.6
            ring.lineCapStyle = .round
            ring.stroke()
            ink.setFill()
            NSBezierPath(ovalIn: NSRect(x: 6.8, y: 6.8, width: 4.4, height: 4.4)).fill()
            if state == .disabled {
                let slash = NSBezierPath()
                slash.move(to: NSPoint(x: 3, y: 15)); slash.line(to: NSPoint(x: 15, y: 3))
                slash.lineWidth = 1.6
                slash.stroke()
            }
            if state == .controlled || state == .ai {
                (state == .ai ? NSColor.systemOrange : NSColor.systemRed).setFill()
                NSBezierPath(ovalIn: NSRect(x: 11.5, y: 0.5, width: 6.5, height: 6.5)).fill()
            }
            return true
        }
        img.isTemplate = template
        return img
    }
}

final class SimItem: NSMenuItem {
    private let handler: () -> Void
    init(_ title: String, _ handler: @escaping () -> Void) {
        self.handler = handler
        super.init(title: title, action: #selector(fire), keyEquivalent: "")
        target = self
    }
    required init(coder: NSCoder) { fatalError() }
    @objc private func fire() { handler() }
}
