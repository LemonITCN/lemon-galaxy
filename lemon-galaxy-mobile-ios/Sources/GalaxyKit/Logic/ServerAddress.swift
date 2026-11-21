import Foundation

/// 服务器地址：host:port#sha256-<证书指纹>（管理后台「概览」页复制，或扫码）。IPv6 写成 [addr]:port。
public struct ServerAddress: Equatable {
    public let host: String
    public let port: Int
    public let fingerprint: String

    public init(host: String, port: Int, fingerprint: String) {
        self.host = host
        self.port = port
        self.fingerprint = fingerprint
    }

    public static func parse(_ raw: String) -> ServerAddress? {
        let s = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let hashIdx = s.firstIndex(of: "#") else { return nil }
        let hostPort = String(s[..<hashIdx])
        let fp = String(s[s.index(after: hashIdx)...])
        guard fp.hasPrefix("sha256-") else { return nil }
        let body = fp.dropFirst("sha256-".count)
        // SHA-256 摘要的 base64 / base64url：43 或 44 个字符
        let allowed = CharacterSet(charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/-_=")
        guard (43...44).contains(body.count), body.unicodeScalars.allSatisfy(allowed.contains) else { return nil }
        var host = ""
        var portStr = ""
        if hostPort.hasPrefix("[") {
            guard let close = hostPort.firstIndex(of: "]") else { return nil }
            host = String(hostPort[hostPort.index(after: hostPort.startIndex)..<close])
            let rest = hostPort[hostPort.index(after: close)...]
            guard rest.hasPrefix(":") else { return nil }
            portStr = String(rest.dropFirst())
            let v6 = CharacterSet(charactersIn: "0123456789abcdefABCDEF:.")
            guard host.contains(":"), host.unicodeScalars.allSatisfy(v6.contains) else { return nil }
        } else {
            guard let colon = hostPort.lastIndex(of: ":") else { return nil }
            host = String(hostPort[..<colon])
            portStr = String(hostPort[hostPort.index(after: colon)...])
            let hostChars = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-")
            guard !host.isEmpty, host.unicodeScalars.allSatisfy(hostChars.contains), !host.hasPrefix("."), !host.hasPrefix("-") else { return nil }
        }
        guard !portStr.isEmpty, portStr.count <= 5, portStr.allSatisfy(\.isASCII), portStr.allSatisfy(\.isNumber),
              let port = Int(portStr), (1...65535).contains(port) else { return nil }
        return ServerAddress(host: host, port: port, fingerprint: String(fp))
    }

    public static func isValid(_ s: String) -> Bool { parse(s) != nil }

    /// 扫码结果：可以直接是地址，也可以是 lemongalaxy://login?server=<地址>。返回规范化的地址串。
    public static func parseScanned(_ raw: String) -> String? {
        let t = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if let a = parse(t) { return a.text }
        guard let c = URLComponents(string: t), c.scheme?.lowercased() == "lemongalaxy",
              let s = c.queryItems?.first(where: { $0.name == "server" })?.value,
              let a = parse(s) else { return nil }
        return a.text
    }

    /// 规范写法（登录时交给内核）
    public var text: String { "\(hostPort)#\(fingerprint)" }

    /// 只给人看的 host:port
    public var hostPort: String { host.contains(":") ? "[\(host)]:\(port)" : "\(host):\(port)" }
}
