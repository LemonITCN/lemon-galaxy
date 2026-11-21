import Foundation

/// 设备列表 + 加密库：主机页、AI 目标选择、文件页共用的只读视图。
public struct HostsData {
    public var devices: [DeviceInfo]
    public var vault: [VaultEntry]

    public init(devices: [DeviceInfo] = [], vault: [VaultEntry] = []) {
        self.devices = devices
        self.vault = vault
    }

    public func entry(_ id: String) -> VaultEntry? { vault.first { $0.id == id } }
    public func device(_ id: String) -> DeviceInfo? { devices.first { $0.id == id } }

    public func metaFor(_ deviceId: String) -> VaultEntry? {
        vault.first { if case .deviceMeta(let d, _, _, _, _) = $0.data { return d == deviceId }; return false }
    }

    public var folders: [VaultEntry] { vault.filter { if case .folder = $0.data { return true }; return false } }
    public var credentials: [VaultEntry] { vault.filter { if case .credential = $0.data { return true }; return false }.sorted { ($0.sort, $0.id) < ($1.sort, $1.id) } }
    public var snippets: [VaultEntry] { vault.filter { if case .snippet = $0.data { return true }; return false }.sorted { ($0.sort, $0.id) < ($1.sort, $1.id) } }
    public var knownHosts: [VaultEntry] { vault.filter { if case .knownHost = $0.data { return true }; return false } }
    public var sshHosts: [VaultEntry] { vault.filter { if case .sshHost = $0.data { return true }; return false } }

    public func deviceName(_ id: String) -> String {
        if let m = metaFor(id), case .deviceMeta(_, let alias?, _, _, _) = m.data, !alias.trimmingCharacters(in: .whitespaces).isEmpty { return alias }
        return device(id)?.hostname ?? String(id.prefix(8))
    }

    public func sshLabel(_ id: String) -> String {
        if let e = entry(id), case .sshHost(let label, _, _, _, _, _, _, _, _, _, _) = e.data { return label }
        return id
    }

    public func targetName(_ t: AgentTarget) -> String {
        switch t.kind {
        case .device: return deviceName(t.id)
        case .sshHost: return sshLabel(t.id)
        }
    }

    public func folderName(_ id: String) -> String? {
        if let e = entry(id), case .folder(let name, _, _) = e.data { return name }
        return nil
    }

    /// 文件夹的完整路径（移动对话框与搜索结果用）
    public func folderPath(_ folderId: String?) -> String {
        var parts: [String] = []
        var cur = folderId
        var guardCount = 0
        while let c = cur, guardCount < 32 {
            guardCount += 1
            guard let e = entry(c), case .folder(let name, _, _) = e.data else { break }
            parts.insert(name, at: 0)
            cur = e.folderId
        }
        return parts.joined(separator: " / ")
    }
}

/// 树里的一个可连接条目。
public enum HostItem: Identifiable {
    case device(DeviceInfo, meta: VaultEntry?)
    case ssh(VaultEntry)
    case rdp(VaultEntry)

    public var id: String { key }

    public var key: String {
        switch self {
        case .device(let d, _): return "device:" + d.id
        case .ssh(let e): return "ssh:" + e.id
        case .rdp(let e): return "rdp:" + e.id
        }
    }

    public var folderId: String? {
        switch self {
        case .device(_, let m): return m?.folderId
        case .ssh(let e), .rdp(let e): return e.folderId
        }
    }

    public var title: String {
        switch self {
        case .device(let d, let m):
            if let m, case .deviceMeta(_, let alias?, _, _, _) = m.data, !alias.trimmingCharacters(in: .whitespaces).isEmpty { return alias }
            return d.hostname
        case .ssh(let e):
            if case .sshHost(let label, _, _, _, _, _, _, _, _, _, _) = e.data { return label }
            return ""
        case .rdp(let e):
            if case .rdpHost(let label, _, _, _, _, _, _, _, _, _) = e.data { return label }
            return ""
        }
    }

    public var tags: [String] {
        switch self {
        case .device(_, let m):
            if let m, case .deviceMeta(_, _, let tags, _, _) = m.data { return tags }
            return []
        case .ssh(let e):
            if case .sshHost(_, _, _, _, _, _, _, let tags, _, _, _) = e.data { return tags }
            return []
        case .rdp(let e):
            if case .rdpHost(_, _, _, _, _, _, _, let tags, _, _) = e.data { return tags }
            return []
        }
    }

    public var note: String {
        switch self {
        case .device(_, let m):
            if let m, case .deviceMeta(_, _, _, let note, _) = m.data { return note }
            return ""
        case .ssh(let e):
            if case .sshHost(_, _, _, _, _, _, _, _, let note, _, _) = e.data { return note }
            return ""
        case .rdp(let e):
            if case .rdpHost(_, _, _, _, _, _, _, _, let note, _) = e.data { return note }
            return ""
        }
    }

    public var favorite: Bool {
        switch self {
        case .device(_, let m):
            if let m, case .deviceMeta(_, _, _, _, let fav) = m.data { return fav }
            return false
        case .ssh(let e):
            if case .sshHost(_, _, _, _, _, _, _, _, _, let fav, _) = e.data { return fav }
            return false
        case .rdp(let e):
            if case .rdpHost(_, _, _, _, _, _, _, _, _, let fav) = e.data { return fav }
            return false
        }
    }

    public var sort: Int64 {
        switch self {
        case .device(_, let m): return m?.sort ?? Int64.max
        case .ssh(let e), .rdp(let e): return e.sort
        }
    }

    /// 地址行：设备是系统版本，SSH 是 user@host[:port]，RDP 是 [user@]host[:port]
    public var subtitle: String {
        switch self {
        case .device(let d, _): return d.osVersion
        case .ssh(let e):
            if case .sshHost(_, let host, let port, let user, _, _, _, _, _, _, _) = e.data {
                return "\(user)@\(host)" + (port != 22 ? ":\(port)" : "")
            }
            return ""
        case .rdp(let e):
            if case .rdpHost(_, let host, let port, let user, _, _, _, _, _, _) = e.data {
                return (user.isEmpty ? "" : "\(user)@") + host + (port != 3389 ? ":\(port)" : "")
            }
            return ""
        }
    }

    var searchText: String {
        var parts = [title, subtitle, note] + tags
        if case .device(let d, _) = self { parts += [d.hostname, d.osVersion] }
        return parts.joined(separator: "\n").lowercased()
    }

    public var device: DeviceInfo? {
        if case .device(let d, _) = self { return d }
        return nil
    }

    public var vaultEntry: VaultEntry? {
        switch self {
        case .device(_, let m): return m
        case .ssh(let e), .rdp(let e): return e
        }
    }

    /// 列表条目 → AI 对话目标（RDP 主机不行）
    public var agentTarget: AgentTarget? {
        switch self {
        case .device(let d, _): return AgentTarget(kind: .device, id: d.id)
        case .ssh(let e): return AgentTarget(kind: .sshHost, id: e.id)
        case .rdp: return nil
        }
    }
}

/// 列表里的一行：文件夹或条目。
public struct TreeRow: Identifiable {
    public let depth: Int
    public let folder: VaultEntry?
    public let item: HostItem?
    public let expanded: Bool
    public let count: Int
    /// 搜索 / 筛选结果里显示所在文件夹路径
    public let path: String?

    public var id: String { folder.map { "folder:" + $0.id } ?? item!.key }

    public var folderName: String {
        if let f = folder, case .folder(let name, _, _) = f.data { return name }
        return ""
    }

    public var folderColor: String? {
        if let f = folder, case .folder(_, let color, _) = f.data { return color }
        return nil
    }
}

public enum HostTree {
    /// 手机端只能主控、不能被控，不出现在可连接列表里
    public static func items(_ data: HostsData) -> [HostItem] {
        var metaByDevice: [String: VaultEntry] = [:]
        for e in data.vault {
            if case .deviceMeta(let d, _, _, _, _) = e.data { metaByDevice[d] = e }
        }
        var out: [HostItem] = data.devices.filter { $0.kind != .mobile && !$0.isSelf }.map { .device($0, meta: metaByDevice[$0.id]) }
        for e in data.vault {
            switch e.data {
            case .sshHost: out.append(.ssh(e))
            case .rdpHost: out.append(.rdp(e))
            default: break
            }
        }
        return out
    }

    public static func allTags(_ data: HostsData) -> [String] {
        Array(Set(items(data).flatMap(\.tags))).sorted()
    }

    /// query / tag / favoritesOnly 任一生效时返回平铺结果（带文件夹路径），否则返回按展开状态折叠的树。
    public static func rows(_ data: HostsData, query: String = "", tag: String? = nil, favoritesOnly: Bool = false,
                            expandedOverride: [String: Bool] = [:]) -> [TreeRow] {
        let all = items(data)
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if !q.isEmpty || tag != nil || favoritesOnly {
            let parts = q.split(separator: " ").map(String.init)
            return all
                .filter { item in parts.allSatisfy { item.searchText.contains($0) } }
                .filter { tag == nil || $0.tags.contains(tag!) }
                .filter { !favoritesOnly || $0.favorite }
                .sorted { a, b in
                    if a.favorite != b.favorite { return a.favorite }
                    return a.title.lowercased() < b.title.lowercased()
                }
                .map { item in
                    let p = data.folderPath(item.folderId)
                    return TreeRow(depth: 0, folder: nil, item: item, expanded: false, count: 0, path: p.isEmpty ? nil : p)
                }
        }
        let folderIds = Set(data.folders.map(\.id))
        func parentKey(_ id: String?) -> String { (id.flatMap { folderIds.contains($0) ? $0 : nil }) ?? "" }
        var foldersByParent: [String: [VaultEntry]] = [:]
        for f in data.folders where f.id != f.folderId { foldersByParent[parentKey(f.folderId), default: []].append(f) }
        var itemsByFolder: [String: [HostItem]] = [:]
        for it in all { itemsByFolder[parentKey(it.folderId), default: []].append(it) }

        var counted: [String: Int] = [:]
        func countIn(_ folderId: String, depth: Int = 0) -> Int {
            if let c = counted[folderId] { return c }
            guard depth < 32 else { return 0 }
            let c = (itemsByFolder[folderId]?.count ?? 0) + (foldersByParent[folderId] ?? []).reduce(0) { $0 + countIn($1.id, depth: depth + 1) }
            counted[folderId] = c
            return c
        }

        var out: [TreeRow] = []
        var visited = Set<String>()
        func walk(_ parent: String, _ depth: Int) {
            for f in (foldersByParent[parent] ?? []).sorted(by: { ($0.sort, $0.id) < ($1.sort, $1.id) }) {
                guard !visited.contains(f.id), depth < 32 else { continue }
                visited.insert(f.id)
                var expanded = false
                if case .folder(_, _, let e) = f.data { expanded = e }
                expanded = expandedOverride[f.id] ?? expanded
                out.append(TreeRow(depth: depth, folder: f, item: nil, expanded: expanded, count: countIn(f.id), path: nil))
                if expanded { walk(f.id, depth + 1) }
            }
            let items = (itemsByFolder[parent] ?? []).sorted { a, b in
                // 没整理过的设备（没有 DeviceMeta）排在后面；同组按排序值、在线优先、名称
                let am = (a.device != nil && a.vaultEntry == nil) ? 1 : 0
                let bm = (b.device != nil && b.vaultEntry == nil) ? 1 : 0
                if am != bm { return am < bm }
                if a.sort != b.sort { return a.sort < b.sort }
                let ao = a.device?.online ?? false, bo = b.device?.online ?? false
                if ao != bo { return ao }
                return a.title.lowercased() < b.title.lowercased()
            }
            for it in items { out.append(TreeRow(depth: depth, folder: nil, item: it, expanded: false, count: 0, path: nil)) }
        }
        walk("", 0)
        return out
    }

    /// 某个文件夹及其所有子孙文件夹的 id（移动文件夹时不能移进自己）
    public static func descendants(_ data: HostsData, of folderId: String) -> Set<String> {
        var result: Set<String> = [folderId]
        var changed = true
        while changed {
            changed = false
            for f in data.folders {
                if let p = f.folderId, result.contains(p), !result.contains(f.id) {
                    result.insert(f.id)
                    changed = true
                }
            }
        }
        return result
    }

    /// 移动目标：全部文件夹（带完整路径），排除 excluded
    public static func moveTargets(_ data: HostsData, excluding excluded: Set<String> = []) -> [(id: String, path: String)] {
        data.folders.filter { !excluded.contains($0.id) }
            .map { (id: $0.id, path: data.folderPath($0.id)) }
            .sorted { $0.path.lowercased() < $1.path.lowercased() }
    }
}
