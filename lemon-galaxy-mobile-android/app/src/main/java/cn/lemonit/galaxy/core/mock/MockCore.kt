package cn.lemonit.galaxy.core.mock

import cn.lemonit.galaxy.core.bridge.AgentConversationInterface
import cn.lemonit.galaxy.core.bridge.AgentListener
import cn.lemonit.galaxy.core.bridge.AgentMessage
import cn.lemonit.galaxy.core.bridge.AgentTargetKind
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.AiModels
import cn.lemonit.galaxy.core.bridge.AgentTarget
import cn.lemonit.galaxy.core.bridge.ClientListener
import cn.lemonit.galaxy.core.bridge.ClientState
import cn.lemonit.galaxy.core.bridge.ConversationConfig
import cn.lemonit.galaxy.core.bridge.ConversationSummary
import cn.lemonit.galaxy.core.bridge.CoreConfig
import cn.lemonit.galaxy.core.bridge.DeviceInfo
import cn.lemonit.galaxy.core.bridge.GalaxyClientInterface
import cn.lemonit.galaxy.core.bridge.GalaxyCore
import cn.lemonit.galaxy.core.bridge.GalaxyException
import cn.lemonit.galaxy.core.bridge.ImageData
import cn.lemonit.galaxy.core.bridge.LoginResult
import cn.lemonit.galaxy.core.bridge.PeerSessionInterface
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.RiskLevel
import cn.lemonit.galaxy.core.bridge.SecretPrompt
import cn.lemonit.galaxy.core.bridge.SecureStore
import cn.lemonit.galaxy.core.bridge.ServerInfo
import cn.lemonit.galaxy.core.bridge.SshAuth
import cn.lemonit.galaxy.core.bridge.SshInfo
import cn.lemonit.galaxy.core.bridge.SshSessionInterface
import cn.lemonit.galaxy.core.bridge.ToolCallView
import cn.lemonit.galaxy.core.bridge.ToolResultView
import cn.lemonit.galaxy.core.bridge.TotpSetup
import cn.lemonit.galaxy.core.bridge.UserInfo
import cn.lemonit.galaxy.core.bridge.VaultData
import cn.lemonit.galaxy.core.bridge.VaultEntry
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

/**
 * 完整的模拟内核：假设备、假加密库、假终端、假文件、假远程画面、脚本化 AI 对话。
 * 回调都在后台线程触发（和真内核一样），界面自己切回主线程。
 *
 * 模拟账号（登录页 i 弹层里也有说明）：
 *   - 任意用户名 + 任意密码 → 直接登录；密码写 wrong → 密码错误
 *   - 用户名以 totp 开头 → 需要两步验证码，正确的码是 123456
 *   - 用户名以 new 开头 → 首次登录，强制改密码
 */
class MockCore(private val assets: (String) -> ByteArray) : GalaxyCore {
    internal val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    @Volatile internal var config: CoreConfig? = null

    internal val zh: Boolean get() = config?.locale?.startsWith("zh") ?: true
    internal fun tr(zhText: String, en: String) = if (zh) zhText else en

    private val frames by lazy { AnnexB.split(assets("mock/desktop_1280x720.h264")) }
    private val preview by lazy {
        runCatching { ImageData("image/jpeg", assets("mock/desktop_preview.jpg"), 960u, 540u) }.getOrNull()
    }

    internal fun desktopFrames() = frames
    internal fun desktopPreview(): ImageData? = preview

    override fun coreVersion(): String = "0.1.0-mock"

    override fun coreInit(config: CoreConfig) {
        this.config = config
    }

    override fun coreSetLocale(locale: String) {
        config = config?.copy(locale = locale)
    }

    override fun newGalaxyClient(store: SecureStore, listener: ClientListener): GalaxyClientInterface = MockClient(this, store, listener)
}

internal class MockClient(
    private val core: MockCore,
    private val store: SecureStore,
    private val listener: ClientListener,
) : GalaxyClientInterface {
    private val scope = core.scope
    private fun tr(zh: String, en: String) = core.tr(zh, en)

    @Volatile private var state = ClientState.LOGGED_OUT
    @Volatile private var user: UserInfo? = null
    @Volatile private var server: ServerInfo? = null
    @Volatile private var pendingFirstLogin = false
    private var devices = MockData.devices(System.currentTimeMillis())
    private val vault = ConcurrentHashMap<String, VaultEntry>().apply { MockData.vault(System.currentTimeMillis()).forEach { put(it.id, it) } }
    private val hosts = ConcurrentHashMap<String, MockHost>()
    private val conversations = ConcurrentHashMap<String, MockConversation>()
    private var totpEnabled = false
    private var seeded = false
    @Volatile private var lanDiscovery = false

    private fun setState(s: ClientState) {
        state = s
        listener.onState(s)
    }

    override fun state(): ClientState = state
    override fun currentUser(): UserInfo? = user
    override fun server(): ServerInfo? = server

    override suspend fun login(server: String, username: String, password: String, totp: String?): LoginResult {
        setState(ClientState.CONNECTING)
        delay(700)
        if (!SERVER_RE.matches(server)) {
            setState(ClientState.LOGGED_OUT)
            throw GalaxyException.Network(tr("服务器地址格式不对", "Invalid server address"))
        }
        if (password == "wrong") {
            setState(ClientState.LOGGED_OUT)
            throw GalaxyException.Auth("invalid_credentials", tr("用户名或密码错误", "Incorrect username or password"))
        }
        val name = username.trim().lowercase()
        if (name.startsWith("totp")) {
            if (totp == null) {
                setState(ClientState.LOGGED_OUT)
                return LoginResult.NEED_TOTP
            }
            if (totp != "123456") {
                setState(ClientState.LOGGED_OUT)
                throw GalaxyException.Auth("totp_invalid", tr("验证码不正确", "Incorrect verification code"))
            }
            totpEnabled = true
        }
        this.server = ServerInfo(server, "LemonGalaxy", "0.1.0")
        this.user = UserInfo(3u, name, displayName(name), aiEnabled = true, totpEnabled = totpEnabled)
        if (name.startsWith("new")) {
            pendingFirstLogin = true
            setState(ClientState.LOGGED_OUT)
            return LoginResult.MUST_CHANGE_PASSWORD
        }
        finishLogin()
        return LoginResult.SUCCESS
    }

    private fun displayName(name: String) = name.replaceFirstChar { it.uppercase() }

    private fun finishLogin() {
        store.set(SESSION_KEY, "${server?.address}\n${user?.username}".toByteArray())
        setState(ClientState.ONLINE)
        listener.onDevicesChanged()
        listener.onVaultChanged()
    }

    override suspend fun completeFirstLogin(newPassword: String) {
        if (!pendingFirstLogin) throw GalaxyException.Other(tr("当前不需要修改密码", "No password change pending"))
        delay(800)
        if (newPassword.length < 8) throw GalaxyException.Other(tr("新密码至少 8 位", "New password must be at least 8 characters"))
        pendingFirstLogin = false
        finishLogin()
    }

    override fun deviceId(): String = MockData.DEV_SELF

    override fun setLanDiscovery(active: Boolean) {
        lanDiscovery = active
    }

    override suspend fun changePassword(oldPassword: String, newPassword: String, totp: String?) {
        delay(800)
        if (oldPassword == "wrong") throw GalaxyException.Auth("invalid_credentials", tr("原密码不正确", "Current password is incorrect"))
        if (newPassword.length < 8) throw GalaxyException.Other(tr("新密码至少 8 位", "New password must be at least 8 characters"))
    }

    override suspend fun logout() {
        delay(200)
        store.remove(SESSION_KEY)
        user = null
        server = null
        conversations.values.forEach { it.stop() }
        setState(ClientState.LOGGED_OUT)
    }

    override fun restore(): Boolean {
        val saved = store.get(SESSION_KEY)?.decodeToString() ?: return false
        val lines = saved.split('\n')
        if (lines.size < 2) return false
        server = ServerInfo(lines[0], "LemonGalaxy", "0.1.0")
        user = UserInfo(3u, lines[1], displayName(lines[1]), aiEnabled = true, totpEnabled = totpEnabled)
        setState(ClientState.ONLINE)
        return true
    }

    override fun devices(): List<DeviceInfo> = devices

    override suspend fun refreshDevices() {
        delay(500)
        devices = devices.map { if (it.online) it.copy(lastSeenAt = System.currentTimeMillis()) else it }
        listener.onDevicesChanged()
    }

    override suspend fun removeDevice(deviceId: String) {
        delay(300)
        devices = devices.filterNot { it.id == deviceId }
        listener.onDevicesChanged()
    }

    override fun vault(): List<VaultEntry> = vault.values.sortedBy { it.sort }

    override suspend fun vaultSave(entry: VaultEntry): VaultEntry {
        delay(150)
        val saved = if (entry.id.isEmpty()) {
            entry.copy(id = UUID.randomUUID().toString(), sort = (vault.values.maxOfOrNull { it.sort } ?: 0) + 1, updatedAt = System.currentTimeMillis())
        } else {
            entry.copy(updatedAt = System.currentTimeMillis())
        }
        vault[saved.id] = saved
        listener.onVaultChanged()
        return saved
    }

    override suspend fun vaultDelete(id: String) {
        delay(120)
        val removed = vault.remove(id)
        // 删文件夹时，里面的条目挪到上一级（与界面提示一致）
        val parent = removed?.folderId
        vault.values.filter { it.folderId == id }.forEach { vault[it.id] = it.copy(folderId = parent) }
        listener.onVaultChanged()
    }

    private fun hostFor(d: DeviceInfo) = hosts.getOrPut(d.id) {
        MockHost(d.id, d.hostname, d.platform, "demo", scope, ::tr)
    }

    override suspend fun connect(deviceId: String): PeerSessionInterface {
        val d = devices.firstOrNull { it.id == deviceId } ?: throw GalaxyException.NotFound(tr("找不到设备", "Device not found"))
        delay(600)
        if (!d.online && !d.lanSeen) throw GalaxyException.Network(tr("设备离线", "Device is offline"))
        if (d.isSelf) throw GalaxyException.Permission(tr("不能连接本机", "Cannot connect to this device"))
        return MockPeerSession(hostFor(d), core, d.osVersion, d.arch)
    }

    override suspend fun connectAssist(deviceCode: String, verificationCode: String): PeerSessionInterface {
        delay(1200)
        if (deviceCode.filter { it.isDigit() }.length != 9) throw GalaxyException.NotFound(tr("设备码不存在", "Unknown device code"))
        if (verificationCode.length < 4) throw GalaxyException.Auth("invalid_credentials", tr("验证码不正确", "Incorrect verification code"))
        val d = DeviceInfo("assist-$deviceCode", cn.lemonit.galaxy.core.bridge.DeviceKind.WORKER, Platform.WINDOWS, "ASSIST-PC", "Windows 11", "x86_64", "0.1.0", true, false, System.currentTimeMillis(), false)
        return MockPeerSession(hostFor(d), core, d.osVersion, d.arch)
    }

    override suspend fun connectSsh(hostId: String, prompt: SecretPrompt): SshSessionInterface {
        val entry = vault[hostId] ?: throw GalaxyException.NotFound(tr("找不到主机", "Host not found"))
        val h = entry.data as? VaultData.SshHost ?: throw GalaxyException.NotFound(tr("不是 SSH 主机", "Not an SSH host"))
        delay(500)
        withContext(Dispatchers.IO) {
            // 与真内核一样：没记录过主机密钥就请用户确认，确认后存进加密库
            val fp = "SHA256:" + UUID.nameUUIDFromBytes(h.host.toByteArray()).toString().replace("-", "").take(43)
            val known = vault.values.mapNotNull { it.data as? VaultData.KnownHost }.firstOrNull { it.host.equals(h.host, true) && it.port == h.port }
            if (known == null || known.fingerprint != fp) {
                if (!prompt.confirmHostKey(h.host, h.port, "ssh-ed25519", fp, known != null)) throw GalaxyException.Cancelled()
                val old = vault.values.firstOrNull { (it.data as? VaultData.KnownHost)?.let { k -> k.host.equals(h.host, true) && k.port == h.port } == true }
                val kh = VaultEntry(old?.id ?: UUID.randomUUID().toString(), null, 999, VaultData.KnownHost(h.host, h.port, "ssh-ed25519", fp), System.currentTimeMillis())
                vault[kh.id] = kh
                listener.onVaultChanged()
            }
            when (val a = h.auth) {
                is SshAuth.Password -> if (a.password == null) {
                    val pw = prompt.ask(tr("${h.username}@${h.host} 的密码：", "${h.username}@${h.host}'s password:"), false)
                        ?: throw GalaxyException.Cancelled()
                    if (pw == "wrong") throw GalaxyException.Auth("invalid_credentials", tr("密码错误", "Permission denied"))
                }
                SshAuth.Interactive -> prompt.ask(tr("验证码：", "Verification code:"), true) ?: throw GalaxyException.Cancelled()
                SshAuth.Agent -> throw GalaxyException.Permission(tr("暂不支持 SSH Agent，请改用私钥或密码", "SSH agent is not supported yet; use a private key or password"))
                else -> Unit
            }
            Unit
        }
        val via = when {
            !h.jumpDeviceId.isNullOrEmpty() -> "device:" + (devices.firstOrNull { it.id == h.jumpDeviceId }?.hostname ?: h.jumpDeviceId)
            !h.jumpHostId.isNullOrEmpty() -> "ssh:" + ((vault[h.jumpHostId]?.data as? VaultData.SshHost)?.label ?: h.jumpHostId)
            else -> "direct"
        }
        val mh = hosts.getOrPut("ssh-$hostId") { MockHost("ssh-$hostId", h.label, Platform.LINUX, h.username, scope, ::tr) }
        return MockSshSession(mh, SshInfo(hostId, h.label, h.host, h.port, h.username, via))
    }

    override suspend fun aiModels(): AiModels {
        delay(300)
        return AiModels(enabled = true, maxMode = MockData.maxAiMode, items = MockData.models())
    }

    private fun targetOf(t: AgentTarget): MockTarget = when (t.kind) {
        AgentTargetKind.DEVICE -> {
            val d = devices.firstOrNull { it.id == t.id }
            val alias = vault.values.mapNotNull { it.data as? VaultData.DeviceMeta }.firstOrNull { it.deviceId == t.id }?.alias
            MockTarget(t.id, alias ?: d?.hostname ?: t.id, d?.platform ?: Platform.LINUX)
        }
        AgentTargetKind.SSH_HOST -> MockTarget(t.id, (vault[t.id]?.data as? VaultData.SshHost)?.label ?: t.id, Platform.LINUX)
    }

    override suspend fun newConversation(config: ConversationConfig, listener: AgentListener): AgentConversationInterface {
        val model = MockData.models().firstOrNull { it.id == config.modelId }?.name ?: config.modelId
        val c = MockConversation(
            MockConversation.newId(), config, model, config.targets.map(::targetOf), listener, scope, core.zh,
            core::desktopPreview,
        )
        conversations[c.id()] = c
        return c
    }

    override suspend fun totpSetup(): TotpSetup {
        delay(300)
        val secret = "JBSWY3DPEHPK3PXP"
        return TotpSetup(secret, "otpauth://totp/LemonGalaxy:${user?.username ?: "user"}?secret=$secret&issuer=LemonGalaxy")
    }

    override suspend fun totpVerify(code: String) {
        delay(300)
        if (code != "123456") throw GalaxyException.Auth("totp_invalid", tr("验证码不正确（模拟：123456）", "Incorrect code (mock: 123456)"))
        totpEnabled = true
        user = user?.copy(totpEnabled = true)
    }

    override suspend fun totpDisable(code: String) {
        delay(300)
        if (code != "123456") throw GalaxyException.Auth("totp_invalid", tr("验证码不正确（模拟：123456）", "Incorrect code (mock: 123456)"))
        totpEnabled = false
        user = user?.copy(totpEnabled = false)
    }

    override fun listConversations(): List<ConversationSummary> {
        seedConversations()
        return conversations.values.map { it.summary() }.sortedByDescending { it.updatedAt }
    }

    override suspend fun openConversation(id: String, config: ConversationConfig, listener: AgentListener): AgentConversationInterface {
        seedConversations()
        val c = conversations[id] ?: throw GalaxyException.NotFound(tr("对话不存在", "Conversation not found"))
        c.listener = listener
        c.config = config.copy(title = c.title)
        return c
    }

    override fun deleteConversation(id: String) {
        conversations.remove(id)?.stop()
    }

    /** 预置两段历史对话，列表页一打开就有内容。 */
    @Synchronized
    private fun seedConversations() {
        if (seeded) return
        seeded = true
        val noop = object : AgentListener {
            override fun onEvent(event: cn.lemonit.galaxy.core.bridge.AgentEvent) = Unit
        }
        val t0 = MockData.NOW - 3 * 3600_000L
        val c1 = MockConversation(
            "seed-nginx", ConversationConfig(1u, "glm-5.2", listOf(AgentTarget(AgentTargetKind.DEVICE, MockData.DEV_WEB_01)), AiMode.CONFIRM_DANGEROUS, false, null, null),
            "GLM-5.2", listOf(MockTarget(MockData.DEV_WEB_01, "web-01 (nginx)", Platform.LINUX)), noop, scope, core.zh, core::desktopPreview,
        )
        val call = ToolCallView(
            "seed-call-1", "run_command", MockData.DEV_WEB_01, "web-01 (nginx)", "sudo systemctl reload nginx",
            "{\"command\":\"sudo systemctl reload nginx\"}", RiskLevel.DANGEROUS,
            listOf(tr("sudo 提权执行", "Runs with sudo")), tr("让新配置生效", "Apply the new configuration"), null,
        )
        c1.seed(
            listOf(
                AgentMessage.User(tr("nginx 改了配置，帮我检查并重新加载", "I changed the nginx config, please check it and reload")),
                AgentMessage.Assistant("a1", tr("先检查配置语法。", "Checking the configuration syntax first."), ""),
                AgentMessage.Tool(
                    ToolCallView("seed-call-0", "run_command", MockData.DEV_WEB_01, "web-01 (nginx)", "sudo nginx -t", "{}", RiskLevel.DANGEROUS, listOf(tr("sudo 提权执行", "Runs with sudo")), tr("检查配置语法", "Validate syntax"), null),
                    ToolResultView(true, tr("完成，退出码 0", "Done, exit code 0"), "nginx: the configuration file /etc/nginx/nginx.conf syntax is ok\nnginx: configuration file /etc/nginx/nginx.conf test is successful", null, false),
                ),
                AgentMessage.Tool(call, ToolResultView(true, tr("完成，退出码 0", "Done, exit code 0"), "", null, false)),
                AgentMessage.Assistant("a2", tr("配置检查通过，nginx 已重新加载。", "The configuration is valid and nginx has been reloaded."), ""),
            ),
            tr("nginx 改了配置，帮我检查并重新加载", "I changed the nginx config, please check it and reload"),
            t0 + 16000,
        )
        val t1 = MockData.NOW - 26 * 3600_000L
        val c2 = MockConversation(
            "seed-mac", ConversationConfig(1u, "kimi-k2.6", listOf(AgentTarget(AgentTargetKind.DEVICE, MockData.DEV_BUILD_MAC)), AiMode.CONFIRM_ALL, false, null, null),
            "Kimi K2.6", listOf(MockTarget(MockData.DEV_BUILD_MAC, "Build Mac", Platform.MACOS)), noop, scope, core.zh, core::desktopPreview,
        )
        c2.seed(
            listOf(
                AgentMessage.User(tr("看看 Xcode 为什么卡住了", "Why is Xcode stuck?")),
                AgentMessage.Assistant("a3", tr("Xcode 正在索引，占用 CPU 较高，等它完成即可。", "Xcode is indexing and using a lot of CPU; it should finish shortly."), ""),
            ),
            tr("看看 Xcode 为什么卡住了", "Why is Xcode stuck?"),
            t1 + 30000,
        )
        conversations[c1.id()] = c1
        conversations[c2.id()] = c2
    }

    companion object {
        private const val SESSION_KEY = "mock.session"
        val SERVER_RE = Regex("^[A-Za-z0-9.\\-\\[\\]:]+:\\d{1,5}#sha256-[A-Za-z0-9+/_=\\-]{8,}$")
    }
}
