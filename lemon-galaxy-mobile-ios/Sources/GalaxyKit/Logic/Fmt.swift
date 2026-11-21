import Foundation

/// 与平台无关的格式化：大小、权限、相对时间、路径。
public enum Fmt {
    public static func bytes(_ n: UInt64) -> String {
        let units = ["B", "KB", "MB", "GB", "TB"]
        var v = Double(n)
        var i = 0
        while v >= 1024, i < units.count - 1 { v /= 1024; i += 1 }
        if i == 0 { return "\(n) B" }
        return String(format: v >= 100 ? "%.0f %@" : (v >= 10 ? "%.1f %@" : "%.2f %@"), v, units[i])
    }

    /// 速度（每秒字节）
    public static func rate(_ bytesPerSecond: Double) -> String { bytes(UInt64(max(0, bytesPerSecond))) + "/s" }

    /// Unix 权限位 → drwxr-xr-x
    public static func mode(_ mode: UInt32, kind: FileKind = .file) -> String {
        let t: String
        switch kind {
        case .dir: t = "d"
        case .symlink: t = "l"
        case .other: t = "-"
        case .file: t = "-"
        }
        let chars = ["r", "w", "x"]
        var s = t
        for shift in stride(from: 6, through: 0, by: -3) {
            for (i, c) in chars.enumerated() {
                s += (mode >> UInt32(shift + 2 - i)) & 1 == 1 ? c : "-"
            }
        }
        return s
    }

    /// 0o755 → "755"
    public static func octal(_ mode: UInt32) -> String { String(mode & 0o7777, radix: 8) }

    /// "755" → 0o755；不合法返回 nil
    public static func parseOctal(_ s: String) -> UInt32? {
        let t = s.trimmingCharacters(in: .whitespaces)
        guard !t.isEmpty, t.count <= 4, let v = UInt32(t, radix: 8) else { return nil }
        return v
    }

    /// 毫秒时间戳 → 「刚刚 / N 分钟前 / N 小时前 / N 天前」，超过 30 天给日期
    public static func relative(_ ms: Int64, now: Int64 = Clock.nowMs, strings: Strings = L) -> String {
        let d = max(0, (now - ms) / 1000)
        if d < 60 { return strings("time.justNow") }
        if d < 3600 { return strings("time.minutesAgo", d / 60) }
        if d < 86_400 { return strings("time.hoursAgo", d / 3600) }
        if d < 86_400 * 30 { return strings("time.daysAgo", d / 86_400) }
        return date(ms)
    }

    public static func date(_ ms: Int64) -> String {
        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd HH:mm"
        return f.string(from: Date(timeIntervalSince1970: TimeInterval(ms) / 1000))
    }

    /// 秒级时间戳（文件修改时间）
    public static func fileDate(_ seconds: Int64) -> String {
        let ms = seconds > 10_000_000_000 ? seconds : seconds * 1000
        return date(ms)
    }

    public static func tokens(_ n: UInt64) -> String {
        if n >= 1_000_000 { return String(format: "%.1fM", Double(n) / 1_000_000) }
        if n >= 1000 { return String(format: "%.1fK", Double(n) / 1000) }
        return "\(n)"
    }
}

/// 远端路径（POSIX 风格；Windows 被控端的路径内核已映射成 /C:/... 形式，同样按 / 处理）。
public enum RemotePath {
    public static func join(_ dir: String, _ name: String) -> String {
        if dir.isEmpty { return name }
        return dir.hasSuffix("/") ? dir + name : dir + "/" + name
    }

    public static func parent(_ path: String) -> String {
        var p = path
        while p.count > 1, p.hasSuffix("/") { p.removeLast() }
        guard let i = p.lastIndex(of: "/") else { return "/" }
        let parent = String(p[..<i])
        return parent.isEmpty ? "/" : parent
    }

    public static func name(_ path: String) -> String {
        var p = path
        while p.count > 1, p.hasSuffix("/") { p.removeLast() }
        if let i = p.lastIndex(of: "/") { return String(p[p.index(after: i)...]) }
        return p
    }

    /// 面包屑：/home/deploy/app → [("/", "/"), ("home", "/home"), ("deploy", "/home/deploy"), ("app", "/home/deploy/app")]
    public static func crumbs(_ path: String) -> [(name: String, path: String)] {
        var out: [(String, String)] = [("/", "/")]
        var cur = ""
        for part in path.split(separator: "/") {
            cur += "/" + part
            out.append((String(part), cur))
        }
        return out
    }

    /// 文件名合法性（不能为空、不能含 /、不能是 . 或 ..）
    public static func isValidName(_ name: String) -> Bool {
        let t = name.trimmingCharacters(in: .whitespaces)
        return !t.isEmpty && !t.contains("/") && t != "." && t != ".."
    }
}

public enum FileSort: String, CaseIterable {
    case name, size, modified

    public static func sorted(_ entries: [FileEntry], by sort: FileSort, descending: Bool, showHidden: Bool) -> [FileEntry] {
        let visible = showHidden ? entries : entries.filter { !$0.name.hasPrefix(".") }
        return visible.sorted { a, b in
            let ad = a.kind == .dir, bd = b.kind == .dir
            if ad != bd { return ad }   // 文件夹始终在前
            let r: Bool
            switch sort {
            case .name: r = a.name.localizedStandardCompare(b.name) == .orderedAscending
            case .size: r = a.size == b.size ? a.name.localizedStandardCompare(b.name) == .orderedAscending : a.size < b.size
            case .modified: r = a.modifiedAt == b.modifiedAt ? a.name.localizedStandardCompare(b.name) == .orderedAscending : a.modifiedAt < b.modifiedAt
            }
            return descending ? !r : r
        }
    }
}

/// 远程画面画质档位：码率与帧率上限（与安卓一致）。auto 码率交给被控端按网络自适应。
public enum VideoQuality: String, CaseIterable, Identifiable {
    case auto, smooth, balanced, hd, ultra
    public var id: String { rawValue }

    public var bitrateKbps: UInt32 {
        switch self {
        case .auto: return 0
        case .smooth: return 2_000
        case .balanced: return 6_000
        case .hd: return 12_000
        case .ultra: return 30_000
        }
    }

    public var maxFps: UInt32 { self == .smooth ? 30 : 60 }
}

/// AI 权限档位的顺序（off < read_only < confirm_all < confirm_dangerous < full_trust）
public extension AiMode {
    var rank: Int {
        switch self {
        case .off: return 0
        case .readOnly: return 1
        case .confirmAll: return 2
        case .confirmDangerous: return 3
        case .fullTrust: return 4
        }
    }

    var key: String {
        switch self {
        case .off: return "off"
        case .readOnly: return "readOnly"
        case .confirmAll: return "confirmAll"
        case .confirmDangerous: return "confirmDangerous"
        case .fullTrust: return "fullTrust"
        }
    }

    static func fromKey(_ k: String?) -> AiMode? {
        [AiMode.off, .readOnly, .confirmAll, .confirmDangerous, .fullTrust].first { $0.key == k }
    }

    /// 不超过上限（off 上限时退到只读，界面不会选 off）
    static func clamp(_ mode: AiMode, to maxMode: AiMode) -> AiMode {
        mode.rank <= maxMode.rank ? mode : (maxMode == .off ? .readOnly : maxMode)
    }

    /// 界面能选的档位（不超过管理员给的上限）
    static func selectable(maxMode: AiMode) -> [AiMode] {
        [AiMode.readOnly, .confirmAll, .confirmDangerous, .fullTrust].filter { $0.rank <= maxMode.rank }
    }
}

extension FileEntry: Identifiable {
    public var id: String { path }
}
