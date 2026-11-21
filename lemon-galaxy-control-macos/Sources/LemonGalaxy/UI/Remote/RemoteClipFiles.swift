import SwiftUI
import AppKit

// 远程桌面里的复制粘贴文件（protocol.md 8.4.1）：本机剪贴板里的文件经内核传到对方暂存区并设成对方剪贴板；
// 对方剪贴板里的文件下载到本机缓存目录再放进本机剪贴板。这里是剪贴板读写、缓存目录、进度回调与进度条视图。

/// 剪贴板与缓存目录（selftest 换成私有剪贴板与临时目录，不碰用户的）
enum ClipboardFiles {
    static var pasteboardOverride: NSPasteboard?
    static var pasteboard: NSPasteboard { pasteboardOverride ?? .general }

    static var cacheRootOverride: URL?
    /// ~/Library/Caches/cn.lemonit.galaxy.control/clipboard
    static var cacheRoot: URL {
        cacheRootOverride ?? FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("cn.lemonit.galaxy.control/clipboard", isDirectory: true)
    }

    /// 每次拉取一个新目录（<毫秒时间戳>-<随机数>），同名文件不会互相覆盖
    static func newPullDir() -> URL {
        cacheRoot.appendingPathComponent("\(Clock.nowMs)-\(UInt32.random(in: 0...UInt32.max) % 1_000_000)", isDirectory: true)
    }

    /// 删掉一天前的拉取目录（应用启动时在后台调用）
    @discardableResult
    static func cleanStale(in root: URL, olderThan age: TimeInterval = 86_400, now: Date = Date()) -> Int {
        let fm = FileManager.default
        let keys: [URLResourceKey] = [.creationDateKey, .contentModificationDateKey]
        guard let items = try? fm.contentsOfDirectory(at: root, includingPropertiesForKeys: keys, options: []) else { return 0 }
        var removed = 0
        for url in items {
            let v = try? url.resourceValues(forKeys: Set(keys))
            let latest = [v?.creationDate, v?.contentModificationDate].compactMap { $0 }.max() ?? .distantPast
            if now.timeIntervalSince(latest) > age, (try? fm.removeItem(at: url)) != nil { removed += 1 }
        }
        return removed
    }

    /// 剪贴板里的本机文件（访达复制的文件、文件夹）
    static func fileURLs(on pb: NSPasteboard) -> [URL] {
        let objs = pb.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL]
        return (objs ?? []).filter(\.isFileURL)
    }

    /// 把文件放进剪贴板，访达里粘贴即复制
    static func write(_ urls: [URL], to pb: NSPasteboard) {
        pb.clearContents()
        pb.writeObjects(urls as [NSURL])
    }

    /// 两份远端文件列表是不是同一批（被控端回来的路径可能换了斜杠或盘符大小写）
    static func sameFiles(_ a: [String]?, _ b: [String]?) -> Bool {
        guard let a, let b, a.count == b.count else { return false }
        func norm(_ p: String) -> String {
            var s = p.replacingOccurrences(of: "\\", with: "/").lowercased()
            while s.count > 1, s.hasSuffix("/") { s.removeLast() }
            return s
        }
        return zip(a, b).allSatisfy { norm($0) == norm($1) }
    }

    /// 远端路径的文件名（协议格式是 /C:/x 或 /home/x，兼容反斜杠）
    static func baseName(_ path: String) -> String {
        let trimmed = path.hasSuffix("/") || path.hasSuffix("\\") ? String(path.dropLast()) : path
        return trimmed.split(whereSeparator: { $0 == "/" || $0 == "\\" }).last.map(String.init) ?? path
    }
}

/// 一次复制粘贴文件在界面上的状态
struct RemoteFileClip: Equatable {
    enum Direction { case push, pull }
    enum State: Equatable { case running, done, failed(String) }

    let id = UUID()
    let direction: Direction
    /// 顶层文件 / 文件夹名
    let names: [String]
    var done: UInt64 = 0
    var total: UInt64 = 0
    var state: State = .running

    var fraction: Double { total > 0 ? min(1, Double(done) / Double(total)) : 0 }
    var percent: Int { Int((fraction * 100).rounded(.down)) }
}

/// 内核的进度回调（内核线程）→ 主线程；进度合并成每 0.1 秒最多一次刷新
final class ClipTransferListener: TransferListener, @unchecked Sendable {
    private let lock = NSLock()
    private var latest: (UInt64, UInt64) = (0, 0)
    private var scheduled = false
    private let progress: @MainActor (UInt64, UInt64) -> Void
    private let finished: @MainActor (String?) -> Void

    init(progress: @escaping @MainActor (UInt64, UInt64) -> Void, finished: @escaping @MainActor (String?) -> Void) {
        self.progress = progress
        self.finished = finished
    }

    func onProgress(done: UInt64, total: UInt64) {
        let first: Bool = lock.locked {
            latest = (done, total)
            defer { scheduled = true }
            return !scheduled
        }
        guard first else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { [self] in
            let v: (UInt64, UInt64) = lock.locked {
                scheduled = false
                return latest
            }
            MainActor.assumeIsolated { progress(v.0, v.1) }
        }
    }

    func onFinished(error: String?) {
        DispatchQueue.main.async { [self] in MainActor.assumeIsolated { finished(error) } }
    }
}

/// 远程画面上方的进度条：正在复制（百分比 + 取消）→ 完成提示 / 失败原因
struct RemoteFileClipPill: View {
    @ObservedObject var tab: RemoteDesktopModel
    let clip: RemoteFileClip
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        HStack(spacing: 10) {
            icon.frame(width: 18)
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 8) {
                    Text(title)
                        .font(.system(size: 12.5, weight: clip.state == .running ? .regular : .medium))
                        .foregroundStyle(p.text)
                        .lineLimit(clip.state == .running ? 1 : 2)
                        .truncationMode(.middle)
                        .fixedSize(horizontal: false, vertical: true)
                    if clip.state == .running {
                        Spacer(minLength: 0)
                        Text(verbatim: "\(clip.percent)%")
                            .font(.system(size: 12, weight: .medium).monospacedDigit())
                            .foregroundStyle(p.secondaryText)
                    }
                }
                if clip.state == .running { bar }
            }
            Button {
                if clip.state == .running { tab.cancelFileClip() } else { tab.dismissFileClip() }
            } label: {
                Image(systemName: "xmark").font(.system(size: 9.5, weight: .bold))
                    .foregroundStyle(p.tertiaryText)
                    .frame(width: 20, height: 20)
                    .background(Circle().fill(p.hover.opacity(2)))
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .help(loc(clip.state == .running ? "common.cancel" : "common.close"))
        }
        .padding(.leading, 12).padding(.trailing, 9).padding(.vertical, 9)
        .frame(width: clip.state == .running ? 380 : nil)
        .background(RoundedRectangle(cornerRadius: 10).fill(p.elevated).shadow(color: .black.opacity(0.28), radius: 12, y: 4))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(p.border, lineWidth: 0.5))
    }

    private var title: String {
        switch clip.state {
        case .running:
            let key = clip.direction == .push ? "remote.files.pushing" : "remote.files.pulling"
            return clip.names.count == 1 ? loc(key + "One", clip.names[0]) : loc(key + "Many", clip.names.count)
        case .done: return loc(clip.direction == .push ? "remote.files.pushed" : "remote.files.pulled")
        case .failed(let e): return loc("remote.files.failed", e)
        }
    }

    @ViewBuilder private var icon: some View {
        switch clip.state {
        case .running:
            Image(systemName: clip.direction == .push ? "arrow.up.doc" : "arrow.down.doc")
                .font(.system(size: 14, weight: .medium)).foregroundStyle(p.accent)
        case .done:
            Image(systemName: "checkmark.circle.fill").font(.system(size: 15)).foregroundStyle(p.success)
        case .failed:
            Image(systemName: "exclamationmark.triangle.fill").font(.system(size: 14)).foregroundStyle(p.danger)
        }
    }

    private var bar: some View {
        GeometryReader { g in
            ZStack(alignment: .leading) {
                Capsule().fill(p.hover.opacity(3))
                Capsule().fill(p.accent).frame(width: max(4, g.size.width * clip.fraction))
            }
        }
        .frame(height: 4)
    }
}
