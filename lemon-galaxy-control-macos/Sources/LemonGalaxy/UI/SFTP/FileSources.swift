import Foundation

/// 双栏文件管理里一侧的数据来源：本机（FileManager）或远端（内核 FileClient：设备文件通道或 SSH 的 SFTP）。
enum FileEndpoint: Equatable, Hashable {
    case local
    case device(String)
    case ssh(String)
}

struct FileItem: Identifiable, Hashable {
    var id: String { path }
    let name: String
    let path: String
    let kind: FileKind
    let size: UInt64
    let mode: UInt32
    let modified: Int64
    let owner: String
    let linkTarget: String?

    var isDir: Bool { kind == .dir || (kind == .symlink && linkTarget.map { !$0.contains(".") } == true) }
    var modeString: String { (kind == .dir ? "d" : kind == .symlink ? "l" : "-") + Fmt.mode(mode) }
    var sortName: String { (kind == .dir ? "0" : "1") + name.lowercased() }
    var sortSize: UInt64 { kind == .dir ? 0 : size }

    init(_ e: FileEntry) {
        name = e.name; path = e.path; kind = e.kind; size = e.size; mode = e.mode; modified = e.modifiedAt; owner = e.owner; linkTarget = e.linkTarget
    }

    init(name: String, path: String, kind: FileKind, size: UInt64, mode: UInt32, modified: Int64, owner: String, linkTarget: String?) {
        self.name = name; self.path = path; self.kind = kind; self.size = size; self.mode = mode; self.modified = modified
        self.owner = owner; self.linkTarget = linkTarget
    }

    /// 文本文件（可在线编辑）的粗略判断：按扩展名，或没有扩展名的小文件
    var looksLikeText: Bool {
        let ext = (name as NSString).pathExtension.lowercased()
        let textExt: Set<String> = ["txt", "md", "log", "yml", "yaml", "json", "toml", "ini", "conf", "cfg", "sh", "bash", "zsh", "py", "js", "ts",
                                    "html", "css", "xml", "env", "service", "csv", "rs", "go", "swift", "c", "h", "cpp", "java", "kt", "sql", "properties"]
        if textExt.contains(ext) { return true }
        return ext.isEmpty && kind == .file && size < 512 * 1024
    }
}

protocol FileSource: AnyObject {
    var endpoint: FileEndpoint { get }
    var separator: String { get }
    func home() async throws -> String
    func list(_ path: String) async throws -> [FileItem]
    func mkdir(_ path: String) async throws
    func rename(_ from: String, _ to: String) async throws
    func remove(_ path: String, recursive: Bool) async throws
    func chmod(_ path: String, mode: UInt32) async throws
    func readText(_ path: String) async throws -> String
    func writeText(_ path: String, _ text: String) async throws
}

extension FileSource {
    func join(_ dir: String, _ name: String) -> String {
        dir.hasSuffix("/") ? dir + name : dir + "/" + name
    }
    func parent(_ path: String) -> String {
        let p = (path as NSString).deletingLastPathComponent
        return p.isEmpty ? "/" : p
    }
}

final class LocalFileSource: FileSource {
    /// selftest 用假目录，截图里不出现本机真实文件
    static var homeOverride: String?
    let endpoint = FileEndpoint.local
    let separator = "/"
    private let fm = FileManager.default

    func home() async throws -> String { Self.homeOverride ?? fm.homeDirectoryForCurrentUser.path }

    func list(_ path: String) async throws -> [FileItem] {
        let url = URL(fileURLWithPath: path)
        let keys: [URLResourceKey] = [.isDirectoryKey, .isSymbolicLinkKey, .fileSizeKey, .contentModificationDateKey]
        let items = try fm.contentsOfDirectory(at: url, includingPropertiesForKeys: keys, options: [])
        return items.compactMap { u in
            let attrs = try? fm.attributesOfItem(atPath: u.path)
            let type = attrs?[.type] as? FileAttributeType
            let kind: FileKind = type == .typeDirectory ? .dir : type == .typeSymbolicLink ? .symlink : type == .typeRegular ? .file : .other
            let size = (attrs?[.size] as? NSNumber)?.uint64Value ?? 0
            let mode = (attrs?[.posixPermissions] as? NSNumber)?.uint32Value ?? 0
            let date = (attrs?[.modificationDate] as? Date) ?? Date()
            let owner = attrs?[.ownerAccountName] as? String ?? ""
            var target: String?
            if kind == .symlink { target = try? fm.destinationOfSymbolicLink(atPath: u.path) }
            var k = kind
            if kind == .symlink, (try? u.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true { k = .dir }
            return FileItem(name: u.lastPathComponent, path: u.path, kind: k, size: size, mode: mode,
                            modified: Int64(date.timeIntervalSince1970 * 1000), owner: owner, linkTarget: target)
        }
    }

    func mkdir(_ path: String) async throws { try fm.createDirectory(atPath: path, withIntermediateDirectories: false) }
    func rename(_ from: String, _ to: String) async throws { try fm.moveItem(atPath: from, toPath: to) }
    func remove(_ path: String, recursive: Bool) async throws {
        // 本机删除走废纸篓，可以找回
        try fm.trashItem(at: URL(fileURLWithPath: path), resultingItemURL: nil)
    }
    func chmod(_ path: String, mode: UInt32) async throws {
        try fm.setAttributes([.posixPermissions: NSNumber(value: mode)], ofItemAtPath: path)
    }
    func readText(_ path: String) async throws -> String { try String(contentsOfFile: path, encoding: .utf8) }
    func writeText(_ path: String, _ text: String) async throws { try text.write(toFile: path, atomically: true, encoding: .utf8) }
}

final class RemoteFileSource: FileSource {
    let endpoint: FileEndpoint
    let client: FileClient
    let separator = "/"
    init(endpoint: FileEndpoint, client: FileClient) { self.endpoint = endpoint; self.client = client }

    func home() async throws -> String { try await client.home() }
    func list(_ path: String) async throws -> [FileItem] { try await client.list(path: path).map(FileItem.init) }
    func mkdir(_ path: String) async throws { try await client.mkdir(path: path) }
    func rename(_ from: String, _ to: String) async throws { try await client.rename(from: from, to: to) }
    func remove(_ path: String, recursive: Bool) async throws { try await client.remove(path: path, recursive: recursive) }
    func chmod(_ path: String, mode: UInt32) async throws { try await client.chmod(path: path, mode: mode) }
    func readText(_ path: String) async throws -> String { try await client.readText(path: path, maxBytes: 2 * 1024 * 1024) }
    func writeText(_ path: String, _ text: String) async throws { try await client.writeText(path: path, text: text) }
}

// MARK: - 传输队列

@MainActor
final class TransferItem: ObservableObject, Identifiable {
    enum State: Equatable { case queued, running, paused, done, failed(String), cancelled }
    enum Direction { case upload, download }
    let id = UUID().uuidString
    let name: String
    let direction: Direction
    let source: String
    let destination: String
    let endpointName: String
    @Published var done: UInt64 = 0
    @Published var total: UInt64 = 0
    @Published var state: State = .queued
    @Published var speed: Double = 0
    var transfer: Transfer?
    var restart: (() -> Void)?
    var onFinished: (@MainActor () -> Void)?
    private var lastSample: (Date, UInt64)?

    init(name: String, direction: Direction, source: String, destination: String, endpointName: String) {
        self.name = name; self.direction = direction; self.source = source; self.destination = destination; self.endpointName = endpointName
    }

    var fraction: Double { total == 0 ? 0 : min(1, Double(done) / Double(total)) }

    func progress(_ d: UInt64, _ t: UInt64) {
        let now = Date()
        if let (at, prev) = lastSample, now.timeIntervalSince(at) > 0.3 {
            speed = Double(d - min(d, prev)) / now.timeIntervalSince(at)
            lastSample = (now, d)
        } else if lastSample == nil { lastSample = (now, d) }
        done = d
        total = t
        if state == .queued { state = .running }
    }

    func finished(_ error: String?) {
        if case .cancelled = state { return }
        if let error { state = .failed(error) } else { state = .done; done = total; onFinished?() }
        transfer = nil
    }
}

final class TransferListenerAdapter: TransferListener, @unchecked Sendable {
    weak var item: TransferItem?
    init(item: TransferItem) { self.item = item }
    func onProgress(done: UInt64, total: UInt64) {
        DispatchQueue.main.async { [weak self] in
            guard let item = self?.item else { return }
            MainActor.assumeIsolated { item.progress(done, total) }
        }
    }
    func onFinished(error: String?) {
        DispatchQueue.main.async { [weak self] in
            guard let item = self?.item else { return }
            MainActor.assumeIsolated { item.finished(error) }
        }
    }
}

@MainActor
final class TransferQueue: ObservableObject {
    @Published var items: [TransferItem] = []
    private var listeners: [String: TransferListenerAdapter] = [:]

    var activeCount: Int { items.filter { $0.state == .running || $0.state == .queued || $0.state == .paused }.count }

    func start(name: String, direction: TransferItem.Direction, client: FileClient, local: String, remote: String,
               endpointName: String, onFinished: (@MainActor () -> Void)? = nil) -> TransferItem {
        let item = TransferItem(name: name, direction: direction, source: direction == .upload ? local : remote,
                                destination: direction == .upload ? remote : local, endpointName: endpointName)
        item.onFinished = onFinished
        items.insert(item, at: 0)
        let launch: () -> Void = { [weak self, weak item] in
            guard let self, let item else { return }
            let l = TransferListenerAdapter(item: item)
            self.listeners[item.id] = l
            item.state = .running
            item.done = 0
            item.transfer = direction == .upload
                ? client.upload(local: local, remote: remote, listener: l)
                : client.download(remote: remote, local: local, listener: l)
        }
        item.restart = launch
        launch()
        return item
    }

    func pause(_ item: TransferItem) { item.transfer?.pause(); item.state = .paused }
    func resume(_ item: TransferItem) { item.transfer?.resume(); item.state = .running }
    func cancel(_ item: TransferItem) { item.transfer?.cancel(); item.state = .cancelled; item.transfer = nil }
    func retry(_ item: TransferItem) { item.restart?() }
    func clearFinished() {
        items.removeAll { if case .done = $0.state { return true }; if case .cancelled = $0.state { return true }; return false }
    }
}
