namespace LemonGalaxy;

/// <summary>服务器地址：host:port#sha256-指纹（security.md 第 7 节）。host 可以是域名、IPv4 或 [IPv6]。</summary>
internal sealed record ServerAddress(string Host, int Port, string Fingerprint)
{
    private const string Fp = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/-_=";
    private const string HostChars = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-";

    public static ServerAddress? Parse(string raw)
    {
        var s = raw.Trim();
        var hash = s.IndexOf('#');
        if (hash < 0) return null;
        var hostPort = s.Substring(0, hash);
        var fp = s.Substring(hash + 1);
        if (!fp.StartsWith("sha256-", StringComparison.Ordinal)) return null;
        var body = fp.Substring(7);
        // SHA-256 摘要的 base64 / base64url 编码：43 或 44 个字符
        if (body.Length < 43 || body.Length > 44 || body.Any(c => Fp.IndexOf(c) < 0)) return null;
        string host, portStr;
        if (hostPort.StartsWith("[", StringComparison.Ordinal))
        {
            var close = hostPort.IndexOf(']');
            if (close < 0) return null;
            host = hostPort.Substring(1, close - 1);
            var rest = hostPort.Substring(close + 1);
            if (!rest.StartsWith(":", StringComparison.Ordinal) || !host.Contains(":")) return null;
            portStr = rest.Substring(1);
        }
        else
        {
            var colon = hostPort.LastIndexOf(':');
            if (colon < 0) return null;
            host = hostPort.Substring(0, colon);
            portStr = hostPort.Substring(colon + 1);
            if (host.Length == 0 || host.Any(c => HostChars.IndexOf(c) < 0) || host.StartsWith(".", StringComparison.Ordinal) || host.StartsWith("-", StringComparison.Ordinal)) return null;
        }
        if (!int.TryParse(portStr, out var port) || port < 1 || port > 65535) return null;
        return new ServerAddress(host, port, fp);
    }
}
