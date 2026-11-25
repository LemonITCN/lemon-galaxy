import AppKit

/// 粘贴时才传（LazyFilePaste.swift）的自检：不碰用户的剪贴板（私有剪贴板）、不开访达，
/// 用 FileManager 把标记文件拷进临时文件夹来模拟访达的粘贴，核对监视认得出来、文件按名字与内容落盘、
/// 访达式重名、断点续读、停止、失败、剪贴板换掉后停止监视、读剪贴板时过滤自己的标记。
/// Mac 主控端（--paste-selftest、--selftest）与 Mac 被控端（--paste-selftest）共用（被控端经 Shared/ 符号链接）。
enum LazyPasteSelfTest {
    /// 返回发现的问题（空表示全部通过）；verbose 时逐项打印
    static func run(verbose: Bool = true) -> [String] {
        var problems: [String] = []
        func check(_ name: String, _ ok: Bool, _ detail: @autoclosure () -> String = "") {
            if !ok { problems.append(name + (detail().isEmpty ? "" : "：" + detail())) }
            if verbose { print("\(ok ? "[通过]" : "[失败]") 粘贴时才传：\(name)\(ok ? "" : detail().isEmpty ? "" : "：" + detail())") }
        }
        let fm = FileManager.default
        let root = fm.temporaryDirectory.appendingPathComponent("lg-paste-\(getpid())-\(UInt32.random(in: 0...UInt32.max))", isDirectory: true)
        try? fm.createDirectory(at: root, withIntermediateDirectories: true)
        let pb = NSPasteboard(name: NSPasteboard.Name("cn.lemonit.galaxy.pastetest.\(getpid())"))
        let lp = LazyPaste.shared
        let saved = (lp.pasteboard, lp.markerDir, lp.showsWindow, lp.retireGrace, lp.onJobFinished)
        lp.pasteboard = pb
        lp.markerDir = root.appendingPathComponent("markers", isDirectory: true)
        lp.showsWindow = false
        lp.retireGrace = 0.4
        let finished = FinishedJobs()
        lp.onJobFinished = { finished.add($0) }
        defer {
            // 中途失败时留下的拷贝与清单收掉，不影响之后的检查
            for j in lp.jobs { j.cancel() }
            if let id = lp.currentOfferId { lp.withdraw(offerId: id) }
            _ = wait(3) { lp.busy ? nil : true }
            lp.pasteboard = saved.0
            lp.markerDir = saved.1
            lp.showsWindow = saved.2
            lp.retireGrace = saved.3
            lp.onJobFinished = saved.4
            pb.releaseGlobally()
            try? fm.removeItem(at: root)
        }

        // 纯函数
        check("访达式重名", LazyPaste.uniqueName("a.txt", isDir: false) { ["a.txt", "a 2.txt"].contains($0) } == "a 3.txt"
              && LazyPaste.uniqueName("x.y", isDir: true) { $0 == "x.y" } == "x.y 2"
              && LazyPaste.uniqueName(".hidden", isDir: false) { $0 == ".hidden" } == ".hidden 2"
              && LazyPaste.uniqueName("free.bin", isDir: false) { _ in false } == "free.bin")
        let badLists: [[ClipOfferEntry]] = [
            [entry("../x", false, 1)], [entry("a//b", false, 1)], [entry("a\\b", false, 1)], [entry("a/b", false, 1)],
            [entry("a", true, 0), entry("a/.", false, 1)], [entry("x", false, 1), entry("x", false, 1)], [],
        ]
        check("不安全的清单拒收", badLists.allSatisfy { LazyPaste.validate(ClipOfferInfo(id: 1, entries: $0, totalBytes: 1)) != nil })

        // 清单 id 每次随机（标记名里带 id；FSEvents 盯全盘，同一台机器上同时跑的另一份自检不会认错）
        let base = UInt64.random(in: 1...(UInt64.max >> 8)) << 8
        let idMain = base | 1, idHold = base | 2, idFail = base | 3, idHost = base | 4

        // 1. 放清单 → 模拟访达粘贴 → 落盘（顶层重名、文件夹、空文件、断点续读）
        let report = pattern(300_000, seed: 1)
        let big = pattern(1_500_000, seed: 7)
        let notes = Data("# 说明\n粘贴时才传\n".utf8)
        let entries = [
            entry("report.txt", false, UInt64(report.count)),
            entry("Assets", true, 0),
            entry("Assets/a.bin", false, UInt64(big.count)),
            entry("Assets/sub", true, 0),
            entry("Assets/sub/empty.txt", false, 0),
            entry("说明.md", false, UInt64(notes.count)),
        ]
        let source = FakeSource([report, nil, big, nil, Data(), notes])
        source.failOnceAt[2] = 700_000
        let info = ClipOfferInfo(id: idMain, entries: entries, totalBytes: UInt64(report.count + big.count + notes.count))
        let err = lp.offer(info) { i, o in try source.open(index: i, offset: o) }
        check("放清单", err == nil, err ?? "")
        let markerURL = lp.currentMarker
        let onPb = pb.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL] ?? []
        check("剪贴板里只有隐藏的标记文件", onPb.count == 1 && onPb.first?.lastPathComponent == ".lemongalaxy-paste-" + String(format: "%016llx", idMain)
              && onPb.first.map { fm.fileExists(atPath: $0.path) } == true && LazyPaste.holdsMarker(pb), onPb.map(\.path).joined(separator: ","))
        let host = MacHostPlatform(appKit: false)
        check("读剪贴板时过滤自己的标记（不回传）", host.clipboardGet() == nil)

        let dest = root.appendingPathComponent("粘贴到这里", isDirectory: true)
        try? fm.createDirectory(at: dest.appendingPathComponent("Assets"), withIntermediateDirectories: true)
        try? Data("old".utf8).write(to: dest.appendingPathComponent("report.txt"))
        // 让 FSEvents 先跑起来（新建的流有几十毫秒才开始收事件）
        Thread.sleep(forTimeInterval: 0.3)
        paste(markerURL, into: dest)
        let job1 = waitJob(finished, count: 1, timeout: 15)
        check("认出访达的粘贴并拷完", job1?.state.phase == .done, "\(String(describing: job1?.state))")
        check("拷过去的标记被删掉", !fm.fileExists(atPath: dest.appendingPathComponent(markerURL?.lastPathComponent ?? "x").path))
        check("顶层重名加「 2」", job1?.topNames == ["report 2.txt", "Assets 2", "说明.md"], "\(job1?.topNames ?? [])")
        check("原来的同名文件不动", (try? Data(contentsOf: dest.appendingPathComponent("report.txt"))) == Data("old".utf8))
        check("文件内容一致", (try? Data(contentsOf: dest.appendingPathComponent("report 2.txt"))) == report
              && (try? Data(contentsOf: dest.appendingPathComponent("Assets 2/a.bin"))) == big
              && (try? Data(contentsOf: dest.appendingPathComponent("说明.md"))) == notes)
        var isDir: ObjCBool = false
        check("文件夹与空文件", fm.fileExists(atPath: dest.appendingPathComponent("Assets 2/sub").path, isDirectory: &isDir) && isDir.boolValue
              && (try? fm.attributesOfItem(atPath: dest.appendingPathComponent("Assets 2/sub/empty.txt").path)[.size] as? NSNumber)?.intValue == 0)
        check("断线后从断点续读", source.opens.contains { $0.0 == 2 && $0.1 == 700_000 }, "\(source.opens)")
        check("进度总量", job1.map { $0.state.done == $0.state.total && $0.state.total == info.totalBytes } == true, "\(String(describing: job1?.state))")
        let mtime = (try? fm.attributesOfItem(atPath: dest.appendingPathComponent("说明.md").path)[.modificationDate] as? Date)?.timeIntervalSince1970
        check("修改时间照清单", mtime == 1_795_000_000, "\(mtime ?? 0)")

        // 2. 同一份清单可以再粘贴到别处
        let dest2 = root.appendingPathComponent("second", isDirectory: true)
        try? fm.createDirectory(at: dest2, withIntermediateDirectories: true)
        check("自己目录里的标记还在", markerURL.map { fm.fileExists(atPath: $0.path) } == true)
        paste(markerURL, into: dest2)
        let job2 = waitJob(finished, count: 2, timeout: 15)
        check("再粘贴一次", job2?.state.phase == .done && (try? Data(contentsOf: dest2.appendingPathComponent("report.txt"))) == report,
              "\(String(describing: job2?.state))")

        // 3. 停止：读到一半停住的文件，停止后半截文件删掉
        let held = pattern(800_000, seed: 3)
        let holdSource = FakeSource([held])
        holdSource.holdAt[0] = 300_000
        let holdInfo = ClipOfferInfo(id: idHold, entries: [entry("held.bin", false, UInt64(held.count))], totalBytes: UInt64(held.count))
        check("换一份清单", lp.offer(holdInfo) { i, o in try holdSource.open(index: i, offset: o) } == nil && lp.currentOfferId == idHold)
        let dest3 = root.appendingPathComponent("stop", isDirectory: true)
        try? fm.createDirectory(at: dest3, withIntermediateDirectories: true)
        Thread.sleep(forTimeInterval: 0.3)
        paste(lp.currentMarker, into: dest3)
        let running = wait(10) { lp.jobs.first { $0.state.done >= 300_000 } }
        check("拷贝中途的进度", running?.state.phase == .copying && running?.state.total == UInt64(held.count), "\(String(describing: running?.state))")
        running?.cancel()
        let job3 = waitJob(finished, count: 3, timeout: 10)
        check("停止", job3?.state.phase == .cancelled && !fm.fileExists(atPath: dest3.appendingPathComponent("held.bin").path)
              && holdSource.closedAll, "\(String(describing: job3?.state))")

        // 4. 失败：打开一直失败（重试两次后放弃），原因带文件名，半截不留
        let failSource = FakeSource([pattern(1000, seed: 5), pattern(1000, seed: 6)])
        failSource.alwaysFail.insert(1)
        let failInfo = ClipOfferInfo(id: idFail, entries: [entry("ok.bin", false, 1000), entry("bad.bin", false, 1000)], totalBytes: 2000)
        _ = lp.offer(failInfo) { i, o in try failSource.open(index: i, offset: o) }
        let dest4 = root.appendingPathComponent("fail", isDirectory: true)
        try? fm.createDirectory(at: dest4, withIntermediateDirectories: true)
        Thread.sleep(forTimeInterval: 0.3)
        paste(lp.currentMarker, into: dest4)
        let job4 = waitJob(finished, count: 4, timeout: 15)
        var failMsg = ""
        if case .failed(let m)? = job4?.state.phase { failMsg = m }
        check("失败显示原因", failMsg.contains("bad.bin") && failMsg.contains("模拟断线"), failMsg)
        check("失败时拷完的留着、没拷完的删掉", fm.fileExists(atPath: dest4.appendingPathComponent("ok.bin").path)
              && !fm.fileExists(atPath: dest4.appendingPathComponent("bad.bin").path))
        check("失败前重试了两次", failSource.opens.filter { $0.0 == 1 }.count == 3, "\(failSource.opens)")

        // 5. 经被控端平台层放清单（内核给的 ClipSource）
        let viaHost = FakeClipSource(FakeSource([notes]))
        let hostInfo = ClipOfferInfo(id: idHost, entries: [entry("host.md", false, UInt64(notes.count))], totalBytes: UInt64(notes.count))
        check("被控端平台层放清单", host.clipboardSetOffer(offer: hostInfo, source: viaHost) == nil && lp.currentOfferId == idHost)
        check("被控端报 clipboard_offer 能力", host.capabilities().contains("clipboard_offer"))
        let dest5 = root.appendingPathComponent("host", isDirectory: true)
        try? fm.createDirectory(at: dest5, withIntermediateDirectories: true)
        Thread.sleep(forTimeInterval: 0.3)
        paste(lp.currentMarker, into: dest5)
        let job5 = waitJob(finished, count: 5, timeout: 15)
        check("经平台层粘贴", job5?.state.phase == .done && (try? Data(contentsOf: dest5.appendingPathComponent("host.md"))) == notes)

        // 6. 剪贴板换掉：稍后停止监视、删掉标记，之后再出现同名文件不理
        let lastMarker = lp.currentMarker
        pb.clearContents()
        pb.setString("别的内容", forType: .string)
        let stoppedWatching = wait(5) { lp.busy ? nil : true } ?? false
        check("剪贴板换掉后停止监视", stoppedWatching && lp.currentOfferId == nil)
        check("标记删掉", lastMarker.map { !fm.fileExists(atPath: $0.path) } == true)
        let dest6 = root.appendingPathComponent("late", isDirectory: true)
        try? fm.createDirectory(at: dest6, withIntermediateDirectories: true)
        if let name = lastMarker?.lastPathComponent { try? Data().write(to: dest6.appendingPathComponent(name)) }
        Thread.sleep(forTimeInterval: 1)
        check("停止监视后不再粘贴", finished.count == 5 && !fm.fileExists(atPath: dest6.appendingPathComponent("host.md").path))

        // 7. 撤回：剪贴板还是它时清空
        _ = lp.offer(hostInfo) { i, o in try FakeSource([notes]).open(index: i, offset: o) }
        lp.withdraw(offerId: idHost)
        check("撤回清单并清空剪贴板", !LazyPaste.holdsMarker(pb) && (wait(3) { lp.busy ? nil : true } ?? false))

        return problems
    }

    // MARK: 工具

    private static func entry(_ path: String, _ dir: Bool, _ size: UInt64) -> ClipOfferEntry {
        ClipOfferEntry(path: path, isDir: dir, size: size, mtime: 1_795_000_000)
    }

    private static func pattern(_ n: Int, seed: Int) -> Data {
        Data((0..<n).map { UInt8(truncatingIfNeeded: $0 &* 31 &+ seed &* 7 &+ $0 / 997) })
    }

    /// 模拟访达粘贴：把标记文件拷进目标文件夹
    private static func paste(_ marker: URL?, into dir: URL) {
        guard let marker else { return }
        try? FileManager.default.copyItem(at: marker, to: dir.appendingPathComponent(marker.lastPathComponent))
    }

    /// 等条件成立（主线程上转 RunLoop，界面状态的推送照常进行）
    private static func wait<T>(_ timeout: Double, _ cond: () -> T?) -> T? {
        let end = Date().addingTimeInterval(timeout)
        while Date() < end {
            if let v = cond() { return v }
            if Thread.isMainThread { RunLoop.current.run(until: Date().addingTimeInterval(0.02)) } else { Thread.sleep(forTimeInterval: 0.02) }
        }
        return cond()
    }

    private static func waitJob(_ f: FinishedJobs, count: Int, timeout: Double) -> PasteJob? {
        wait(timeout) { f.count >= count ? f.at(count - 1) : nil }
    }

    final class FinishedJobs: @unchecked Sendable {
        private let lock = NSLock()
        private var jobs: [PasteJob] = []
        func add(_ j: PasteJob) { lock.locked { jobs.append(j) } }
        var count: Int { lock.locked { jobs.count } }
        func at(_ i: Int) -> PasteJob? { lock.locked { i < jobs.count ? jobs[i] : nil } }
    }

    /// 假的内容来源：按项给数据（nil 是文件夹）；可以在某处断一次、在某处停住、一直打不开
    final class FakeSource: @unchecked Sendable {
        let contents: [Data?]
        private let lock = NSLock()
        var failOnceAt: [Int: Int] = [:]
        var holdAt: [Int: Int] = [:]
        var alwaysFail = Set<Int>()
        private var _opens: [(Int, UInt64)] = []
        private var readers: [FakeReader] = []
        init(_ contents: [Data?]) { self.contents = contents }
        var opens: [(Int, UInt64)] { lock.locked { _opens } }
        var closedAll: Bool { lock.locked { readers.allSatisfy(\.isClosed) } }

        func open(index: UInt32, offset: UInt64) throws -> FakeReader {
            let i = Int(index)
            let failAt: Int? = lock.locked {
                _opens.append((i, offset))
                return failOnceAt.removeValue(forKey: i)
            }
            if alwaysFail.contains(i) { throw GalaxyError.Network(detail: "模拟断线") }
            guard i < contents.count, let d = contents[i] else { throw GalaxyError.NotFound(detail: "不是文件") }
            let r = FakeReader(data: d, offset: Int(offset), failAt: failAt, holdAt: holdAt[i])
            lock.locked { readers.append(r) }
            return r
        }
    }

    /// 假的读取器（ClipReader 子类，平台层与内核给的一样用）
    final class FakeReader: ClipReader, @unchecked Sendable {
        required init(unsafeFromHandle handle: UInt64) { fatalError("假对象不能由内核指针构造") }
        private let data: Data
        private var pos: Int
        private var failAt: Int?
        private let holdAt: Int?
        private let lock = NSLock()
        private var closed = false

        init(data: Data, offset: Int, failAt: Int?, holdAt: Int?) {
            self.data = data
            self.pos = min(offset, data.count)
            self.failAt = failAt
            self.holdAt = holdAt
            super.init(noHandle: NoHandle())
        }

        var isClosed: Bool { lock.locked { closed } }
        override func size() -> UInt64 { UInt64(data.count) }
        override func close() { lock.locked { closed = true } }

        override func read(max: UInt32) throws -> Data {
            while true {
                let (c, p) = lock.locked { (closed, pos) }
                if c { throw GalaxyError.Cancelled }
                if let h = holdAt, p >= h { Thread.sleep(forTimeInterval: 0.01); continue }
                if let f = lock.locked({ failAt }), p >= f {
                    lock.locked { failAt = nil }
                    throw GalaxyError.Network(detail: "模拟断线")
                }
                if p >= data.count { return Data() }
                var end = min(data.count, p + Int(Swift.min(max, 128 << 10)))
                if let f = lock.locked({ failAt }) { end = Swift.min(end, f) }
                if let h = holdAt { end = Swift.min(end, Swift.max(h, p + 1)) }
                lock.locked { pos = end }
                return data.subdata(in: p..<end)
            }
        }
    }

    /// 被控端平台层收到的内核内容来源（ClipSource 子类）
    final class FakeClipSource: ClipSource, @unchecked Sendable {
        required init(unsafeFromHandle handle: UInt64) { fatalError("假对象不能由内核指针构造") }
        let inner: FakeSource
        init(_ inner: FakeSource) {
            self.inner = inner
            super.init(noHandle: NoHandle())
        }
        override func open(index: UInt32, offset: UInt64) throws -> ClipReader { try inner.open(index: index, offset: offset) }
    }
}
