import AppKit
import ApplicationServices
import ScreenCaptureKit
import Carbon.HIToolbox

/// AI 应用操作（docs/computer-use.md）的 macOS 实现：不抢前台、不动真实鼠标键盘，操作某个应用的窗口。
///   - 观察：辅助功能控件树（按应用缓存控件句柄，编号到下一次 get_app_state 前有效）+ 只截目标窗口
///   - 点击：优先 AXPress；只有坐标时用进程定向鼠标事件（NSEvent(windowNumber:) → CGEvent，字段 91/92、
///     私有 CGEventSetWindowLocation，CGEvent.postToPid）；Chromium 系另用 SkyLight 的 SLEventPostToPid 并临时给目标进程
///     一个只对它自己生效的「激活」记录（SLPSPostEventRecordTo），不碰真实前台应用
///   - 文字：焦点控件可写 AXSelectedText 时直接插入，否则 Unicode 键盘事件 postToPid
///   - 私有接口一律 dlopen/dlsym 运行时探测，缺了就退到公开接口
/// 主控端（本机 AI）与被控端（远程 AI，经会话代理）共用。调用在内核的后台线程上，可以阻塞；一次只处理一个请求。
final class ComputerUse: @unchecked Sendable {
    static let shared = ComputerUse()

    /// 一次只处理一个请求（控件缓存、AI 光标都是全局的）
    private let serial = NSLock()
    private let lock = NSLock()
    private var snapshots: [pid_t: Snapshot] = [:]
    private var chromiumCache: [pid_t: Bool] = [:]

    /// 最近一次 get_app_state 的结果（按应用）
    struct Snapshot {
        var elements: [AXUIElement]
        var labels: [String]
        var windowId: CGWindowID
        /// 截图时窗口的位置大小（CG 全局坐标，点）
        var windowFrame: CGRect
        /// 截图像素 / 点
        var scale: CGFloat
    }

    struct Failure: Error { let message: String }

    /// 读类动作不需要 AI 光标（被控端会话代理据此决定要不要起 AppKit）
    static func needsCursor(_ requestJson: String) -> Bool {
        guard let d = try? JSONSerialization.jsonObject(with: Data(requestJson.utf8)) as? [String: Any],
              let a = d["action"] as? String else { return false }
        return !["list_apps", "get_app_state", "end"].contains(a)
    }

    func handle(_ requestJson: String) -> ComputerUseReply {
        serial.lock()
        defer { serial.unlock() }
        do {
            guard let req = try? JSONSerialization.jsonObject(with: Data(requestJson.utf8)) as? [String: Any] else {
                throw Failure(message: "请求不是合法的 JSON 对象")
            }
            let action = (req["action"] as? String ?? "").lowercased()
            if action == "end" {
                AICursorOverlay.shared.hide()
                return ComputerUseReply(text: "已结束，AI 光标已收起", image: nil)
            }
            guard AXIsProcessTrusted() else {
                throw Failure(message: "没有「辅助功能」授权，无法读取或操作应用。请在 系统设置 > 隐私与安全性 > 辅助功能 里允许 LemonGalaxy")
            }
            if action == "list_apps" { return ComputerUseReply(text: listApps(), image: nil) }
            guard let q = Self.string(req["app"]), !q.isEmpty else { throw Failure(message: "缺少参数 app") }
            if action == "open_app" { return try openApp(q) }
            let app = try resolveApp(q)
            switch action {
            case "get_app_state":
                return try getAppState(app, window: Self.int(req["window"]), maxNodes: Self.int(req["max_nodes"]) ?? 400)
            case "click": return try click(app, req)
            case "set_value": return try setValue(app, req)
            case "type_text": return try typeText(app, req)
            case "press_key": return try pressKey(app, req)
            case "scroll": return try scroll(app, req)
            case "perform_action": return try performAction(app, req)
            default:
                throw Failure(message: "不支持的动作「\(action)」。可用：list_apps、get_app_state、click、set_value、type_text、press_key、scroll、perform_action、open_app、end")
            }
        } catch let f as Failure {
            return ComputerUseReply(text: "错误：" + f.message, image: nil)
        } catch {
            return ComputerUseReply(text: "错误：\(error)", image: nil)
        }
    }

    // MARK: 应用

    private func listApps() -> String {
        let counts = Self.windowCounts()
        let front = NSWorkspace.shared.frontmostApplication?.processIdentifier
        var lines: [String] = []
        for app in NSWorkspace.shared.runningApplications where app.activationPolicy == .regular && !app.isTerminated {
            if Self.forbiddenReason(app) != nil { continue }
            let n = counts[app.processIdentifier] ?? 0
            let id = app.bundleIdentifier ?? app.executableURL?.lastPathComponent ?? "?"
            var line = "- \(app.localizedName ?? id)（\(id)，pid \(app.processIdentifier)）窗口 \(n)"
            if app.processIdentifier == front { line += "，前台" }
            if app.isHidden { line += "，已隐藏" }
            lines.append(line)
        }
        return "运行中的应用（\(lines.count) 个）：\n" + lines.joined(separator: "\n")
    }

    /// 每个进程的普通窗口数（第 0 层、不太小；含其他桌面空间与最小化的）
    private static func windowCounts() -> [pid_t: Int] {
        var out: [pid_t: Int] = [:]
        guard let list = CGWindowListCopyWindowInfo([.optionAll, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else { return out }
        for w in list {
            guard (w[kCGWindowLayer as String] as? Int) == 0,
                  let pid = w[kCGWindowOwnerPID as String] as? pid_t,
                  let b = w[kCGWindowBounds as String] as? NSDictionary, let r = CGRect(dictionaryRepresentation: b),
                  r.width >= 60, r.height >= 40 else { continue }
            let onScreen = w[kCGWindowIsOnscreen as String] as? Bool ?? false
            let named = !((w[kCGWindowName as String] as? String) ?? "").isEmpty
            if onScreen || named { out[pid, default: 0] += 1 }
        }
        return out
    }

    /// 按名称 / Bundle ID / 可执行文件名 / pid:N 找运行中的应用（大小写不敏感，先精确再前缀）
    private func resolveApp(_ q: String) throws -> NSRunningApplication {
        let key = q.trimmingCharacters(in: .whitespaces).lowercased()
        var found: NSRunningApplication?
        if key.hasPrefix("pid:"), let pid = pid_t(key.dropFirst(4).trimmingCharacters(in: .whitespaces)) {
            found = NSRunningApplication(processIdentifier: pid)
        } else {
            let apps = NSWorkspace.shared.runningApplications.filter { !$0.isTerminated }
                .sorted { ($0.activationPolicy == .regular ? 0 : 1) < ($1.activationPolicy == .regular ? 0 : 1) }
            func names(_ a: NSRunningApplication) -> [String] {
                [a.localizedName, a.bundleIdentifier, a.executableURL?.lastPathComponent,
                 a.bundleURL?.deletingPathExtension().lastPathComponent].compactMap { $0?.lowercased() }
            }
            found = apps.first { names($0).contains(key) } ?? apps.first { names($0).contains { $0.hasPrefix(key) } }
        }
        guard let app = found else {
            throw Failure(message: "没有找到运行中的应用「\(q)」。先用 list_apps 看有哪些，或用 open_app 启动")
        }
        if let why = Self.forbiddenReason(app) { throw Failure(message: why) }
        return app
    }

    private static let ownBundlePrefix = "cn.lemonit.galaxy"
    /// 密码管理器与系统认证界面：一律不操作（内核也会拦）
    private static let forbiddenBundles: Set<String> = [
        "com.apple.keychainaccess", "com.apple.Passwords", "com.apple.SecurityAgent", "com.apple.LocalAuthentication.UIAgent",
        "com.apple.loginwindow", "in.sinew.Enpass-Desktop", "com.markmcguill.strongbox.mac",
    ]
    private static let forbiddenPrefixes = [
        "com.agilebits.", "com.1password.", "com.bitwarden.", "com.lastpass.", "com.dashlane.", "org.keepassxc.",
        "com.keepassxc.", "com.enpass.", "com.nordpass.", "me.proton.pass", "com.roboform.", "com.keepersecurity.",
    ]
    private static let forbiddenExecutables: Set<String> = ["securityagent", "coreautha", "loginwindow", "keychain access"]

    static func forbiddenReason(_ app: NSRunningApplication) -> String? {
        forbiddenReason(bundleId: app.bundleIdentifier ?? "", pid: app.processIdentifier,
                        executable: app.executableURL?.lastPathComponent ?? "", name: app.localizedName)
    }

    static func forbiddenReason(bundleId bid: String, pid: pid_t, executable: String, name: String?) -> String? {
        if pid == ProcessInfo.processInfo.processIdentifier || bid.hasPrefix(ownBundlePrefix) {
            return "不能操作 LemonGalaxy 自己"
        }
        if forbiddenBundles.contains(bid) || forbiddenPrefixes.contains(where: { bid.lowercased().hasPrefix($0) })
            || forbiddenExecutables.contains(executable.lowercased()) {
            return "出于安全考虑，不能操作密码管理器或系统认证界面（\(name ?? (bid.isEmpty ? executable : bid))）"
        }
        return nil
    }

    private func openApp(_ q: String) throws -> ComputerUseReply {
        if let running = try? resolveApp(q) {
            return ComputerUseReply(text: "\(running.localizedName ?? q) 已在运行（pid \(running.processIdentifier)），未切到前台", image: nil)
        }
        guard let url = Self.findApplication(q) else { throw Failure(message: "找不到应用「\(q)」") }
        let bid = Bundle(url: url)?.bundleIdentifier ?? ""
        if bid.hasPrefix(Self.ownBundlePrefix) { throw Failure(message: "不能操作 LemonGalaxy 自己") }
        if Self.forbiddenBundles.contains(bid) || Self.forbiddenPrefixes.contains(where: { bid.lowercased().hasPrefix($0) }) {
            throw Failure(message: "出于安全考虑，不能操作密码管理器")
        }
        let cfg = NSWorkspace.OpenConfiguration()
        cfg.activates = false
        cfg.addsToRecentItems = false
        let sem = DispatchSemaphore(value: 0)
        nonisolated(unsafe) var result: NSRunningApplication?
        nonisolated(unsafe) var failure: Error?
        NSWorkspace.shared.openApplication(at: url, configuration: cfg) { app, err in
            result = app
            failure = err
            sem.signal()
        }
        guard sem.wait(timeout: .now() + 10) == .success else { throw Failure(message: "启动「\(q)」超时") }
        if let failure { throw Failure(message: "启动「\(q)」失败：\(failure.localizedDescription)") }
        let name = result?.localizedName ?? url.deletingPathExtension().lastPathComponent
        // 等它出窗口（最多约 3 秒），模型下一步通常就是 get_app_state
        if let pid = result?.processIdentifier {
            for _ in 0..<15 where (Self.windowCounts()[pid] ?? 0) == 0 { Thread.sleep(forTimeInterval: 0.2) }
            return ComputerUseReply(text: "已在后台启动 \(name)（pid \(pid)），未切到前台", image: nil)
        }
        return ComputerUseReply(text: "已在后台启动 \(name)", image: nil)
    }

    private static func findApplication(_ q: String) -> URL? {
        if q.hasPrefix("/") { return FileManager.default.fileExists(atPath: q) ? URL(fileURLWithPath: q) : nil }
        if let u = NSWorkspace.shared.urlForApplication(withBundleIdentifier: q) { return u }
        let want = q.lowercased().hasSuffix(".app") ? q.lowercased() : q.lowercased() + ".app"
        let dirs = ["/Applications", "/System/Applications", "/System/Applications/Utilities", "/Applications/Utilities",
                    NSHomeDirectory() + "/Applications"]
        for d in dirs {
            guard let items = try? FileManager.default.contentsOfDirectory(atPath: d) else { continue }
            if let hit = items.first(where: { $0.lowercased() == want }) { return URL(fileURLWithPath: d).appendingPathComponent(hit) }
        }
        return nil
    }

    // MARK: 观察

    private func getAppState(_ app: NSRunningApplication, window: Int?, maxNodes: Int) throws -> ComputerUseReply {
        let pid = app.processIdentifier
        let appEl = AXUIElementCreateApplication(pid)
        AXUIElementSetMessagingTimeout(appEl, 2)
        // Electron / Chromium：打开它们的辅助功能树（不设的话网页内容是空的）
        AXUIElementSetAttributeValue(appEl, "AXManualAccessibility" as CFString, kCFBooleanTrue)
        let (win, index, total) = try pickWindow(appEl, app: app, index: window)
        let frame = AX.frame(win) ?? .zero
        let wid = Self.windowId(of: win, pid: pid, frame: frame)
        // 截图（没有屏幕录制授权时没有图，坐标仍按同样的比例换算）
        let shot = wid.flatMap { Self.captureWindow($0) }
        let liveFrame = wid.flatMap { Self.currentFrame($0) } ?? frame
        let scale = Self.shotScale(frame: liveFrame, image: shot)
        var result = walkTree(win, windowFrame: liveFrame, scale: scale, maxNodes: max(10, min(maxNodes, 2000)))
        if result.emptyWebArea {
            // Chromium 的网页树懒加载：第一次常常是空的，等一下再取一次
            Thread.sleep(forTimeInterval: 0.8)
            result = walkTree(win, windowFrame: liveFrame, scale: scale, maxNodes: max(10, min(maxNodes, 2000)))
        }
        let snapshot = Snapshot(elements: result.elements, labels: result.labels, windowId: wid ?? 0, windowFrame: liveFrame, scale: scale)
        lock.locked { snapshots[pid] = snapshot }
        let title = AX.string(win, kAXTitleAttribute) ?? ""
        let sw = shot.map { $0.width } ?? Int((liveFrame.width * scale).rounded())
        let sh = shot.map { $0.height } ?? Int((liveFrame.height * scale).rounded())
        var header = "应用 \(app.localizedName ?? "?")（\(app.bundleIdentifier ?? app.executableURL?.lastPathComponent ?? "?")，pid \(pid)），"
        header += "窗口 \(index)/\(total)「\(title)」 位置 (\(Int(liveFrame.minX)),\(Int(liveFrame.minY))) 大小 \(Int(liveFrame.width))×\(Int(liveFrame.height))，"
        header += shot == nil ? "没有截图（未授权屏幕录制或窗口截不到），坐标系 \(sw)×\(sh)" : "截图 \(sw)×\(sh)"
        if app.isHidden { header += "\n注意：应用处于隐藏状态，有些操作可能无效" }
        if AX.bool(win, kAXMinimizedAttribute) == true { header += "\n注意：窗口已最小化" }
        var text = header + "\n" + result.lines.joined(separator: "\n")
        if result.truncated { text += "\n（控件太多，只列出前 \(result.elements.count) 个；可调大 max_nodes）" }
        if result.emptyWebArea { text += "\n（网页内容还没加载出辅助功能信息，可稍后再 get_app_state，或按截图坐标点击）" }
        return ComputerUseReply(text: text, image: shot.flatMap { Self.encodeJPEG($0) })
    }

    /// 选窗口：给了编号就用它，否则用最前面的（跳过最小化的）
    private func pickWindow(_ appEl: AXUIElement, app: NSRunningApplication, index: Int?) throws -> (AXUIElement, Int, Int) {
        var windows = AX.elements(appEl, kAXWindowsAttribute)
        if windows.isEmpty {
            // 刚启动或懒加载的应用：稍等再取一次
            Thread.sleep(forTimeInterval: 0.4)
            windows = AX.elements(appEl, kAXWindowsAttribute)
        }
        guard !windows.isEmpty else {
            throw Failure(message: "\(app.localizedName ?? "应用") 没有可操作的窗口（可能在其他桌面空间、已关闭，或应用没有窗口）")
        }
        if let i = index {
            guard i >= 0 && i < windows.count else { throw Failure(message: "窗口编号 \(i) 超出范围（共 \(windows.count) 个，从 0 开始）") }
            return (windows[i], i, windows.count)
        }
        let i = windows.firstIndex { AX.bool($0, kAXMinimizedAttribute) != true } ?? 0
        return (windows[i], i, windows.count)
    }

    struct TreeResult {
        var lines: [String] = []
        var labels: [String] = []
        var elements: [AXUIElement] = []
        var truncated = false
        var emptyWebArea = false
    }

    /// 纯布局容器：没有标题、值和动作时不编号，只把子节点按原缩进列出
    private static let layoutRoles: Set<String> = ["AXGroup", "AXSplitGroup", "AXScrollArea", "AXLayoutArea", "AXLayoutItem",
                                                    "AXUnknown", "AXGenericElement", "AXSplitter", "AXMatte", "AXGrowArea"]
    /// 不值得列出的动作
    private static let noisyActions: Set<String> = ["AXScrollToVisible", "AXShowDefaultUI", "AXShowAlternateUI"]

    private func walkTree(_ root: AXUIElement, windowFrame: CGRect, scale: CGFloat, maxNodes: Int) -> TreeResult {
        var r = TreeResult()
        let deadline = Date().addingTimeInterval(6)
        func visit(_ el: AXUIElement, depth: Int, level: Int) {
            if r.elements.count >= maxNodes { r.truncated = true; return }
            if depth > 60 || Date() > deadline { r.truncated = true; return }
            let info = AX.info(el)
            let role = info.role ?? "AXUnknown"
            // 不可见（在窗口外，如滚出视野的列表项）：整棵子树跳过
            if let f = info.frame, f.width > 0, f.height > 0, depth > 0, !f.intersects(windowFrame.insetBy(dx: -2, dy: -2)) { return }
            let label = info.title ?? info.desc ?? ""
            let actions = AX.actions(el).filter { !Self.noisyActions.contains($0) }
            let settable = AX.settable(el, kAXValueAttribute)
            let meaningful = depth == 0 || !Self.layoutRoles.contains(role) || !label.isEmpty || info.value != nil
                || !actions.isEmpty || settable
            let children = AX.elements(el, kAXChildrenAttribute)
            if role == "AXWebArea" && children.isEmpty { r.emptyWebArea = true }
            var childLevel = level
            if meaningful {
                let n = r.elements.count
                var line = String(repeating: "  ", count: level) + "[\(n)] \(role)"
                var short = "[\(n)] \(role)"
                if !label.isEmpty {
                    line += " \"\(Self.clip(label, 120))\""
                    short += " \"\(Self.clip(label, 40))\""
                }
                if let v = info.value, v != label { line += " value=\"\(Self.clip(v, 200))\"" }
                if settable { line += " settable" }
                if info.focused { line += " focused" }
                if info.enabled == false { line += " disabled" }
                if !actions.isEmpty { line += " actions=" + actions.joined(separator: ",") }
                if let f = info.frame, f.width > 0 || f.height > 0 {
                    let x = Int(((f.minX - windowFrame.minX) * scale).rounded()), y = Int(((f.minY - windowFrame.minY) * scale).rounded())
                    line += " (\(x),\(y) \(Int((f.width * scale).rounded()))×\(Int((f.height * scale).rounded())))"
                }
                r.lines.append(line)
                r.labels.append(short)
                r.elements.append(el)
                childLevel = level + 1
            }
            for c in children {
                if r.elements.count >= maxNodes { r.truncated = true; return }
                visit(c, depth: depth + 1, level: childLevel)
            }
        }
        visit(root, depth: 0, level: 0)
        return r
    }

    private static func clip(_ s: String, _ n: Int) -> String {
        var t = s.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"")
            .replacingOccurrences(of: "\n", with: "\\n").replacingOccurrences(of: "\r", with: "")
        if t.count > n { t = String(t.prefix(n)) + "…" }
        return t
    }

    // MARK: 窗口与截图

    /// 私有 _AXUIElementGetWindow（AX 窗口 → CGWindowID）；没有就按进程 + 位置大小在窗口列表里找
    private static func windowId(of win: AXUIElement, pid: pid_t, frame: CGRect) -> CGWindowID? {
        if let f = Private.axGetWindow {
            var wid: CGWindowID = 0
            if f(win, &wid) == .success, wid != 0 { return wid }
        }
        guard let list = CGWindowListCopyWindowInfo([.optionAll, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else { return nil }
        var best: (CGWindowID, CGFloat)?
        for w in list where (w[kCGWindowOwnerPID as String] as? pid_t) == pid && (w[kCGWindowLayer as String] as? Int) == 0 {
            guard let b = w[kCGWindowBounds as String] as? NSDictionary, let r = CGRect(dictionaryRepresentation: b),
                  let n = w[kCGWindowNumber as String] as? Int else { continue }
            let d = abs(r.minX - frame.minX) + abs(r.minY - frame.minY) + abs(r.width - frame.width) + abs(r.height - frame.height)
            if best == nil || d < best!.1 { best = (CGWindowID(n), d) }
        }
        return best.flatMap { $0.1 < 40 ? $0.0 : nil }
    }

    /// 窗口当前的位置大小（CG 全局坐标）；窗口已不存在返回 nil
    static func currentFrame(_ wid: CGWindowID) -> CGRect? {
        guard wid != 0, let arr = CGWindowListCreateDescriptionFromArray([NSNumber(value: wid)] as CFArray) as? [[String: Any]],
              let b = arr.first?[kCGWindowBounds as String] as? NSDictionary else { return nil }
        return CGRect(dictionaryRepresentation: b)
    }

    /// 只截这个窗口（被遮挡、在其他桌面空间也能截），长边不超过 1280
    static func captureWindow(_ wid: CGWindowID) -> CGImage? {
        guard SystemPermissions.screenRecordingGranted else { return nil }
        var image: CGImage?
        if #available(macOS 14.0, *) {
            let sem = DispatchSemaphore(value: 0)
            nonisolated(unsafe) var out: CGImage?
            Task.detached {
                do {
                    let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
                    if let w = content.windows.first(where: { $0.windowID == wid }) {
                        let filter = SCContentFilter(desktopIndependentWindow: w)
                        let cfg = SCStreamConfiguration()
                        let s = CGFloat(filter.pointPixelScale)
                        cfg.width = max(1, Int(w.frame.width * s))
                        cfg.height = max(1, Int(w.frame.height * s))
                        cfg.showsCursor = false
                        cfg.ignoreShadowsSingleWindow = true
                        out = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: cfg)
                    }
                } catch {}
                sem.signal()
            }
            if sem.wait(timeout: .now() + 6) == .success { image = out }
        } else {
            image = CGWindowListCreateImage(.null, .optionIncludingWindow, wid, [.boundsIgnoreFraming, .bestResolution])
        }
        guard let src = image else { return nil }
        return downscale(src, longSide: 1280)
    }

    private static func downscale(_ src: CGImage, longSide: Int) -> CGImage? {
        let long = max(src.width, src.height)
        guard long > longSide else { return src }
        let k = Double(longSide) / Double(long)
        let w = max(1, Int(Double(src.width) * k)), h = max(1, Int(Double(src.height) * k))
        guard let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                                  space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return src }
        ctx.interpolationQuality = .high
        ctx.draw(src, in: CGRect(x: 0, y: 0, width: w, height: h))
        return ctx.makeImage() ?? src
    }

    /// 截图像素 / 点。没有截图时按「长边不超过 1280 且不放大」虚拟一张，坐标系照样成立
    private static func shotScale(frame: CGRect, image: CGImage?) -> CGFloat {
        if let img = image, frame.width > 0 { return CGFloat(img.width) / frame.width }
        let long = max(frame.width, frame.height)
        return long > 1280 ? 1280 / long : 1
    }

    static func encodeJPEG(_ img: CGImage) -> ImageData? {
        let rep = NSBitmapImageRep(cgImage: img)
        guard let data = rep.representation(using: .jpeg, properties: [.compressionFactor: 0.8]) else { return nil }
        return ImageData(mime: "image/jpeg", data: data, width: UInt32(img.width), height: UInt32(img.height))
    }

    /// 动作后约 300 ms 截一张窗口图（顺带更新坐标换算，窗口挪动或缩放后下一次坐标点击仍然对得上）
    private func afterAction(_ app: NSRunningApplication, _ text: String) -> ComputerUseReply {
        Thread.sleep(forTimeInterval: 0.3)
        let pid = app.processIdentifier
        guard let snap = lock.locked({ snapshots[pid] }), snap.windowId != 0 else { return ComputerUseReply(text: text, image: nil) }
        guard let frame = Self.currentFrame(snap.windowId) else {
            return ComputerUseReply(text: text + "（窗口已关闭，请重新 get_app_state）", image: nil)
        }
        let shot = Self.captureWindow(snap.windowId)
        let scale = Self.shotScale(frame: frame, image: shot)
        lock.locked {
            snapshots[pid]?.windowFrame = frame
            snapshots[pid]?.scale = scale
        }
        return ComputerUseReply(text: text, image: shot.flatMap { Self.encodeJPEG($0) })
    }

    // MARK: 控件与坐标

    private func snapshot(_ app: NSRunningApplication) throws -> Snapshot {
        guard let s = lock.locked({ snapshots[app.processIdentifier] }) else {
            throw Failure(message: "还没有读取过 \(app.localizedName ?? "这个应用") 的界面，请先 get_app_state")
        }
        return s
    }

    private func element(_ app: NSRunningApplication, _ v: Any?) throws -> (AXUIElement, String) {
        guard let n = Self.int(v) else { throw Failure(message: "element 必须是控件编号") }
        let s = try snapshot(app)
        guard n >= 0 && n < s.elements.count else { throw Failure(message: "控件编号 \(n) 不存在，请先 get_app_state") }
        let el = s.elements[n]
        var role: CFTypeRef?
        if AXUIElementCopyAttributeValue(el, kAXRoleAttribute as CFString, &role) == .invalidUIElement {
            throw Failure(message: "控件编号 \(n) 已失效（界面变了），请先 get_app_state")
        }
        return (el, s.labels[n])
    }

    /// 截图像素 → 屏幕点（按窗口当前位置）
    private func screenPoint(_ app: NSRunningApplication, x: Double, y: Double) throws -> (CGPoint, Snapshot) {
        let s = try snapshot(app)
        let frame = Self.currentFrame(s.windowId) ?? s.windowFrame
        let p = CGPoint(x: frame.minX + CGFloat(x) / s.scale, y: frame.minY + CGFloat(y) / s.scale)
        guard frame.insetBy(dx: -1, dy: -1).contains(p) else {
            throw Failure(message: "坐标 (\(Int(x)),\(Int(y))) 不在窗口截图范围内（\(Int(frame.width * s.scale))×\(Int(frame.height * s.scale))）")
        }
        return (p, s)
    }

    private static func center(_ el: AXUIElement) -> CGPoint? {
        guard let f = AX.frame(el), f.width > 0 || f.height > 0 else { return nil }
        return CGPoint(x: f.midX, y: f.midY)
    }

    private func moveCursor(_ app: NSRunningApplication, to p: CGPoint?, press: Bool) {
        guard let p else { return }
        let wid = lock.locked { snapshots[app.processIdentifier]?.windowId } ?? 0
        AICursorOverlay.shared.move(to: p, above: wid, press: false)
        Thread.sleep(forTimeInterval: AICursorOverlay.travelDuration + 0.03)
        if press { AICursorOverlay.shared.move(to: p, above: wid, press: true) }
    }

    // MARK: 动作

    private func click(_ app: NSRunningApplication, _ req: [String: Any]) throws -> ComputerUseReply {
        let count = max(1, min(Self.int(req["count"]) ?? 1, 3))
        let right = (Self.string(req["button"]) ?? "left").lowercased() == "right"
        if req["element"] != nil {
            let (el, label) = try element(app, req["element"])
            let actions = AX.actions(el)
            let c = Self.center(el)
            if right, actions.contains("AXShowMenu") {
                moveCursor(app, to: c, press: true)
                if AXUIElementPerformAction(el, "AXShowMenu" as CFString) == .success {
                    return afterAction(app, "已对 \(label) 打开右键菜单")
                }
            }
            if !right && count == 1 {
                for a in ["AXPress", "AXConfirm", "AXPick", "AXOpen"] where actions.contains(a) {
                    moveCursor(app, to: c, press: true)
                    let err = AXUIElementPerformAction(el, a as CFString)
                    if err == .success { return afterAction(app, "已点击 \(label)") }
                    if err == .invalidUIElement { throw Failure(message: "控件已失效（界面变了），请先 get_app_state") }
                    break
                }
                // 文本框之类：点击 = 获得焦点
                if AX.settable(el, kAXFocusedAttribute), actions.isEmpty,
                   AXUIElementSetAttributeValue(el, kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success {
                    moveCursor(app, to: c, press: true)
                    return afterAction(app, "已点击 \(label)（让它获得焦点）")
                }
            }
            guard let p = c else { throw Failure(message: "\(label) 没有位置信息，无法按坐标点击；可试 perform_action") }
            let s = try snapshot(app)
            moveCursor(app, to: p, press: true)
            try postClick(app, window: s.windowId, at: p, right: right, count: count)
            return afterAction(app, "已\(right ? "右键" : "")\(count == 2 ? "双击" : "点击") \(label)（按位置）")
        }
        guard let x = Self.double(req["x"]), let y = Self.double(req["y"]) else { throw Failure(message: "需要 element，或 x 和 y") }
        let (p, s) = try screenPoint(app, x: x, y: y)
        moveCursor(app, to: p, press: true)
        try postClick(app, window: s.windowId, at: p, right: right, count: count)
        return afterAction(app, "已\(right ? "右键" : "")\(count == 2 ? "双击" : "点击")截图坐标 (\(Int(x)),\(Int(y)))")
    }

    private func setValue(_ app: NSRunningApplication, _ req: [String: Any]) throws -> ComputerUseReply {
        let (el, label) = try element(app, req["element"])
        guard let raw = req["value"] else { throw Failure(message: "缺少参数 value") }
        guard AX.settable(el, kAXValueAttribute) else {
            throw Failure(message: "\(label) 的值不能直接设置；可以先 click 它再 type_text")
        }
        let old = AX.rawValue(el)
        let value: CFTypeRef
        switch old {
        case is NSNumber where (old as? NSNumber).map({ CFGetTypeID($0) == CFBooleanGetTypeID() }) == true:
            let s = Self.string(raw)?.lowercased() ?? ""
            value = (["1", "true", "yes", "on"].contains(s) || (raw as? Bool) == true) ? kCFBooleanTrue : kCFBooleanFalse
        case is NSNumber:
            guard let d = Self.double(raw) else { throw Failure(message: "\(label) 的值是数字") }
            value = NSNumber(value: d)
        default:
            value = (Self.string(raw) ?? "\(raw)") as CFString
        }
        moveCursor(app, to: Self.center(el), press: false)
        let err = AXUIElementSetAttributeValue(el, kAXValueAttribute as CFString, value)
        guard err == .success else { throw Failure(message: "设置 \(label) 的值失败（AX 错误 \(err.rawValue)）") }
        return afterAction(app, "已设置 \(label) 的值")
    }

    private func typeText(_ app: NSRunningApplication, _ req: [String: Any]) throws -> ComputerUseReply {
        guard let text = Self.string(req["text"]) else { throw Failure(message: "缺少参数 text") }
        let pid = app.processIdentifier
        let appEl = AXUIElementCreateApplication(pid)
        AXUIElementSetMessagingTimeout(appEl, 2)
        var target = "当前焦点"
        if req["element"] != nil {
            let (el, label) = try element(app, req["element"])
            target = label
            moveCursor(app, to: Self.center(el), press: true)
            // 只在应用内部移动焦点，不激活应用
            AXUIElementSetAttributeValue(el, kAXFocusedAttribute as CFString, kCFBooleanTrue)
        }
        var focused: CFTypeRef?
        AXUIElementCopyAttributeValue(appEl, kAXFocusedUIElementAttribute as CFString, &focused)
        if req["element"] == nil, let f = focused, CFGetTypeID(f) == AXUIElementGetTypeID() {
            moveCursor(app, to: Self.center(f as! AXUIElement), press: false)
        }
        // 优先：直接插入到焦点控件的光标处（AXSelectedText），并确认值真的变了
        if let f = focused, CFGetTypeID(f) == AXUIElementGetTypeID(), !text.contains("\n") {
            let el = f as! AXUIElement
            if AX.settable(el, kAXSelectedTextAttribute) {
                let before = AX.rawValue(el) as? String
                if AXUIElementSetAttributeValue(el, kAXSelectedTextAttribute as CFString, text as CFString) == .success {
                    let after = AX.rawValue(el) as? String
                    if before == nil || after != before || text.isEmpty {
                        return afterAction(app, "已在 \(target) 输入 \(text.count) 个字")
                    }
                }
            }
        }
        // 退路：Unicode 键盘事件直接投递给这个进程
        let wid = lock.locked { snapshots[pid]?.windowId } ?? 0
        try withKeyFocus(app, window: wid) { Self.postText(text, pid: pid) }
        return afterAction(app, "已向 \(target) 键入 \(text.count) 个字")
    }

    private func pressKey(_ app: NSRunningApplication, _ req: [String: Any]) throws -> ComputerUseReply {
        guard let spec = Self.string(req["key"]), !spec.isEmpty else { throw Failure(message: "缺少参数 key") }
        let combo = try KeyCombo(spec)
        let pid = app.processIdentifier
        // 带 cmd / ctrl 的快捷键：先在菜单栏里找对应菜单项（后台应用收不到菜单快捷键事件，但菜单项能按）
        if combo.flags.contains(.maskCommand) || combo.flags.contains(.maskControl) {
            switch Self.findMenuItem(pid: pid, combo: combo) {
            case .found(let item, let title):
                if AXUIElementPerformAction(item, kAXPressAction as CFString) == .success {
                    return afterAction(app, "已按 \(spec)（执行菜单项「\(title)」）")
                }
            case .disabled(let title):
                throw Failure(message: "\(spec) 对应的菜单项「\(title)」当前不可用")
            case .none:
                break
            }
        }
        let wid = lock.locked { snapshots[pid]?.windowId } ?? 0
        try withKeyFocus(app, window: wid) {
            Self.postKey(combo.keyCode, flags: combo.flags, pid: pid)
        }
        return afterAction(app, "已按 \(spec)")
    }

    private func scroll(_ app: NSRunningApplication, _ req: [String: Any]) throws -> ComputerUseReply {
        let dir = (Self.string(req["direction"]) ?? "down").lowercased()
        guard ["up", "down", "left", "right"].contains(dir) else { throw Failure(message: "direction 只能是 up/down/left/right") }
        let pages = max(0.1, min(Self.double(req["pages"]) ?? 1, 20))
        let vertical = dir == "up" || dir == "down"
        let s = try snapshot(app)
        var point: CGPoint
        var extent: CGFloat
        var label = "窗口"
        if req["element"] != nil {
            let (el, l) = try element(app, req["element"])
            label = l
            // 先试滚动区域的滚动条（原生控件最稳）
            if let done = Self.scrollByScrollBar(el, vertical: vertical, forward: dir == "down" || dir == "right", pages: pages) {
                moveCursor(app, to: Self.center(el), press: false)
                return afterAction(app, "已在 \(label) 向\(Self.dirName(dir))滚动 \(Self.fmt(pages)) 页\(done)")
            }
            guard let f = AX.frame(el) else { throw Failure(message: "\(label) 没有位置信息") }
            point = CGPoint(x: f.midX, y: f.midY)
            extent = vertical ? f.height : f.width
        } else if let x = Self.double(req["x"]), let y = Self.double(req["y"]) {
            point = try screenPoint(app, x: x, y: y).0
            let frame = Self.currentFrame(s.windowId) ?? s.windowFrame
            extent = vertical ? frame.height : frame.width
        } else {
            let frame = Self.currentFrame(s.windowId) ?? s.windowFrame
            point = CGPoint(x: frame.midX, y: frame.midY)
            extent = vertical ? frame.height : frame.width
        }
        moveCursor(app, to: point, press: false)
        var total = Int((CGFloat(pages) * max(extent, 100) * 0.8).rounded())
        // 滚轮正值 = 内容往下走（看上面）
        let sign: Int32 = (dir == "up" || dir == "left") ? 1 : -1
        let chromium = isChromium(app)
        let frame = Self.currentFrame(s.windowId) ?? s.windowFrame
        while total > 0 {
            let step = Int32(min(total, 120))
            total -= Int(step)
            guard let ev = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 2,
                                   wheel1: vertical ? sign * step : 0, wheel2: vertical ? 0 : sign * step, wheel3: 0) else { break }
            ev.location = point
            Self.target(ev, window: s.windowId, windowFrame: frame, point: point)
            Self.post(ev, pid: app.processIdentifier, chromium: chromium)
            Thread.sleep(forTimeInterval: 0.012)
        }
        return afterAction(app, "已在 \(label) 向\(Self.dirName(dir))滚动 \(Self.fmt(pages)) 页")
    }

    private func performAction(_ app: NSRunningApplication, _ req: [String: Any]) throws -> ComputerUseReply {
        let (el, label) = try element(app, req["element"])
        guard var name = Self.string(req["name"]), !name.isEmpty else { throw Failure(message: "缺少参数 name") }
        let actions = AX.actions(el)
        if !name.hasPrefix("AX"), actions.contains("AX" + name) { name = "AX" + name }
        moveCursor(app, to: Self.center(el), press: true)
        let err = AXUIElementPerformAction(el, name as CFString)
        guard err == .success else {
            throw Failure(message: "\(label) 执行 \(name) 失败（AX 错误 \(err.rawValue)）；它支持的动作：\(actions.isEmpty ? "无" : actions.joined(separator: ","))")
        }
        return afterAction(app, "已对 \(label) 执行 \(name)")
    }

    // MARK: 滚动条

    /// 能用滚动条就用，返回附加说明；不能返回 nil
    private static func scrollByScrollBar(_ el: AXUIElement, vertical: Bool, forward: Bool, pages: Double) -> String? {
        var area: AXUIElement? = el
        for _ in 0..<8 {
            guard let a = area else { return nil }
            if AX.string(a, kAXRoleAttribute) == "AXScrollArea" { break }
            var p: CFTypeRef?
            guard AXUIElementCopyAttributeValue(a, kAXParentAttribute as CFString, &p) == .success, let pp = p,
                  CFGetTypeID(pp) == AXUIElementGetTypeID() else { return nil }
            area = (pp as! AXUIElement)
        }
        guard let a = area, AX.string(a, kAXRoleAttribute) == "AXScrollArea" else { return nil }
        var barRef: CFTypeRef?
        let barAttr = vertical ? kAXVerticalScrollBarAttribute : kAXHorizontalScrollBarAttribute
        guard AXUIElementCopyAttributeValue(a, barAttr as CFString, &barRef) == .success, let b = barRef,
              CFGetTypeID(b) == AXUIElementGetTypeID() else { return nil }
        let bar = b as! AXUIElement
        guard AX.settable(bar, kAXValueAttribute), let v = (AX.rawValue(bar) as? NSNumber)?.doubleValue,
              let af = AX.frame(a) else { return nil }
        let visible = vertical ? af.height : af.width
        let content = AX.elements(a, kAXChildrenAttribute).compactMap { c -> CGFloat? in
            let r = AX.string(c, kAXRoleAttribute)
            guard r != "AXScrollBar", let f = AX.frame(c) else { return nil }
            return vertical ? f.height : f.width
        }.max() ?? 0
        guard content > visible + 1 else { return nil }
        let per = Double(visible * 0.9 / (content - visible))
        let nv = max(0, min(1, v + (forward ? 1 : -1) * per * pages))
        guard AXUIElementSetAttributeValue(bar, kAXValueAttribute as CFString, NSNumber(value: nv)) == .success else { return nil }
        return nv == 0 || nv == 1 ? "（已到\(nv == 0 ? "开头" : "末尾")）" : ""
    }

    // MARK: 菜单快捷键

    enum MenuHit {
        case found(AXUIElement, String)
        case disabled(String)
        case none
    }

    private static func findMenuItem(pid: pid_t, combo: KeyCombo) -> MenuHit {
        let appEl = AXUIElementCreateApplication(pid)
        AXUIElementSetMessagingTimeout(appEl, 1)
        var barRef: CFTypeRef?
        guard AXUIElementCopyAttributeValue(appEl, kAXMenuBarAttribute as CFString, &barRef) == .success, let b = barRef,
              CFGetTypeID(b) == AXUIElementGetTypeID() else { return .none }
        // AXMenuItemCmdModifiers：0 = 只有 cmd；1 shift、2 option、4 control、8 不带 cmd
        var want = 0
        if combo.flags.contains(.maskShift) { want |= 1 }
        if combo.flags.contains(.maskAlternate) { want |= 2 }
        if combo.flags.contains(.maskControl) { want |= 4 }
        if !combo.flags.contains(.maskCommand) { want |= 8 }
        let deadline = Date().addingTimeInterval(2.5)
        var visited = 0
        var hit: MenuHit = .none
        func visit(_ el: AXUIElement, depth: Int) -> Bool {
            visited += 1
            if depth > 5 || visited > 4000 || Date() > deadline { return true }
            for c in AX.elements(el, kAXChildrenAttribute) {
                if AX.string(c, kAXRoleAttribute) == "AXMenuItem" {
                    let mods = (AX.rawValue(c, "AXMenuItemCmdModifiers") as? NSNumber)?.intValue ?? -1
                    var match = false
                    if mods == want {
                        if let ch = combo.char, let cmdChar = AX.string(c, "AXMenuItemCmdChar"), !cmdChar.isEmpty {
                            match = cmdChar.uppercased() == ch.uppercased()
                        } else if combo.char == nil, let vk = (AX.rawValue(c, "AXMenuItemCmdVirtualKey") as? NSNumber)?.intValue {
                            match = vk == Int(combo.keyCode)
                        }
                    }
                    if match {
                        let title = AX.string(c, kAXTitleAttribute) ?? ""
                        hit = AX.bool(c, kAXEnabledAttribute) == false ? .disabled(title) : .found(c, title)
                        return true
                    }
                }
                if visit(c, depth: depth + 1) { return true }
            }
            return false
        }
        _ = visit(b as! AXUIElement, depth: 0)
        return hit
    }

    // MARK: 后台事件投递

    /// Chromium / Electron 系应用（网页内容只认「窗口是 key」时的输入，要 SkyLight 路径）
    private func isChromium(_ app: NSRunningApplication) -> Bool {
        let pid = app.processIdentifier
        if let v = lock.locked({ chromiumCache[pid] }) { return v }
        var v = false
        let bid = (app.bundleIdentifier ?? "").lowercased()
        if ["com.google.chrome", "org.chromium.", "com.microsoft.edgemac", "com.brave.browser", "com.vivaldi.",
            "com.operasoftware.", "company.thebrowser."].contains(where: { bid.hasPrefix($0) }) {
            v = true
        } else if let fw = app.bundleURL?.appendingPathComponent("Contents/Frameworks"),
                  let items = try? FileManager.default.contentsOfDirectory(atPath: fw.path) {
            v = items.contains { n in
                let l = n.lowercased()
                return l.contains("electron framework") || l.contains("chromium") || l.contains("chrome framework")
                    || l.contains("edge framework") || (l.contains("cef") && l.hasSuffix(".framework"))
            }
        }
        lock.locked { chromiumCache[pid] = v }
        return v
    }

    /// 键盘输入前：Chromium 系给目标进程一个只对它生效的「激活 + key 窗口」记录（不碰真实前台），完了撤销
    private func withKeyFocus(_ app: NSRunningApplication, window: CGWindowID, _ body: () -> Void) throws {
        let pid = app.processIdentifier
        guard isChromium(app), window != 0, Private.canSyntheticFocus else {
            body()
            return
        }
        if app.isHidden { throw Failure(message: "\(app.localizedName ?? "应用") 处于隐藏状态，无法在后台输入") }
        Private.syntheticActivate(pid: pid, window: window, on: true)
        Private.makeKeyWindow(pid: pid, window: window)
        Thread.sleep(forTimeInterval: 0.3)
        body()
        Thread.sleep(forTimeInterval: 0.1)
        Private.syntheticActivate(pid: pid, window: window, on: false)
    }

    private func postClick(_ app: NSRunningApplication, window: CGWindowID, at p: CGPoint, right: Bool, count: Int) throws {
        guard window != 0, let frame = Self.currentFrame(window) else {
            throw Failure(message: "找不到目标窗口（可能已关闭），请先 get_app_state")
        }
        let pid = app.processIdentifier
        let chromium = isChromium(app)
        let synthetic = chromium && Private.canSyntheticFocus
        if synthetic { Private.syntheticActivate(pid: pid, window: window, on: true) }
        defer {
            if synthetic {
                Thread.sleep(forTimeInterval: 0.15)
                Private.syntheticActivate(pid: pid, window: window, on: false)
            }
        }
        let down: NSEvent.EventType = right ? .rightMouseDown : .leftMouseDown
        let up: NSEvent.EventType = right ? .rightMouseUp : .leftMouseUp
        if let mv = Self.mouseEvent(.mouseMoved, at: p, window: window, frame: frame, button: 0, clickCount: 0) {
            Self.post(mv, pid: pid, chromium: chromium)
        }
        Thread.sleep(forTimeInterval: 0.02)
        for i in 1...count {
            guard let d = Self.mouseEvent(down, at: p, window: window, frame: frame, button: right ? 1 : 0, clickCount: i),
                  let u = Self.mouseEvent(up, at: p, window: window, frame: frame, button: right ? 1 : 0, clickCount: i) else {
                throw Failure(message: "无法构造鼠标事件")
            }
            Self.post(d, pid: pid, chromium: chromium)
            Thread.sleep(forTimeInterval: 0.03)
            Self.post(u, pid: pid, chromium: chromium)
            if i < count { Thread.sleep(forTimeInterval: 0.06) }
        }
    }

    /// NSEvent(windowNumber:) → CGEvent，补上窗口定向字段与窗口内坐标
    private static func mouseEvent(_ type: NSEvent.EventType, at p: CGPoint, window: CGWindowID, frame: CGRect,
                                   button: Int64, clickCount: Int) -> CGEvent? {
        let local = CGPoint(x: p.x - frame.minX, y: p.y - frame.minY)
        // NSEvent 的窗口坐标以窗口左下角为原点
        let nsLocal = NSPoint(x: local.x, y: frame.height - local.y)
        guard let ns = NSEvent.mouseEvent(with: type, location: nsLocal, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                                          windowNumber: Int(window), context: nil, eventNumber: 0, clickCount: clickCount,
                                          pressure: type == .leftMouseUp || type == .rightMouseUp || type == .mouseMoved ? 0 : 1),
              let ev = ns.cgEvent?.copy() else { return nil }
        ev.location = p
        ev.setIntegerValueField(.mouseEventButtonNumber, value: button)
        if clickCount > 0 { ev.setIntegerValueField(.mouseEventClickState, value: Int64(clickCount)) }
        ev.setIntegerValueField(Self.field(7), value: 3)
        target(ev, window: window, windowFrame: frame, point: p)
        return ev
    }

    /// 窗口定向：字段 91（指针下的窗口）/ 92（能处理这个事件的窗口）+ 私有 CGEventSetWindowLocation（窗口内坐标）
    private static func target(_ ev: CGEvent, window: CGWindowID, windowFrame: CGRect, point: CGPoint) {
        guard window != 0 else { return }
        ev.setIntegerValueField(field(91), value: Int64(window))
        ev.setIntegerValueField(field(92), value: Int64(window))
        Private.setWindowLocation?(ev, CGPoint(x: point.x - windowFrame.minX, y: point.y - windowFrame.minY))
    }

    private static func field(_ raw: UInt32) -> CGEventField { unsafeBitCast(raw, to: CGEventField.self) }

    /// 只投给目标进程：Chromium 系同时走 SkyLight 的 SLEventPostToPid（它的网页层只认这条路），其余走公开 postToPid
    private static func post(_ ev: CGEvent, pid: pid_t, chromium: Bool) {
        if chromium, let sl = Private.slPostToPid {
            sl(pid, ev)
        }
        ev.postToPid(pid)
    }

    private static func postText(_ text: String, pid: pid_t) {
        var buffer: [UniChar] = []
        func flush() {
            guard !buffer.isEmpty else { return }
            for down in [true, false] {
                guard let ev = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: down) else { continue }
                ev.flags = []
                buffer.withUnsafeBufferPointer { ev.keyboardSetUnicodeString(stringLength: $0.count, unicodeString: $0.baseAddress) }
                ev.postToPid(pid)
            }
            buffer.removeAll()
            Thread.sleep(forTimeInterval: 0.008)
        }
        for ch in text {
            if ch == "\n" || ch == "\r\n" {
                flush()
                postKey(CGKeyCode(kVK_Return), flags: [], pid: pid)
            } else if ch == "\t" {
                flush()
                postKey(CGKeyCode(kVK_Tab), flags: [], pid: pid)
            } else {
                let units = Array(String(ch).utf16)
                if buffer.count + units.count > 16 { flush() }
                buffer += units
            }
        }
        flush()
    }

    private static func postKey(_ code: CGKeyCode, flags: CGEventFlags, pid: pid_t) {
        for down in [true, false] {
            guard let ev = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: down) else { continue }
            ev.flags = flags
            ev.postToPid(pid)
            Thread.sleep(forTimeInterval: 0.01)
        }
    }

    // MARK: 小工具

    private static func dirName(_ d: String) -> String { ["up": "上", "down": "下", "left": "左", "right": "右"][d] ?? d }
    private static func fmt(_ v: Double) -> String { v == v.rounded() ? "\(Int(v))" : String(format: "%.1f", v) }

    static func string(_ v: Any?) -> String? {
        switch v {
        case let s as String: return s
        case let n as NSNumber: return n.stringValue
        default: return nil
        }
    }

    static func int(_ v: Any?) -> Int? {
        switch v {
        case let n as NSNumber: return n.intValue
        case let s as String: return Int(s.trimmingCharacters(in: CharacterSet(charactersIn: " []")))
        default: return nil
        }
    }

    static func double(_ v: Any?) -> Double? {
        switch v {
        case let n as NSNumber: return n.doubleValue
        case let s as String: return Double(s.trimmingCharacters(in: .whitespaces))
        default: return nil
        }
    }
}

// MARK: - 按键解析

/// 「cmd+shift+t」「Return」「ctrl+alt+Delete」→ 虚拟键码 + 修饰键
struct KeyCombo {
    var keyCode: CGKeyCode
    var flags: CGEventFlags
    /// 单个字符键（菜单快捷键按字符匹配）
    var char: String?

    init(_ spec: String) throws {
        var parts = spec.split(separator: "+", omittingEmptySubsequences: false).map { $0.trimmingCharacters(in: .whitespaces) }
        // 「cmd++」：最后一个是加号本身
        if spec.hasSuffix("++") { parts = Array(parts.dropLast(2)) + ["+"] }
        guard let keyName = parts.last, !keyName.isEmpty else { throw ComputerUse.Failure(message: "按键「\(spec)」格式不对") }
        var flags: CGEventFlags = []
        for m in parts.dropLast() {
            switch m.lowercased() {
            case "cmd", "command", "meta", "super", "win", "⌘": flags.insert(.maskCommand)
            case "ctrl", "control", "⌃": flags.insert(.maskControl)
            case "alt", "option", "opt", "⌥": flags.insert(.maskAlternate)
            case "shift", "⇧": flags.insert(.maskShift)
            case "fn": flags.insert(.maskSecondaryFn)
            default: throw ComputerUse.Failure(message: "不认识的修饰键「\(m)」（可用 cmd、ctrl、alt/option、shift、fn）")
            }
        }
        let lower = keyName.lowercased()
        if let code = Self.named[lower] {
            keyCode = CGKeyCode(code)
            char = nil
        } else if keyName.count == 1, let code = Self.chars[lower] {
            keyCode = CGKeyCode(code)
            char = keyName
        } else if lower.hasPrefix("f"), let n = Int(lower.dropFirst()), (1...20).contains(n) {
            let fk = [kVK_F1, kVK_F2, kVK_F3, kVK_F4, kVK_F5, kVK_F6, kVK_F7, kVK_F8, kVK_F9, kVK_F10,
                      kVK_F11, kVK_F12, kVK_F13, kVK_F14, kVK_F15, kVK_F16, kVK_F17, kVK_F18, kVK_F19, kVK_F20]
            keyCode = CGKeyCode(fk[n - 1])
            char = nil
        } else {
            throw ComputerUse.Failure(message: "不认识的按键「\(keyName)」（如 Return、Tab、Escape、Delete、Up、PageDown、F5、a、1）")
        }
        self.flags = flags
    }

    static let named: [String: Int] = [
        "return": kVK_Return, "enter": kVK_Return, "tab": kVK_Tab, "space": kVK_Space, "escape": kVK_Escape, "esc": kVK_Escape,
        "delete": kVK_Delete, "backspace": kVK_Delete, "forwarddelete": kVK_ForwardDelete, "del": kVK_ForwardDelete,
        "up": kVK_UpArrow, "down": kVK_DownArrow, "left": kVK_LeftArrow, "right": kVK_RightArrow,
        "arrowup": kVK_UpArrow, "arrowdown": kVK_DownArrow, "arrowleft": kVK_LeftArrow, "arrowright": kVK_RightArrow,
        "home": kVK_Home, "end": kVK_End, "pageup": kVK_PageUp, "pagedown": kVK_PageDown, "page_up": kVK_PageUp, "page_down": kVK_PageDown,
        "help": kVK_Help, "keypadenter": kVK_ANSI_KeypadEnter,
    ]

    static let chars: [String: Int] = [
        "a": kVK_ANSI_A, "b": kVK_ANSI_B, "c": kVK_ANSI_C, "d": kVK_ANSI_D, "e": kVK_ANSI_E, "f": kVK_ANSI_F, "g": kVK_ANSI_G,
        "h": kVK_ANSI_H, "i": kVK_ANSI_I, "j": kVK_ANSI_J, "k": kVK_ANSI_K, "l": kVK_ANSI_L, "m": kVK_ANSI_M, "n": kVK_ANSI_N,
        "o": kVK_ANSI_O, "p": kVK_ANSI_P, "q": kVK_ANSI_Q, "r": kVK_ANSI_R, "s": kVK_ANSI_S, "t": kVK_ANSI_T, "u": kVK_ANSI_U,
        "v": kVK_ANSI_V, "w": kVK_ANSI_W, "x": kVK_ANSI_X, "y": kVK_ANSI_Y, "z": kVK_ANSI_Z,
        "0": kVK_ANSI_0, "1": kVK_ANSI_1, "2": kVK_ANSI_2, "3": kVK_ANSI_3, "4": kVK_ANSI_4, "5": kVK_ANSI_5, "6": kVK_ANSI_6,
        "7": kVK_ANSI_7, "8": kVK_ANSI_8, "9": kVK_ANSI_9,
        "-": kVK_ANSI_Minus, "=": kVK_ANSI_Equal, "[": kVK_ANSI_LeftBracket, "]": kVK_ANSI_RightBracket, "\\": kVK_ANSI_Backslash,
        ";": kVK_ANSI_Semicolon, "'": kVK_ANSI_Quote, ",": kVK_ANSI_Comma, ".": kVK_ANSI_Period, "/": kVK_ANSI_Slash, "`": kVK_ANSI_Grave,
        "+": kVK_ANSI_Equal,
    ]
}

// MARK: - 辅助功能读取

enum AX {
    struct Info {
        var role: String?
        var title: String?
        var desc: String?
        var value: String?
        var enabled: Bool?
        var focused = false
        var frame: CGRect?
    }

    /// 一次取齐常用属性（比逐个取快很多）
    static func info(_ el: AXUIElement) -> Info {
        let names = [kAXRoleAttribute, kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute, kAXEnabledAttribute,
                     kAXFocusedAttribute, kAXPositionAttribute, kAXSizeAttribute] as CFArray
        var out: CFArray?
        var info = Info()
        guard AXUIElementCopyMultipleAttributeValues(el, names, AXCopyMultipleAttributeOptions(rawValue: 0), &out) == .success,
              let arr = out as? [AnyObject], arr.count == 8 else { return info }
        func s(_ o: AnyObject) -> String? {
            guard let v = o as? String, !v.isEmpty else { return nil }
            return v
        }
        info.role = s(arr[0])
        info.title = s(arr[1])
        info.desc = s(arr[2])
        switch arr[3] {
        case let v as String: info.value = v.isEmpty ? nil : v
        case let n as NSNumber: info.value = n.stringValue
        default: break
        }
        info.enabled = (arr[4] as? NSNumber)?.boolValue
        info.focused = (arr[5] as? NSNumber)?.boolValue ?? false
        if let p = point(arr[6]), let sz = size(arr[7]) { info.frame = CGRect(origin: p, size: sz) }
        return info
    }

    private static func point(_ o: AnyObject) -> CGPoint? {
        guard CFGetTypeID(o) == AXValueGetTypeID() else { return nil }
        let v = o as! AXValue
        var p = CGPoint.zero
        return AXValueGetType(v) == .cgPoint && AXValueGetValue(v, .cgPoint, &p) ? p : nil
    }

    private static func size(_ o: AnyObject) -> CGSize? {
        guard CFGetTypeID(o) == AXValueGetTypeID() else { return nil }
        let v = o as! AXValue
        var s = CGSize.zero
        return AXValueGetType(v) == .cgSize && AXValueGetValue(v, .cgSize, &s) ? s : nil
    }

    static func rawValue(_ el: AXUIElement, _ name: String = kAXValueAttribute) -> CFTypeRef? {
        var v: CFTypeRef?
        return AXUIElementCopyAttributeValue(el, name as CFString, &v) == .success ? v : nil
    }

    static func string(_ el: AXUIElement, _ name: String) -> String? { rawValue(el, name) as? String }
    static func bool(_ el: AXUIElement, _ name: String) -> Bool? { (rawValue(el, name) as? NSNumber)?.boolValue }

    static func elements(_ el: AXUIElement, _ name: String) -> [AXUIElement] {
        guard let v = rawValue(el, name), CFGetTypeID(v) == CFArrayGetTypeID() else { return [] }
        return (v as! [AnyObject]).compactMap { CFGetTypeID($0) == AXUIElementGetTypeID() ? ($0 as! AXUIElement) : nil }
    }

    static func frame(_ el: AXUIElement) -> CGRect? {
        guard let p = rawValue(el, kAXPositionAttribute).flatMap({ point($0 as AnyObject) }),
              let s = rawValue(el, kAXSizeAttribute).flatMap({ size($0 as AnyObject) }) else { return nil }
        return CGRect(origin: p, size: s)
    }

    static func actions(_ el: AXUIElement) -> [String] {
        var names: CFArray?
        guard AXUIElementCopyActionNames(el, &names) == .success, let arr = names as? [String] else { return [] }
        return arr
    }

    static func settable(_ el: AXUIElement, _ name: String) -> Bool {
        var ok: DarwinBoolean = false
        return AXUIElementIsAttributeSettable(el, name as CFString, &ok) == .success && ok.boolValue
    }
}

// MARK: - 私有接口（运行时探测，缺了就退到公开接口）

enum Private {
    typealias AXGetWindow = @convention(c) (AXUIElement, UnsafeMutablePointer<CGWindowID>) -> AXError
    typealias SetWindowLocation = @convention(c) (CGEvent, CGPoint) -> Void
    typealias SLPostToPid = @convention(c) (pid_t, CGEvent) -> Void
    typealias PostEventRecordTo = @convention(c) (UnsafeMutablePointer<ProcessSerialNumber>, UnsafeMutablePointer<UInt8>) -> CGError
    typealias GetProcessForPID = @convention(c) (pid_t, UnsafeMutablePointer<ProcessSerialNumber>) -> OSStatus

    private static let skyLight: UnsafeMutableRawPointer? = dlopen("/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight", RTLD_LAZY)

    private static func sym<T>(_ name: String, in handle: UnsafeMutableRawPointer? = nil, as: T.Type) -> T? {
        let p = handle.flatMap { dlsym($0, name) } ?? dlsym(UnsafeMutableRawPointer(bitPattern: -2), name)  // RTLD_DEFAULT
        return p.map { unsafeBitCast($0, to: T.self) }
    }

    static let axGetWindow: AXGetWindow? = sym("_AXUIElementGetWindow", as: AXGetWindow.self)
    static let setWindowLocation: SetWindowLocation? = sym("CGEventSetWindowLocation", as: SetWindowLocation.self)
    static let slPostToPid: SLPostToPid? = sym("SLEventPostToPid", in: skyLight, as: SLPostToPid.self)
    static let postEventRecordTo: PostEventRecordTo? = sym("SLPSPostEventRecordTo", in: skyLight, as: PostEventRecordTo.self)
    static let getProcessForPID: GetProcessForPID? = sym("GetProcessForPID", as: GetProcessForPID.self)

    static var canSyntheticFocus: Bool { postEventRecordTo != nil && getProcessForPID != nil }

    private static func psn(_ pid: pid_t) -> ProcessSerialNumber? {
        guard let f = getProcessForPID else { return nil }
        var psn = ProcessSerialNumber()
        return f(pid, &psn) == noErr ? psn : nil
    }

    private static func record(window: CGWindowID) -> [UInt8] {
        var bytes = [UInt8](repeating: 0, count: 0xF8)
        bytes[0x04] = 0xF8
        withUnsafeBytes(of: window.littleEndian) { for (i, b) in $0.enumerated() { bytes[0x3C + i] = b } }
        return bytes
    }

    /// 只发给目标进程的「激活 / 撤销激活」记录（yabai 的 focus-without-raise 记录，不给真实前台应用发失活记录）
    static func syntheticActivate(pid: pid_t, window: CGWindowID, on: Bool) {
        guard let post = postEventRecordTo, var p = psn(pid) else { return }
        var bytes = record(window: window)
        bytes[0x08] = 0x0D
        bytes[0x8A] = on ? 0x01 : 0x02
        _ = bytes.withUnsafeMutableBufferPointer { post(&p, $0.baseAddress!) }
    }

    /// 让目标应用在进程内把这个窗口设为 key window（不改变系统前台、层级和桌面空间）
    static func makeKeyWindow(pid: pid_t, window: CGWindowID) {
        guard let post = postEventRecordTo, var p = psn(pid) else { return }
        for kind: UInt8 in [0x01, 0x02] {
            var bytes = record(window: window)
            bytes[0x08] = kind
            bytes[0x3A] = 0x10
            for i in 0x20..<0x30 { bytes[i] = 0xFF }
            _ = bytes.withUnsafeMutableBufferPointer { post(&p, $0.baseAddress!) }
        }
    }
}

// MARK: - 只读自检

/// --cu-selftest [应用名] [--out 截图路径]：只读地跑一次 list_apps 与 get_app_state（默认 Finder），打印结果后退出。
/// 不点击、不输入、不弹窗、不画 AI 光标。
enum ComputerUseSelfTest {
    static func run(_ args: [String]) -> Int32 {
        let i = args.firstIndex(of: "--cu-selftest") ?? 0
        let app = i + 1 < args.count && !args[i + 1].hasPrefix("--") ? args[i + 1] : "Finder"
        let out = args.firstIndex(of: "--out").flatMap { $0 + 1 < args.count ? args[$0 + 1] : nil }
        print("辅助功能授权：\(AXIsProcessTrusted())，屏幕录制授权：\(SystemPermissions.screenRecordingGranted)")
        print("私有接口：_AXUIElementGetWindow=\(Private.axGetWindow != nil) CGEventSetWindowLocation=\(Private.setWindowLocation != nil) "
              + "SLEventPostToPid=\(Private.slPostToPid != nil) SLPSPostEventRecordTo=\(Private.postEventRecordTo != nil) GetProcessForPID=\(Private.getProcessForPID != nil)")
        let list = ComputerUse.shared.handle(#"{"action":"list_apps"}"#)
        print("== list_apps ==\n" + list.text)
        let req = try! JSONSerialization.data(withJSONObject: ["action": "get_app_state", "app": app, "max_nodes": 120])
        let t0 = Date()
        let state = ComputerUse.shared.handle(String(decoding: req, as: UTF8.self))
        print("== get_app_state \(app)（\(Int(Date().timeIntervalSince(t0) * 1000)) ms）==\n" + state.text)
        if let img = state.image {
            print("截图：\(img.mime) \(img.width)×\(img.height) \(img.data.count) 字节")
            if let out { try? img.data.write(to: URL(fileURLWithPath: out)) }
        } else {
            print("截图：无")
        }
        let guards: [(String, pid_t, String)] = [("cn.lemonit.galaxy.worker", 1, ""), ("com.1password.1password", 1, ""),
                                                 ("com.apple.keychainaccess", 1, ""), ("", 1, "SecurityAgent"),
                                                 ("", ProcessInfo.processInfo.processIdentifier, "")]
        var guardsOK = true
        for (bid, pid, exe) in guards {
            let r = ComputerUse.forbiddenReason(bundleId: bid, pid: pid, executable: exe, name: nil)
            guardsOK = guardsOK && r != nil
            print("自我保护 \(bid.isEmpty ? exe.isEmpty ? "本进程" : exe : bid)：\(r ?? "未拦截（不对）")")
        }
        if ComputerUse.forbiddenReason(bundleId: "com.apple.finder", pid: 1, executable: "Finder", name: nil) != nil { guardsOK = false }
        if !guardsOK { return 1 }
        return list.text.hasPrefix("错误") || state.text.hasPrefix("错误") ? 1 : 0
    }
}
