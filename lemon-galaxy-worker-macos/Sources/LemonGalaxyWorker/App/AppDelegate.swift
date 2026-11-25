import AppKit
import SwiftUI
import Combine

/// 应用入口：菜单栏常驻图标 + 设置窗口。没有 Dock 图标（LSUIElement）；设置窗口打开时临时出现在 Dock 与 ⌘Tab 里。
/// 关掉设置窗口不退出（被控要一直在线）。
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    static let showNotification = Notification.Name("cn.lemonit.galaxy.worker.show")
    var model: WorkerModel!
    var window: NSWindow?
    private var cancellables: Set<AnyCancellable> = []

    func applicationWillFinishLaunching(_ notification: Notification) {
        // 同一用户只跑一份：已有实例时叫它打开设置窗口，自己退出（两份会抢同一个设备身份）
        let me = ProcessInfo.processInfo.processIdentifier
        // 无人值守的会话代理也是本应用的进程（同一个可执行文件），不算另一份
        if let id = Bundle.main.bundleIdentifier,
           NSRunningApplication.runningApplications(withBundleIdentifier: id)
            .contains(where: { $0.processIdentifier != me && !Unattended.isHelperProcess($0.processIdentifier) }) {
            DistributedNotificationCenter.default().postNotificationName(Self.showNotification, object: nil, userInfo: nil, deliverImmediately: true)
            exit(0)
        }
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        let settings = WorkerSettings.shared
        settings.load()
        Localization.shared.apply(setting: settings.language)
        settings.applyAppearance()
        model = WorkerModel(settings: settings, selfTest: false)
        Notifier.install(model)
        buildMainMenu()
        StatusMenu.shared.install(model: model)
        model.boot()
        // 没有账号（第一次打开）时直接打开设置窗口引导登录；开机自启等情况下安静地待在菜单栏
        // （启动事件只在这里有效，先记下来）
        let quiet = launchedAsLoginItem
        Task { @MainActor in
            for _ in 0..<50 where model.serviceStarting { try? await Task.sleep(nanoseconds: 100_000_000) }
            if !model.hasAccounts && !quiet { showSettings() }
        }
        Localization.shared.$strings.dropFirst().receive(on: DispatchQueue.main).sink { [weak self] strings in
            guard let self else { return }
            WorkerCore.setLocale(strings.pack.coreLocale, isMock: self.model.core.isMock)
            (self.model.worker as? RemoteWorkerHost)?.setLocale(strings.pack.coreLocale)
            self.buildMainMenu()
            Notifier.install(self.model)
            StatusMenu.shared.update(self.model)
            self.window?.title = "LemonGalaxyWorker"
        }.store(in: &cancellables)
        DistributedNotificationCenter.default().addObserver(forName: Self.showNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.showSettings() }
        }
    }

    /// 开机自启时系统给的启动事件带「登录项」标记
    private var launchedAsLoginItem: Bool {
        guard let event = NSAppleEventManager.shared().currentAppleEvent else { return false }
        return event.eventID == kAEOpenApplication
            && event.paramDescriptor(forKeyword: keyAEPropData)?.enumCodeValue == keyAELaunchedAsLogInItem
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        showSettings()
        return true
    }

    func showSettings() {
        if window == nil {
            let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 880, height: 620),
                             styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
                             backing: .buffered, defer: false)
            w.titlebarAppearsTransparent = true
            w.titleVisibility = .hidden
            w.title = "LemonGalaxyWorker"
            w.minSize = NSSize(width: 760, height: 520)
            w.isReleasedWhenClosed = false
            w.contentView = NSHostingView(rootView: WorkerRootView(model: model, settings: model.settings))
            w.setFrameAutosaveName("LemonGalaxyWorkerSettings")
            if !w.setFrameUsingName("LemonGalaxyWorkerSettings") { w.center() }
            w.delegate = self
            w.tabbingMode = .disallowed
            window = w
        }
        NSApp.setActivationPolicy(.regular)
        window?.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        model.setWindowVisible(true)
    }

    func windowWillClose(_ notification: Notification) {
        model.setWindowVisible(false)
        model.watchPermissions(false)
        // 设置窗口关了只留菜单栏图标
        NSApp.setActivationPolicy(.accessory)
    }

    // MARK: 主菜单（设置窗口打开时显示；文本框的复制粘贴快捷键也靠它）

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
        appMenu.addItem(item(L("menu.settings"), #selector(openSettings), ",", target: self))
        appMenu.addItem(.separator())
        appMenu.addItem(item(L("menu.hide"), #selector(NSApplication.hide(_:)), "h"))
        appMenu.addItem(.separator())
        appMenu.addItem(item(L("menu.quit"), #selector(NSApplication.terminate(_:)), "q"))
        appItem.submenu = appMenu
        main.addItem(appItem)

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

        let winItem = NSMenuItem()
        let winMenu = NSMenu(title: L("menu.window"))
        winMenu.addItem(item(L("menu.minimize"), #selector(NSWindow.performMiniaturize(_:)), "m"))
        winMenu.addItem(item(L("menu.close"), #selector(NSWindow.performClose(_:)), "w"))
        winItem.submenu = winMenu
        main.addItem(winItem)
        NSApp.mainMenu = main
        NSApp.windowsMenu = winMenu
    }

    @objc func showAbout() { model.page = .about; showSettings() }
    @objc func openSettings() { showSettings() }
}
