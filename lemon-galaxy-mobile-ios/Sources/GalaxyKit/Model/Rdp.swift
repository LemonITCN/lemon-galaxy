import Foundation

/// 交给微软 Windows App（原「远程桌面」）的 rdp:// 链接。
/// 格式（微软文档）：rdp://full%20address=s:<host>:<port>&username=s:<用户名>，值按 URL 编码（地址里的冒号与方括号保留）。
/// 这个格式按 RFC 3986 不是合法 URL（「full%20address=s」会被当成主机、后面当端口），Foundation 的 URL(string:) 会返回 nil，
/// 所以用宽松的 CFURLCreateWithString 构造，原样交给系统打开。
/// 密码绝不放进链接（会留在系统日志里），由用户在 Windows App 里输入。
public enum RdpLink {
    /// App Store 上的 Windows App
    public static let appStoreURL = URL(string: "https://apps.apple.com/app/windows-app/id714464092")!

    /// 值里除字母数字、-._~ 与 keep 以外全部编码（& = \\ 空格等）
    static func encode(_ s: String, keep: String = "") -> String {
        var allowed = CharacterSet(charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
        allowed.insert(charactersIn: keep)
        return s.addingPercentEncoding(withAllowedCharacters: allowed) ?? s
    }

    /// 链接文字（单元测试与日志用）
    public static func string(for t: RdpTarget) -> String {
        // IPv6 地址要加方括号才能和端口区分
        let host = t.host.contains(":") && !t.host.hasPrefix("[") ? "[\(t.host)]" : t.host
        var parts = ["full%20address=s:" + encode("\(host):\(t.port)", keep: ":[]")]
        var user = t.username
        if let d = t.domain, !d.isEmpty, !user.isEmpty { user = d + "\\" + user }
        if !user.isEmpty { parts.append("username=s:" + encode(user)) }
        return "rdp://" + parts.joined(separator: "&")
    }

    public static func url(for t: RdpTarget) -> URL? {
        CFURLCreateWithString(nil, string(for: t) as CFString, nil) as URL?
    }
}

/// 一条进行中的 RDP 端口转发（经设备转发到本机端口，用完要关）。
public struct RdpForward: Identifiable, Equatable {
    public let id: String          // forward_id
    public let hostId: String
    public let label: String
    public let localPort: UInt16
    public let startedAt: Date
}

extension GalaxyAccount {
    /// 准备打开 RDP 主机：直连时就是主机地址；设了经由设备时内核在本机开临时端口转发，记下来以便停止
    public func openRdp(_ hostId: String) async throws -> RdpTarget {
        let t = try await client.openRdp(hostId: hostId)
        if t.forwarded {
            rdpForwards.append(RdpForward(id: t.forwardId, hostId: hostId, label: t.label, localPort: t.port, startedAt: Date()))
        }
        return t
    }

    public func closeRdp(_ forwardId: String) {
        client.closeRdp(forwardId: forwardId)
        rdpForwards.removeAll { $0.id == forwardId }
    }

    /// 退出登录时全部关闭
    public func closeAllRdp() {
        for f in rdpForwards { client.closeRdp(forwardId: f.id) }
        rdpForwards.removeAll()
    }
}

extension RdpTarget: Identifiable {
    public var id: String { forwardId.isEmpty ? "\(host):\(port)" : forwardId }
}
