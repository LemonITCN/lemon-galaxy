import AppKit
import SwiftUI

/// --selftest <输出目录> [--scale 2] [--lang zh-Hans|en] [--theme light|dark] [--only 名称片段]
/// 用模拟内核把主要界面离屏渲染成 PNG：浅色 / 深色 × 中文 / 英文。
/// 不弹窗口（窗口放在屏幕外且从不显示）、不申请任何系统权限、不连网络、不读写用户设置与钥匙串。
/// 有缺失的语言键或渲染问题时退出码为 1（清单写在输出目录的 report.txt）。
@MainActor
enum SelfTest {
    static var scale: CGFloat = 2
    static let size = CGSize(width: 880, height: 620)
    static var dir = URL(fileURLWithPath: "selftest")
    static var count = 0
    static var problems: [String] = []
    static var only: String?

    /// --make-icon <目录>：把应用标志（WorkerMark）渲染成 AppIcon.iconset，供 iconutil 生成 .icns
    static func makeIcon(_ dir: String) -> Int32 {
        let set = URL(fileURLWithPath: dir).appendingPathComponent("AppIcon.iconset", isDirectory: true)
        try? FileManager.default.createDirectory(at: set, withIntermediateDirectories: true)
        _ = NSApplication.shared
        for base in [16, 32, 128, 256, 512] {
            for mul in [1, 2] {
                let px = base * mul
                // 图标外留 10% 透明边，符合 macOS 图标网格
                let view = WorkerMark(size: CGFloat(px) * 0.8).frame(width: CGFloat(px), height: CGFloat(px))
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
        let settings = WorkerSettings.shared
        settings.persistent = false
        SystemPermissions.fake = (screen: false, accessibility: true)
        Clock.frozenMs = 1_795_225_692_000      // 2026-11-21 09:48:12（北京时间）

        let langs = (arg("--lang").map { [$0] }) ?? ["zh-Hans", "en"]
        let themes = (arg("--theme").map { [$0] }) ?? ["light", "dark"]
        for lang in langs {
            for theme in themes {
                settings.language = lang
                settings.appearance = theme
                Localization.shared.apply(setting: lang)
                renderAll(prefix: "\(lang == "en" ? "en" : "zh")-\(theme)", settings: settings, dark: theme == "dark")
            }
        }

        var report = "LemonGalaxyWorker selftest\n输出 \(count) 张，窗口 \(Int(size.width))×\(Int(size.height)) @\(scale)x\n"
        let missing = Localization.shared.missingKeys
        report += "\n缺失的语言键（\(missing.count)）：\n" + missing.joined(separator: "\n") + "\n"
        report += "\n问题（\(problems.count)）：\n" + problems.joined(separator: "\n") + "\n"
        try? report.write(to: dir.appendingPathComponent("report.txt"), atomically: true, encoding: .utf8)
        print("selftest: 输出 \(count) 张到 \(dir.path)；缺失语言键 \(missing.count) 个；问题 \(problems.count) 个")
        if !missing.isEmpty { print("缺失语言键：" + missing.joined(separator: ", ")) }
        for p in problems { print("问题：" + p) }
        return missing.isEmpty && problems.isEmpty ? 0 : 1
    }

    // MARK: 工具

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

    /// 渲染整个设置窗口当前状态
    static func shot(_ name: String, window: NSWindow, host: NSView, settle: Double = 0.2) {
        if let only, !name.contains(only) { return }
        pump(settle)
        capture(name, window: window, view: host)
    }

    /// 单独渲染一个视图（套主题环境）
    static func shotView<V: View>(_ name: String, _ model: WorkerModel, dark: Bool, size: CGSize, @ViewBuilder _ content: () -> V) {
        if let only, !name.contains(only) { return }
        let w = makeWindow(dark: dark, size: size)
        let host = NSHostingView(rootView: Themed(model: model, settings: model.settings) { content() })
        w.contentView = host
        pump(0.15)
        capture(name, window: w, view: host, size: size)
        w.contentView = nil
    }

    static func makeModel(_ settings: WorkerSettings, preset: Bool) -> WorkerModel {
        let m = WorkerModel(settings: settings, selfTest: true, forceMock: true, mockOptions: MockWorkerOptions(latencyScale: 0, presetAccounts: preset))
        m.boot()
        wait("被控服务启动") { !m.serviceStarting }
        return m
    }

    // MARK: 全部界面

    static func renderAll(prefix: String, settings: WorkerSettings, dark: Bool) {
        let zh = prefix.hasPrefix("zh")
        let window = makeWindow(dark: dark)

        // 1. 第一次打开：还没登录账号
        let fresh = makeModel(settings, preset: false)
        let freshHost = NSHostingView(rootView: WorkerRootView(model: fresh, settings: settings))
        window.contentView = freshHost
        fresh.page = .status
        shot("\(prefix)-01-first-run", window: window, host: freshHost)
        fresh.page = .assist
        shot("\(prefix)-02-assist-no-server", window: window, host: freshHost)

        // 2. 登录三步与报错（窗口内浮层）
        fresh.openLogin()
        if let f = fresh.loginForm {
            f.server = MockWorkerHost.demoServer
            f.username = "alice"
            f.password = "password"
        }
        shot("\(prefix)-03-login", window: window, host: freshHost)
        if let f = fresh.loginForm {
            f.username = "alice-totp"
            f.step = .totp
            f.totp = "482913"
        }
        shot("\(prefix)-04-login-totp", window: window, host: freshHost)
        if let f = fresh.loginForm {
            f.username = "new-user"
            f.step = .changePassword
            f.newPassword = "Galaxy-2026"
            f.confirmPassword = "Galaxy-202"
        }
        shot("\(prefix)-05-login-change-password", window: window, host: freshHost)
        // 真走一遍模拟登录：错误密码 → 报错
        if let f = fresh.loginForm {
            f.step = .credentials
            f.username = "alice"
            f.password = "wrong"
            fresh.submitLogin(f)
            wait("登录报错") { f.error != nil && !f.busy }
        }
        shot("\(prefix)-06-login-error", window: window, host: freshHost)
        // 再用正确密码登录：账号出现在列表里
        if let f = fresh.loginForm {
            f.password = "password"
            fresh.submitLogin(f)
            wait("模拟登录成功") { fresh.loginForm == nil && fresh.hasAccounts }
            if fresh.service == nil { problems.append("登录后没有接上被控服务") }
            fresh.notice = nil
        }
        window.contentView = nil

        // 3. 已有两个账号、有人在控制（含 AI）
        let model = makeModel(settings, preset: true)
        let host = NSHostingView(rootView: WorkerRootView(model: model, settings: settings))
        window.contentView = host
        if let mock = (model.worker as? MockWorkerHost)?.mockService {
            mock.simulateSession(user: zh ? "王爱丽（alice）" : "Alice Wang (alice)", device: "4f2a9c1d77e04b18a3c5d6e7f8091a2b", ai: true)
        }
        wait("被控会话") { !model.status.sessions.isEmpty }
        wait("AI 活动") { !model.aiActivity.isEmpty }
        model.page = .status
        shot("\(prefix)-10-status", window: window, host: host)
        model.page = .accounts
        shot("\(prefix)-11-accounts", window: window, host: host)
        model.confirmLogout = model.accounts.first
        shot("\(prefix)-12-signout-confirm", window: window, host: host)
        model.confirmLogout = nil
        model.startAssist()
        wait("临时协助设备码") { model.status.assist != nil }
        (model.worker as? MockWorkerHost)?.mockService?.simulateAssistRequest()
        wait("协助请求") { model.pendingRequest != nil }
        model.page = .assist
        shot("\(prefix)-13-assist", window: window, host: host)
        model.page = .events
        shot("\(prefix)-14-events", window: window, host: host)
        model.page = .permissions
        model.refreshPermissions()
        shot("\(prefix)-15-permissions", window: window, host: host)
        model.page = .general
        shot("\(prefix)-16-general", window: window, host: host)
        model.page = .about
        shot("\(prefix)-17-about", window: window, host: host)
        model.setRemoteEnabled(false)
        wait("禁止远程") { !model.status.remoteEnabled }
        model.page = .status
        shot("\(prefix)-18-status-disabled", window: window, host: host)
        model.setRemoteEnabled(true)
        if let mock = (model.worker as? MockWorkerHost)?.mockService {
            mock.simulateSession(user: zh ? "王爱丽（alice）" : "Alice Wang (alice)", device: "4f2a9c1d77e04b18a3c5d6e7f8091a2b", ai: true)
        }
        wait("被控会话（恢复）") { !model.status.sessions.isEmpty }
        wait("AI 活动（恢复）") { !model.aiActivity.isEmpty }
        window.contentView = nil

        // 4. 悬浮提示条、协助请求面板、菜单栏图标（独立窗口里的视图）
        shotView("\(prefix)-20-control-banner", model, dark: dark, size: CGSize(width: 380, height: 110)) {
            ZStack {
                LinearGradient(colors: [Color(hex: "#5B7BB5"), Color(hex: "#C9837A")], startPoint: .topLeading, endPoint: .bottomTrailing)
                ControlBannerView(model: model)
            }
        }
        if let r = model.pendingRequest {
            shotView("\(prefix)-21-assist-request", model, dark: dark, size: CGSize(width: 460, height: 190)) {
                ZStack {
                    LinearGradient(colors: [Color(hex: "#5B7BB5"), Color(hex: "#C9837A")], startPoint: .topLeading, endPoint: .bottomTrailing)
                    AssistRequestView(request: r).frame(width: 420)
                }
            }
        }
        shotView("\(prefix)-22-menubar-icons", model, dark: dark, size: CGSize(width: 420, height: 150)) {
            MenuIconStrip()
        }
    }
}

/// 菜单栏图标各状态（浅色与深色菜单栏各一行）
struct MenuIconStrip: View {
    var body: some View {
        VStack(spacing: 0) {
            row(dark: false)
            row(dark: true)
        }
    }

    private func row(dark: Bool) -> some View {
        HStack(spacing: 18) {
            ForEach(StatusMenu.IconState.allCases, id: \.self) { s in
                VStack(spacing: 6) {
                    Image(nsImage: Self.preview(s, dark: dark)).resizable().interpolation(.high).frame(width: 36, height: 36)
                    Text(String(describing: s)).font(.system(size: 9.5)).foregroundStyle(dark ? Color.white.opacity(0.7) : Color.black.opacity(0.6))
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(dark ? Color(hex: "#2B2D31") : Color(hex: "#ECECEE"))
    }

    /// 按指定深浅外观把图标画成位图：模板图按菜单栏着色，彩色图里的 labelColor 按外观解析
    static func preview(_ s: StatusMenu.IconState, dark: Bool) -> NSImage {
        let img = StatusMenu.icon(state: s)
        let rect = NSRect(x: 0, y: 0, width: 18, height: 18)
        guard let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 72, pixelsHigh: 72, bitsPerSample: 8, samplesPerPixel: 4,
                                         hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0) else { return img }
        rep.size = rect.size
        NSAppearance(named: dark ? .darkAqua : .aqua)?.performAsCurrentDrawingAppearance {
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
            img.draw(in: rect)
            if img.isTemplate {
                (dark ? NSColor.white : NSColor.black).set()
                rect.fill(using: .sourceAtop)
            }
            NSGraphicsContext.restoreGraphicsState()
        }
        let out = NSImage(size: rect.size)
        out.addRepresentation(rep)
        return out
    }
}
