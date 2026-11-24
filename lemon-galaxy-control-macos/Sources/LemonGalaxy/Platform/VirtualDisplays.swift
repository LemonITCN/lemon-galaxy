import Foundation
import CoreGraphics
import ScreenCaptureKit
import IOKit
import CGVirtualDisplayBridge

/// 本进程用 CGVirtualDisplay（私有接口，见 CGVirtualDisplayBridge 与 docs/research/virtual-display.md 第 4 节）加的虚拟屏。
///
/// - 一个进程一份：屏随 LGVirtualDisplay 对象存亡，对象由这里一直持有；删屏就是释放对象，进程退出时系统自动回收。
/// - 归属、上限、无头兜底屏的时机都由内核（galaxy-host 的 vdisplay.rs）决定，这里只管加、删、列、判断有没有亮着的真实显示器。
/// - 建屏、删屏都放到主线程上做（DeskPad、Chromium 都在主线程建），调用方（内核的阻塞线程）在这边等；
///   主线程一直不空时最多等 10 秒后报错，不死等（主线程可能正同步等内核）。
/// - 新屏的屏幕号就是 CGDirectDisplayID，与 CGGetActiveDisplayList、ScreenCaptureKit 的 SCDisplay.displayID 一致，
///   现有截屏（MacHostPlatform / 会话代理）按它直接采集。
/// 必须在连着窗口服务器的进程里用（主控端、菜单栏应用、无人值守的会话代理）；无人值守的后台服务（root 守护进程）不用它。
final class VirtualDisplays: @unchecked Sendable {
    static let shared = VirtualDisplays()

    /// 主控最多能加几块（与 Windows 被控端相同）；另留一个槽位给无头兜底屏
    static let controllerMax: UInt32 = 3
    private static let slotCount = 4
    /// EDID 厂商码「LGX」的 PNP 压缩写法（与 Windows 虚拟屏驱动的 EDID 厂商一致）：L=12、G=7、X=24 → (12<<10)|(7<<5)|24
    private static let vendorID: UInt32 = 0x30F8

    private struct Entry {
        let display: LGVirtualDisplay
        /// 每次建屏一个新编号：系统的结束回调按它找屏（槽位会被后来的屏复用）
        let token: Int
        let slot: Int
        let name: String
        let width: UInt32
        let height: UInt32
    }

    private let lock = NSLock()
    private var entries: [CGDirectDisplayID: Entry] = [:]
    private var nextToken = 1
    private var reserved = Set<Int>()
    /// 系统结束虚拟屏时的回调队列
    private let queue = DispatchQueue(label: "galaxy.vdisplay")

    /// 系统有 CGVirtualDisplay，且没有用环境变量 LEMONGALAXY_NO_VIRTUAL_DISPLAY=1 关掉（新系统上出问题时的应急开关）
    var available: Bool {
        ProcessInfo.processInfo.environment["LEMONGALAXY_NO_VIRTUAL_DISPLAY"] != "1" && LGVirtualDisplayAvailable()
    }

    /// 能力列表里的上限写法（内核与无人值守的后台服务按它读）
    static func capability(_ max: UInt32) -> String { "virtual_display_max=\(max)" }

    static func capabilityMax(_ caps: [String]) -> UInt32 {
        caps.lazy.compactMap { c -> UInt32? in
            guard c.hasPrefix("virtual_display_max=") else { return nil }
            return UInt32(c.dropFirst("virtual_display_max=".count))
        }.first ?? 0
    }

    // MARK: 查询

    var ids: [UInt32] { lock.locked { Array(entries.keys) } }
    func contains(_ id: CGDirectDisplayID) -> Bool { lock.locked { entries[id] != nil } }
    func name(_ id: CGDirectDisplayID) -> String? { lock.locked { entries[id]?.name } }

    /// 本机没有亮着的显示器（不算本进程加的虚拟屏）：没接显示器的 Mac mini、合盖且没接外接屏的笔记本。
    /// 合着盖时内建屏即使还在屏幕列表里也不算（系统是否把它移出列表因机型而异，两种情况都按没有处理）。
    /// 别的进程（DeskPad、隔空播放、随航等）的虚拟屏算作有屏：主控已经有东西可看。
    /// 环境变量 LEMON_GALAXY_TEST_HEADLESS=1 时总是返回真（与 Windows 相同，测试无头兜底屏用）。
    func needsHeadless() -> Bool {
        if ProcessInfo.processInfo.environment["LEMON_GALAXY_TEST_HEADLESS"] == "1" { return true }
        let mine = lock.locked { Set(entries.keys) }
        let lidClosed = Self.lidClosed()
        return !Self.activeDisplays().contains { id in
            !mine.contains(id) && !(lidClosed && CGDisplayIsBuiltin(id) != 0)
        }
    }

    /// 笔记本合着盖（电源管理根节点的 AppleClamshellState；台式机没有这一项，为假）
    static func lidClosed() -> Bool {
        let root = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOPMrootDomain"))
        guard root != 0 else { return false }
        defer { IOObjectRelease(root) }
        let v = IORegistryEntryCreateCFProperty(root, "AppleClamshellState" as CFString, kCFAllocatorDefault, 0)?.takeRetainedValue()
        return (v as? Bool) ?? false
    }

    static func activeDisplays() -> [CGDirectDisplayID] {
        var count: UInt32 = 0
        guard CGGetActiveDisplayList(0, nil, &count) == .success, count > 0 else { return [] }
        var ids = [CGDirectDisplayID](repeating: 0, count: Int(count))
        guard CGGetActiveDisplayList(count, &ids, &count) == .success else { return [] }
        return Array(ids.prefix(Int(count)))
    }

    // MARK: 加删

    /// 加一块虚拟屏，等它进入屏幕列表（截屏用的 ScreenCaptureKit 也看得到）后返回。
    /// 返回的 DisplayInfo 尺寸是像素，is_virtual 为真。hiDPI 默认关（内核协议里没有这个参数），
    /// 环境变量 LEMONGALAXY_VD_HIDPI=1 时按 2 倍缩放建（还没在真机上验证过效果）。
    func add(width: UInt32, height: UInt32, refresh: UInt32) -> Result<DisplayInfo, VirtualDisplayError> {
        guard available else { return .failure(.init("这台 Mac 的系统不支持虚拟屏")) }
        // 占住一个空槽位（建屏期间也算占用），返回前放开
        let slot: Int? = lock.locked {
            let used = Set(entries.values.map(\.slot)).union(reserved)
            let s = (0..<Self.slotCount).first { !used.contains($0) }
            if let s { reserved.insert(s) }
            return s
        }
        guard let slot else { return .failure(.init("虚拟屏已用完（最多 \(Self.slotCount) 块）")) }
        defer { lock.locked { _ = reserved.remove(slot) } }
        let hiDPI = ProcessInfo.processInfo.environment["LEMONGALAXY_VD_HIDPI"] == "1" && width % 2 == 0 && height % 2 == 0
        let name = Self.displayName(slot)
        // 序列号按「系统用户 + 槽位」固定：系统按厂商 / 型号 / 序列号记住每块屏的排列；
        // 同一台 Mac 上登录界面与各用户的代理、主控端与纯被控端同时有屏时也不会撞号（macOS 14 起要求互不相同）
        let serial = 0x4C47_0000 | (UInt32(getuid() & 0xFFF) << 4) | UInt32(slot)
        let product: UInt32 = Bundle.main.bundleIdentifier?.hasSuffix(".worker") == true ? 0x0002 : 0x0001
        let token = lock.locked { () -> Int in defer { nextToken += 1 }; return nextToken }

        // 主线程上建；超时后主线程才执行到的那次立即删掉，免得留下没人管的屏
        let done = DispatchSemaphore(value: 0)
        let state = CreateState()
        let create: @Sendable () -> Void = { [self] in
            if state.abandoned { return }
            var err: NSString?
            let d = LGVirtualDisplay(name: name, vendorID: Self.vendorID, productID: product, serialNumber: serial,
                                     width: width, height: height, refreshRate: Double(refresh), hiDPI: hiDPI, queue: queue,
                                     onTerminated: { [weak self] in self?.terminated(token: token) }, error: &err)
            state.set(d, error: err as String?)
            done.signal()
        }
        if Thread.isMainThread {
            create()
        } else {
            DispatchQueue.main.async(execute: create)
            if done.wait(timeout: .now() + 10) == .timedOut {
                if let late = state.abandon() { DispatchQueue.main.async { late.invalidate() } }
                return .failure(.init("加虚拟屏超时（主线程没有响应）"))
            }
        }
        guard let display = state.display else { return .failure(.init(state.error ?? "加虚拟屏失败")) }
        let id = display.displayID
        lock.locked { entries[id] = Entry(display: display, token: token, slot: slot, name: name, width: width, height: height) }
        NSLog("LemonGalaxy: 已加虚拟屏 %u（%@，%ux%u@%u%@）", id, name, width, height, refresh, hiDPI ? "，HiDPI" : "")

        // 等它进入屏幕列表（系统异步接入，一般 1 秒内）
        let active = Self.waitUntil(5) { Self.activeDisplays().contains(id) }
        if !active {
            _ = remove(id)
            return .failure(.init("虚拟屏没能接入桌面"))
        }
        // 再等截屏框架看到它：之后主控马上按这个屏幕号开画面
        if SystemPermissions.screenRecordingGranted, !Self.waitForShareable(id, timeout: 3) {
            NSLog("LemonGalaxy: 虚拟屏 %u 3 秒内没出现在 ScreenCaptureKit 的屏幕列表里，先照常返回", id)
        }
        let b = CGDisplayBounds(id)
        let scale = displayScale(id)
        return .success(DisplayInfo(id: id, name: name, width: UInt32(b.width * scale), height: UInt32(b.height * scale),
                                    scale: Float(scale), primary: id == CGMainDisplayID(), isVirtual: true))
    }

    /// 删掉本进程加的一块虚拟屏；不是本进程的或已经没有了算成功（返回 nil）。
    @discardableResult
    func remove(_ id: CGDirectDisplayID) -> String? {
        guard let e = lock.locked({ entries.removeValue(forKey: id) }) else { return nil }
        release(e.display)
        // 系统异步移除（实测约 0.7 秒，见调研文档）；等它从屏幕列表里消失，内核随后列屏时就不会再看到它
        if !Self.waitUntil(3, { !Self.activeDisplays().contains(id) }) {
            NSLog("LemonGalaxy: 虚拟屏 %u 释放后 3 秒仍在屏幕列表里", id)
        } else {
            NSLog("LemonGalaxy: 已删虚拟屏 %u（%@）", id, e.name)
        }
        return nil
    }

    /// 删掉本进程的全部虚拟屏（无人值守的会话代理离开屏幕、与后台服务断开时）
    func removeAll() {
        for id in ids { remove(id) }
    }

    // MARK: 内部

    private func release(_ d: LGVirtualDisplay) {
        if Thread.isMainThread {
            d.invalidate()
            return
        }
        let done = DispatchSemaphore(value: 0)
        DispatchQueue.main.async {
            d.invalidate()
            done.signal()
        }
        // 主线程忙时不死等：对象已从表里拿掉，主线程空下来就会释放
        _ = done.wait(timeout: .now() + 5)
    }

    /// 系统自己结束了某块屏（窗口服务器重启等）：从表里去掉，内核下次对账时就知道它没了。
    /// 我们自己删屏后系统也可能回调，那时表里已经没有这个编号，什么也不做
    private func terminated(token: Int) {
        let gone = lock.locked { () -> [CGDirectDisplayID] in
            let ids = entries.filter { $0.value.token == token }.map(\.key)
            for id in ids { entries[id] = nil }
            return ids
        }
        for id in gone { NSLog("LemonGalaxy: 虚拟屏 %u 被系统结束", id) }
    }

    private static func displayName(_ slot: Int) -> String {
        let zh = Locale.preferredLanguages.first?.hasPrefix("zh") ?? false
        return zh ? "LemonGalaxy 虚拟屏 \(slot + 1)" : "LemonGalaxy Virtual Display \(slot + 1)"
    }

    private static func waitUntil(_ seconds: Double, _ cond: () -> Bool) -> Bool {
        let end = Date().addingTimeInterval(seconds)
        while true {
            if cond() { return true }
            if Date() >= end { return false }
            Thread.sleep(forTimeInterval: 0.05)
        }
    }

    private static func waitForShareable(_ id: CGDirectDisplayID, timeout: Double) -> Bool {
        let end = Date().addingTimeInterval(timeout)
        while Date() < end {
            let sem = DispatchSemaphore(value: 0)
            nonisolated(unsafe) var found = false
            Task.detached {
                let c = try? await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
                found = c?.displays.contains { $0.displayID == id } ?? false
                sem.signal()
            }
            if sem.wait(timeout: .now() + max(0.1, end.timeIntervalSinceNow)) == .timedOut { return false }
            if found { return true }
            Thread.sleep(forTimeInterval: 0.1)
        }
        return false
    }

    /// 主线程建屏与等待方之间交接结果（等待方超时放弃后，晚到的结果要删掉）
    private final class CreateState: @unchecked Sendable {
        private let lock = NSLock()
        private var _display: LGVirtualDisplay?
        private var _error: String?
        private var _abandoned = false
        var abandoned: Bool { lock.locked { _abandoned } }
        var display: LGVirtualDisplay? { lock.locked { _display } }
        var error: String? { lock.locked { _error } }

        func set(_ d: LGVirtualDisplay?, error: String?) {
            let late = lock.locked { () -> LGVirtualDisplay? in
                if _abandoned { return d }
                _display = d
                _error = error
                return nil
            }
            late?.invalidate()
        }

        /// 放弃等待；已经建好的返回给调用方去删
        func abandon() -> LGVirtualDisplay? {
            lock.locked {
                _abandoned = true
                defer { _display = nil }
                return _display
            }
        }
    }
}

struct VirtualDisplayError: Error {
    let message: String
    init(_ message: String) { self.message = message }
}
