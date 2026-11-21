import AppKit
import SwiftUI

/// --selftest <输出目录> [--scale 2] [--only 名称片段]
/// 用模拟内核把主要界面离屏渲染成 PNG：浅色 / 深色 × 中文 / 英文。
/// 不弹窗口（窗口放在屏幕外且从不显示）、不申请任何系统权限、不连网络、不读写用户设置与钥匙串。
@MainActor
enum SelfTest {
    static var scale: CGFloat = 2
    static let size = CGSize(width: 1280, height: 800)
    static var dir = URL(fileURLWithPath: "selftest")
    static var count = 0
    static var problems: [String] = []
    static var only: String?

    /// --make-icon <目录>：把应用图标（AppMark）渲染成 AppIcon.iconset，供 iconutil 生成 .icns
    static func makeIcon(_ dir: String) -> Int32 {
        let set = URL(fileURLWithPath: dir).appendingPathComponent("AppIcon.iconset", isDirectory: true)
        try? FileManager.default.createDirectory(at: set, withIntermediateDirectories: true)
        _ = NSApplication.shared
        for base in [16, 32, 128, 256, 512] {
            for mul in [1, 2] {
                let px = base * mul
                // 图标外留 10% 透明边，符合 macOS 图标网格
                let view = AppMark(size: CGFloat(px) * 0.8).frame(width: CGFloat(px), height: CGFloat(px))
                let renderer = ImageRenderer(content: view)
                renderer.scale = 1
                guard let cg = renderer.cgImage else { return 1 }
                let rep = NSBitmapImageRep(cgImage: cg)
                let name = "icon_\(base)x\(base)" + (mul == 2 ? "@2x" : "") + ".png"
                try? rep.representation(using: .png, properties: [:])?.write(to: set.appendingPathComponent(name))
            }
        }
        print(set.path)
        return 0
    }

    static func run(outputDir: String, args: [String]) -> Int32 {
        func arg(_ name: String) -> String? {
            guard let i = args.firstIndex(of: name), i + 1 < args.count else { return nil }
            return args[i + 1]
        }
        scale = CGFloat(Double(arg("--scale") ?? "2") ?? 2)
        only = arg("--only")
        dir = URL(fileURLWithPath: outputDir, isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)

        let nsapp = NSApplication.shared
        nsapp.setActivationPolicy(.prohibited)
        let settings = AppSettings.shared
        settings.persistent = false
        SystemPermissions.fake = (screen: false, accessibility: true)
        Clock.frozenMs = 1_795_225_692_000      // 2026-11-21 09:48:12（北京时间）
        let fakeHome = makeFakeHome()
        LocalFileSource.homeOverride = fakeHome.path
        defer { try? FileManager.default.removeItem(at: fakeHome.deletingLastPathComponent()) }
        RemoteVideoNSView.snapshotMode = true

        let langs = (arg("--lang").map { [$0] }) ?? ["zh-Hans", "en"]
        let themes = (arg("--theme").map { [$0] }) ?? ["light", "dark"]
        for lang in langs {
            for theme in themes {
                settings.language = lang
                settings.appearance = theme
                settings.palette = "galaxy"
                settings.terminalTheme = theme == "dark" ? "galaxy-dark" : "galaxy-light"
                Localization.shared.apply(setting: lang)
                renderAll(prefix: "\(lang == "en" ? "en" : "zh")-\(theme)", settings: settings, dark: theme == "dark")
            }
        }

        var report = "LemonGalaxy selftest\n输出 \(count) 张，尺寸 \(Int(size.width))×\(Int(size.height)) @\(scale)x\n"
        let missing = Localization.shared.missingKeys
        report += "\n缺失的语言键（\(missing.count)）：\n" + missing.joined(separator: "\n") + "\n"
        report += "\n问题（\(problems.count)）：\n" + problems.joined(separator: "\n") + "\n"
        try? report.write(to: dir.appendingPathComponent("report.txt"), atomically: true, encoding: .utf8)
        print("selftest: 输出 \(count) 张到 \(dir.path)；缺失语言键 \(missing.count) 个；问题 \(problems.count) 个")
        if !missing.isEmpty { print("缺失语言键：" + missing.joined(separator: ", ")) }
        for p in problems { print("问题：" + p) }
        return 0
    }

    // MARK: 工具

    /// 本机一侧用的假主目录（文件名、时间都固定）
    static func makeFakeHome() -> URL {
        let base = FileManager.default.temporaryDirectory.appendingPathComponent("lemongalaxy-selftest-\(ProcessInfo.processInfo.processIdentifier)", isDirectory: true)
        let home = base.appendingPathComponent("alice", isDirectory: true)
        let fm = FileManager.default
        try? fm.createDirectory(at: home, withIntermediateDirectories: true)
        let t = Date(timeIntervalSince1970: TimeInterval(1_795_225_692_000 / 1000))
        let items: [(String, Bool, Int, Double)] = [
            ("Desktop", true, 0, 2), ("Documents", true, 0, 5), ("Downloads", true, 0, 0.2), ("Projects", true, 0, 9),
            (".ssh", true, 0, 40), ("Pictures", true, 0, 60),
            ("deploy-notes.md", false, 2_430, 1), ("galaxy-server.toml", false, 812, 3), ("report-2026Q3.pdf", false, 1_284_112, 12),
            ("screenshot.png", false, 482_330, 0.5), ("backup-2026-11-01.tar.gz", false, 3_412_000, 20),
        ]
        for (name, dir, size, daysAgo) in items {
            let url = home.appendingPathComponent(name)
            if dir { try? fm.createDirectory(at: url, withIntermediateDirectories: true) }
            else { try? Data(count: size).write(to: url) }
            try? fm.setAttributes([.modificationDate: t.addingTimeInterval(-daysAgo * 86_400)], ofItemAtPath: url.path)
        }
        return home
    }

    final class OffscreenWindow: NSWindow {
        override var isKeyWindow: Bool { true }
        override var isMainWindow: Bool { true }
        override var canBecomeKey: Bool { true }
    }

    static func pump(_ seconds: Double = 0.15) {
        let end = Date().addingTimeInterval(seconds)
        while Date() < end { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
    }

    @discardableResult
    static func wait(_ what: String, timeout: Double = 5, _ cond: () -> Bool) -> Bool {
        let end = Date().addingTimeInterval(timeout)
        while Date() < end {
            if cond() { return true }
            RunLoop.main.run(until: Date().addingTimeInterval(0.02))
        }
        problems.append("等待超时：\(what)")
        return false
    }

    static func makeWindow(dark: Bool, size: CGSize? = nil) -> OffscreenWindow {
        let size = size ?? Self.size
        let w = OffscreenWindow(contentRect: NSRect(x: -30000, y: -30000, width: size.width, height: size.height),
                                styleMask: [.borderless], backing: .buffered, defer: false)
        w.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        w.isReleasedWhenClosed = false
        return w
    }

    static func capture(_ name: String, window: NSWindow, view: NSView, size: CGSize? = nil) {
        let size = size ?? Self.size
        if let only, !name.contains(only) { return }
        view.frame = NSRect(origin: .zero, size: size)
        window.setContentSize(size)
        for _ in 0..<3 { view.layoutSubtreeIfNeeded(); pump(0.05) }
        view.displayIfNeeded()
        guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { problems.append("无法渲染 \(name)"); return }
        view.cacheDisplay(in: view.bounds, to: rep)
        var out = rep
        let targetW = Int(size.width * scale), targetH = Int(size.height * scale)
        if rep.pixelsWide != targetW, let cg = rep.cgImage {
            // 按要求的倍率重采样
            if let ctx = CGContext(data: nil, width: targetW, height: targetH, bitsPerComponent: 8, bytesPerRow: 0,
                                   space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) {
                ctx.interpolationQuality = .high
                ctx.draw(cg, in: CGRect(x: 0, y: 0, width: targetW, height: targetH))
                if let img = ctx.makeImage() { out = NSBitmapImageRep(cgImage: img) }
            }
        }
        guard let data = out.representation(using: .png, properties: [:]) else { problems.append("PNG 编码失败 \(name)"); return }
        do {
            try data.write(to: dir.appendingPathComponent(name + ".png"))
            count += 1
        } catch {
            problems.append("写文件失败 \(name)")
        }
    }

    /// 渲染整个主窗口（RootView）当前状态
    static func shot(_ name: String, _ app: AppModel, window: NSWindow, host: NSHostingView<RootView>, settle: Double = 0.2) {
        if let only, !name.contains(only) { return }
        pump(settle)
        capture(name, window: window, view: host)
    }

    /// 单独渲染一个视图（套主题环境）
    static func shotView<V: View>(_ name: String, _ app: AppModel, dark: Bool, size: CGSize, background: Bool = true, @ViewBuilder _ content: () -> V) {
        if let only, !name.contains(only) { return }
        let w = makeWindow(dark: dark, size: size)
        let view = ThemedRoot(app: app, settings: app.settings) {
            BackgroundWrap(enabled: background) { content() }
        }
        let host = NSHostingView(rootView: view)
        w.contentView = host
        pump(0.15)
        capture(name, window: w, view: host, size: size)
        w.contentView = nil
    }

    struct BackgroundWrap<C: View>: View {
        let enabled: Bool
        @ViewBuilder let content: C
        @Environment(\.palette) var p
        var body: some View {
            if enabled { content.frame(maxWidth: .infinity, maxHeight: .infinity).background(p.content) } else { content }
        }
    }

    // MARK: 全部界面

    static func renderAll(prefix: String, settings: AppSettings, dark: Bool) {
        let app = AppModel(settings: settings, selfTest: true, forceMock: true, mockOptions: MockOptions(latencyScale: 0, liveSimulation: false))
        let zh = prefix.hasPrefix("zh")
        let server = "galaxy.lemonit.cn:7443#sha256-Q2F0Y2hNZUlmWW91Q2FuX0xlbW9uR2FsYXh5X1Rlc3Q"

        // 1. 登录三步
        shotView("\(prefix)-01-login", app, dark: dark, size: size) {
            LoginView(preset: { m in m.server = server; m.username = "alice"; m.password = "password" })
        }
        shotView("\(prefix)-02-login-totp", app, dark: dark, size: size) {
            LoginView(preset: { m in m.server = server; m.username = "alice-totp"; m.password = "x"; m.step = .totp; m.totp = "48213" })
        }
        shotView("\(prefix)-03-login-change-password", app, dark: dark, size: size) {
            LoginView(preset: { m in
                m.server = server; m.username = "new-user"; m.step = .changePassword; m.newPassword = "Galaxy-2026"; m.confirmPassword = "Galaxy-202"
            })
        }
        shotView("\(prefix)-04-login-invalid-server", app, dark: dark, size: size) {
            LoginView(preset: { m in m.server = "galaxy.lemonit.cn:7443"; m.username = "alice"; m.error = zh ? "用户名或密码错误" : "Incorrect username or password" })
        }

        // 登录（模拟内核立即返回）
        var loggedIn = false
        Task {
            _ = try? await app.client.login(server: server, username: "alice", password: "pw", totp: nil)
            app.didLogin()
            loggedIn = true
        }
        wait("登录") { loggedIn }

        let window = makeWindow(dark: dark)
        let host = NSHostingView(rootView: RootView(app: app, settings: settings))
        window.contentView = host

        // 2. 设备与主机
        app.section = .hosts
        shot("\(prefix)-10-hosts", app, window: window, host: host)
        app.selection = "dev:dev-office"
        shot("\(prefix)-11-hosts-device", app, window: window, host: host)
        app.selection = "s-dbp"
        app.editing = .vault("s-dbp")
        shot("\(prefix)-12-hosts-ssh-editor", app, window: window, host: host)
        app.editing = nil
        app.selection = "dev:dev-office"
        app.modal = .connectChooser(deviceId: "dev-office")
        shot("\(prefix)-13-hosts-connect-chooser", app, window: window, host: host)
        app.modal = nil
        app.hostsTab = .credentials
        app.selection = "c-deploy"
        app.editing = .vault("c-deploy")
        shot("\(prefix)-14-hosts-credential", app, window: window, host: host)
        app.editing = nil
        app.hostsTab = .hosts
        app.selection = nil
        app.searchText = "web"
        shot("\(prefix)-15-hosts-search", app, window: window, host: host)
        app.searchText = ""
        app.modal = .terminalSessions(deviceId: "dev-build", sessions: MockTerminalRegistry.shared.sessions(for: "dev-build").filter { !$0.attached })
        shot("\(prefix)-16-terminal-reattach", app, window: window, host: host)
        app.modal = nil

        // 3. 终端
        app.openTerminal(deviceId: "dev-build")
        if let t = app.activeTab as? TerminalTabModel {
            wait("终端打开") { t.phase == .running }
            pump(0.2)
            t.sendText("ls -la\r")
            pump(0.2)
            t.sendText("df -h\r")
            pump(0.2)
            t.sendText("colors\r")
            pump(0.3)
        }
        (app.activeTab as? TerminalTabModel)?.makeSnapshot()
        shot("\(prefix)-20-terminal", app, window: window, host: host, settle: 0.3)
        app.openTerminal(sshHostId: "s-web1")
        if let t = app.activeTab as? TerminalTabModel {
            wait("SSH 终端打开") { t.phase == .running }
            t.sendText("top\r")
            pump(0.3)
            t.makeSnapshot()
        }
        shot("\(prefix)-21-terminal-ssh", app, window: window, host: host, settle: 0.3)

        // 4. SFTP
        app.openSFTP(deviceId: "dev-build")
        if let s = app.activeTab as? SFTPTabModel {
            wait("SFTP 远端列表") { !s.right.items.isEmpty && s.right.loading == false }
            wait("SFTP 本机列表") { s.left.loading == false }
            // 演示传输队列：一个完成、一个失败
            if let r = s.right.source as? RemoteFileSource {
                let tmp = FileManager.default.temporaryDirectory.appendingPathComponent("lemongalaxy-selftest", isDirectory: true)
                try? FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
                _ = app.transfers.start(name: "config.yaml", direction: .download, client: r.client, local: tmp.appendingPathComponent("config.yaml").path,
                                        remote: r.join(s.right.path, "app/config.yaml"), endpointName: s.right.endpointName)
                _ = app.transfers.start(name: "backup-fail.tar.gz", direction: .download, client: r.client, local: tmp.appendingPathComponent("b.tgz").path,
                                        remote: r.join(s.right.path, "backup-fail.tar.gz"), endpointName: s.right.endpointName)
                wait("传输完成") { app.transfers.items.allSatisfy { if case .running = $0.state { return false }; return true } }
                try? FileManager.default.removeItem(at: tmp)
            }
            s.right.selection = [s.right.visible.first { $0.name == "deploy.sh" }?.path ?? ""]
        }
        shot("\(prefix)-30-sftp", app, window: window, host: host, settle: 0.3)
        if let s = app.activeTab as? SFTPTabModel, let item = s.right.items.first(where: { $0.name == "deploy.sh" }) {
            s.dialog = .chmod(s.right, item)
            shot("\(prefix)-31-sftp-chmod", app, window: window, host: host)
            s.dialog = nil
        }

        // 5. 远程桌面
        app.openRemoteDesktop(deviceId: "dev-office")
        if let r = app.activeTab as? RemoteDesktopModel {
            wait("远程画面解码", timeout: 8) { r.framesDecoded > 0 }
            pump(0.3)
            r.measuredFps = 30
            r.measuredKbps = 8_400
        }
        shot("\(prefix)-40-remote-desktop", app, window: window, host: host, settle: 0.2)

        // 6. AI 智能体
        app.selectHome(.ai)
        app.ai.loadModels()
        wait("AI 模型列表") { app.ai.models != nil }
        app.ai.draftTargets = [AgentTarget(kind: .device, id: "dev-build"), AgentTarget(kind: .sshHost, id: "s-dbp")]
        app.ai.draftPrompt = zh ? "build-server-01 的 /data 快满了，帮我看看是什么占的空间，清理一下旧备份。" : "/data on build-server-01 is almost full. Find out what uses the space and clean up old backups."
        shot("\(prefix)-50-ai-new", app, window: window, host: host)
        app.ai.draftTargets = [AgentTarget(kind: .device, id: "dev-build")]
        let conv = app.ai.start()
        if let conv {
            wait("AI 确认卡片") { conv.items.contains { if case .tool(let t) = $0 { return t.decision == .pending }; return false } }
            pump(0.2)
        }
        shot("\(prefix)-51-ai-approval", app, window: window, host: host, settle: 0.3)
        if let conv, let card = conv.items.compactMap({ if case .tool(let t) = $0, t.decision == .pending { return t }; return nil }).first {
            conv.decide(card, .allowOnce)
            wait("AI 提问") { conv.items.contains { if case .ask = $0 { return true }; return false } }
            if let ask = conv.items.compactMap({ if case .ask(let a) = $0 { return a }; return nil }).first {
                conv.answer(ask, ask.options[0])
                wait("AI 请求密码") { conv.activeSecret != nil }
            }
        }
        shot("\(prefix)-52-ai-secret", app, window: window, host: host, settle: 0.3)
        if let conv, let s = conv.activeSecret { conv.cancelSecret(s) }
        // 图形界面目标：截图 + 点击标注图，每步确认档
        app.selectHome(.ai)
        app.ai.draftTargets = [AgentTarget(kind: .device, id: "dev-office")]
        app.ai.draftMode = .confirmAll
        app.ai.draftModel = "1:qwen3-vl-plus"
        app.ai.draftPrompt = zh ? "把记事本里的内容保存一下" : "Save the Notepad content"
        if let gui = app.ai.start() {
            wait("AI 图形界面确认") { gui.items.contains { if case .tool(let t) = $0 { return t.decision == .pending }; return false } }
        }
        shot("\(prefix)-53-ai-gui", app, window: window, host: host, settle: 0.3)

        // 7. 片段
        app.selectHome(.snippets)
        app.selectedSnippet = "n-docker"
        shot("\(prefix)-60-snippets", app, window: window, host: host)

        // 8. 本机被控
        if let mock = (app.client as? MockGalaxyClient)?.mockHost {
            mock.simulateSession(user: "alice", device: "OFFICE-PC-07", ai: true)
        }
        wait("被控会话") { !app.host.status.sessions.isEmpty }
        pump(0.4)
        app.selectHome(.local)
        app.localPage = .overview
        shot("\(prefix)-70-local-overview", app, window: window, host: host)
        app.localPage = .accounts
        shot("\(prefix)-71-local-accounts", app, window: window, host: host)
        app.host.startAssist()
        if let mock = (app.client as? MockGalaxyClient)?.mockHost { mock.simulateAssistRequest() }
        wait("协助请求") { app.host.pendingRequest != nil }
        app.localPage = .assist
        shot("\(prefix)-72-local-assist", app, window: window, host: host)
        app.localPage = .events
        shot("\(prefix)-73-local-events", app, window: window, host: host)
        app.localPage = .permissions
        app.host.refreshPermissions()
        shot("\(prefix)-74-local-permissions", app, window: window, host: host)

        // 悬浮提示条、协助请求面板（独立窗口里的视图）
        shotView("\(prefix)-75-control-banner", app, dark: dark, size: CGSize(width: 380, height: 110)) {
            ZStack {
                LinearGradient(colors: [Color(hex: "#5B7BB5"), Color(hex: "#C9837A")], startPoint: .topLeading, endPoint: .bottomTrailing)
                ControlBannerView(host: app.host, settings: settings)
            }
        }
        if let r = app.host.pendingRequest {
            shotView("\(prefix)-76-assist-request", app, dark: dark, size: CGSize(width: 460, height: 190)) {
                AssistRequestView(request: r).frame(width: 420)
            }
        }

        // 9. 设置
        app.selectHome(.settings)
        for page in SettingsPage.allCases {
            app.settingsPage = page
            shot("\(prefix)-8\(SettingsPage.allCases.firstIndex(of: page)!)-settings-\(page.rawValue)", app, window: window, host: host)
        }

        // 10. 文本编辑器窗口、紧凑侧栏
        if let src = (app.tabs.compactMap { $0 as? SFTPTabModel }.first?.right.source) {
            let model = TextEditorModel(source: src, path: "/home/deploy/app/config.yaml", title: "config.yaml")
            model.load()
            wait("编辑器读入") { !model.loading }
            model.text += "  timeout: 30s\n"
            shotView("\(prefix)-90-text-editor", app, dark: dark, size: CGSize(width: 760, height: 480), background: false) {
                TextEditorView(model: model, settings: settings)
            }
        }
        settings.sidebarCompact = true
        app.section = .hosts
        settings.autoHideList = false
        if let t = app.tabs.first(where: { $0.kind == .terminal }) as? TerminalTabModel {
            app.activeTabId = t.id
            pump(0.2)
            t.makeSnapshot()
        }
        shot("\(prefix)-91-compact-terminal", app, window: window, host: host)
        settings.autoHideList = true
        settings.sidebarCompact = false

        // 收尾
        app.closeAllTabs()
        pump(0.2)
        window.contentView = nil
    }
}
