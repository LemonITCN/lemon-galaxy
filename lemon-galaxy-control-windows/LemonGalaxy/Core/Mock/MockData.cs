namespace LemonGalaxy.Core.Mock;

/// <summary>模拟内核的固定假数据（与 Mac 端同一套，本机换成 Windows）。按界面语言给出中文或英文名称。</summary>
internal static class MockData
{
    public static long Now => Clock.NowMs;

    /// <summary>服务器上被控端的新版本（可升级的设备的 UpdateVersion）</summary>
    public const string UpgradeVersion = "0.1.0.72";

    public static DeviceInfo[] Devices(bool zh)
    {
        var t = Now;
        return new[]
        {
            new DeviceInfo("dev-self", DeviceKind.Control, Platform.Windows, "ALICE-WORKSTATION", "Windows 11 24H2", "x86_64", "0.1.0", true, true, t, true),
            // 可升级：构建服务器（在线）、Design-iMac（在线）、家里 NAS（离线，详情里的「升级被控端」不可用）
            new DeviceInfo("dev-build", DeviceKind.Worker, Platform.Linux, "build-server-01", "Ubuntu 24.04.1 LTS", "x86_64", "0.1.0.68", true, false, t, false, UpgradeVersion),
            new DeviceInfo("dev-gpu", DeviceKind.Worker, Platform.Linux, "gpu-node-02", "Debian GNU/Linux 12", "x86_64", "0.1.0", true, false, t, false),
            new DeviceInfo("dev-office", DeviceKind.Control, Platform.Windows, "OFFICE-PC-07", "Windows 11 23H2", "x86_64", "0.1.0", true, true, t, false),
            new DeviceInfo("dev-imac", DeviceKind.Worker, Platform.Macos, "Design-iMac", "macOS 14.7", "arm64", "0.1.0.68", true, true, t, false, UpgradeVersion),
            new DeviceInfo("dev-nas", DeviceKind.Worker, Platform.Linux, "nas", "Debian GNU/Linux 12", "aarch64", "0.1.0.61", false, false, t - 3_600_000L * 26, false, UpgradeVersion),
            new DeviceInfo("dev-reception", DeviceKind.Worker, Platform.Windows, "RECEPTION", "Windows 10 22H2", "x86_64", "0.1.0", false, false, t - 3_600_000L * 72, false),
            new DeviceInfo("dev-phone", DeviceKind.Mobile, Platform.Android, "Pixel 8", "Android 15", "aarch64", "0.1.0", true, false, t, false),
        };
    }

    public static List<VaultEntry> Vault(bool zh)
    {
        var t = Now;
        VaultEntry E(string id, string? folder, long sort, VaultData data) => new(id, folder, sort, data, t);
        const string key = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACBkZW1vLWtleS1ub3QtcmVhbC1sZW1vbmdhbGF4eS1tb2NrLWRhdGEAAAAA\n-----END OPENSSH PRIVATE KEY-----\n";
        string Z(string a, string b) => zh ? a : b;
        return new List<VaultEntry>
        {
            E("f-prod", null, 0, new VaultData.Folder(Z("生产环境", "Production"), "#3B82F6", true)),
            E("f-db", "f-prod", 10, new VaultData.Folder(Z("数据库", "Databases"), null, true)),
            E("f-office", null, 1, new VaultData.Folder(Z("办公室", "Office"), "#10B981", true)),
            E("f-home", null, 2, new VaultData.Folder(Z("个人", "Personal"), "#F59E0B", false)),

            E("m-build", "f-prod", 0, new VaultData.DeviceMeta("dev-build", Z("构建服务器", "Build server"), new[] { "ci", "linux" }, "", true)),
            E("m-gpu", "f-prod", 1, new VaultData.DeviceMeta("dev-gpu", null, new[] { "gpu" }, "", false)),
            E("m-office", "f-office", 0, new VaultData.DeviceMeta("dev-office", Z("办公室电脑", "Office PC"), new[] { "windows" }, "", false)),
            E("m-imac", "f-office", 1, new VaultData.DeviceMeta("dev-imac", null, new[] { "design" }, "", true)),
            E("m-nas", "f-home", 0, new VaultData.DeviceMeta("dev-nas", Z("家里 NAS", "Home NAS"), Array.Empty<string>(), "", false)),
            E("m-reception", "f-office", 2, new VaultData.DeviceMeta("dev-reception", Z("前台", "Reception"), new[] { "windows" }, "", false)),

            E("s-web1", "f-prod", 2, new VaultData.SshHost("web-01", "10.0.1.21", 22, "deploy", new SshAuth.PrivateKey("c-deploy"), null, null, new[] { "web", "prod" }, "", false, null)),
            E("s-web2", "f-prod", 3, new VaultData.SshHost("web-02", "10.0.1.22", 22, "deploy", new SshAuth.PrivateKey("c-deploy"), null, null, new[] { "web", "prod" }, "", false, null)),
            E("s-dbp", "f-db", 0, new VaultData.SshHost("db-primary", "10.0.2.10", 22, "postgres", new SshAuth.Password(null), "dev-build", null, new[] { "db", "prod" }, "", false, "nord")),
            E("s-dbr", "f-db", 1, new VaultData.SshHost("db-replica", "10.0.2.11", 22, "postgres", new SshAuth.Interactive(), "dev-build", null, new[] { "db" }, "", false, null)),
            E("s-router", "f-office", 3, new VaultData.SshHost(Z("办公室路由器", "Office router"), "192.168.1.1", 22, "admin", new SshAuth.Password("demo-password"), null, null, new[] { "network" }, "", false, null)),
            E("s-vps", null, 5, new VaultData.SshHost("vps-tokyo", "203.0.113.8", 2222, "root", new SshAuth.PrivateKey("c-deploy"), null, null, new[] { "vps" }, "", true, null)),
            E("r-win", "f-prod", 4, new VaultData.RdpHost("WIN-SRV-2022", "10.0.3.5", 3389, "Administrator", null, "CORP", "dev-build", new[] { "windows" }, "", false)),

            E("c-deploy", null, 0, new VaultData.Credential(Z("部署密钥", "Deploy key"), "deploy", null, key, null)),
            E("c-router", null, 1, new VaultData.Credential(Z("路由器管理员", "Router admin"), "admin", "demo-password", null, null)),

            E("n-df", null, 0, new VaultData.Snippet(Z("磁盘占用", "Disk usage"), "df -h", new[] { "linux" })),
            E("n-log", null, 1, new VaultData.Snippet(Z("最近系统日志", "Recent system log"), "journalctl -n 100 --no-pager", new[] { "linux" })),
            E("n-docker", null, 2, new VaultData.Snippet(Z("容器状态", "Container status"), "docker ps --format 'table {{.Names}}\\t{{.Status}}\\t{{.Ports}}'", new[] { "docker" })),
            E("n-update", null, 3, new VaultData.Snippet(Z("更新系统", "Update system"), "sudo apt update && sudo apt upgrade -y", new[] { "linux" })),
            E("n-top", null, 4, new VaultData.Snippet(Z("内存占用前十", "Top 10 by memory"), "ps aux --sort=-%mem | head -n 11", new[] { "linux" })),

            E("k-web1", null, 0, new VaultData.KnownHost("10.0.1.21", 22, "ssh-ed25519", "SHA256:7k3F0Zb3QmH1pRk2nB8xYw9eLqT4cVd5sJ6uG2aN1oE")),
            E("k-vps", null, 1, new VaultData.KnownHost("203.0.113.8", 2222, "ecdsa-sha2-nistp256", "SHA256:Qe2mW8rT5yU7iO9pA1sD3fG5hJ7kL9zX2cV4bN6mQ8w")),

            // 端口映射：经由设备（登录后自动开启）、经由 SSH 主机（自动选端口 / 要问密码）、经由离线设备（自动开启会失败）
            E("p-db", null, 0, new VaultData.PortForward(Z("生产数据库", "Production database"), "dev-build", null, 15432, "10.0.2.10", 5432, true, "")),
            E("p-grafana", null, 1, new VaultData.PortForward(Z("监控面板", "Grafana dashboard"), null, "s-web1", 0, "127.0.0.1", 3000, false, "")),
            E("p-redis", null, 2, new VaultData.PortForward(Z("缓存 Redis", "Redis cache"), null, "s-dbp", 16379, "127.0.0.1", 6379, false, Z("经 db-primary 登录，连接时要输入密码。", "Signs in through db-primary and asks for the password."))),
            E("p-nas", null, 3, new VaultData.PortForward(Z("家里 NAS 管理页", "Home NAS admin"), "dev-nas", null, 8443, "127.0.0.1", 443, true, "")),
        };
    }

    /// <summary>上报过网卡地址的设备（能远程开机；家里 NAS 没有上报过）</summary>
    public static bool HasMac(string id) => id is "dev-reception" or "dev-office" or "dev-build" or "dev-gpu" or "dev-imac";

    public static string Username(DeviceInfo d) => d.Platform switch
    {
        Platform.Windows or Platform.Macos => "alice",
        _ => d.Id == "dev-gpu" ? "ml" : "deploy",
    };

    public static PathInfo PathFor(DeviceInfo d)
    {
        if (d.LanSeen) return new PathInfo(PathKind.Lan, 2, "192.168.1." + (20 + d.Id.Sum(c => c) % 30) + ":51820");
        if (d.Id == "dev-gpu") return new PathInfo(PathKind.RelayUdp, 46, "relay");
        return new PathInfo(PathKind.Direct, 18, "203.0.113.24:51820");
    }

    public static Permissions PermissionsFor(DeviceInfo d) => d.Id == "dev-imac"
        ? new Permissions(true, true, false, true, true, false, AiMode.ConfirmAll)
        : new Permissions(true, true, true, true, true, true, AiMode.FullTrust);

    /// <summary>主控最多能在被控端加几块虚拟屏</summary>
    public const uint VirtualDisplayMax = 3;

    public static string[] Capabilities(DeviceInfo d)
    {
        var c = new List<string> { "terminal", "files", "exec", "port_forward" };
        // clipboard_offer：复制文件时只交换清单，粘贴时才传（protocol.md 8.4.1，新版被控端都有）
        if (d.Platform is Platform.Windows or Platform.Macos) c.AddRange(new[] { "display", "input", "clipboard", "clipboard_offer", "screenshot", "ui_tree" });
        if (d.Platform == Platform.Windows) c.Add("privacy");
        // 与内核一致：对方给了听声音的权限才有 "audio"
        if ((d.Platform is Platform.Windows or Platform.Macos) && PermissionsFor(d).Audio) c.Add("audio");
        // 与内核一致：被控端装了虚拟屏驱动、主控有控制权限才有（Windows、macOS 最多 3 块）
        if ((d.Platform is Platform.Windows or Platform.Macos) && PermissionsFor(d).Control) c.AddRange(new[] { "virtual_display", "virtual_display_max=" + VirtualDisplayMax });
        return c.ToArray();
    }

    public static DisplayInfo[] Displays(DeviceInfo d) => d.Platform switch
    {
        Platform.Windows => new[]
        {
            new DisplayInfo(1, "DELL U2723QE", 2560, 1440, 1.5f, true, false),
            new DisplayInfo(2, "HP E24 G5", 1920, 1080, 1f, false, false),
        },
        Platform.Macos => new[]
        {
            new DisplayInfo(1, "Built-in Retina Display", 2880, 1800, 2f, true, false),
            new DisplayInfo(3, "Virtual Display 1", 1920, 1080, 1f, false, true),
        },
        _ => Array.Empty<DisplayInfo>(),
    };
}

/// <summary>串行执行队列（模拟内核里代替内核线程：回调在线程池上按顺序触发）</summary>
internal sealed class SerialQueue
{
    private Task last = Task.CompletedTask;
    private readonly object lk = new();

    public void Post(Action a)
    {
        lock (lk)
        {
            last = last.ContinueWith(_ =>
            {
                try { a(); }
                catch (Exception e) { Log.Error("模拟内核回调出错：" + e); }
            }, TaskScheduler.Default);
        }
    }
}
