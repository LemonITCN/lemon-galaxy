package cn.lemonit.galaxy.core.mock

import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.AiModel
import cn.lemonit.galaxy.core.bridge.DeviceInfo
import cn.lemonit.galaxy.core.bridge.DeviceKind
import cn.lemonit.galaxy.core.bridge.DisplayInfo
import cn.lemonit.galaxy.core.bridge.PathKind
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.SshAuth
import cn.lemonit.galaxy.core.bridge.UpdateInfo
import cn.lemonit.galaxy.core.bridge.VaultData
import cn.lemonit.galaxy.core.bridge.VaultEntry

/**
 * 模拟数据：设备、加密库、模型、显示器。
 * 截图测试与界面预览也直接用这里的数据，保证和运行时看到的一致。
 */
object MockData {
    /** 固定基准时间（2026-11-21 21:30 UTC+8），截图里的相对时间稳定。 */
    const val NOW = 1_795_267_800_000L
    private const val MIN = 60_000L
    private const val HOUR = 60 * MIN
    private const val DAY = 24 * HOUR

    const val DEV_BUILD_MAC = "9f2c4e1a7b3d4c58a0e6f1b2c3d4e5f6"
    const val DEV_WEB_01 = "1a2b3c4d5e6f708192a3b4c5d6e7f801"
    const val DEV_DB_01 = "2b3c4d5e6f708192a3b4c5d6e7f80112"
    const val DEV_OFFICE_PC = "3c4d5e6f708192a3b4c5d6e7f8011223"
    const val DEV_NAS = "4d5e6f708192a3b4c5d6e7f801122334"
    const val DEV_RENDER = "5e6f708192a3b4c5d6e7f80112233445"
    const val DEV_MACBOOK = "6f708192a3b4c5d6e7f8011223344556"
    const val DEV_SELF = "708192a3b4c5d6e7f801122334455667"

    /**
     * 服务器上被控端的新版本。web-01、build-mac 在线可升级；render-ws 有新版本但离线（详情里按钮不可用）；
     * OFFICE-PC 是 Windows 主控端（不被控），服务器也报了新版本，但界面不当作可升级的被控端。
     */
    const val UPGRADE_TO = "0.1.1"

    fun devices(now: Long = NOW): List<DeviceInfo> = listOf(
        DeviceInfo(DEV_BUILD_MAC, DeviceKind.CONTROL, Platform.MACOS, "build-mac", "macOS 27.2", "arm64", "0.1.0", true, true, now, false, UPGRADE_TO),
        DeviceInfo(DEV_WEB_01, DeviceKind.WORKER, Platform.LINUX, "web-01", "Ubuntu 24.04.3 LTS", "x86_64", "0.1.0", true, false, now, false, UPGRADE_TO),
        DeviceInfo(DEV_DB_01, DeviceKind.WORKER, Platform.LINUX, "db-01", "Debian 13", "x86_64", "0.1.0", false, false, now - 3 * HOUR, false),
        DeviceInfo(DEV_OFFICE_PC, DeviceKind.CONTROL, Platform.WINDOWS, "OFFICE-PC", "Windows 11 24H2", "x86_64", "0.1.0", true, false, now, false, UPGRADE_TO),
        DeviceInfo(DEV_NAS, DeviceKind.WORKER, Platform.LINUX, "nas", "Ubuntu 22.04 LTS", "aarch64", "0.1.0", false, true, now - 20 * MIN, false),
        DeviceInfo(DEV_RENDER, DeviceKind.WORKER, Platform.WINDOWS, "render-ws", "Windows 10 22H2", "x86_64", "0.1.0", false, false, now - 2 * DAY, false, UPGRADE_TO),
        DeviceInfo(DEV_MACBOOK, DeviceKind.CONTROL, Platform.MACOS, "MacBook-Air", "macOS 26.4", "arm64", "0.1.0", true, false, now, false),
        DeviceInfo(DEV_SELF, DeviceKind.MOBILE, Platform.ANDROID, "Pixel 9", "Android 16", "arm64-v8a", "0.1.0", true, true, now, true),
    )

    /** 连接到各设备时模拟的路径。 */
    fun pathFor(deviceId: String): Pair<PathKind, Int> = when (deviceId) {
        DEV_BUILD_MAC, DEV_NAS -> PathKind.LAN to 3
        DEV_WEB_01, DEV_MACBOOK -> PathKind.DIRECT to 28
        DEV_OFFICE_PC -> PathKind.RELAY_UDP to 46
        else -> PathKind.RELAY_TCP to 88
    }

    fun displaysFor(platform: Platform): List<DisplayInfo> = when (platform) {
        Platform.MACOS -> listOf(
            DisplayInfo(1u, "Built-in Display", 1920u, 1080u, 2.0f, true, false),
            DisplayInfo(2u, "DELL U2723QE", 2560u, 1440u, 1.0f, false, false),
        )
        Platform.WINDOWS -> listOf(
            DisplayInfo(1u, "\\\\.\\DISPLAY1", 1920u, 1080u, 1.25f, true, false),
            DisplayInfo(2u, "Virtual 1", 1280u, 720u, 1.0f, false, true),
        )
        else -> emptyList()
    }

    const val F_PROD = "f-prod"
    const val F_DB = "f-prod-db"
    const val F_OFFICE = "f-office"
    const val F_LAB = "f-lab"
    const val SSH_BASTION = "h-bastion"
    const val SSH_GITLAB = "h-gitlab"
    const val SSH_PG = "h-pg-replica"
    const val SSH_PI = "h-pi"
    const val RDP_FINANCE = "r-finance"

    /** 模拟服务器上的最新安卓版本。 */
    fun update(zh: Boolean) = UpdateInfo(
        version = "0.2.0",
        fileName = "LemonGalaxy-0.2.0.apk",
        size = 46_137_344u,
        sha256 = "ab".repeat(32),
        url = "/updates/files/mobile/android/LemonGalaxy-0.2.0.apk",
        notes = if (zh) {
            "- SSH 直连：主机密钥确认、连接路径显示\n- RDP 主机经设备转发，用 Windows App 打开\n- 远程桌面加隐私屏开关\n- 修复若干问题"
        } else {
            "- SSH direct: host key confirmation and route display\n- RDP hosts forwarded through a device, opened in Windows App\n- Privacy screen toggle in remote desktop\n- Bug fixes"
        },
    )

    /** 按数字段比较版本号（"0.10.0" 比 "0.9.1" 新）。 */
    fun isNewer(candidate: String, current: String): Boolean {
        val a = candidate.split('.', '-').map { it.toIntOrNull() ?: 0 }
        val b = current.split('.', '-').map { it.toIntOrNull() ?: 0 }
        for (i in 0 until maxOf(a.size, b.size)) {
            val x = a.getOrElse(i) { 0 }
            val y = b.getOrElse(i) { 0 }
            if (x != y) return x > y
        }
        return false
    }
    const val CRED_DEPLOY = "c-deploy"
    const val CRED_ROOT = "c-root-lab"

    fun vault(now: Long = NOW): List<VaultEntry> {
        var sort = 0L
        fun e(id: String, folder: String?, data: VaultData, age: Long = DAY) = VaultEntry(id, folder, sort++, data, now - age)
        return listOf(
            e(F_PROD, null, VaultData.Folder("Production", "#4C8BF5", true)),
            e(F_DB, F_PROD, VaultData.Folder("Databases", null, true)),
            e(F_OFFICE, null, VaultData.Folder("Office", "#F5A524", true)),
            e(F_LAB, null, VaultData.Folder("Home Lab", "#17C964", false)),
            e("m-web", F_PROD, VaultData.DeviceMeta(DEV_WEB_01, "web-01 (nginx)", listOf("prod", "web"), "", true)),
            e("m-db", F_DB, VaultData.DeviceMeta(DEV_DB_01, null, listOf("prod", "db"), "Primary PostgreSQL 17", false)),
            e("m-office", F_OFFICE, VaultData.DeviceMeta(DEV_OFFICE_PC, "Office PC", listOf("windows"), "", true)),
            e("m-render", F_OFFICE, VaultData.DeviceMeta(DEV_RENDER, null, listOf("windows", "gpu"), "", false)),
            e("m-nas", F_LAB, VaultData.DeviceMeta(DEV_NAS, "NAS", listOf("storage"), "", false)),
            e("m-mac", null, VaultData.DeviceMeta(DEV_BUILD_MAC, "Build Mac", listOf("ci"), "Xcode builds", true)),
            e(
                SSH_BASTION, F_PROD,
                VaultData.SshHost("bastion", "bastion.example.com", 22u, "ops", SshAuth.PrivateKey(CRED_DEPLOY), null, null, listOf("prod", "jump"), "", false, null),
            ),
            e(
                SSH_GITLAB, F_PROD,
                VaultData.SshHost("gitlab", "10.0.2.15", 22u, "git", SshAuth.PrivateKey(CRED_DEPLOY), null, SSH_BASTION, listOf("prod"), "", false, "nord"),
            ),
            e(
                SSH_PG, F_DB,
                VaultData.SshHost("pg-replica", "10.0.3.21", 22u, "postgres", SshAuth.Password(null), DEV_WEB_01, null, listOf("prod", "db"), "Streaming replica", false, null),
            ),
            e(
                SSH_PI, F_LAB,
                VaultData.SshHost("raspberrypi", "192.168.1.40", 22u, "pi", SshAuth.Interactive, null, null, listOf("iot"), "", false, null),
            ),
            e(
                RDP_FINANCE, F_OFFICE,
                VaultData.RdpHost("Finance PC", "192.168.10.23", 3389u, "finance", null, "CORP", DEV_OFFICE_PC, listOf("windows"), "", false),
            ),
            e(CRED_DEPLOY, null, VaultData.Credential("deploy key", "ops", null, "-----BEGIN OPENSSH PRIVATE KEY-----\n(mock)\n-----END OPENSSH PRIVATE KEY-----", null)),
            e(CRED_ROOT, null, VaultData.Credential("root@lab", "root", "mock-password", null, null)),
            e("s-df", null, VaultData.Snippet("Disk usage", "df -h", listOf("ops"))),
            e("s-nginx", null, VaultData.Snippet("Tail nginx errors", "tail -f /var/log/nginx/error.log", listOf("web"))),
            e("s-docker", null, VaultData.Snippet("Docker cleanup", "docker system prune -f", listOf("docker"))),
            e("k-bastion", null, VaultData.KnownHost("bastion.example.com", 22u, "ssh-ed25519", "SHA256:q3Vb1oX9mH2c7uY0kN4tLwR8zPaF5sDgE6jK1lQ2xYc")),
            // 桌面端建的端口映射规则：手机端不跑映射，各列表里都应忽略
            e(PF_PG, F_DB, VaultData.PortForward("pg via web-01", DEV_WEB_01, null, 15432u, "10.0.3.21", 5432u, false, "")),
        )
    }

    const val PF_PG = "p-pg"

    /**
     * 能远程开机的离线设备 → 代发唤醒包的设备名（空字符串表示本机）。
     * render-ws 与 Office PC 在同一网络、经它代发；db-01 与本机同一网络、本机直接发。
     */
    val WAKE_RELAYS: Map<String, List<String>> = mapOf(
        DEV_RENDER to listOf("OFFICE-PC"),
        DEV_DB_01 to listOf(""),
    )

    fun models(): List<AiModel> = listOf(
        AiModel(1u, "OpenCode Go", "glm-5.2", "GLM-5.2", vision = false, tools = true, contextLength = 200_000u),
        AiModel(1u, "OpenCode Go", "kimi-k2.6", "Kimi K2.6", vision = true, tools = true, contextLength = 256_000u),
        AiModel(1u, "OpenCode Go", "qwen3.6-plus", "Qwen3.6 Plus", vision = true, tools = true, contextLength = 1_000_000u),
        AiModel(1u, "OpenCode Go", "deepseek-v4", "DeepSeek V4", vision = false, tools = true, contextLength = 128_000u),
    )

    val maxAiMode = AiMode.FULL_TRUST

    /** 账号最近的登录与连接记录（新的在前）。 */
    fun activity(): List<cn.lemonit.galaxy.core.bridge.ActivityItem> = listOf(
        cn.lemonit.galaxy.core.bridge.ActivityItem(NOW - 5 * MIN, "connection.start", "alice", "device", DEV_WEB_01, "web-01", "203.0.113.24", ""),
        cn.lemonit.galaxy.core.bridge.ActivityItem(NOW - 6 * MIN, "user.login", "alice", "user", "", "", "203.0.113.24", "Pixel 9"),
        cn.lemonit.galaxy.core.bridge.ActivityItem(NOW - 11 * HOUR, "connection.end", "alice", "device", DEV_BUILD_MAC, "build-mac", "198.51.100.7", ""),
        cn.lemonit.galaxy.core.bridge.ActivityItem(NOW - 12 * HOUR, "connection.start", "alice", "device", DEV_BUILD_MAC, "build-mac", "198.51.100.7", ""),
    )
}
