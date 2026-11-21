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
 * 真绑定到位后，这个包换成 uniffi.galaxy_ffi 的类型（或在这里写 typealias 指过去），界面代码不用改。
 *
 * ffi.md 里引用了但没定义的类型（ExecRequest、ClipboardData、AgentMessage 等）在这里按协议文档补了定义，
 * 标注「ffi.md 未定义」，需要内核按此或另行在 ffi.md 里定下来。
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

/** ffi.md 只写了「kind: Lan | Direct | RelayUdp | RelayTcp」，这里按此定义。 */
enum class PathKind { LAN, DIRECT, RELAY_UDP, RELAY_TCP }

data class PathInfo(
    val kind: PathKind,
    val rttMs: UInt,
    val remoteAddr: String,
)

/** ffi.md 未定义：按 protocol.md 8.2 的 Exec 流补充。 */
data class ExecRequest(
    val command: String,
    val shell: String?,
    val cwd: String?,
    val timeoutMs: UInt?,
    val maxOutput: ULong?,
)

/** ffi.md 未定义：按 protocol.md 8.2 的 Exec Done 补充。 */
data class ExecResult(
    val code: Int?,
    val signal: Int?,
    val stdout: ByteArray,
    val stderr: ByteArray,
    val timedOut: Boolean,
    val truncated: Boolean,
    val durationMs: ULong,
)

/** ffi.md 未定义：剪贴板内容（文字或图片，二者至多其一）。 */
data class ClipboardData(
    val text: String?,
    val image: ImageData?,
)

// ---------- 3.1 终端 ----------

data class TerminalOptions(
    val attachSession: String?,
    val shell: String?,
    val cols: UShort,
    val rows: UShort,
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
)

/** protocol.md 8.4：Left | Right | Middle | Back | Forward。 */
enum class MouseButton { LEFT, RIGHT, MIDDLE, BACK, FORWARD }

/** ffi.md 只写了「CtrlAltDel | LockScreen | ...」，其余成员为补充建议。 */
enum class SpecialCombo { CTRL_ALT_DEL, LOCK_SCREEN, TASK_MANAGER, SHOW_DESKTOP }

sealed class InputEvent {
    /** 坐标为被控屏幕像素。 */
    data class MouseMove(val x: Int, val y: Int) : InputEvent()

    // 注意：成员名与枚举 MouseButton 同名，uniffi 生成代码里需要全限定名（见报告里的 ffi.md 建议）
    data class MouseButton(
        val button: cn.lemonit.galaxy.core.bridge.MouseButton,
        val down: Boolean,
        val x: Int,
        val y: Int,
    ) : InputEvent()

    data class Wheel(val dx: Int, val dy: Int) : InputEvent()

    /** hidUsage 为 USB HID 用法码（Keyboard/Keypad Page 0x07）。 */
    data class Key(val hidUsage: UInt, val down: Boolean) : InputEvent()

    data class Text(val text: String) : InputEvent()

    data class SpecialCombo(val combo: cn.lemonit.galaxy.core.bridge.SpecialCombo) : InputEvent()
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
    data class Error(val message: String) : AgentEvent()
}

/** ffi.md 未定义：对话历史中的角色。 */
enum class AgentRole { USER, ASSISTANT, TOOL, SYSTEM }

/**
 * ffi.md 未定义：`AgentConversation.messages()` 返回的历史条目（恢复界面用）。
 * 工具调用一条一个，toolCall 与 toolResult 同时给出；approval 记录用户当时的决定（自动放行为空）。
 */
data class AgentMessage(
    val id: String,
    val role: AgentRole,
    val text: String,
    val thinking: String?,
    val toolCall: ToolCallView?,
    val toolResult: ToolResultView?,
    val approval: ApprovalDecision?,
    val createdAt: Long,
)

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
