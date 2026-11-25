import AppKit
import SwiftUI
import Combine
import CoreServices
import UniformTypeIdentifiers

// 复制粘贴文件「粘贴时才传」的 macOS 粘贴方（docs/protocol.md 8.4.1）。Mac 主控端（远端复制 → 本机粘贴）、
// 菜单栏被控端与无人值守的会话代理（主控端复制 → 这台 Mac 粘贴）共用这一份（被控端经 Shared/ 符号链接）。
// 访达不支持从剪贴板延迟粘贴，照 UU 远程的做法：
//   1. 剪贴板里只放一个隐藏的 0 字节标记文件 .lemongalaxy-paste-<清单 id 十六进制>（在本应用的缓存目录里）；
//   2. FSEvents 盯全盘（文件级事件）：访达在哪个文件夹粘贴，标记文件就被拷到哪里；
//   3. 删掉拷过去的标记，在那个文件夹按清单写入（顶层重名按访达的规则加「 2」），弹出仿访达的拷贝进度窗口，可停止；
//   4. 剪贴板被换掉（changeCount 变了）后再盯几秒（正在进行的粘贴还认得出来）就停止监视、删掉标记。
// 读内容的 open(index, offset) 会阻塞到对方应答，只在后台队列上调用。粘贴到桌面、文稿、下载等受保护的文件夹时，
// 系统会在第一次写入时问一次是否允许本应用访问。

/// 打开清单第 index 项（文件），从 offset 读到末尾；阻塞，只在后台线程调用
typealias ClipOpen = @Sendable (_ index: UInt32, _ offset: UInt64) throws -> ClipReaderProtocol

/// 放清单、盯粘贴、管正在进行的拷贝（每个进程一份）
final class LazyPaste: @unchecked Sendable {
    static let shared = LazyPaste()
    static let markerPrefix = ".lemongalaxy-paste-"

    private let lock = NSLock()
    private var _pasteboard: NSPasteboard?
    private var _markerDir: URL?
    private var _showsWindow = true
    private var _retireGrace: Double = 2
    private var _prepareUI: (@Sendable () -> Void)?
    private var _onIdle: (@Sendable () -> Void)?
    private var _onJobFinished: (@Sendable (PasteJob) -> Void)?
    private var current: PasteOffer?
    private var retiring: [PasteOffer] = []
    private var running: [PasteJob] = []

    /// 剪贴板（自检换成私有剪贴板，不碰用户的）
    var pasteboard: NSPasteboard {
        get { lock.locked { _pasteboard } ?? .general }
        set { lock.locked { _pasteboard = newValue } }
    }

    /// 标记文件所在目录：~/Library/Caches/<Bundle ID>/paste（自检换成临时目录）
    var markerDir: URL {
        get {
            if let d = lock.locked({ _markerDir }) { return d }
            let id = Bundle.main.bundleIdentifier ?? "cn.lemonit.galaxy"
            return FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
                .appendingPathComponent(id, isDirectory: true).appendingPathComponent("paste", isDirectory: true)
        }
        set { lock.locked { _markerDir = newValue } }
    }

    /// 粘贴时弹不弹进度窗口（自检关掉）
    var showsWindow: Bool {
        get { lock.locked { _showsWindow } }
        set { lock.locked { _showsWindow = newValue } }
    }

    /// 剪贴板被换掉后继续监视的秒数
    var retireGrace: Double {
        get { lock.locked { _retireGrace } }
        set { lock.locked { _retireGrace = newValue } }
    }

    /// 弹窗口之前要做的事（无人值守的会话代理平时不建 NSApplication，弹窗前先接上 AppKit）；在后台线程调用，可以阻塞
    var prepareUI: (@Sendable () -> Void)? {
        get { lock.locked { _prepareUI } }
        set { lock.locked { _prepareUI = newValue } }
    }

    /// 变得空闲（没有清单在监视、也没有正在拷贝）时调用（会话代理据此决定能否退出重启）
    var onIdle: (@Sendable () -> Void)? {
        get { lock.locked { _onIdle } }
        set { lock.locked { _onIdle = newValue } }
    }

    /// 每次拷贝结束（完成、失败或停止）时在后台线程调用（自检用）
    var onJobFinished: (@Sendable (PasteJob) -> Void)? {
        get { lock.locked { _onJobFinished } }
        set { lock.locked { _onJobFinished = newValue } }
    }

    /// 有清单在监视或正在拷贝
    var busy: Bool { lock.locked { current != nil || !retiring.isEmpty || !running.isEmpty } }
    /// 正在拷贝的
    var jobs: [PasteJob] { lock.locked { running } }
    /// 当前剪贴板里的清单 id（没有为 nil）
    var currentOfferId: UInt64? { lock.locked { current?.info.id } }
    /// 当前清单的标记文件
    var currentMarker: URL? { lock.locked { current?.marker } }

    // MARK: 标记识别（剪贴板同步时不把自己放的标记当成「复制了文件」）

    static func isMarker(_ url: URL) -> Bool { url.lastPathComponent.hasPrefix(markerPrefix) }

    static func isMarker(path: String) -> Bool { (path as NSString).lastPathComponent.hasPrefix(markerPrefix) }

    /// 剪贴板里有没有本应用放的标记文件
    static func holdsMarker(_ pb: NSPasteboard) -> Bool {
        let urls = pb.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL] ?? []
        return urls.contains(where: isMarker)
    }

    static func markerName(_ id: UInt64) -> String { markerPrefix + String(format: "%016llx", id) }

    // MARK: 放清单

    /// 把清单放进剪贴板，等有人在访达里粘贴（任意线程；很快返回）。成功返回 nil，失败返回原因
    @discardableResult
    func offer(_ info: ClipOfferInfo, open: @escaping ClipOpen) -> String? {
        if let bad = Self.validate(info) { return bad }
        let fm = FileManager.default
        let dir = markerDir
        let name = Self.markerName(info.id)
        let marker = dir.appendingPathComponent(name)
        do {
            try fm.createDirectory(at: dir, withIntermediateDirectories: true)
            // 以前留下的标记（进程退出时没来得及删）一并清掉；还在收尾的清单的标记留着
            let keep = lock.locked { Set(retiring.map(\.marker.lastPathComponent)) }
            for old in (try? fm.contentsOfDirectory(atPath: dir.path)) ?? [] where old.hasPrefix(Self.markerPrefix) && old != name && !keep.contains(old) {
                try? fm.removeItem(at: dir.appendingPathComponent(old))
            }
            if !fm.fileExists(atPath: marker.path) { try Data().write(to: marker) }
        } catch {
            return L("paste.error.marker", error.localizedDescription)
        }
        let o = PasteOffer(owner: self, info: info, open: open, marker: marker, markerDir: dir)
        // 先盯上再放进剪贴板，放进去之后的粘贴一定能认出来
        guard o.startWatching() else {
            try? fm.removeItem(at: marker)
            return L("paste.error.watch")
        }
        let pb = pasteboard
        pb.clearContents()
        pb.writeObjects([marker as NSURL])
        o.arm(changeCount: pb.changeCount, pasteboard: pb)
        let old = lock.locked { () -> PasteOffer? in
            defer { current = o }
            return current
        }
        if let old { retire(old, grace: 0) }
        return nil
    }

    /// 撤回清单（只在剪贴板还是它时清空剪贴板）；正在进行的拷贝不受影响
    func withdraw(offerId: UInt64) {
        guard let o = lock.locked({ current?.info.id == offerId ? current : nil }) else { return }
        let pb = pasteboard
        if pb.changeCount == o.changeCount { pb.clearContents() }
        retire(o, grace: 0)
    }

    /// 清单不再是剪贴板内容：grace 秒后停止监视、删掉标记
    fileprivate func retire(_ o: PasteOffer, grace: Double) {
        let first = lock.locked { () -> Bool in
            if current === o { current = nil }
            guard !retiring.contains(where: { $0 === o }) else { return false }
            retiring.append(o)
            return true
        }
        guard first else { return }
        o.stop(after: grace)
    }

    /// 监视停了（PasteOffer 自己调用）
    fileprivate func stopped(_ o: PasteOffer) {
        // 先删标记再从列表里拿掉（拿掉之后 busy 才变假）；同名标记还有别的清单在用就不删
        let inUse = lock.locked {
            (current !== o && current?.marker == o.marker) || retiring.contains { $0 !== o && $0.marker == o.marker }
        }
        if !inUse { try? FileManager.default.removeItem(at: o.marker) }
        let idle = lock.locked { () -> Bool in
            retiring.removeAll { $0 === o }
            if current === o { current = nil }
            return current == nil && retiring.isEmpty && running.isEmpty
        }
        if idle { onIdle?() }
    }

    // MARK: 粘贴

    /// 在 folder 里认出了标记（监视队列上）
    fileprivate func pasted(_ o: PasteOffer, folder: URL, marker: URL) {
        let job = PasteJob(info: o.info, open: o.open, folder: folder, marker: marker, owner: self)
        lock.locked { running.append(job) }
        NSLog("LemonGalaxy: 在 %@ 粘贴清单 %016llx（%d 项，%llu 字节）", folder.path, o.info.id, o.info.entries.count, o.info.totalBytes)
        if showsWindow {
            let prepare = prepareUI
            DispatchQueue.global(qos: .userInitiated).async {
                prepare?()
                DispatchQueue.main.async { MainActor.assumeIsolated { PasteProgressCenter.shared.add(job) } }
            }
        }
        job.start()
    }

    fileprivate func finished(_ job: PasteJob) {
        let idle = lock.locked { () -> Bool in
            running.removeAll { $0 === job }
            return current == nil && retiring.isEmpty && running.isEmpty
        }
        onJobFinished?(job)
        if idle { onIdle?() }
    }

    // MARK: 清单校验（内核校验过，这里落盘前再守一道）

    /// 只接受安全的相对路径：没有空段、`.`、`..`、反斜杠、NUL；父文件夹排在前面；不重复
    static func validate(_ info: ClipOfferInfo) -> String? {
        guard !info.entries.isEmpty else { return L("paste.error.badList") }
        var dirs = Set<String>()
        var seen = Set<String>()
        for e in info.entries {
            let parts = e.path.split(separator: "/", omittingEmptySubsequences: false)
            let ok = !parts.isEmpty && parts.allSatisfy { p in
                !p.isEmpty && p != "." && p != ".." && !p.contains("\\") && !p.contains("\0")
            }
            guard ok, seen.insert(e.path).inserted else { return L("paste.error.badList") }
            if parts.count > 1 && !dirs.contains(parts.dropLast().joined(separator: "/")) { return L("paste.error.badList") }
            if e.isDir { dirs.insert(e.path) }
        }
        return nil
    }

    /// 访达的重名规则：「名字 2.扩展名」「名字 3.扩展名」…（文件夹不拆扩展名）
    static func uniqueName(_ name: String, isDir: Bool, taken: (String) -> Bool) -> String {
        guard taken(name) else { return name }
        let ns = name as NSString
        let ext = isDir ? "" : ns.pathExtension
        let base = ext.isEmpty ? name : ns.deletingPathExtension
        var n = 2
        while true {
            let candidate = ext.isEmpty ? "\(base) \(n)" : "\(base) \(n).\(ext)"
            if !taken(candidate) { return candidate }
            n += 1
        }
    }

    /// 出错原因（内核错误取说明文字）
    static func describe(_ error: Error) -> String {
        if let g = error as? GalaxyError {
            switch g {
            case .Network(let m), .Permission(let m), .NotFound(let m), .Protocol(let m), .Other(let m): return m
            case .Auth(_, let m): return m
            case .Cancelled: return L("paste.cancelled")
            }
        }
        if let p = error as? PasteJob.Failure { return p.message }
        return error.localizedDescription
    }
}

// MARK: - 一份清单的监视

/// 剪贴板里的一份清单：FSEvents 盯全盘的标记文件出现、定时看剪贴板换没换
final class PasteOffer: @unchecked Sendable {
    let info: ClipOfferInfo
    let open: ClipOpen
    let marker: URL
    private weak var owner: LazyPaste?
    private let markerName: String
    private let markerDir: URL
    private let queue = DispatchQueue(label: "galaxy.paste.watch", qos: .userInitiated)
    private var stream: FSEventStreamRef?
    private var timer: DispatchSourceTimer?
    private weak var pasteboard: NSPasteboard?
    private let lock = NSLock()
    private var _changeCount = -1
    private var replacedAt: Date?
    private var stopped = false
    /// 认出过的标记（设备号:inode:路径 → 时间）：同一次拷贝会来好几个事件
    private var handled: [String: Date] = [:]
    /// 自己目录的设备号与 inode（不认自己目录里的标记）
    private var ownDir: (dev_t, ino_t)?

    var changeCount: Int { lock.locked { _changeCount } }

    init(owner: LazyPaste, info: ClipOfferInfo, open: @escaping ClipOpen, marker: URL, markerDir: URL) {
        self.owner = owner
        self.info = info
        self.open = open
        self.marker = marker
        self.markerName = marker.lastPathComponent
        self.markerDir = markerDir
        var st = stat()
        if stat(markerDir.path, &st) == 0 { ownDir = (st.st_dev, st.st_ino) }
    }

    /// 全盘（/ 与挂在 /Volumes 下的本地卷）文件级事件，延迟 0.05 秒
    func startWatching() -> Bool {
        var paths = ["/"]
        let vols = FileManager.default.mountedVolumeURLs(includingResourceValuesForKeys: [.volumeIsLocalKey], options: [.skipHiddenVolumes]) ?? []
        for v in vols where v.path != "/" && (try? v.resourceValues(forKeys: [.volumeIsLocalKey]).volumeIsLocal) == true {
            paths.append(v.path)
        }
        var ctx = FSEventStreamContext(version: 0, info: Unmanaged.passUnretained(self).toOpaque(), retain: nil, release: nil, copyDescription: nil)
        let flags = UInt32(kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagNoDefer)
        let callback: FSEventStreamCallback = { _, info, count, rawPaths, _, _ in
            guard let info else { return }
            let me = Unmanaged<PasteOffer>.fromOpaque(info).takeUnretainedValue()
            me.events(rawPaths.assumingMemoryBound(to: UnsafePointer<CChar>.self), count)
        }
        guard let s = FSEventStreamCreate(nil, callback, &ctx, paths as CFArray, FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 0.05, flags) else {
            return false
        }
        FSEventStreamSetDispatchQueue(s, queue)
        guard FSEventStreamStart(s) else {
            FSEventStreamInvalidate(s)
            FSEventStreamRelease(s)
            return false
        }
        stream = s
        return true
    }

    /// 已放进剪贴板：记下 changeCount，之后每 0.3 秒看一次剪贴板换没换
    func arm(changeCount: Int, pasteboard pb: NSPasteboard) {
        lock.locked { _changeCount = changeCount }
        pasteboard = pb
        let t = DispatchSource.makeTimerSource(queue: queue)
        t.schedule(deadline: .now() + 0.3, repeating: 0.3)
        t.setEventHandler { [weak self] in self?.checkPasteboard() }
        timer = t
        t.resume()
    }

    private func checkPasteboard() {
        guard let pb = pasteboard, !lock.locked({ stopped || replacedAt != nil }) else { return }
        if pb.changeCount != changeCount {
            lock.locked { replacedAt = Date() }
            owner?.retire(self, grace: owner?.retireGrace ?? 2)
        }
    }

    /// grace 秒后停止监视（监视队列上做，回调不会再来）
    func stop(after grace: Double) {
        queue.asyncAfter(deadline: .now() + max(0, grace)) { [self] in
            guard lock.locked({ () -> Bool in defer { stopped = true }; return !stopped }) else { return }
            timer?.cancel()
            timer = nil
            if let s = stream {
                FSEventStreamStop(s)
                FSEventStreamInvalidate(s)
                FSEventStreamRelease(s)
                stream = nil
            }
            owner?.stopped(self)
        }
    }

    /// FSEvents 回调（监视队列上）：只挑名字等于标记名的路径
    fileprivate func events(_ paths: UnsafeMutablePointer<UnsafePointer<CChar>>, _ count: Int) {
        guard !lock.locked({ stopped }) else { return }
        let needle = markerName
        for i in 0..<count {
            let raw = paths[i]
            // 先用 C 字符串比较（全盘事件很多），命中了再建 String
            guard needle.withCString({ strstr(raw, $0) }) != nil else { continue }
            let path = String(cString: raw)
            guard (path as NSString).lastPathComponent == needle else { continue }
            check(path)
        }
    }

    private func check(_ path: String) {
        let dir = (path as NSString).deletingLastPathComponent
        var st = stat()
        let r = lstat(path, &st)
        // 不存在（删除事件、自己刚删掉的）不算；受保护的文件夹里没授权时 stat 可能被拒，照样当成粘贴，写入时系统会问
        if r != 0 && errno == ENOENT { return }
        if r == 0 && (st.st_mode & S_IFMT) != S_IFREG { return }
        var dst = stat()
        if stat(dir, &dst) == 0, let own = ownDir, dst.st_dev == own.0, dst.st_ino == own.1 { return }
        if dir == markerDir.path { return }
        let key = r == 0 ? "\(st.st_dev):\(st.st_ino):\(path)" : "?:\(path)"
        let now = Date()
        handled = handled.filter { now.timeIntervalSince($0.value) < 10 }
        guard handled[key] == nil else { return }
        handled[key] = now
        // 「剪切」粘贴（⌥⌘V）会把我们的标记移走：补一个，下次还能粘贴
        if !FileManager.default.fileExists(atPath: marker.path) { try? Data().write(to: marker) }
        owner?.pasted(self, folder: URL(fileURLWithPath: dir, isDirectory: true), marker: URL(fileURLWithPath: path))
    }
}

// MARK: - 一次粘贴

/// 把清单写进访达粘贴的那个文件夹：后台队列上按项读、写，界面状态合并成每 0.1 秒最多一次推到主线程
final class PasteJob: ObservableObject, Identifiable, @unchecked Sendable {
    enum Phase: Equatable { case preparing, waiting, copying, done, cancelled, failed(String) }

    struct Failure: Error { let message: String }

    let id = UUID()
    let info: ClipOfferInfo
    let folder: URL
    /// 落盘后的顶层名字（重名时改过）
    var topNames: [String] { lock.locked { _topNames } }
    private let open: ClipOpen
    private let marker: URL?
    private weak var owner: LazyPaste?
    private let queue = DispatchQueue(label: "galaxy.paste.job", qos: .userInitiated)

    // 界面状态（主线程）
    @Published private(set) var phase: Phase = .preparing
    @Published private(set) var done: UInt64 = 0
    @Published private(set) var total: UInt64
    /// 估计还要多少秒（速度稳定后才有）
    @Published private(set) var remaining: Double?

    // 后台状态
    private let lock = NSLock()
    private var cancelled = false
    private var reader: ClipReaderProtocol?
    private var bgPhase: Phase = .preparing
    private var bgDone: UInt64 = 0
    private var bgTotal: UInt64
    private var bgRemaining: Double?
    private var samples: [(Date, UInt64)] = []
    private var copyingSince: Date?
    private var uiScheduled = false
    private var _topNames: [String] = []
    /// 图标（主线程上取一次）
    @MainActor private var _icon: NSImage?

    static let chunk: UInt32 = 1 << 20

    init(info: ClipOfferInfo, open: @escaping ClipOpen, folder: URL, marker: URL?, owner: LazyPaste?) {
        self.info = info
        self.open = open
        self.folder = folder
        self.marker = marker
        self.owner = owner
        total = info.totalBytes
        bgTotal = info.totalBytes
        _topNames = info.entries.filter { !$0.path.contains("/") }.map(\.path)
    }

    /// 线程安全的当前状态（自检与非界面代码用）
    var state: (phase: Phase, done: UInt64, total: UInt64) { lock.locked { (bgPhase, bgDone, bgTotal) } }

    var folderName: String { FileManager.default.displayName(atPath: folder.path) }

    func start() { queue.async { [self] in run() } }

    /// 界面预览用（selftest 截图）：直接设成某个状态，不拷贝（主线程）
    @MainActor func preview(_ p: Phase, done d: UInt64, remaining r: Double?) {
        lock.locked {
            bgPhase = p
            bgDone = d
            bgRemaining = r
        }
        phase = p
        done = d
        remaining = r
    }

    /// 停止：关掉正在读的（阻塞的 read 随之报错返回），写了一半的顶层项删掉
    func cancel() {
        let (go, r) = lock.locked { () -> (Bool, ClipReaderProtocol?) in
            guard !isFinished(bgPhase), !cancelled else { return (false, nil) }
            cancelled = true
            // 界面马上显示已停止（还在等对方应答的 open 返回后才真正收尾）
            bgPhase = .cancelled
            return (true, reader)
        }
        guard go else { return }
        postUI()
        if let r { DispatchQueue.global().async { r.cancel() } }
    }

    private func isFinished(_ p: Phase) -> Bool {
        switch p {
        case .done, .cancelled, .failed: return true
        default: return false
        }
    }

    private var isCancelled: Bool { lock.locked { cancelled } }

    private func setPhase(_ p: Phase) {
        let changed = lock.locked { () -> Bool in
            // 停止之后只认最终状态
            if cancelled && !isFinished(p) { return false }
            if p == .copying && copyingSince == nil { copyingSince = Date() }
            bgPhase = p
            return true
        }
        if changed { postUI() }
    }

    private func add(_ n: UInt64) {
        lock.locked {
            bgDone += n
            let now = Date()
            samples.append((now, bgDone))
            samples.removeAll { now.timeIntervalSince($0.0) > 4 }
            if let since = copyingSince, now.timeIntervalSince(since) >= 1.5, let first = samples.first, now.timeIntervalSince(first.0) > 0.5 {
                let speed = Double(bgDone - first.1) / now.timeIntervalSince(first.0)
                bgRemaining = speed > 0 ? Double(bgTotal > bgDone ? bgTotal - bgDone : 0) / speed : nil
            }
        }
        postUI()
    }

    private func postUI() {
        let go = lock.locked { () -> Bool in
            guard !uiScheduled else { return false }
            uiScheduled = true
            return true
        }
        guard go else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { [self] in
            let s = lock.locked { () -> (Phase, UInt64, UInt64, Double?) in
                uiScheduled = false
                return (bgPhase, bgDone, bgTotal, bgRemaining)
            }
            if phase != s.0 { phase = s.0 }
            done = s.1
            total = s.2
            remaining = s.3
        }
    }

    // MARK: 拷贝

    private func run() {
        let fm = FileManager.default
        // 访达拷过来的标记：先删掉（受保护的文件夹里这一步可能先等用户在系统的询问里点允许）
        if let marker { try? fm.removeItem(at: marker) }
        let entries = info.entries
        // 顶层重名按访达的规则改名（同一次粘贴里已经占用的名字也算）
        var chosen = Set<String>()
        var rename: [String: String] = [:]
        var lastIndex: [String: Int] = [:]
        for (i, e) in entries.enumerated() {
            let top = String(e.path.split(separator: "/").first ?? "")
            lastIndex[top] = i
            guard !e.path.contains("/") else { continue }
            let name = LazyPaste.uniqueName(e.path, isDir: e.isDir) { n in
                chosen.contains(n.lowercased()) || fm.fileExists(atPath: folder.appendingPathComponent(n).path)
            }
            chosen.insert(name.lowercased())
            rename[e.path] = name
        }
        let renamed = entries.filter { !$0.path.contains("/") }.map { rename[$0.path] ?? $0.path }
        lock.locked { _topNames = renamed }
        var started = Set<String>()
        var completed = Set<String>()
        var failure: String?
        for (i, e) in entries.enumerated() {
            if isCancelled { break }
            var parts = e.path.split(separator: "/").map(String.init)
            let top = parts[0]
            parts[0] = rename[top] ?? top
            let dest = parts.reduce(folder) { $0.appendingPathComponent($1) }
            started.insert(top)
            do {
                if e.isDir {
                    do {
                        try fm.createDirectory(at: dest, withIntermediateDirectories: false)
                    } catch {
                        var isDir: ObjCBool = false
                        guard fm.fileExists(atPath: dest.path, isDirectory: &isDir), isDir.boolValue else {
                            throw Failure(message: L("paste.error.write", dest.lastPathComponent, error.localizedDescription))
                        }
                    }
                } else {
                    try copyFile(UInt32(i), e, to: dest)
                }
                if e.mtime > 0 { try? fm.setAttributes([.modificationDate: Date(timeIntervalSince1970: TimeInterval(e.mtime))], ofItemAtPath: dest.path) }
            } catch {
                if !isCancelled { failure = LazyPaste.describe(error) }
                break
            }
            if lastIndex[top] == i { completed.insert(top) }
        }
        let aborted = failure != nil || isCancelled
        if aborted {
            // 没拷完的顶层项删掉（与访达停止拷贝时一样，不留半截文件）
            for top in started.subtracting(completed) {
                try? fm.removeItem(at: folder.appendingPathComponent(rename[top] ?? top))
            }
        } else {
            // 文件夹的修改时间在写完内容后再设一次（写内容会把它改成现在）
            for e in entries.reversed() where e.isDir && e.mtime > 0 {
                var parts = e.path.split(separator: "/").map(String.init)
                parts[0] = rename[parts[0]] ?? parts[0]
                let dest = parts.reduce(folder) { $0.appendingPathComponent($1) }
                try? fm.setAttributes([.modificationDate: Date(timeIntervalSince1970: TimeInterval(e.mtime))], ofItemAtPath: dest.path)
            }
        }
        let final: Phase = failure.map { .failed($0) } ?? (isCancelled ? .cancelled : .done)
        if case .failed(let m) = final { NSLog("LemonGalaxy: 粘贴到 %@ 失败：%@", folder.path, m) }
        lock.locked { reader = nil }
        setPhase(final)
        owner?.finished(self)
    }

    /// 写入失败（不重试）
    private struct WriteFailure: Error { let message: String }

    /// 一个文件：打开读取器从 offset 读到末尾写进 dest；读失败（断线等）从断点重新打开，最多重试 2 次
    private func copyFile(_ index: UInt32, _ e: ClipOfferEntry, to dest: URL) throws {
        let fd = Darwin.open(dest.path, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0o644)
        guard fd >= 0 else {
            throw Failure(message: L("paste.error.write", dest.lastPathComponent, String(cString: strerror(errno))))
        }
        defer { Darwin.close(fd) }
        let name = (e.path as NSString).lastPathComponent
        var offset: UInt64 = 0
        var retries = 0
        var sized = false
        while true {
            if isCancelled { throw Failure(message: L("paste.cancelled")) }
            // 还没收到任何内容或断线重连时显示「正在等待」
            if retries > 0 || lock.locked({ bgDone == 0 }) { setPhase(.waiting) }
            let r: ClipReaderProtocol
            do {
                r = try open(index, offset)
            } catch {
                if isCancelled { throw error }
                if retries < 2 { retries += 1; Thread.sleep(forTimeInterval: 1); continue }
                throw Failure(message: L("paste.error.read", name, LazyPaste.describe(error)))
            }
            let cancelledNow = lock.locked { () -> Bool in
                reader = r
                return cancelled
            }
            if cancelledNow { r.cancel(); throw Failure(message: L("paste.cancelled")) }
            if !sized {
                sized = true
                // 文件复制后又变了：总量按现在的大小算
                let now = r.size()
                if now != e.size { lock.locked { bgTotal = bgTotal - min(bgTotal, e.size) + now } }
            }
            setPhase(.copying)
            do {
                while !isCancelled {
                    let data = try r.read(max: Self.chunk)
                    if data.isEmpty { break }
                    try write(fd, data, name: dest.lastPathComponent)
                    offset += UInt64(data.count)
                    add(UInt64(data.count))
                }
                lock.locked { reader = nil }
                r.cancel()
                if isCancelled { throw Failure(message: L("paste.cancelled")) }
                return
            } catch let w as WriteFailure {
                lock.locked { reader = nil }
                r.cancel()
                throw Failure(message: w.message)
            } catch {
                lock.locked { reader = nil }
                r.cancel()
                if isCancelled { throw error }
                if retries < 2 { retries += 1; Thread.sleep(forTimeInterval: 1); continue }
                throw Failure(message: L("paste.error.read", name, LazyPaste.describe(error)))
            }
        }
    }

    private func write(_ fd: Int32, _ data: Data, name: String) throws {
        let ok = data.withUnsafeBytes { raw -> Bool in
            guard var p = raw.baseAddress else { return true }
            var left = raw.count
            while left > 0 {
                let n = Darwin.write(fd, p, left)
                if n < 0 {
                    if errno == EINTR { continue }
                    return false
                }
                left -= n
                p += n
            }
            return true
        }
        if !ok { throw WriteFailure(message: L("paste.error.write", name, String(cString: strerror(errno)))) }
    }

    /// 一项时用它的图标（文件夹、按扩展名的文件类型），多项时用「多个文稿」
    @MainActor var icon: NSImage {
        if let i = _icon { return i }
        let tops = info.entries.filter { !$0.path.contains("/") }
        let image: NSImage
        if tops.count == 1, let e = tops.first {
            image = e.isDir ? NSWorkspace.shared.icon(for: .folder)
                : NSWorkspace.shared.icon(for: UTType(filenameExtension: (e.path as NSString).pathExtension) ?? .data)
        } else {
            image = NSImage(named: NSImage.multipleDocumentsName) ?? NSWorkspace.shared.icon(for: .data)
        }
        _icon = image
        return image
    }
}

// MARK: - 仿访达的拷贝进度窗口

/// 所有正在进行的拷贝放在同一个窗口里（与访达相同），一项一行；完成或停止的行自动收起，失败的留着等关闭
@MainActor
final class PasteProgressCenter: ObservableObject {
    static let shared = PasteProgressCenter()

    @Published private(set) var jobs: [PasteJob] = []
    private var window: NSWindow?
    private var watchers: [UUID: AnyObject] = [:]

    func add(_ job: PasteJob) {
        guard !jobs.contains(where: { $0 === job }) else { return }
        jobs.append(job)
        // 结束了：完成、停止的稍后收起；失败的留着
        let token = job.$phase.sink { [weak self, weak job] p in
            guard let job else { return }
            switch p {
            case .done, .cancelled:
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) { MainActor.assumeIsolated { self?.remove(job) } }
            default: break
            }
        }
        watchers[job.id] = token as AnyObject
        show()
    }

    func remove(_ job: PasteJob) {
        jobs.removeAll { $0 === job }
        watchers[job.id] = nil
        if jobs.isEmpty { window?.orderOut(nil) }
    }

    /// 行尾的按钮：进行中是停止，失败后是关闭
    func stopOrDismiss(_ job: PasteJob) {
        switch job.phase {
        case .failed, .done, .cancelled: remove(job)
        default: job.cancel()
        }
    }

    private func show() {
        if window == nil {
            let host = NSHostingController(rootView: PasteProgressView(center: self))
            host.sizingOptions = [.preferredContentSize]
            let w = NSWindow(contentViewController: host)
            w.styleMask = [.titled, .closable, .miniaturizable]
            w.isReleasedWhenClosed = false
            w.collectionBehavior = [.moveToActiveSpace]
            w.title = L("paste.window.title")
            w.center()
            window = w
        }
        window?.title = L("paste.window.title")
        window?.orderFrontRegardless()
    }
}

struct PasteProgressView: View {
    @ObservedObject var center: PasteProgressCenter

    var body: some View {
        PasteProgressList(jobs: center.jobs) { center.stopOrDismiss($0) }
    }
}

/// 窗口内容：一项一行（selftest 直接拿它离屏截图）
struct PasteProgressList: View {
    let jobs: [PasteJob]
    let action: (PasteJob) -> Void
    @ObservedObject private var l10n = Localization.shared

    var body: some View {
        VStack(spacing: 0) {
            ForEach(Array(jobs.enumerated()), id: \.element.id) { i, job in
                if i > 0 { Divider().padding(.horizontal, 16) }
                PasteJobRow(job: job, loc: l10n.strings) { action(job) }
            }
        }
        .frame(width: 460)
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// 一行：图标、「正在将 … 拷贝到 …」、进度条与停止按钮、已拷贝多少与剩余时间（失败时是原因）
struct PasteJobRow: View {
    @ObservedObject var job: PasteJob
    let loc: Strings
    let action: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(nsImage: job.icon).resizable().interpolation(.high).frame(width: 36, height: 36)
            VStack(alignment: .leading, spacing: 6) {
                Text(title).font(.system(size: 13)).lineLimit(1).truncationMode(.middle)
                HStack(spacing: 8) {
                    bar
                    Button(action: action) {
                        Image(systemName: "xmark.circle.fill")
                            .font(.system(size: 14))
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                    .help(loc(isFailed ? "paste.close" : "paste.stop"))
                }
                Text(detail)
                    .font(.system(size: 11).monospacedDigit())
                    .foregroundStyle(isFailed ? Color(nsColor: .systemRed) : .secondary)
                    .lineLimit(isFailed ? 3 : 1)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
    }

    private var isFailed: Bool { if case .failed = job.phase { return true }; return false }

    @ViewBuilder private var bar: some View {
        switch job.phase {
        case .preparing:
            ProgressView().progressViewStyle(.linear).controlSize(.small)
        case .waiting where job.done == 0:
            ProgressView().progressViewStyle(.linear).controlSize(.small)
        case .failed:
            ProgressView(value: fraction).progressViewStyle(.linear).controlSize(.small).tint(Color(nsColor: .systemRed))
        default:
            ProgressView(value: fraction).progressViewStyle(.linear).controlSize(.small)
        }
    }

    private var fraction: Double {
        if job.phase == .done { return 1 }
        return job.total > 0 ? min(1, Double(job.done) / Double(job.total)) : 0
    }

    private var title: String {
        let names = job.topNames
        let folder = job.folderName
        return names.count == 1 ? loc("paste.copyingOne", names[0], folder) : loc("paste.copyingMany", names.count, folder)
    }

    private var detail: String {
        switch job.phase {
        case .preparing: return loc("paste.preparing")
        case .waiting where job.done == 0: return loc("paste.waiting")
        case .failed(let m): return loc("paste.failed", m)
        case .cancelled: return loc("paste.cancelled")
        default:
            let f = ByteCountFormatter()
            f.countStyle = .file
            let progress = loc("paste.progress", f.string(fromByteCount: Int64(job.done)), f.string(fromByteCount: Int64(job.total)))
            guard job.phase != .done, let r = job.remaining else { return progress }
            return progress + " — " + loc("paste.remaining", Self.duration(r, loc))
        }
    }

    static func duration(_ seconds: Double, _ loc: Strings) -> String {
        let s = Int(seconds.rounded(.up))
        if s < 5 { return loc("paste.fewSeconds") }
        if s < 60 { return loc("paste.seconds", (s + 4) / 5 * 5) }
        let m = (s + 59) / 60
        if m < 60 { return m == 1 ? loc("paste.minute") : loc("paste.minutes", m) }
        return loc("paste.hours", m / 60, m % 60)
    }
}
