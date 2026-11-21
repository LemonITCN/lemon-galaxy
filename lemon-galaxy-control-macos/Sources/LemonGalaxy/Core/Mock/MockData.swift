import Foundation

/// 模拟内核的固定假数据。按界面语言给出中文或英文的名称，截图在两种语言下都自然。
enum MockData {
    static var now: Int64 { Clock.nowMs }
    /// 假数据的时间基准（启动时的「现在」；selftest 下是冻结的时间）
    static var fixedNow: Int64 { Clock.nowMs }

    static func isValidServer(_ s: String) -> Bool { ServerAddress.parse(s) != nil }

    static func devices(zh: Bool) -> [DeviceInfo] {
        let t = fixedNow
        return [
            DeviceInfo(id: "dev-self", kind: .control, platform: .macos, hostname: "Alice-MacBook-Pro", osVersion: "macOS 15.3",
                       arch: "arm64", appVersion: "0.1.0", online: true, lanSeen: true, lastSeenAt: t, isSelf: true),
            DeviceInfo(id: "dev-build", kind: .worker, platform: .linux, hostname: "build-server-01", osVersion: "Ubuntu 24.04.1 LTS",
                       arch: "x86_64", appVersion: "0.1.0", online: true, lanSeen: false, lastSeenAt: t, isSelf: false),
            DeviceInfo(id: "dev-gpu", kind: .worker, platform: .linux, hostname: "gpu-node-02", osVersion: "Debian GNU/Linux 12",
                       arch: "x86_64", appVersion: "0.1.0", online: true, lanSeen: false, lastSeenAt: t, isSelf: false),
            DeviceInfo(id: "dev-office", kind: .control, platform: .windows, hostname: "OFFICE-PC-07", osVersion: "Windows 11 23H2",
                       arch: "x86_64", appVersion: "0.1.0", online: true, lanSeen: true, lastSeenAt: t, isSelf: false),
            DeviceInfo(id: "dev-imac", kind: .worker, platform: .macos, hostname: "Design-iMac", osVersion: "macOS 14.7",
                       arch: "arm64", appVersion: "0.1.0", online: true, lanSeen: true, lastSeenAt: t, isSelf: false),
            DeviceInfo(id: "dev-nas", kind: .worker, platform: .linux, hostname: "nas", osVersion: "Debian GNU/Linux 12",
                       arch: "aarch64", appVersion: "0.1.0", online: false, lanSeen: false, lastSeenAt: t - 3_600_000 * 26, isSelf: false),
            DeviceInfo(id: "dev-reception", kind: .worker, platform: .windows, hostname: "RECEPTION", osVersion: "Windows 10 22H2",
                       arch: "x86_64", appVersion: "0.1.0", online: false, lanSeen: false, lastSeenAt: t - 3_600_000 * 72, isSelf: false),
            DeviceInfo(id: "dev-phone", kind: .mobile, platform: .android, hostname: "Pixel 8", osVersion: "Android 15",
                       arch: "aarch64", appVersion: "0.1.0", online: true, lanSeen: false, lastSeenAt: t, isSelf: false),
        ]
    }

    static func vault(zh: Bool) -> [VaultEntry] {
        let t = fixedNow
        func e(_ id: String, _ folder: String?, _ sort: Int64, _ data: VaultData) -> VaultEntry {
            VaultEntry(id: id, folderId: folder, sort: sort, data: data, updatedAt: t)
        }
        let key = """
        -----BEGIN OPENSSH PRIVATE KEY-----
        b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
        QyNTUxOQAAACBkZW1vLWtleS1ub3QtcmVhbC1sZW1vbmdhbGF4eS1tb2NrLWRhdGEAAAAA
        -----END OPENSSH PRIVATE KEY-----
        """
        return [
            e("f-prod", nil, 0, .folder(name: zh ? "生产环境" : "Production", color: "#3B82F6", expanded: true)),
            e("f-db", "f-prod", 10, .folder(name: zh ? "数据库" : "Databases", color: nil, expanded: true)),
            e("f-office", nil, 1, .folder(name: zh ? "办公室" : "Office", color: "#10B981", expanded: true)),
            e("f-home", nil, 2, .folder(name: zh ? "个人" : "Personal", color: "#F59E0B", expanded: false)),

            e("m-build", "f-prod", 0, .deviceMeta(deviceId: "dev-build", alias: zh ? "构建服务器" : "Build server", tags: ["ci", "linux"], note: "", favorite: true)),
            e("m-gpu", "f-prod", 1, .deviceMeta(deviceId: "dev-gpu", alias: nil, tags: ["gpu"], note: "", favorite: false)),
            e("m-office", "f-office", 0, .deviceMeta(deviceId: "dev-office", alias: zh ? "办公室电脑" : "Office PC", tags: ["windows"], note: "", favorite: false)),
            e("m-imac", "f-office", 1, .deviceMeta(deviceId: "dev-imac", alias: nil, tags: ["design"], note: "", favorite: true)),
            e("m-nas", "f-home", 0, .deviceMeta(deviceId: "dev-nas", alias: zh ? "家里 NAS" : "Home NAS", tags: [], note: "", favorite: false)),
            e("m-reception", "f-office", 2, .deviceMeta(deviceId: "dev-reception", alias: zh ? "前台" : "Reception", tags: ["windows"], note: "", favorite: false)),

            e("s-web1", "f-prod", 2, .sshHost(label: "web-01", host: "10.0.1.21", port: 22, username: "deploy", auth: .privateKey(credentialId: "c-deploy"),
                                              jumpDeviceId: nil, jumpHostId: nil, tags: ["web", "prod"], note: "", favorite: false, terminalTheme: nil)),
            e("s-web2", "f-prod", 3, .sshHost(label: "web-02", host: "10.0.1.22", port: 22, username: "deploy", auth: .privateKey(credentialId: "c-deploy"),
                                              jumpDeviceId: nil, jumpHostId: nil, tags: ["web", "prod"], note: "", favorite: false, terminalTheme: nil)),
            e("s-dbp", "f-db", 0, .sshHost(label: "db-primary", host: "10.0.2.10", port: 22, username: "postgres", auth: .password(password: nil),
                                           jumpDeviceId: "dev-build", jumpHostId: nil, tags: ["db", "prod"], note: "", favorite: false, terminalTheme: "nord")),
            e("s-dbr", "f-db", 1, .sshHost(label: "db-replica", host: "10.0.2.11", port: 22, username: "postgres", auth: .interactive,
                                           jumpDeviceId: "dev-build", jumpHostId: nil, tags: ["db"], note: "", favorite: false, terminalTheme: nil)),
            e("s-router", "f-office", 3, .sshHost(label: zh ? "办公室路由器" : "Office router", host: "192.168.1.1", port: 22, username: "admin",
                                                  auth: .password(password: "••••"), jumpDeviceId: nil, jumpHostId: nil, tags: ["network"], note: "", favorite: false, terminalTheme: nil)),
            e("s-vps", nil, 5, .sshHost(label: "vps-tokyo", host: "203.0.113.8", port: 2222, username: "root", auth: .agent,
                                        jumpDeviceId: nil, jumpHostId: nil, tags: ["vps"], note: "", favorite: true, terminalTheme: nil)),
            e("r-win", "f-prod", 4, .rdpHost(label: "WIN-SRV-2022", host: "10.0.3.5", port: 3389, username: "Administrator", password: nil, domain: "CORP",
                                             jumpDeviceId: "dev-build", tags: ["windows"], note: "", favorite: false)),

            e("c-deploy", nil, 0, .credential(label: zh ? "部署密钥" : "Deploy key", username: "deploy", password: nil, privateKey: key, passphrase: nil)),
            e("c-router", nil, 1, .credential(label: zh ? "路由器管理员" : "Router admin", username: "admin", password: "demo-password", privateKey: nil, passphrase: nil)),

            e("n-df", nil, 0, .snippet(label: zh ? "磁盘占用" : "Disk usage", command: "df -h", tags: ["linux"])),
            e("n-log", nil, 1, .snippet(label: zh ? "最近系统日志" : "Recent system log", command: "journalctl -n 100 --no-pager", tags: ["linux"])),
            e("n-docker", nil, 2, .snippet(label: zh ? "容器状态" : "Container status", command: "docker ps --format 'table {{.Names}}\\t{{.Status}}\\t{{.Ports}}'", tags: ["docker"])),
            e("n-update", nil, 3, .snippet(label: zh ? "更新系统" : "Update system", command: "sudo apt update && sudo apt upgrade -y", tags: ["linux"])),
            e("n-top", nil, 4, .snippet(label: zh ? "内存占用前十" : "Top 10 by memory", command: "ps aux --sort=-%mem | head -n 11", tags: ["linux"])),

            e("k-web1", nil, 0, .knownHost(host: "10.0.1.21", port: 22, keyType: "ssh-ed25519", fingerprint: knownFingerprints["10.0.1.21"]!)),
            e("k-vps", nil, 1, .knownHost(host: "203.0.113.8", port: 2222, keyType: "ecdsa-sha2-nistp256", fingerprint: knownFingerprints["203.0.113.8"]!)),
        ]
    }

    /// 模拟 SSH 主机的主机密钥指纹（加密库里的 known_host 与模拟连接时报出的一致）
    static let knownFingerprints: [String: String] = [
        "10.0.1.21": "SHA256:7k3F0Zb3QmH1pRk2nB8xYw9eLqT4cVd5sJ6uG2aN1oE",
        "203.0.113.8": "SHA256:Qe2mW8rT5yU7iO9pA1sD3fG5hJ7kL9zX2cV4bN6mQ8w",
    ]

    static func username(for d: DeviceInfo) -> String {
        switch d.platform {
        case .windows: return "alice"
        case .macos: return "alice"
        default: return d.id == "dev-gpu" ? "ml" : "deploy"
        }
    }

    static func path(for d: DeviceInfo) -> PathInfo {
        if d.lanSeen { return PathInfo(kind: .lan, rttMs: 2, remoteAddr: "192.168.1.\(20 + d.id.unicodeScalars.reduce(0) { $0 + Int($1.value) } % 30):51820") }
        if d.id == "dev-gpu" { return PathInfo(kind: .relayUdp, rttMs: 46, remoteAddr: "relay") }
        return PathInfo(kind: .direct, rttMs: 18, remoteAddr: "203.0.113.24:51820")
    }

    static func permissions(for d: DeviceInfo) -> Permissions {
        if d.id == "dev-imac" {
            return Permissions(view: true, control: true, files: false, terminal: true, clipboard: true, audio: false, aiCap: .confirmAll)
        }
        return Permissions(view: true, control: true, files: true, terminal: true, clipboard: true, audio: true, aiCap: .fullTrust)
    }

    static func capabilities(for d: DeviceInfo) -> [String] {
        var c = ["terminal", "files", "exec", "port_forward"]
        if d.platform == .windows || d.platform == .macos { c += ["display", "input", "clipboard", "screenshot", "ui_tree", "privacy"] }
        return c
    }

    static func displays(for d: DeviceInfo) -> [DisplayInfo] {
        switch d.platform {
        case .windows:
            return [DisplayInfo(id: 1, name: "DELL U2723QE", width: 2560, height: 1440, scale: 1.5, primary: true, isVirtual: false),
                    DisplayInfo(id: 2, name: "HP E24 G5", width: 1920, height: 1080, scale: 1, primary: false, isVirtual: false)]
        case .macos:
            return [DisplayInfo(id: 1, name: "Built-in Retina Display", width: 2880, height: 1800, scale: 2, primary: true, isVirtual: false),
                    DisplayInfo(id: 3, name: "Virtual Display 1", width: 1920, height: 1080, scale: 1, primary: false, isVirtual: true)]
        default:
            return []
        }
    }
}
