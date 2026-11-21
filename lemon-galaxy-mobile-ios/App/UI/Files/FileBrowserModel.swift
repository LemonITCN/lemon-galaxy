import Foundation
import SwiftUI
import GalaxyKit

/// 一个目标（设备或 SSH 主机）的远端文件浏览。设备走端到端会话，SSH 主机走 SFTP，接口相同。
@MainActor
final class FileBrowserModel: ObservableObject {
    let target: RemoteTarget
    @Published private(set) var path = ""
    @Published private(set) var entries: [FileEntry] = []
    @Published private(set) var loading = false
    @Published private(set) var error: String?
    @Published var searchResults: [FileEntry]?
    @Published private(set) var searching = false
    private var files: FileClient?
    private weak var app: AppModel?
    private var history: [String] = []

    init(app: AppModel, target: RemoteTarget) {
        self.app = app
        self.target = target
    }

    var name: String { app?.targetName(target) ?? "" }

    private func client() async throws -> FileClient {
        if let files { return files }
        guard let app else { throw GalaxyError.Cancelled }
        let f = try await app.account.files(target)
        files = f
        return f
    }

    func start() async {
        guard path.isEmpty else { return }
        loading = true
        do {
            let f = try await client()
            let home = try await f.home()
            await load(home)
        } catch {
            self.error = describeError(error)
            loading = false
        }
    }

    func load(_ p: String) async {
        loading = true
        error = nil
        do {
            let list = try await client().list(path: p)
            if !path.isEmpty, path != p { history.append(path) }
            path = p
            entries = list
        } catch {
            // 连接断了：重新拿一次客户端再试
            files = nil
            self.error = describeError(error)
        }
        loading = false
    }

    func refresh() async { await load(path.isEmpty ? "~" : path) }

    func goUp() async {
        let parent = RemotePath.parent(path)
        guard parent != path else { return }
        await load(parent)
    }

    var canGoUp: Bool { path != "/" && !path.isEmpty }

    func sorted(_ settings: AppSettings) -> [FileEntry] {
        FileSort.sorted(entries, by: settings.fileSort, descending: settings.fileSortDescending, showHidden: settings.showHiddenFiles)
    }

    // MARK: 文件操作

    func mkdir(_ name: String) async {
        await run { try await self.client().mkdir(path: RemotePath.join(self.path, name)) }
    }

    func rename(_ e: FileEntry, to name: String) async {
        await run { try await self.client().rename(from: e.path, to: RemotePath.join(RemotePath.parent(e.path), name)) }
    }

    func delete(_ e: FileEntry) async {
        await run { try await self.client().remove(path: e.path, recursive: e.kind == .dir) }
    }

    func chmod(_ e: FileEntry, _ mode: UInt32) async {
        await run { try await self.client().chmod(path: e.path, mode: mode) }
    }

    private func run(_ body: @escaping () async throws -> Void) async {
        do {
            try await body()
            await refresh()
        } catch {
            app?.showError(error)
        }
    }

    func readText(_ e: FileEntry) async throws -> String {
        try await client().readText(path: e.path, maxBytes: 2 * 1024 * 1024)
    }

    func writeText(_ e: FileEntry, _ text: String) async throws {
        try await client().writeText(path: e.path, text: text)
    }

    func search(_ pattern: String, content: Bool) async {
        guard !pattern.isEmpty else { searchResults = nil; return }
        searching = true
        do {
            searchResults = try await client().search(root: path, pattern: pattern, content: content)
        } catch {
            app?.showError(error)
        }
        searching = false
    }

    // MARK: 传输

    /// 下载到「文稿/Downloads」（「文件」App 里能看到），同名时自动加序号
    @discardableResult
    func download(_ e: FileEntry) async -> TransferItem? {
        guard let app else { return nil }
        do {
            let f = try await client()
            let dir = CoreEnvironment.downloadsDir
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let local = Self.uniqueURL(dir.appendingPathComponent(e.name))
            app.showToast(L("files.downloadStarted"))
            return app.transfers.download(f, deviceId: target.id, deviceName: name, remote: e.path, to: local)
        } catch {
            app.showError(error)
            return nil
        }
    }

    /// 上传：先把选中的文件拷进本机临时目录（安全范围访问在传输期间不能一直持有）
    func upload(_ urls: [URL]) async {
        guard let app else { return }
        do {
            let f = try await client()
            let tmp = FileManager.default.temporaryDirectory.appendingPathComponent("uploads", isDirectory: true)
            try? FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
            for url in urls {
                let ok = url.startAccessingSecurityScopedResource()
                defer { if ok { url.stopAccessingSecurityScopedResource() } }
                let dir = tmp.appendingPathComponent(UUID().uuidString, isDirectory: true)
                try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
                let local = dir.appendingPathComponent(url.lastPathComponent)
                try FileManager.default.copyItem(at: url, to: local)
                let item = app.transfers.upload(f, deviceId: target.id, deviceName: name, local: local, remote: RemotePath.join(path, url.lastPathComponent))
                watchUpload(item)
            }
            app.showToast(L("files.uploadStarted"))
        } catch {
            app.showError(error)
        }
    }

    func upload(data: Data, name fileName: String) async {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("uploads/" + UUID().uuidString, isDirectory: true)
        do {
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let url = dir.appendingPathComponent(fileName)
            try data.write(to: url)
            await upload([url])
        } catch {
            app?.showError(error)
        }
    }

    private func watchUpload(_ item: TransferItem) {
        Task { [weak self] in
            while item.isActive { try? await Task.sleep(nanoseconds: 300_000_000) }
            if item.status == .done { await self?.refresh() }
        }
    }

    static func uniqueURL(_ url: URL) -> URL {
        let fm = FileManager.default
        guard fm.fileExists(atPath: url.path) else { return url }
        let base = url.deletingPathExtension().lastPathComponent
        let ext = url.pathExtension
        for i in 2..<1000 {
            let name = ext.isEmpty ? "\(base) \(i)" : "\(base) \(i).\(ext)"
            let u = url.deletingLastPathComponent().appendingPathComponent(name)
            if !fm.fileExists(atPath: u.path) { return u }
        }
        return url
    }
}
