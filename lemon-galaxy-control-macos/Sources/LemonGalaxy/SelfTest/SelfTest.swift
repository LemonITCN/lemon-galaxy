import AppKit
import SwiftUI
import CoreMedia

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
    /// 本机一侧的假主目录（复制粘贴文件从这里取文件）
    static var fakeHome: URL?

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
        // 录像写进假主目录里的「影片」，不碰真的 ~/Movies
        RemoteDesktopModel.recordingDirOverride = fakeHome.appendingPathComponent("Movies/LemonGalaxy", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: fakeHome.deletingLastPathComponent()) }
        Self.fakeHome = fakeHome
        // 复制粘贴文件：用私有剪贴板与临时缓存目录，不碰用户的剪贴板和 ~/Library/Caches
        let pasteboard = NSPasteboard(name: NSPasteboard.Name("cn.lemonit.galaxy.selftest.\(ProcessInfo.processInfo.processIdentifier)"))
        ClipboardFiles.pasteboardOverride = pasteboard
        ClipboardFiles.cacheRootOverride = fakeHome.deletingLastPathComponent().appendingPathComponent("clipboard-cache", isDirectory: true)
        defer { pasteboard.releaseGlobally() }
        checkClipboardCacheCleanup()
        RemoteVideoNSView.snapshotMode = true
        checkAudioConversion()

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

    /// 被控端声音采集的格式转换（不碰音频设备）：ScreenCaptureKit 常见的非交错 Float32，以及交错 Int16，
    /// 都应转成内核要的交错 Float32 小端字节
    static func checkAudioConversion() {
        func make(channels: [[Float]], interleaved: Bool, int16: Bool) -> CMSampleBuffer? {
            let n = channels.count, frames = channels[0].count, width = int16 ? 2 : 4
            var flags: AudioFormatFlags = kAudioFormatFlagIsPacked | (int16 ? kAudioFormatFlagIsSignedInteger : kAudioFormatFlagIsFloat)
            if !interleaved { flags |= kAudioFormatFlagIsNonInterleaved }
            let frameBytes = UInt32(interleaved ? width * n : width)
            var asbd = AudioStreamBasicDescription(mSampleRate: 44_100, mFormatID: kAudioFormatLinearPCM, mFormatFlags: flags,
                                                   mBytesPerPacket: frameBytes, mFramesPerPacket: 1, mBytesPerFrame: frameBytes,
                                                   mChannelsPerFrame: UInt32(n), mBitsPerChannel: UInt32(width * 8), mReserved: 0)
            var fmt: CMAudioFormatDescription?
            guard CMAudioFormatDescriptionCreate(allocator: nil, asbd: &asbd, layoutSize: 0, layout: nil, magicCookieSize: 0, magicCookie: nil,
                                                 extensions: nil, formatDescriptionOut: &fmt) == noErr, let fmt else { return nil }
            var sb: CMSampleBuffer?
            var timing = CMSampleTimingInfo(duration: CMTime(value: 1, timescale: 44_100), presentationTimeStamp: .zero, decodeTimeStamp: .invalid)
            guard CMSampleBufferCreate(allocator: nil, dataBuffer: nil, dataReady: false, makeDataReadyCallback: nil, refcon: nil, formatDescription: fmt,
                                       sampleCount: frames, sampleTimingEntryCount: 1, sampleTimingArray: &timing, sampleSizeEntryCount: 0,
                                       sampleSizeArray: nil, sampleBufferOut: &sb) == noErr, let sb else { return nil }
            // 每块缓冲的原始字节
            var blocks: [Data] = []
            func bytes(_ v: Float) -> Data {
                if int16 { var x = Int16(v * 32767); return Data(bytes: &x, count: 2) }
                var x = v
                return Data(bytes: &x, count: 4)
            }
            if interleaved {
                var d = Data()
                for f in 0..<frames { for c in 0..<n { d.append(bytes(channels[c][f])) } }
                blocks = [d]
            } else {
                blocks = channels.map { ch in ch.reduce(into: Data()) { $0.append(bytes($1)) } }
            }
            let list = AudioBufferList.allocate(maximumBuffers: blocks.count)
            defer { free(list.unsafeMutablePointer) }
            var raws: [UnsafeMutableRawPointer] = []
            defer { raws.forEach { $0.deallocate() } }
            for (i, d) in blocks.enumerated() {
                let raw = UnsafeMutableRawPointer.allocate(byteCount: d.count, alignment: 16)
                d.copyBytes(to: raw.assumingMemoryBound(to: UInt8.self), count: d.count)
                raws.append(raw)
                list[i] = AudioBuffer(mNumberChannels: interleaved ? UInt32(n) : 1, mDataByteSize: UInt32(d.count), mData: raw)
            }
            guard CMSampleBufferSetDataBufferFromAudioBufferList(sb, blockBufferAllocator: nil, blockBufferMemoryAllocator: nil, flags: 0,
                                                                 bufferList: list.unsafePointer) == noErr else { return nil }
            return sb
        }
        let left: [Float] = [0.5, -0.25, 0.125, 0], right: [Float] = [-0.5, 0.25, -0.125, 0.75]
        let want = zip(left, right).flatMap { [$0, $1] }
        for (name, interleaved, int16) in [("非交错 Float32", false, false), ("交错 Int16", true, true)] {
            guard let sb = make(channels: [left, right], interleaved: interleaved, int16: int16) else {
                problems.append("声音格式转换（\(name)）：构造样本失败")
                continue
            }
            guard let (data, rate, ch) = AudioCapture.interleavedPCM(sb) else {
                problems.append("声音格式转换（\(name)）：转换失败")
                continue
            }
            let got = data.withUnsafeBytes { Array($0.bindMemory(to: Float.self)) }
            let tol: Float = int16 ? 0.001 : 0
            if rate != 44_100 || ch != 2 || got.count != want.count || zip(got, want).contains(where: { abs($0 - $1) > tol }) {
                problems.append("声音格式转换（\(name)）结果不对：\(rate) Hz \(ch) 声道 \(got)")
            }
        }
    }

    /// 启动时清理复制粘贴文件缓存：一天前的目录删掉，新的留着
    static func checkClipboardCacheCleanup() {
        let fm = FileManager.default
        let root = ClipboardFiles.cacheRoot
        let old = root.appendingPathComponent("1795000000000-123456", isDirectory: true)
        let fresh = root.appendingPathComponent("1795225692000-654321", isDirectory: true)
        for d in [old, fresh] {
            try? fm.createDirectory(at: d, withIntermediateDirectories: true)
            try? Data("x".utf8).write(to: d.appendingPathComponent("a.txt"))
        }
        let past = Date().addingTimeInterval(-2 * 86_400)
        try? fm.setAttributes([.creationDate: past, .modificationDate: past], ofItemAtPath: old.path)
        let removed = ClipboardFiles.cleanStale(in: root)
        if removed != 1 || fm.fileExists(atPath: old.path) || !fm.fileExists(atPath: fresh.path) {
            problems.append("复制粘贴文件缓存清理不对：删了 \(removed) 个，旧目录还在 \(fm.fileExists(atPath: old.path))，新目录还在 \(fm.fileExists(atPath: fresh.path))")
        }
        try? fm.removeItem(at: fresh)
    }

    /// 远程桌面复制粘贴文件：本机复制 → 对方（进度条、取消、完成提示，同一次复制不重复推）；
    /// 对方复制 → 下载到缓存目录并放进本机剪贴板（同一批不重复下载，推过去的暂存文件不再拉回来）；失败显示原因；文字照旧同步
    static func checkFileClip(_ r: RemoteDesktopModel, prefix: String, window: NSWindow, host: NSHostingView<RootView>) {
        guard let session = r.peer?.session as? MockPeerSession, let home = fakeHome else {
            problems.append("复制粘贴文件：没有模拟会话")
            return
        }
        if !r.fileClipSupported { problems.append("复制粘贴文件：模拟的 Windows 设备应支持") }
        let fm = FileManager.default
        let pb = ClipboardFiles.pasteboard
        let report = home.appendingPathComponent("report-2026Q3.pdf")
        let notes = home.appendingPathComponent("deploy-notes.md")
        let held = home.appendingPathComponent("Desktop/assets-hold.zip")
        let failing = home.appendingPathComponent("Desktop/upload-fail.bin")
        try? Data(count: 2_000_000).write(to: held)
        try? Data(count: 300_000).write(to: failing)
        func failed() -> Bool { if case .failed = r.fileClip?.state { return true }; return false }

        // 1. 进度条：其中一项名字带 hold，模拟传输停在 45%
        ClipboardFiles.write([report, home.appendingPathComponent("screenshot.png"), held], to: pb)
        r.pollClipboard()
        wait("复制文件进度到 45%") { r.fileClip?.percent == 45 }
        let heldTransfer = r.clipTransfer as? MockClipboardTransfer
        if r.fileClip?.direction != .push || r.fileClip?.names.count != 3 { problems.append("复制粘贴文件：进度条内容不对 \(String(describing: r.fileClip))") }
        let firstId = r.fileClip?.id
        r.pollClipboard()
        if r.fileClip?.id != firstId { problems.append("复制粘贴文件：同一次复制又推了一遍") }
        shot("\(prefix)-46-remote-files-pushing", app(r), window: window, host: host, settle: 0.2)
        r.cancelFileClip()
        pump(0.05)
        if r.fileClip != nil || heldTransfer?.cancelled != true { problems.append("复制粘贴文件：取消后进度条没收起或传输没停") }

        // 2. 完成提示：对方剪贴板变成暂存区里的这两项
        ClipboardFiles.write([report, notes], to: pb)
        r.pollClipboard()
        wait("复制文件到对方") { r.fileClip?.state == .done || failed() }
        if case .files(let staged)? = session.remoteClipboard {
            let names = staged.map { ($0 as NSString).lastPathComponent }
            if names != ["report-2026Q3.pdf", "deploy-notes.md"] || !staged.allSatisfy({ $0.hasPrefix("/C:/Users/alice/.lemon-galaxy/clipboard/") }) {
                problems.append("复制粘贴文件：对方剪贴板里的暂存路径不对 \(staged)")
            }
        } else {
            problems.append("复制粘贴文件：推送完成后对方剪贴板不是文件")
        }
        shot("\(prefix)-47-remote-files-pushed", app(r), window: window, host: host, settle: 0.2)
        // 推过去的暂存文件不再拉回来
        let pushedId = r.fileClip?.id
        r.pullRemoteClipboard()
        pump(0.15)
        if r.fileClip?.id != pushedId { problems.append("复制粘贴文件：推过去的文件又被拉回本机") }
        r.dismissFileClip()

        // 3. 对方复制了文件 → 下载到缓存目录 → 本机剪贴板
        let remote = MockClipboardTransfer.remoteSampleFiles(device: session.device, zh: session.client.zh)
        session.remoteClipboard = .files(paths: remote)
        r.pullRemoteClipboard()
        wait("从对方复制文件") { (r.fileClip?.state == .done && r.fileClip?.direction == .pull) || failed() }
        let urls = ClipboardFiles.fileURLs(on: pb)
        let root = ClipboardFiles.cacheRoot.resolvingSymlinksInPath().path
        if urls.map(\.lastPathComponent) != remote.map({ ($0 as NSString).lastPathComponent })
            || !urls.allSatisfy({ fm.fileExists(atPath: $0.path) && $0.resolvingSymlinksInPath().path.hasPrefix(root + "/") })
            || !fm.fileExists(atPath: urls.last?.appendingPathComponent("README.md").path ?? "/nonexistent") {
            problems.append("复制粘贴文件：本机剪贴板里的文件不对 \(urls.map(\.path))")
        }
        shot("\(prefix)-48-remote-files-pulled", app(r), window: window, host: host, settle: 0.2)
        let pulledId = r.fileClip?.id
        r.pullRemoteClipboard()
        pump(0.15)
        if r.fileClip?.id != pulledId { problems.append("复制粘贴文件：同一批文件又下载了一遍") }
        // 本机剪贴板是刚放进去的：不当成本机复制再推回去
        r.pollClipboard()
        if r.fileClip?.id != pulledId { problems.append("复制粘贴文件：拉下来的文件又被推回对方") }
        r.dismissFileClip()

        // 4. 失败：名字带 fail 的在 60% 处失败，显示原因
        ClipboardFiles.write([failing], to: pb)
        r.pollClipboard()
        wait("复制文件失败") { failed() }
        shot("\(prefix)-49-remote-files-failed", app(r), window: window, host: host, settle: 0.2)
        r.dismissFileClip()

        // 5. 文字照旧同步
        pb.clearContents()
        pb.setString("selftest", forType: .string)
        r.pollClipboard()
        wait("文字同步到对方") { if case .text(let t)? = session.remoteClipboard { return t == "selftest" }; return false }
        if r.fileClip != nil || r.lastRemoteFiles != nil { problems.append("复制粘贴文件：同步文字后状态没复位") }
    }

    private static func app(_ r: RemoteDesktopModel) -> AppModel { r.app! }

    static func checkVirtualDisplay(_ r: RemoteDesktopModel, prefix: String, dark: Bool, window: NSWindow, host: NSHostingView<RootView>) {
        let app = app(r)
        if r.virtualMax != 3 { problems.append("模拟的 Windows 设备应支持 3 块虚拟屏：\(r.virtualMax)") }
        shotView("\(prefix)-45a-remote-virtual-display", app, dark: dark, size: CGSize(width: 340, height: 340)) {
            RemoteVirtualDisplayPanel(tab: r)
        }
        let before = r.displays.map(\.id)
        let primary = r.displayId
        var added: Bool?
        Task { added = await r.addVirtualDisplay(VirtualDisplayMode(width: 2560, height: 1440, refresh: 60)) }
        wait("加虚拟屏") { added != nil && !r.virtualBusy }
        guard added == true, let id = r.myVirtual.keys.first else {
            problems.append("加虚拟屏失败：\(app.toast?.text ?? "无提示")")
            return
        }
        if r.displays.count != before.count + 1 || before.contains(id) || r.displays.first(where: { $0.id == id })?.isVirtual != true {
            problems.append("加虚拟屏后屏幕列表不对：\(r.displays.map(\.id))")
        }
        if r.displayId != id { problems.append("加虚拟屏后应切到新屏") }
        wait("虚拟屏画面解码", timeout: 8) { r.phase == .streaming }
        shot("\(prefix)-45b-remote-virtual-display-added", app, window: window, host: host, settle: 0.2)
        shotView("\(prefix)-45c-remote-virtual-display-mine", app, dark: dark, size: CGSize(width: 340, height: 420)) {
            RemoteVirtualDisplayPanel(tab: r)
        }
        app.toast = nil
        Task { await r.removeVirtualDisplay(id) }
        wait("删虚拟屏") { !r.virtualBusy && r.myVirtual.isEmpty }
        if r.displays.contains(where: { $0.id == id }) { problems.append("删掉的虚拟屏仍在列表里") }
        if r.displayId != primary { problems.append("删掉正在看的虚拟屏后应切回主屏") }
        if app.toast?.isError != false { problems.append("删虚拟屏后应提示已删掉：\(app.toast?.text ?? "无提示")") }
        wait("切回主屏后画面解码", timeout: 8) { r.phase == .streaming }
        app.toast = nil
    }

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
    /// AI 页面：新对话、引用、不可用的四种提示、use_target 确认、危险命令确认、提问、要密码、图形界面目标、@ 候选
    static func checkAI(prefix: String, zh: Bool, app: AppModel, window: NSWindow, host: NSHostingView<RootView>) {
        let ai = app.ai
        app.selectHome(.ai)
        ai.newChat()
        ai.loadModels()
        wait("AI 模型列表") { ai.models != nil }
        guard let first = ai.current else { problems.append("AI 页面没有当前对话"); return }
        if first.modelId.isEmpty { problems.append("新对话没有默认模型") }
        if first.mode != .confirmDangerous { problems.append("新对话默认档位应为危险操作确认：\(first.mode)") }
        shot("\(prefix)-50-ai-new", app, window: window, host: host)

        // 不可用的四种状态
        let saved = ai.models
        ai.pinModels = true
        ai.models = nil
        ai.modelsError = nil
        shot("\(prefix)-50-ai-state-loading", app, window: window, host: host)
        ai.modelsError = zh ? "连接服务器超时" : "Timed out connecting to the server"
        if ai.availability != .failed(ai.modelsError!) { problems.append("加载失败状态不对") }
        shot("\(prefix)-50-ai-state-failed", app, window: window, host: host)
        ai.modelsError = nil
        ai.models = AiModels(enabled: false, maxMode: .fullTrust, items: [])
        if ai.availability != .disabled { problems.append("没开通 AI 的状态不对") }
        shot("\(prefix)-50-ai-state-disabled", app, window: window, host: host)
        ai.models = AiModels(enabled: true, maxMode: .confirmAll, items: [])
        if ai.availability != .noModels { problems.append("没有模型的状态不对") }
        if first.canSend { problems.append("没有模型时不应能发送") }
        shot("\(prefix)-50-ai-state-nomodels", app, window: window, host: host)
        ai.models = saved
        ai.pinModels = false

        // 输入框：引用标签、@ 候选
        first.addRef(ai.ref(for: AgentTarget(kind: .device, id: "dev-build")))
        first.addRef(ContextRef(kind: .folder, targetId: "dev-build", id: "/data/backups", label: "backups"))
        first.addRef(ContextRef(kind: .snippet, targetId: "", id: "n-docker", label: zh ? "清理 Docker" : "Docker cleanup"))
        first.addRef(first.refs[0])
        if first.refs.count != 3 { problems.append("同一引用不应重复：\(first.refs.count)") }
        first.text = zh ? "/data 快满了，帮我看看是什么占的空间，清理一下旧备份。 @bu" : "/data is almost full. Find out what uses the space and clean up old backups. @bu"
        let ns = first.text as NSString
        let at = ns.range(of: "@bu", options: .backwards)
        first.mention = MentionState(range: at, query: "bu", caret: CGPoint(x: 280, y: 18))
        if !ai.candidates("bu").contains(where: { $0.ref.id == "dev-build" }) { problems.append("@ 候选里找不到 build-server-01") }
        if ai.candidates("").contains(where: { $0.ref.id == "dev-self" || app.device($0.ref.id)?.kind == .mobile }) { problems.append("@ 候选不应含本机与手机") }
        shot("\(prefix)-51-ai-composer", app, window: window, host: host)
        first.pickMention(ai.ref(for: AgentTarget(kind: .sshHost, id: "s-dbp")))
        if first.text.contains("@bu") || first.refs.count != 4 { problems.append("选中 @ 候选后应删掉 @ 文字并加引用") }
        first.removeRef(first.refs[3])

        // 发出第一条消息才建对话；引用随消息交给内核
        first.send()
        wait("AI 建对话") { first.conversationId != nil }
        if let m = first.conversation?.messages().first, case .user(_, let refs) = m {
            if refs.count != 3 { problems.append("引用没有随消息交给内核：\(refs.count)") }
        } else { problems.append("内核里没有用户消息") }
        wait("AI 确认卡片") { first.hasPendingApproval }
        pump(0.2)
        shot("\(prefix)-52-ai-approval", app, window: window, host: host, settle: 0.3)
        if let card = first.items.compactMap({ if case .tool(let t) = $0, t.decision == .pending { return t }; return nil }).first {
            first.decide(card, .allowOnce)
            wait("AI 提问") { first.items.contains { if case .ask = $0 { return true }; return false } }
            if let ask = first.items.compactMap({ if case .ask(let a) = $0 { return a }; return nil }).first {
                first.answer(ask, ask.options[0])
                wait("AI 请求密码") { first.pendingSecret != nil }
            }
        }
        shot("\(prefix)-53-ai-secret", app, window: window, host: host, settle: 0.3)
        if let s = first.pendingSecret { first.cancelSecret(s) }
        wait("AI 对话结束") { !first.running }
        if !ai.rows.contains(where: { $0.id == first.conversationId }) { problems.append("对话列表里没有刚建的对话") }

        // 没引用设备：先查设备，再请用户同意连接（use_target）
        ai.newChat()
        guard let chat = ai.current, chat !== first else { problems.append("新对话没有新建"); return }
        chat.text = zh ? "帮我把记事本里的内容保存一下" : "Save the Notepad content for me"
        chat.setMode(.confirmAll)
        if let vl = ai.models?.items.first(where: { $0.vision }) { chat.setModel(vl) }
        chat.send()
        wait("AI 连接确认") { chat.items.contains { if case .tool(let t) = $0 { return t.isUseTarget && t.decision == .pending }; return false } }
        shot("\(prefix)-54-ai-use-target", app, window: window, host: host, settle: 0.3)
        if let card = chat.items.compactMap({ if case .tool(let t) = $0, t.isUseTarget { return t }; return nil }).first {
            chat.decide(card, .allowOnce)
            wait("AI 图形界面确认") { chat.items.contains { if case .tool(let t) = $0 { return !t.isUseTarget && t.decision == .pending }; return false } }
        }
        shot("\(prefix)-55-ai-gui", app, window: window, host: host, settle: 0.3)
        chat.stop()
        checkAILocalAndPlan(prefix: prefix, zh: zh, app: app, chat: chat, window: window, host: host)
        // 设置记住上次的模型与档位
        if app.settings.aiMode != .confirmAll { problems.append("档位没有记到本机设置") }
        if !app.settings.aiLastModel.hasSuffix(chat.modelId) { problems.append("模型没有记到本机设置") }
        app.settings.aiMode = .confirmDangerous

        // 打开历史对话
        ai.conversations.removeAll { $0 === first }
        if let s = ai.summaries.first(where: { $0.id == first.conversationId }) {
            ai.open(s)
            wait("AI 打开历史对话") { ai.current?.conversation != nil && !(ai.current?.items.isEmpty ?? true) }
            // 展开第一条工具调用：命令、理由、风险原因、输出
            if let t = ai.current?.items.compactMap({ if case .tool(let t) = $0 { return t }; return nil }).first { t.expanded = true }
            shot("\(prefix)-56-ai-history", app, window: window, host: host, settle: 0.3)
        } else { problems.append("历史里找不到刚才的对话") }
        ai.newChat()
    }

    /// 本机目标、任务清单（update_plan）、应用操作确认（use_app）、应用工具行、连续执行暂停后的「继续」
    static func checkAILocalAndPlan(prefix: String, zh: Bool, app: AppModel, chat: AIConversationModel, window: NSWindow, host: NSHostingView<RootView>) {
        let ai = app.ai
        // 本机排在 @ 候选与「设备或主机」第一位，引用为 Device / 空 target_id / "local"
        let local = ai.candidates("").first
        if local?.ref.id != AIHubModel.localId || local?.ref.kind != .device || local?.ref.targetId != "" || local?.online != true {
            problems.append("@ 候选第一项应是本机：\(String(describing: local?.ref))")
        }
        if ai.targetName(AIHubModel.localId) != ai.localRef.label { problems.append("本机的显示名不对") }
        if ai.candidates("build").contains(where: { $0.ref.id == AIHubModel.localId }) { problems.append("过滤后不应再有本机") }
        if PlanCard.parse("[x] 查看磁盘\n[>] 清理日志\n[ ] 重启服务\n").map(\.status) != [.done, .active, .pending] { problems.append("任务清单解析不对") }

        wait("AI 停止") { !chat.running }
        func call(_ id: String, _ tool: String, _ summary: String, _ detail: String, risk: RiskLevel = .read, reasons: [String] = [], reason: String = "") -> ToolCallView {
            ToolCallView(callId: id, tool: tool, targetId: AIHubModel.localId, targetName: ai.localRef.label, summary: summary, detail: detail,
                         risk: risk, riskReasons: reasons, modelReason: reason, image: nil)
        }
        func finished(_ c: ToolCallView, _ summary: String, image: ImageData? = nil) -> ChatItem {
            let card = ToolCardModel(call: c, needsApproval: false)
            card.phase = .finished
            card.result = ToolResultView(ok: true, summary: summary, output: "", image: image, denied: false)
            return .tool(card)
        }
        let user = zh ? "帮我在 Safari 里打开公司内网，看看今天的值班表" : "Open the intranet in Safari and check today's on-call rota"
        chat.items.append(.user(id: "st-u", text: user, refs: [ai.localRef]))
        let plan1 = zh ? "[>] 打开 Safari\n[ ] 进入内网首页\n[ ] 找到值班表\n" : "[>] Open Safari\n[ ] Go to the intranet home page\n[ ] Find the on-call rota\n"
        chat.items.append(finished(call("st-p1", "update_plan", zh ? "任务清单 0/3：打开 Safari" : "Task list 0/3: Open Safari", plan1), "0/3"))
        chat.items.append(finished(call("st-a1", "app_open", zh ? "打开 Safari" : "Open Safari", "Safari"), zh ? "已打开" : "Opened"))
        let shotImg = fakeWindowShot()
        let state = ToolCardModel(call: call("st-a2", "app_state", zh ? "读取 Safari 界面" : "Read the Safari window", "Safari"), needsApproval: false)
        state.phase = .finished
        state.result = ToolResultView(ok: true, summary: zh ? "142 个控件" : "142 elements", output: "[1] AXWindow \"Safari\"\n  [2] AXTextField \"地址\"", image: shotImg, denied: false)
        state.output = state.result?.output ?? ""
        state.expanded = true
        chat.items.append(.tool(state))
        let plan2 = zh ? "[x] 打开 Safari\n[>] 进入内网首页\n[ ] 找到值班表\n" : "[x] Open Safari\n[>] Go to the intranet home page\n[ ] Find the on-call rota\n"
        chat.items.append(finished(call("st-p2", "update_plan", zh ? "任务清单 1/3：进入内网首页" : "Task list 1/3: Go to the intranet home page", plan2), "1/3"))
        chat.items.append(finished(call("st-w", "web_fetch", "https://intranet.example.com/oncall", ""), "200"))
        chat.items.append(finished(call("st-s", "system_info", zh ? "查看系统信息" : "System info", ""), "macOS 26"))
        let useApp = ToolCardModel(call: call("st-ua-app", "use_app", zh ? "AI 想操作「Safari」" : "AI wants to use “Safari”",
                                              zh ? "允许后，本对话里 AI 可以读取「Safari」的界面并在里面点击、输入（不会占用你的鼠标键盘）" : "Once allowed, the agent can read Safari's window and click and type in it during this conversation (without taking over your mouse and keyboard)",
                                              risk: .normal, reasons: [zh ? "本对话第一次操作这个应用" : "First time this conversation uses this app"],
                                              reason: zh ? "要在地址栏输入内网地址" : "Needs to type the intranet address"), needsApproval: true)
        chat.items.append(.tool(useApp))
        chat.contentVersion += 1
        if !chat.isPlanSuperseded(chat.items.compactMap({ if case .tool(let t) = $0, t.id == "st-p1" { return t }; return nil }).first!) { problems.append("前一份任务清单应收起") }
        shot("\(prefix)-57-ai-local-plan-app", app, window: window, host: host, settle: 0.3)
        useApp.decision = .approved
        useApp.phase = .finished
        useApp.result = ToolResultView(ok: true, summary: zh ? "已允许" : "Allowed", output: "", image: nil, denied: false)
        chat.items.append(.error(id: "st-e", text: "已连续执行 40 步，先暂停一下。回复「继续」可以接着做"))
        chat.contentVersion += 1
        shot("\(prefix)-58-ai-continue", app, window: window, host: host, settle: 0.3)
    }

    /// 应用窗口截图的替身：浅色窗口 + 标题栏 + 地址栏
    static func fakeWindowShot() -> ImageData? {
        let w = 640, h = 400
        guard let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                  bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue) else { return nil }
        ctx.setFillColor(CGColor(gray: 0.97, alpha: 1)); ctx.fill(CGRect(x: 0, y: 0, width: w, height: h))
        ctx.setFillColor(CGColor(gray: 0.88, alpha: 1)); ctx.fill(CGRect(x: 0, y: h - 52, width: w, height: 52))
        for (i, c) in [(1.0, 0.37, 0.34), (1.0, 0.74, 0.18), (0.16, 0.79, 0.25)].enumerated() {
            ctx.setFillColor(CGColor(red: c.0, green: c.1, blue: c.2, alpha: 1))
            ctx.fillEllipse(in: CGRect(x: 14 + i * 20, y: h - 32, width: 12, height: 12))
        }
        ctx.setFillColor(CGColor(gray: 1, alpha: 1)); ctx.fill(CGRect(x: 120, y: h - 40, width: 400, height: 26))
        ctx.setFillColor(CGColor(gray: 0.82, alpha: 1))
        for i in 0..<6 { ctx.fill(CGRect(x: 40, y: h - 100 - i * 44, width: [520, 380, 460, 300, 500, 260][i], height: 16)) }
        guard let img = ctx.makeImage(), let data = NSBitmapImageRep(cgImage: img).representation(using: .png, properties: [:]) else { return nil }
        return ImageData(mime: "image/png", data: data, width: UInt32(w), height: UInt32(h))
    }

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
        // RDP 主机经设备转发：底部出现「RDP 转发中」提示条（selftest 不打开远程桌面应用）
        app.selection = "r-win"
        app.openRdp("r-win")
        wait("RDP 转发") { !app.rdpForwards.isEmpty }
        shot("\(prefix)-17-hosts-rdp-forward", app, window: window, host: host)
        app.closeAllRdp()
        app.selection = nil

        // 远程开机：离线且上报过网卡地址的设备才有按钮
        if !app.canWake("dev-reception") { problems.append("前台电脑离线且同网段有在线设备，应能远程开机") }
        if app.canWake("dev-nas") { problems.append("NAS 没上报网卡地址，不应显示远程开机") }
        if app.canWake("dev-build") || app.canWake("dev-self") || app.canWake("dev-phone") { problems.append("在线设备、本机、手机不应显示远程开机") }
        app.selection = "dev:dev-reception"
        shot("\(prefix)-18-hosts-device-wake", app, window: window, host: host)
        app.modal = .connectChooser(deviceId: "dev-reception")
        shot("\(prefix)-18b-hosts-wake-chooser", app, window: window, host: host)
        app.modal = nil
        app.wake("dev-reception")
        wait("远程开机") { !app.waking.contains("dev-reception") && app.toast != nil }
        if app.toast?.isError != false || app.toast?.text.contains("OFFICE-PC-07") != true {
            problems.append("远程开机应提示经由的设备：\(app.toast?.text ?? "无提示")")
        }
        shot("\(prefix)-19-hosts-wake-sent", app, window: window, host: host)
        app.toast = nil
        app.selection = nil

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

        // SSH 主机密钥：web-02 没有记录过，走模拟内核 → SecretPromptBridge → 主窗口卡片的完整路径
        app.openTerminal(sshHostId: "s-web2")
        if wait("SSH 主机密钥确认", { if case .hostKey = app.modal { return true }; return false }) {
            shot("\(prefix)-22-ssh-hostkey", app, window: window, host: host)
            if case .hostKey(let r)? = app.modal { r.respond(true); app.modal = nil }
            if let t = app.activeTab as? TerminalTabModel { wait("SSH 终端（新主机）打开") { t.phase == .running } }
            if !app.knownHosts.contains(where: { if case .knownHost(let h, _, _, _) = $0.data { return h == "10.0.1.22" }; return false }) {
                problems.append("确认后的主机密钥没有存进加密库")
            }
        }
        // 主机密钥变化（警告样式）与密码输入：直接放卡片截图
        app.modal = .hostKey(HostKeyRequest(title: "web-01", host: "10.0.1.21", port: 22, keyType: "ssh-ed25519",
                                            fingerprint: MockGalaxyClient.fakeFingerprint("changed-10.0.1.21"), changed: true) { _ in })
        shot("\(prefix)-23-ssh-hostkey-changed", app, window: window, host: host)
        app.modal = .secretPrompt(SecretPromptRequest(title: "db-primary", prompt: zh ? "postgres@10.0.2.10 的密码：" : "Password for postgres@10.0.2.10:",
                                                      echo: false) { _ in })
        shot("\(prefix)-24-ssh-password", app, window: window, host: host)
        app.modal = nil
        // 已连接的 SSH 主机详情：连接路径来自内核报告的 via
        app.selection = "s-web1"
        app.selectHome(.hosts)
        shot("\(prefix)-25-hosts-ssh-detail", app, window: window, host: host)
        app.selection = nil

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
            // 对方支持声音、设置默认打开：画面开起来后自动开始收听
            if !r.audioSupported { problems.append("模拟的 Windows 设备应支持声音") }
            wait("远端声音自动开始") { r.audioOn && !r.audioBusy }
            pump(0.3)
            r.measuredFps = 30
            r.measuredKbps = 8_400
        }
        shot("\(prefix)-40-remote-desktop", app, window: window, host: host, settle: 0.2)
        // 隐私屏开关（对端能力里有 privacy 才显示）
        if let r = app.activeTab as? RemoteDesktopModel {
            if !r.privacySupported { problems.append("模拟的 Windows 设备应支持隐私屏") }
            r.setPrivacy(true)
            wait("隐私屏开启") { r.privacyOn && !r.privacyBusy }
            shot("\(prefix)-41-remote-privacy", app, window: window, host: host, settle: 0.2)
            app.toast = nil
        }
        // 声音开关：关掉（记进设置）、截图、再打开并调音量
        if let r = app.activeTab as? RemoteDesktopModel {
            r.setAudio(false)
            if r.audioOn || r.audio != nil || settings.remoteAudio { problems.append("关闭声音后状态不对") }
            shot("\(prefix)-42-remote-audio-off", app, window: window, host: host, settle: 0.2)
            r.setAudio(true)
            wait("重新打开声音") { r.audioOn && !r.audioBusy }
            if !settings.remoteAudio { problems.append("打开声音没有记进设置") }
            r.setVolume(0.35)
            if let m = r.audio as? MockAudioPlayback {
                if abs(m.volume - 0.35) > 0.001 { problems.append("音量没有传给播放对象：\(m.volume)") }
            } else {
                problems.append("模拟内核应返回 MockAudioPlayback")
            }
            shotView("\(prefix)-43-remote-audio-volume", app, dark: dark, size: CGSize(width: 280, height: 86)) {
                RemoteAudioVolumePanel(tab: r)
            }
            r.setVolume(1)
        }
        // 录像：正在收听声音，with_audio 应为真；录制中红点 + 计时，停止后提示保存位置
        if let r = app.activeTab as? RemoteDesktopModel {
            if !r.canRecord { problems.append("画面开着时应能录像") }
            r.startRecording()
            wait("开始录像") { r.recording != nil && !r.recordBusy }
            if let rec = r.recording {
                let mock = (rec.channel as? MockDisplayChannel)?.lastRecording
                if !rec.withAudio || mock?.withAudio != true { problems.append("正在收听声音时录像应带声音") }
                if mock?.path != rec.path { problems.append("录像路径没有传给内核") }
                let name = (rec.path as NSString).lastPathComponent
                if !rec.path.hasPrefix(RemoteDesktopModel.recordingDir.path + "/") || !name.hasPrefix("LemonGalaxy-") || !name.hasSuffix(".mp4") {
                    problems.append("录像文件名或位置不对：\(rec.path)")
                }
                if !FileManager.default.fileExists(atPath: RemoteDesktopModel.recordingDir.path) { problems.append("录像目录没有建") }
                r.recording?.startedMs = Clock.nowMs - 83_000
                (rec.channel as? MockDisplayChannel)?.backdateRecording(by: 83)
            }
            shot("\(prefix)-44-remote-recording", app, window: window, host: host, settle: 0.2)
            r.stopRecording()
            if r.recording != nil { problems.append("停止后仍在录像") }
            if app.toast?.action == nil || app.toast?.isError == true { problems.append("停止录像后应提示保存位置并能在访达中显示：\(app.toast?.text ?? "无提示")") }
            shot("\(prefix)-45-remote-recording-saved", app, window: window, host: host, settle: 0.2)
            app.toast = nil
        }

        // 虚拟屏：有控制权限且对端支持才有入口；加一块后列表刷新并切过去，删掉正在看的切回主屏
        if let r = app.activeTab as? RemoteDesktopModel {
            checkVirtualDisplay(r, prefix: prefix, dark: dark, window: window, host: host)
        }

        // 复制粘贴文件
        if let r = app.activeTab as? RemoteDesktopModel {
            checkFileClip(r, prefix: prefix, window: window, host: host)
        }

        // 6. AI 智能体（聊天式，docs/ai-chat-ui.md）
        checkAI(prefix: prefix, zh: zh, app: app, window: window, host: host)

        // 7. 片段
        app.selectHome(.snippets)
        app.selectedSnippet = "n-docker"
        shot("\(prefix)-60-snippets", app, window: window, host: host)

        // 7b. 端口映射：「生产库 Postgres」设了登录后自动开启，登录上线时已经开着
        app.selectHome(.forwards)
        wait("自动开启端口映射") { app.forwards["p-pg"] != nil }
        if let f = app.forwards["p-pg"], f.port != 15432 { problems.append("固定本机端口的映射端口不对：\(f.port)") }
        if app.forwards["p-grafana"] != nil { problems.append("没设自动开启的映射不应自动开") }
        app.startForward("p-grafana")
        wait("开启端口映射（自动端口）") { app.forwards["p-grafana"] != nil }
        // 经由的设备离线：开启失败，记下原因
        app.startForward("p-nas")
        wait("经由离线设备的映射报错") { app.forwardErrors["p-nas"] != nil && !app.forwardBusy.contains("p-nas") }
        if app.forwards["p-nas"] != nil { problems.append("经由离线设备的映射不应开成功") }
        // 经由 SSH 主机：主机密钥确认走与 SSH 连接相同的卡片
        app.startForward("p-router")
        if wait("端口映射的主机密钥确认", { if case .hostKey = app.modal { return true }; return false }) {
            if case .hostKey(let r)? = app.modal { r.respond(true); app.modal = nil }
            wait("经由 SSH 主机的映射") { app.forwards["p-router"] != nil }
        }
        app.toast = nil
        app.selectedForward = nil
        shot("\(prefix)-62-forwards", app, window: window, host: host)
        app.selectedForward = "p-nas"
        shot("\(prefix)-63-forward-detail-error", app, window: window, host: host)
        app.selectedForward = "p-pg"
        shot("\(prefix)-63-forward-detail", app, window: window, host: host)
        app.editForward("p-router")
        if case .portForwardEditor(let d)? = app.modal {
            if d.via != "s:s-router" || d.localPort != "8080" || d.remotePort != "80" || d.sort != 2 || d.autoStart { problems.append("编辑草稿读回不对") }
        } else { problems.append("编辑端口映射应弹出编辑卡片") }
        app.editForward(nil)
        if case .portForwardEditor(let d)? = app.modal, d.sort != 4 || !d.isNew { problems.append("新建端口映射应排在最后：\(d.sort)") }
        app.editForward("p-pg")
        shot("\(prefix)-64-forward-editor", app, window: window, host: host)
        let bad = PortForwardDraft()
        bad.label = zh ? "内网数据库" : "Internal DB"
        bad.remoteHost = "db internal"
        bad.remotePort = "70000"
        bad.localPort = "abc"
        bad.showErrors = true
        if bad.build() != nil || bad.problems.count != 4 { problems.append("端口映射校验不对：\(bad.problems)") }
        app.modal = .portForwardEditor(bad)
        shot("\(prefix)-65-forward-editor-invalid", app, window: window, host: host)
        app.modal = nil
        // 新建 → 保存 → 删除（删除前先停）
        let draft = PortForwardDraft()
        draft.via = "d:dev-imac"
        draft.remoteHost = "127.0.0.1"
        draft.remotePort = "5900"
        var newId: String?
        if let e = draft.build() {
            if case .portForward(let label, let vd, let vs, let lp, _, _, _, _) = e.data, label != "127.0.0.1:5900" || vd != "dev-imac" || vs != nil || lp != 0 {
                problems.append("新建端口映射组装不对：\(e.data)")
            }
            Task { newId = await app.save(e)?.id }
            wait("保存端口映射") { newId != nil }
        } else { problems.append("合法的端口映射草稿组装失败：\(draft.problems)") }
        if let id = newId {
            app.startForward(id)
            wait("开启新建的映射") { app.forwards[id] != nil }
            let handle = app.forwards[id]?.handle as? MockPortForward
            Task { await app.delete(id) }
            wait("删除端口映射") { app.forwardRule(id) == nil && app.forwards[id] == nil }
            if handle?.stopped != true { problems.append("规则删掉后映射应停止") }
        }
        app.toast = nil

        // 8. 本机被控
        if let mock = (app.client as? MockGalaxyClient)?.mockHost {
            mock.simulateSession(user: "alice", device: "OFFICE-PC-07", ai: true)
        }
        wait("被控会话") { !app.host.status.sessions.isEmpty }
        pump(0.4)
        app.selectHome(.local)
        app.localPage = .overview
        app.host.setLockOnEnd(true)
        if !app.host.lockOnEnd || (app.client as? MockGalaxyClient)?.mockHost.lockOnEnd() != true { problems.append("远程结束后自动锁屏没有设进被控服务") }
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

        // 9. 设置（先检查更新：关于页显示新版本，设置图标带小红点）
        app.selectHome(.settings)
        var checked = false
        Task { await app.update.check(manual: true); checked = true }
        wait("检查更新") { checked }
        if app.update.available == nil { problems.append("模拟内核应返回新版本") }
        app.modal = nil
        for page in SettingsPage.allCases {
            app.settingsPage = page
            shot("\(prefix)-8\(SettingsPage.allCases.firstIndex(of: page)!)-settings-\(page.rawValue)", app, window: window, host: host)
        }

        // 更新卡片：有新版本、下载中、下载完成
        app.settingsPage = .about
        app.modal = .update
        shot("\(prefix)-87-update", app, window: window, host: host)
        if let info = app.update.available {
            app.update.state = .downloading(info, done: info.size * 37 / 100, total: info.size)
            shot("\(prefix)-88-update-downloading", app, window: window, host: host)
            app.update.state = .downloaded(info, path: "/Users/alice/Downloads/" + info.fileName)
            shot("\(prefix)-89-update-downloaded", app, window: window, host: host)
        }
        app.modal = nil
        app.update.reset()

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

        // 录像中关掉远程桌面标签：录像自动结束并提示保存位置
        if let r = app.tabs.compactMap({ $0 as? RemoteDesktopModel }).first {
            app.activeTabId = r.id
            r.startRecording()
            wait("再次开始录像") { r.recording != nil && !r.recordBusy }
            let ch = r.recording?.channel
            app.toast = nil
            app.closeTab(r.id)
            if ch?.isRecording() != false || r.recording != nil { problems.append("关标签后录像没有结束") }
            if app.toast?.action == nil { problems.append("关标签结束录像后应提示保存位置") }
            app.toast = nil
        }

        // 退出登录：运行中的端口映射全部停掉
        let running = app.forwards.values.compactMap { $0.handle as? MockPortForward }
        if running.isEmpty { problems.append("此时应有运行中的端口映射") }
        app.stopAllForwards()
        if running.contains(where: { !$0.stopped }) || !app.forwards.isEmpty { problems.append("全部停止后仍有映射在运行") }

        // 收尾
        app.closeAllTabs()
        pump(0.2)
        window.contentView = nil
    }
}
