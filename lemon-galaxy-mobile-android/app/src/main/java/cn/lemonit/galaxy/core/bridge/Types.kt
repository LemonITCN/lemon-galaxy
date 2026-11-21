@file:Suppress("unused")

package cn.lemonit.galaxy.core.bridge

/*
 * docs/ffi.md 的 Kotlin 对照（值类型部分）。
 *
 * 命名按 uniffi 的 Kotlin 生成规则：
 *   - record → data class，字段蛇形转驼峰；
 *   - 无数据的 enum → enum class，成员全大写下划线；
 *   - 带数据的 enum → sealed class，每个成员是嵌套的 data class / object；
 *   - u64/u32/u16 → ULong/UInt/UShort，i64/i32 → Long/Int，Vec<u8> → ByteArray，Option<T> → T?；
 *   - GalaxyError → GalaxyException（uniffi 把 Error 后缀换成 Exception）。
 * 字段与 galaxy-ffi（uniffi 0.29）生成的 uniffi.galaxy_ffi.* 一一对应；真内核由 core/rust/UniffiCore 适配到这里，
 * 模拟内核（截图测试、无 Rust 工具链时）直接实现这里的接口。界面只认这个包。
 */

// ---------- 1. 初始化 ----------

data class CoreConfig(
    val dataDir: String,
    val cacheDir: String,
    val logLevel: String,
    val platform: Platform,
    val deviceKind: DeviceKind,
    val appVersion: String,
    val osVersion: String,
    val hostname: String,
    val locale: String,
)

enum class Platform { MACOS, WINDOWS, LINUX, ANDROID, IOS, HARMONYOS }

enum class DeviceKind { CONTROL, WORKER, MOBILE }

// ---------- 2. GalaxyClient ----------

enum class ClientState { LOGGED_OUT, CONNECTING, ONLINE, OFFLINE }

enum class LoginResult { SUCCESS, NEED_TOTP, MUST_CHANGE_PASSWORD }

data class UserInfo(
    val id: ULong,
    val username: String,
    val displayName: String,
    val aiEnabled: Boolean,
    val totpEnabled: Boolean,
)

data class TotpSetup(
    val secret: String,
    val otpauthUri: String,
)

data class ServerInfo(
    val address: String,
    val name: String,
    val version: String,
)

data class DeviceInfo(
    val id: String,
    val kind: DeviceKind,
    val platform: Platform,
    val hostname: String,
    val osVersion: String,
    val arch: String,
    val appVersion: String,
    val online: Boolean,
    val lanSeen: Boolean,
    val lastSeenAt: Long,
    val isSelf: Boolean,
)

// ---------- 2.1 加密库条目 ----------

data class VaultEntry(
    val id: String,
    val folderId: String?,
    val sort: Long,
    val data: VaultData,
    val updatedAt: Long,
)

sealed class VaultData {
    data class Folder(
        val name: String,
        val color: String?,
        val expanded: Boolean,
    ) : VaultData()

    data class DeviceMeta(
        val deviceId: String,
        val alias: String?,
        val tags: List<String>,
        val note: String,
        val favorite: Boolean,
    ) : VaultData()

    data class SshHost(
        val label: String,
        val host: String,
        val port: UShort,
        val username: String,
        val auth: SshAuth,
        val jumpDeviceId: String?,
        val jumpHostId: String?,
        val tags: List<String>,
        val note: String,
        val favorite: Boolean,
        val terminalTheme: String?,
    ) : VaultData()

    data class RdpHost(
        val label: String,
        val host: String,
        val port: UShort,
        val username: String,
        val password: String?,
        val domain: String?,
        val jumpDeviceId: String?,
        val tags: List<String>,
        val note: String,
        val favorite: Boolean,
    ) : VaultData()

    data class Credential(
        val label: String,
        val username: String,
        val password: String?,
        val privateKey: String?,
        val passphrase: String?,
    ) : VaultData()

    data class Snippet(
        val label: String,
        val command: String,
        val tags: List<String>,
    ) : VaultData()

    data class KnownHost(
        val host: String,
        val port: UShort,
        val keyType: String,
        val fingerprint: String,
    ) : VaultData()

    data class Preferences(
        val json: String,
    ) : VaultData()
}

sealed class SshAuth {
    data class Password(val password: String?) : SshAuth()
    data class PrivateKey(val credentialId: String) : SshAuth()
    object Agent : SshAuth()
    object Interactive : SshAuth()
}

// ---------- 3. PeerSession ----------

data class PeerInfo(
    val deviceId: String,
    val hostname: String,
    val platform: Platform,
    val osVersion: String,
    val arch: String,
    val systemUser: String,
    val shell: String,
    val permissions: Permissions,
    val capabilities: List<String>,
    val path: PathInfo,
)

data class Permissions(
    val view: Boolean,
    val control: Boolean,
    val files: Boolean,
    val terminal: Boolean,
    val clipboard: Boolean,
    val audio: Boolean,
    val aiCap: AiMode,
)

enum class PathKind { CONNECTING, LAN, DIRECT, RELAY_UDP, RELAY_TCP, CLOSED }

data class PathInfo(
    val kind: PathKind,
    val rttMs: UInt,
    val remoteAddr: String,
)

data class ExecRequest(
    val command: String,
    val shell: String?,
    val cwd: String?,
    val timeoutMs: ULong,
    val maxOutput: ULong,
)

data class ExecResult(
    val code: Int?,
    val stdout: String,
    val stderr: String,
    val timedOut: Boolean,
    val truncated: Boolean,
    val durationMs: ULong,
)

sealed class ClipboardData {
    data class Text(val text: String) : ClipboardData()
    data class Image(val image: ImageData) : ClipboardData()
    data class Files(val paths: List<String>) : ClipboardData()
}

// ---------- 3.1 终端 ----------

data class EnvVar(
    val name: String,
    val value: String,
)

data class TerminalOptions(
    val attachSession: String?,
    val shell: String?,
    val cols: UShort,
    val rows: UShort,
    val env: List<EnvVar>,
)

data class TerminalSessionInfo(
    val id: String,
    val title: String,
    val createdAt: Long,
    val attached: Boolean,
)

// ---------- 3.2 文件 ----------

enum class FileKind { FILE, DIR, SYMLINK, OTHER }

data class FileEntry(
    val name: String,
    val path: String,
    val kind: FileKind,
    val size: ULong,
    val mode: UInt,
    val modifiedAt: Long,
    val owner: String,
    val group: String,
    val linkTarget: String?,
)

// ---------- 3.3 远程画面 ----------

data class DisplayInfo(
    val id: UInt,
    val name: String,
    val width: UInt,
    val height: UInt,
    val scale: Float,
    val primary: Boolean,
    val isVirtual: Boolean,
)

enum class VideoCodec { H264, H265 }

data class DisplayOptions(
    val displayId: UInt,
    val maxFps: UInt,
    val bitrateKbps: UInt,
    val codecs: List<VideoCodec>,
    val viewOnly: Boolean,
)

enum class PointerButton { LEFT, RIGHT, MIDDLE, BACK, FORWARD }

enum class ComboKey { CTRL_ALT_DEL, LOCK_SCREEN, ALT_TAB, WIN_KEY, TASK_MANAGER, SHOW_DESKTOP }

enum class TouchPhase { BEGAN, MOVED, ENDED, CANCELLED }

sealed class InputEvent {
    /** 坐标为被控屏幕像素。 */
    data class MouseMove(val x: Int, val y: Int) : InputEvent()

    data class MouseButton(
        val button: PointerButton,
        val down: Boolean,
        val x: Int,
        val y: Int,
    ) : InputEvent()

    data class Wheel(val dx: Int, val dy: Int) : InputEvent()

    /** hidUsage 为 USB HID 用法码（Keyboard/Keypad Page 0x07）。 */
    data class Key(val hidUsage: UInt, val down: Boolean) : InputEvent()

    data class Text(val text: String) : InputEvent()

    data class Touch(val id: UInt, val phase: TouchPhase, val x: Int, val y: Int) : InputEvent()

    data class SpecialCombo(val combo: ComboKey) : InputEvent()
}

// ---------- 4. AI 智能体 ----------

enum class AiMode { OFF, READ_ONLY, CONFIRM_ALL, CONFIRM_DANGEROUS, FULL_TRUST }

data class AiModels(
    val enabled: Boolean,
    val maxMode: AiMode,
    val items: List<AiModel>,
)

data class AiModel(
    val providerId: ULong,
    val providerName: String,
    val id: String,
    val name: String,
    val vision: Boolean,
    val tools: Boolean,
    val contextLength: UInt,
)

enum class AgentTargetKind { DEVICE, SSH_HOST }

data class AgentTarget(
    val kind: AgentTargetKind,
    val id: String,
)

data class ConversationConfig(
    val providerId: ULong,
    val modelId: String,
    val targets: List<AgentTarget>,
    val mode: AiMode,
    val planFirst: Boolean,
    val trustMinutes: UInt?,
    val title: String?,
)

/** 本机保存的对话（内核用设备密钥加密存放）。 */
data class ConversationSummary(
    val id: String,
    val title: String,
    val modelId: String,
    val targetNames: List<String>,
    val updatedAt: Long,
)

sealed class ApprovalDecision {
    object AllowOnce : ApprovalDecision()
    object AllowSimilar : ApprovalDecision()
    data class Deny(val reason: String?) : ApprovalDecision()
    data class EditAndRun(val command: String) : ApprovalDecision()
}

enum class RiskLevel { READ, NORMAL, DANGEROUS, UNKNOWN, FORBIDDEN }

data class ImageData(
    val mime: String,
    val data: ByteArray,
    val width: UInt,
    val height: UInt,
)

data class ToolCallView(
    val callId: String,
    val tool: String,
    val targetId: String,
    val targetName: String,
    val summary: String,
    val detail: String,
    val risk: RiskLevel,
    val riskReasons: List<String>,
    val modelReason: String,
    val image: ImageData?,
)

data class ToolResultView(
    val ok: Boolean,
    val summary: String,
    val output: String,
    val image: ImageData?,
    val denied: Boolean,
)

sealed class AgentEvent {
    data class AssistantDelta(val messageId: String, val text: String) : AgentEvent()
    data class AssistantDone(val messageId: String) : AgentEvent()
    data class Thinking(val messageId: String, val text: String) : AgentEvent()
    data class ToolProposed(val call: ToolCallView, val needsApproval: Boolean) : AgentEvent()
    data class ToolStarted(val callId: String) : AgentEvent()
    data class ToolOutput(val callId: String, val text: String) : AgentEvent()
    data class ToolFinished(val callId: String, val result: ToolResultView) : AgentEvent()
    data class AskUser(val questionId: String, val question: String, val options: List<String>) : AgentEvent()
    data class SecretRequested(val requestId: String, val prompt: String, val targetId: String) : AgentEvent()
    data class Usage(val promptTokens: ULong, val completionTokens: ULong) : AgentEvent()
    data class Status(val running: Boolean, val step: UInt) : AgentEvent()

    /** 档位变化：effective 为与管理员、机主上限取最严后的实际档位；完全信任带到期时间。 */
    data class ModeChanged(val mode: AiMode, val effective: AiMode, val untilMs: Long?) : AgentEvent()
    data class Error(val message: String) : AgentEvent()
}

/** `AgentConversation.messages()` 返回的历史（恢复界面用）。 */
sealed class AgentMessage {
    data class User(val text: String) : AgentMessage()
    data class Assistant(val id: String, val text: String, val thinking: String) : AgentMessage()
    data class Tool(val call: ToolCallView, val result: ToolResultView?) : AgentMessage()
}

// ---------- 7. 错误 ----------

/**
 * ffi.md 的 GalaxyError。message 已按 CoreConfig.locale 本地化，界面直接显示。
 * Auth.code：invalid_credentials | need_totp | locked | disabled | must_change_password
 */
sealed class GalaxyException : Exception() {
    class Network(override val message: String) : GalaxyException()
    class Auth(val code: String, override val message: String) : GalaxyException()
    class Permission(override val message: String) : GalaxyException()
    class NotFound(override val message: String) : GalaxyException()
    class Protocol(override val message: String) : GalaxyException()
    class Cancelled : GalaxyException() {
        override val message: String get() = "cancelled"
    }
    class Other(override val message: String) : GalaxyException()
}
