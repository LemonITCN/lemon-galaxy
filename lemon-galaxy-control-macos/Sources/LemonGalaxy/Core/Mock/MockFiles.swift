import Foundation

/// 模拟远端文件系统：内存里的一棵树。传输按定时器推进进度，文件名含 fail 的传到 60% 失败，用来演示重试。
final class MockFileClient: FileClient, @unchecked Sendable {
    required init(unsafeFromRawPointer pointer: UnsafeMutableRawPointer) { fatalError("模拟对象不能由内核指针构造") }
    final class Node {
        var name: String
        var kind: FileKind
        var size: UInt64
        var mode: UInt32
        var mtime: Int64
        var text: String?
        var children: [String: Node] = [:]
        var linkTarget: String?
        init(_ name: String, _ kind: FileKind, size: UInt64 = 0, mode: UInt32 = 0o644, mtime: Int64 = MockData.fixedNow, text: String? = nil) {
            self.name = name; self.kind = kind; self.size = size; self.mode = mode; self.mtime = mtime; self.text = text
        }
    }

    private let client: MockGalaxyClient
    private let platform: Platform
    private let user: String
    private let root = Node("/", .dir, mode: 0o755)
    private let lock = NSLock()
    let homePath: String

    init(client: MockGalaxyClient, platform: Platform, username: String) {
        self.client = client
        self.platform = platform
        self.user = username
        switch platform {
        case .windows: homePath = "/C:/Users/\(username)"
        case .macos: homePath = "/Users/\(username)"
        default: homePath = username == "root" ? "/root" : "/home/\(username)"
        }
        super.init(noPointer: NoPointer())
        seed()
    }

    private func seed() {
        let t = MockData.fixedNow
        let day: Int64 = 86_400_000
        let home = mkdirs(homePath)
        func dir(_ parent: Node, _ name: String, _ mtime: Int64, mode: UInt32 = 0o755) -> Node {
            let n = Node(name, .dir, size: 4096, mode: mode, mtime: mtime)
            parent.children[name] = n
            return n
        }
        func file(_ parent: Node, _ name: String, _ size: UInt64, _ mtime: Int64, mode: UInt32 = 0o644, text: String? = nil) {
            parent.children[name] = Node(name, .file, size: text.map { UInt64($0.utf8.count) } ?? size, mode: mode, mtime: mtime, text: text)
        }
        if platform == .windows {
            _ = dir(home, "Desktop", t - day)
            let docs = dir(home, "Documents", t - day * 2)
            file(docs, "report-2026Q3.xlsx", 182_440, t - day * 3)
            file(docs, "meeting-notes.txt", 0, t - day, text: "2026-11-20 周会\n- 发布 0.1.0\n- 远程桌面测试\n")
            let dl = dir(home, "Downloads", t - 3_600_000)
            file(dl, "LemonGalaxy-0.1.0.exe", 18_552_112, t - 3_600_000 * 2)
            file(dl, "drivers-fail.zip", 92_331_008, t - day * 9)
            file(home, "todo.txt", 0, t - 600_000, text: "1. 更新显卡驱动\n2. 备份 D 盘\n")
            return
        }
        let ssh = dir(home, ".ssh", t - day * 50, mode: 0o700)
        file(ssh, "authorized_keys", 0, t - day * 50, mode: 0o600, text: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDemoKeyOnly alice@laptop\n")
        let app = dir(home, "app", t - day)
        file(app, "config.yaml", 0, t - day, text: "server:\n  listen: 0.0.0.0:8080\n  workers: 8\nlog:\n  level: info\n  dir: /var/log/app\n")
        file(app, "app.bin", 48_221_184, t - day, mode: 0o755)
        let static_ = dir(app, "static", t - day * 4)
        file(static_, "index.html", 0, t - day * 4, text: "<!doctype html>\n<title>Demo</title>\n<h1>It works</h1>\n")
        file(static_, "logo.png", 24_518, t - day * 4)
        let logs = dir(home, "logs", t - 600_000)
        file(logs, "app.log", 0, t - 120_000, text: MockShell.syslogLines.joined(separator: "\n") + "\n")
        file(logs, "app.log.1.gz", 3_481_993, t - day)
        file(logs, "app.log.2.gz", 3_512_004, t - day * 2)
        let proj = dir(home, "projects", t - day * 3)
        for (i, n) in ["galaxy-web", "infra", "ml-pipeline", "scripts"].enumerated() { _ = dir(proj, n, t - day * Int64(i + 2)) }
        file(home, "docker-compose.yml", 0, t - day * 2, text: "services:\n  nginx:\n    image: nginx:1.27\n    ports: [\"80:80\", \"443:443\"]\n  redis:\n    image: redis:7\n")
        file(home, "deploy.sh", 0, t - day * 9, mode: 0o755, text: "#!/bin/sh\nset -e\ncd ~/app\n./app.bin --check\nsystemctl --user restart app\n")
        file(home, "README.md", 0, t - day * 50, text: "# build-server-01\n\nCI runner and artifact cache.\n")
        file(home, "backup-fail.tar.gz", 734_003_200, t - day * 6)
        let link = Node("current", .symlink, size: 18, mode: 0o777, mtime: t - day)
        link.linkTarget = homePath + "/app"
        home.children["current"] = link
        // 系统目录，给「上一级」浏览用
        let etc = mkdirs("/etc")
        file(etc, "hostname", 0, t - day * 100, text: "build-server-01\n")
        file(etc, "hosts", 0, t - day * 100, text: "127.0.0.1 localhost\n10.0.2.10 db-primary\n")
        _ = mkdirs("/var/log")
        _ = mkdirs("/tmp")
    }

    @discardableResult
    private func mkdirs(_ path: String) -> Node {
        var n = root
        for comp in path.split(separator: "/").map(String.init) {
            if let c = n.children[comp] { n = c } else {
                let c = Node(comp, .dir, size: 4096, mode: 0o755)
                n.children[comp] = c
                n = c
            }
        }
        return n
    }

    private func node(_ path: String) -> Node? {
        var n = root
        for comp in path.split(separator: "/").map(String.init) {
            guard let c = n.children[comp] else { return nil }
            n = c
        }
        return n
    }

    private func split(_ path: String) -> (parent: String, name: String) {
        let trimmed = path.hasSuffix("/") && path.count > 1 ? String(path.dropLast()) : path
        guard let i = trimmed.lastIndex(of: "/") else { return ("/", trimmed) }
        let parent = String(trimmed[..<i])
        return (parent.isEmpty ? "/" : parent, String(trimmed[trimmed.index(after: i)...]))
    }

    private func entry(_ n: Node, path: String) -> FileEntry {
        FileEntry(name: n.name, path: path, kind: n.kind, size: n.size, mode: n.mode, modifiedAt: n.mtime,
                  owner: path.hasPrefix(homePath) ? user : "root", group: path.hasPrefix(homePath) ? user : "root", linkTarget: n.linkTarget)
    }

    private func notFound(_ p: String) -> GalaxyError {
        .NotFound(detail: client.zh ? "找不到 \(p)" : "No such file or directory: \(p)")
    }

    override func home() async throws -> String { await client.delay(60); return homePath }

    override func list(path: String) async throws -> [FileEntry] {
        await client.delay(180)
        return try lock.locked {
            guard let n = node(path), n.kind == .dir else { throw notFound(path) }
            let base = path == "/" ? "" : path
            return n.children.values.map { entry($0, path: base + "/" + $0.name) }
        }
    }

    override func stat(path: String) async throws -> FileEntry {
        await client.delay(40)
        return try lock.locked {
            guard let n = node(path) else { throw notFound(path) }
            return entry(n, path: path)
        }
    }

    override func realpath(path: String) async throws -> String {
        await client.delay(40)
        return try lock.locked {
            guard let n = node(path) else { throw notFound(path) }
            return n.linkTarget ?? path
        }
    }

    override func search(root: String, pattern: String, content: Bool) async throws -> [FileEntry] {
        await client.delay(200)
        return lock.locked {
            var out: [FileEntry] = []
            func walk(_ n: Node, _ p: String) {
                for c in n.children.values {
                    let cp = (p == "/" ? "" : p) + "/" + c.name
                    let hit = c.name.localizedCaseInsensitiveContains(pattern) || (content && (c.text?.contains(pattern) ?? false))
                    if hit { out.append(entry(c, path: cp)) }
                    if c.kind == .dir, out.count < 200 { walk(c, cp) }
                }
            }
            if let r = node(root) { walk(r, root) }
            return out
        }
    }

    override func mkdir(path: String) async throws {
        await client.delay(100)
        try lock.locked {
            let (p, name) = split(path)
            guard let parent = node(p) else { throw notFound(p) }
            guard parent.children[name] == nil else {
                throw GalaxyError.Other(detail: client.zh ? "已存在同名文件或文件夹" : "A file or folder with that name already exists")
            }
            parent.children[name] = Node(name, .dir, size: 4096, mode: 0o755, mtime: MockData.now)
        }
    }

    override func rename(from: String, to: String) async throws {
        await client.delay(100)
        try lock.locked {
            let (fp, fname) = split(from)
            let (tp, tname) = split(to)
            guard let src = node(fp), let n = src.children[fname] else { throw notFound(from) }
            guard let dst = node(tp) else { throw notFound(tp) }
            src.children[fname] = nil
            n.name = tname
            n.mtime = MockData.now
            dst.children[tname] = n
        }
    }

    override func remove(path: String, recursive: Bool) async throws {
        await client.delay(120)
        try lock.locked {
            let (p, name) = split(path)
            guard let parent = node(p), let n = parent.children[name] else { throw notFound(path) }
            if n.kind == .dir, !n.children.isEmpty, !recursive {
                throw GalaxyError.Other(detail: client.zh ? "文件夹不是空的" : "The folder is not empty")
            }
            parent.children[name] = nil
        }
    }

    override func chmod(path: String, mode: UInt32) async throws {
        await client.delay(80)
        try lock.locked {
            guard let n = node(path) else { throw notFound(path) }
            n.mode = mode & 0o7777
        }
    }

    override func readText(path: String, maxBytes: UInt64) async throws -> String {
        await client.delay(150)
        return try lock.locked {
            guard let n = node(path) else { throw notFound(path) }
            guard let t = n.text else {
                throw GalaxyError.Other(detail: client.zh ? "这不是文本文件" : "This is not a text file")
            }
            return String(t.prefix(Int(maxBytes)))
        }
    }

    override func writeText(path: String, text: String) async throws {
        await client.delay(150)
        try lock.locked {
            let (p, name) = split(path)
            guard let parent = node(p) else { throw notFound(p) }
            let n = parent.children[name] ?? Node(name, .file)
            n.text = text
            n.size = UInt64(text.utf8.count)
            n.mtime = MockData.now
            parent.children[name] = n
        }
    }

    override func download(remote: String, local: String, listener: TransferListener) -> Transfer {
        lock.lock()
        let n = node(remote)
        lock.unlock()
        let total = n?.size ?? 0
        let content = n?.text.map { Data($0.utf8) }
        let fail = remote.lowercased().contains("fail")
        return MockTransfer(total: max(total, 1), fail: fail, zh: client.zh, listener: listener, speedScale: client.options.latencyScale) { ok in
            guard ok else { return }
            // 文本文件写真实内容；二进制写占位字节（大文件只写 1 MB，避免占用磁盘）
            let data = content ?? Data(count: Int(min(total, 1_048_576)))
            try? data.write(to: URL(fileURLWithPath: local))
        }
    }

    override func upload(local: String, remote: String, listener: TransferListener) -> Transfer {
        let attrs = try? FileManager.default.attributesOfItem(atPath: local)
        let size = (attrs?[.size] as? NSNumber)?.uint64Value ?? 0
        let isDir = (attrs?[.type] as? FileAttributeType) == .typeDirectory
        let text: String? = size < 256 * 1024 ? (try? String(contentsOfFile: local, encoding: .utf8)) : nil
        let fail = local.lowercased().contains("fail")
        return MockTransfer(total: max(size, 1), fail: fail, zh: client.zh, listener: listener, speedScale: client.options.latencyScale) { [weak self] ok in
            guard ok, let self else { return }
            self.lock.lock(); defer { self.lock.unlock() }
            let (p, name) = self.split(remote)
            guard let parent = self.node(p) else { return }
            let n = Node(name, isDir ? .dir : .file, size: size, mode: isDir ? 0o755 : 0o644, mtime: MockData.now, text: text)
            parent.children[name] = n
        }
    }
}

/// 模拟传输：按约 40 MB/s 推进进度，支持暂停、继续、取消。
final class MockTransfer: Transfer, @unchecked Sendable {
    required init(unsafeFromRawPointer pointer: UnsafeMutableRawPointer) { fatalError("模拟对象不能由内核指针构造") }
    private let total: UInt64
    private var done: UInt64 = 0
    private var paused = false
    private var cancelled = false
    private var finished = false
    private let fail: Bool
    private let zh: Bool
    private weak var listener: TransferListener?
    private let completion: (Bool) -> Void
    private let timer: DispatchSourceTimer
    private let queue = DispatchQueue(label: "mock.transfer")

    init(total: UInt64, fail: Bool, zh: Bool, listener: TransferListener, speedScale: Double, completion: @escaping (Bool) -> Void) {
        self.total = total
        self.fail = fail
        self.zh = zh
        self.listener = listener
        self.completion = completion
        self.timer = DispatchSource.makeTimerSource(queue: queue)
        super.init(noPointer: NoPointer())
        // 每 100 毫秒推进一次；小文件至少走 3 步，好看到进度
        let step = speedScale == 0 ? total / 3 + 1 : max(total / 3, UInt64(4_000_000))
        timer.schedule(deadline: .now() + 0.1, repeating: speedScale == 0 ? 0.001 : 0.1)
        timer.setEventHandler { [weak self] in
            guard let self, !self.paused, !self.cancelled, !self.finished else { return }
            self.done = min(self.total, self.done + step)
            self.listener?.onProgress(done: self.done, total: self.total)
            if self.fail, self.done * 10 >= self.total * 6 {
                self.finish(error: zh ? "连接中断（模拟失败）" : "Connection lost (simulated failure)")
            } else if self.done >= self.total {
                self.completion(true)
                self.finish(error: nil)
            }
        }
        timer.resume()
    }

    deinit { timer.cancel() }

    private func finish(error: String?) {
        finished = true
        timer.cancel()
        listener?.onFinished(error: error)
    }

    override func cancel() {
        queue.async { [weak self] in
            guard let self, !self.finished else { return }
            self.cancelled = true
            self.finish(error: self.zh ? "已取消" : "Cancelled")
        }
    }
    override func pause() { queue.async { [weak self] in self?.paused = true } }
    override func resume() { queue.async { [weak self] in self?.paused = false } }
}
