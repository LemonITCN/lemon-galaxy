import Foundation
import Combine

/// 一项文件传输（上传或下载）。
@MainActor
public final class TransferItem: ObservableObject, Identifiable {
    public enum Direction { case upload, download }
    public enum Status: Equatable { case running, paused, done, failed(String), cancelled }

    public let id = UUID().uuidString
    public let direction: Direction
    public let deviceId: String
    public let deviceName: String
    public let remotePath: String
    public let localURL: URL
    public let startedAt = Date()
    @Published public private(set) var done: UInt64 = 0
    @Published public private(set) var total: UInt64 = 0
    @Published public private(set) var status: Status = .running
    @Published public private(set) var bytesPerSecond: Double = 0

    fileprivate var transfer: Transfer?
    fileprivate let adapter = TransferListenerAdapter()
    private var lastSample: (Date, UInt64) = (Date(), 0)
    var onFinish: ((TransferItem) -> Void)?

    init(direction: Direction, deviceId: String, deviceName: String, remotePath: String, localURL: URL) {
        self.direction = direction
        self.deviceId = deviceId
        self.deviceName = deviceName
        self.remotePath = remotePath
        self.localURL = localURL
        adapter.progress = { [weak self] d, t in onMain { self?.progress(d, t) } }
        adapter.finished = { [weak self] e in onMain { self?.finished(e) } }
    }

    public var name: String { direction == .upload ? localURL.lastPathComponent : RemotePath.name(remotePath) }
    public var fraction: Double { total == 0 ? 0 : min(1, Double(done) / Double(total)) }
    public var isActive: Bool { status == .running || status == .paused }

    private func progress(_ d: UInt64, _ t: UInt64) {
        done = d
        total = t
        let now = Date()
        let dt = now.timeIntervalSince(lastSample.0)
        if dt >= 0.5 {
            bytesPerSecond = Double(d &- min(d, lastSample.1)) / dt
            lastSample = (now, d)
        }
    }

    private func finished(_ error: String?) {
        guard status != .cancelled else { return }
        if let error, !error.isEmpty { status = .failed(error) } else {
            status = .done
            if total > 0 { done = total }
        }
        bytesPerSecond = 0
        onFinish?(self)
    }

    public func pause() {
        guard status == .running else { return }
        transfer?.pause()
        status = .paused
        bytesPerSecond = 0
    }

    public func resume() {
        guard status == .paused else { return }
        transfer?.resume()
        status = .running
        lastSample = (Date(), done)
    }

    public func cancel() {
        guard isActive else { return }
        transfer?.cancel()
        status = .cancelled
        bytesPerSecond = 0
    }

    fileprivate func restart(_ t: Transfer) {
        transfer = t
        status = .running
        lastSample = (Date(), done)
    }
}

/// 传输队列：上传、下载都支持断点续传（内核负责），这里管进度、暂停、继续、取消、重试。
@MainActor
public final class TransferCenter: ObservableObject {
    @Published public private(set) var items: [TransferItem] = []
    /// 一项完成（界面弹提示、下载完可以分享）
    public var onFinished: ((TransferItem) -> Void)?

    public init() {}

    public var activeCount: Int { items.filter(\.isActive).count }

    @discardableResult
    public func download(_ files: FileClient, deviceId: String, deviceName: String, remote: String, to local: URL) -> TransferItem {
        try? FileManager.default.createDirectory(at: local.deletingLastPathComponent(), withIntermediateDirectories: true)
        let item = TransferItem(direction: .download, deviceId: deviceId, deviceName: deviceName, remotePath: remote, localURL: local)
        start(item, files)
        return item
    }

    @discardableResult
    public func upload(_ files: FileClient, deviceId: String, deviceName: String, local: URL, remote: String) -> TransferItem {
        let item = TransferItem(direction: .upload, deviceId: deviceId, deviceName: deviceName, remotePath: remote, localURL: local)
        start(item, files)
        return item
    }

    private var clients: [String: FileClient] = [:]

    private func start(_ item: TransferItem, _ files: FileClient) {
        clients[item.id] = files
        item.onFinish = { [weak self] it in self?.onFinished?(it) }
        items.insert(item, at: 0)
        item.restart(makeTransfer(item, files))
    }

    private func makeTransfer(_ item: TransferItem, _ files: FileClient) -> Transfer {
        switch item.direction {
        case .download: return files.download(remote: item.remotePath, local: item.localURL.path, listener: item.adapter)
        case .upload: return files.upload(local: item.localURL.path, remote: item.remotePath, listener: item.adapter)
        }
    }

    /// 失败或取消后重试（内核按已传部分续传）
    public func retry(_ item: TransferItem) {
        guard !item.isActive, let files = clients[item.id] else { return }
        item.restart(makeTransfer(item, files))
    }

    public func clearFinished() {
        let keep = items.filter(\.isActive)
        let removed = Set(items.map(\.id)).subtracting(keep.map(\.id))
        for id in removed { clients[id] = nil }
        items = keep
    }

    public func cancelAll() {
        for i in items { i.cancel() }
    }
}
