// 内核接口层：记录（record → struct）与枚举（enum → enum），与 docs/ffi.md 一一对应。
// 命名按 uniffi 生成 Swift 的规则：类型名不变，字段与方法驼峰，枚举分支首字母小写，Vec<u8> → Data。
// 真内核接入后（编译条件 LEMONGALAXY_REAL_CORE），本文件整体由 Core/Generated/galaxy_ffi.swift 取代。
// 标「ffi.md 未定义」的类型是 ffi.md 引用了但没给出字段的，这里按界面需要先定，最终以内核为准（见最终报告的修改建议）。
#if !LEMONGALAXY_REAL_CORE
import Foundation

// MARK: - 1. 初始化

public enum Platform: Equatable, Hashable {
    case macos, windows, linux, android, ios, harmonyos
}

public enum DeviceKind: Equatable, Hashable {
    case control, worker, mobile
}

public struct CoreConfig: Equatable, Hashable {
    public var dataDir: String
    public var cacheDir: String
    public var logLevel: String
    public var platform: Platform
    public var deviceKind: DeviceKind
    public var appVersion: String
    public var osVersion: String
    public var hostname: String
    public var locale: String
    public init(dataDir: String, cacheDir: String, logLevel: String, platform: Platform, deviceKind: DeviceKind,
                appVersion: String, osVersion: String, hostname: String, locale: String) {
        self.dataDir = dataDir; self.cacheDir = cacheDir; self.logLevel = logLevel; self.platform = platform
        self.deviceKind = deviceKind; self.appVersion = appVersion; self.osVersion = osVersion
        self.hostname = hostname; self.locale = locale
    }
}

// MARK: - 2. GalaxyClient

public enum ClientState: Equatable, Hashable {
    case loggedOut, connecting, online, offline
}

public enum LoginResult: Equatable, Hashable {
    case success, needTotp, mustChangePassword
}

public struct UserInfo: Equatable, Hashable {
    public var id: UInt64
    public var username: String
    public var displayName: String
    public var aiEnabled: Bool
    public init(id: UInt64, username: String, displayName: String, aiEnabled: Bool) {
        self.id = id; self.username = username; self.displayName = displayName; self.aiEnabled = aiEnabled
    }
}

public struct ServerInfo: Equatable, Hashable {
    public var address: String
    public var name: String
    public var version: String
    public init(address: String, name: String, version: String) {
        self.address = address; self.name = name; self.version = version
    }
}

public struct DeviceInfo: Equatable, Hashable {
    public var id: String
    public var kind: DeviceKind
    public var platform: Platform
    public var hostname: String
    public var osVersion: String
    public var arch: String
    public var appVersion: String
    public var online: Bool
    public var lanSeen: Bool
    public var lastSeenAt: Int64
    public var isSelf: Bool
    public init(id: String, kind: DeviceKind, platform: Platform, hostname: String, osVersion: String, arch: String,
                appVersion: String, online: Bool, lanSeen: Bool, lastSeenAt: Int64, isSelf: Bool) {
        self.id = id; self.kind = kind; self.platform = platform; self.hostname = hostname; self.osVersion = osVersion
        self.arch = arch; self.appVersion = appVersion; self.online = online; self.lanSeen = lanSeen
        self.lastSeenAt = lastSeenAt; self.isSelf = isSelf
    }
}

// MARK: 2.1 加密库条目

public struct VaultEntry: Equatable, Hashable {
    public var id: String
    public var folderId: String?
    public var sort: Int64
    public var data: VaultData
    public var updatedAt: Int64
    public init(id: String, folderId: String?, sort: Int64, data: VaultData, updatedAt: Int64) {
        self.id = id; self.folderId = folderId; self.sort = sort; self.data = data; self.updatedAt = updatedAt
    }
}

public enum VaultData: Equatable, Hashable {
    case folder(name: String, color: String?, expanded: Bool)
    case deviceMeta(deviceId: String, alias: String?, tags: [String], note: String, favorite: Bool)
    case sshHost(label: String, host: String, port: UInt16, username: String, auth: SshAuth, jumpDeviceId: String?,
                 jumpHostId: String?, tags: [String], note: String, favorite: Bool, terminalTheme: String?)
    case rdpHost(label: String, host: String, port: UInt16, username: String, password: String?, domain: String?,
                 jumpDeviceId: String?, tags: [String], note: String, favorite: Bool)
    case credential(label: String, username: String, password: String?, privateKey: String?, passphrase: String?)
    case snippet(label: String, command: String, tags: [String])
    case knownHost(host: String, port: UInt16, keyType: String, fingerprint: String)
    case preferences(json: String)
}

public enum SshAuth: Equatable, Hashable {
    case password(password: String?)
    case privateKey(credentialId: String)
    case agent
    case interactive
}

// MARK: - 3. PeerSession

public enum PathKind: Equatable, Hashable {
    case lan, direct, relayUdp, relayTcp
}

public struct PathInfo: Equatable, Hashable {
    public var kind: PathKind
    public var rttMs: UInt32
    public var remoteAddr: String
    public init(kind: PathKind, rttMs: UInt32, remoteAddr: String) {
        self.kind = kind; self.rttMs = rttMs; self.remoteAddr = remoteAddr
    }
}

public struct Permissions: Equatable, Hashable {
    public var view: Bool
    public var control: Bool
    public var files: Bool
    public var terminal: Bool
    public var clipboard: Bool
    public var audio: Bool
    public var aiCap: AiMode
    public init(view: Bool, control: Bool, files: Bool, terminal: Bool, clipboard: Bool, audio: Bool, aiCap: AiMode) {
        self.view = view; self.control = control; self.files = files; self.terminal = terminal
        self.clipboard = clipboard; self.audio = audio; self.aiCap = aiCap
    }
}

public struct PeerInfo: Equatable, Hashable {
    public var deviceId: String
    public var hostname: String
    public var platform: Platform
    public var osVersion: String
    public var arch: String
    public var permissions: Permissions
    public var capabilities: [String]
    public var path: PathInfo
    public init(deviceId: String, hostname: String, platform: Platform, osVersion: String, arch: String,
                permissions: Permissions, capabilities: [String], path: PathInfo) {
        self.deviceId = deviceId; self.hostname = hostname; self.platform = platform; self.osVersion = osVersion
        self.arch = arch; self.permissions = permissions; self.capabilities = capabilities; self.path = path
    }
}

// MARK: 3.1 终端

public struct TerminalOptions: Equatable, Hashable {
    public var attachSession: String?
    public var shell: String?
    public var cols: UInt16
    public var rows: UInt16
    public init(attachSession: String?, shell: String?, cols: UInt16, rows: UInt16) {
        self.attachSession = attachSession; self.shell = shell; self.cols = cols; self.rows = rows
    }
}

public struct TerminalSessionInfo: Equatable, Hashable {
    public var id: String
    public var title: String
    public var createdAt: Int64
    public var attached: Bool
    public init(id: String, title: String, createdAt: Int64, attached: Bool) {
        self.id = id; self.title = title; self.createdAt = createdAt; self.attached = attached
    }
}

// MARK: 3.2 文件

public enum FileKind: Equatable, Hashable {
    case file, dir, symlink, other
}

public struct FileEntry: Equatable, Hashable {
    public var name: String
    public var path: String
    public var kind: FileKind
    public var size: UInt64
    public var mode: UInt32
    public var modifiedAt: Int64
    public var owner: String
    public var group: String
    public var linkTarget: String?
    public init(name: String, path: String, kind: FileKind, size: UInt64, mode: UInt32, modifiedAt: Int64,
                owner: String, group: String, linkTarget: String?) {
        self.name = name; self.path = path; self.kind = kind; self.size = size; self.mode = mode
        self.modifiedAt = modifiedAt; self.owner = owner; self.group = group; self.linkTarget = linkTarget
    }
}

// MARK: 命令执行（ffi.md 未定义字段，按 protocol.md 8.2 的 Exec 流）

public struct ExecRequest: Equatable, Hashable {
    public var command: String
    public var shell: String?
    public var cwd: String?
    public var timeoutMs: UInt32
    public var maxOutput: UInt32
    public init(command: String, shell: String?, cwd: String?, timeoutMs: UInt32, maxOutput: UInt32) {
        self.command = command; self.shell = shell; self.cwd = cwd; self.timeoutMs = timeoutMs; self.maxOutput = maxOutput
    }
}

public struct ExecResult: Equatable, Hashable {
    public var code: Int32?
    public var stdout: String
    public var stderr: String
    public var timedOut: Bool
    public var truncated: Bool
    public var durationMs: UInt64
    public init(code: Int32?, stdout: String, stderr: String, timedOut: Bool, truncated: Bool, durationMs: UInt64) {
        self.code = code; self.stdout = stdout; self.stderr = stderr; self.timedOut = timedOut
        self.truncated = truncated; self.durationMs = durationMs
    }
}

// MARK: 3.3 远程画面

public enum VideoCodec: Equatable, Hashable {
    case h264, h265
}

public struct DisplayInfo: Equatable, Hashable {
    public var id: UInt32
    public var name: String
    public var width: UInt32
    public var height: UInt32
    public var scale: Float
    public var primary: Bool
    public var isVirtual: Bool
    public init(id: UInt32, name: String, width: UInt32, height: UInt32, scale: Float, primary: Bool, isVirtual: Bool) {
        self.id = id; self.name = name; self.width = width; self.height = height; self.scale = scale
        self.primary = primary; self.isVirtual = isVirtual
    }
}

public struct DisplayOptions: Equatable, Hashable {
    public var displayId: UInt32
    public var maxFps: UInt32
    public var bitrateKbps: UInt32
    public var codecs: [VideoCodec]
    public init(displayId: UInt32, maxFps: UInt32, bitrateKbps: UInt32, codecs: [VideoCodec]) {
        self.displayId = displayId; self.maxFps = maxFps; self.bitrateKbps = bitrateKbps; self.codecs = codecs
    }
}

public enum MouseButton: Equatable, Hashable {
    case left, right, middle, back, forward
}

/// ffi.md 只写了 CtrlAltDel | LockScreen | ...，其余分支是界面工具栏用到的建议值。
public enum SpecialCombo: Equatable, Hashable {
    case ctrlAltDel, lockScreen, altTab, altF4, printScreen
}

public enum InputEvent: Equatable, Hashable {
    case mouseMove(x: Int32, y: Int32)
    case mouseButton(button: MouseButton, down: Bool, x: Int32, y: Int32)
    case wheel(dx: Int32, dy: Int32)
    case key(hidUsage: UInt32, down: Bool)
    case text(text: String)
    case specialCombo(combo: SpecialCombo)
}

/// ffi.md 未定义字段。文字、图片、文件三种内容可同时存在。
public struct ClipboardData: Equatable, Hashable {
    public var text: String?
    public var image: ImageData?
    public var files: [String]
    public init(text: String?, image: ImageData?, files: [String]) {
        self.text = text; self.image = image; self.files = files
    }
}

// MARK: - 4. AI 智能体

public enum AiMode: Equatable, Hashable {
    case off, readOnly, confirmAll, confirmDangerous, fullTrust
}

public struct AiModel: Equatable, Hashable {
    public var providerId: UInt64
    public var providerName: String
    public var id: String
    public var name: String
    public var vision: Bool
    public var tools: Bool
    public var contextLength: UInt32
    public init(providerId: UInt64, providerName: String, id: String, name: String, vision: Bool, tools: Bool, contextLength: UInt32) {
        self.providerId = providerId; self.providerName = providerName; self.id = id; self.name = name
        self.vision = vision; self.tools = tools; self.contextLength = contextLength
    }
}

public struct AiModels: Equatable, Hashable {
    public var enabled: Bool
    public var maxMode: AiMode
    public var items: [AiModel]
    public init(enabled: Bool, maxMode: AiMode, items: [AiModel]) {
        self.enabled = enabled; self.maxMode = maxMode; self.items = items
    }
}

public enum AgentTargetKind: Equatable, Hashable {
    case device, sshHost
}

public struct AgentTarget: Equatable, Hashable {
    public var kind: AgentTargetKind
    public var id: String
    public init(kind: AgentTargetKind, id: String) { self.kind = kind; self.id = id }
}

public struct ConversationConfig: Equatable, Hashable {
    public var providerId: UInt64
    public var modelId: String
    public var targets: [AgentTarget]
    public var mode: AiMode
    public var planFirst: Bool
    public init(providerId: UInt64, modelId: String, targets: [AgentTarget], mode: AiMode, planFirst: Bool) {
        self.providerId = providerId; self.modelId = modelId; self.targets = targets; self.mode = mode; self.planFirst = planFirst
    }
}

public enum ApprovalDecision: Equatable, Hashable {
    case allowOnce
    case allowSimilar
    case deny(reason: String?)
    case editAndRun(command: String)
}

public enum RiskLevel: Equatable, Hashable {
    case read, normal, dangerous, unknown, forbidden
}

public struct ImageData: Equatable, Hashable {
    public var mime: String
    public var data: Data
    public var width: UInt32
    public var height: UInt32
    public init(mime: String, data: Data, width: UInt32, height: UInt32) {
        self.mime = mime; self.data = data; self.width = width; self.height = height
    }
}

public struct ToolCallView: Equatable, Hashable {
    public var callId: String
    public var tool: String
    public var targetId: String
    public var targetName: String
    public var summary: String
    public var detail: String
    public var risk: RiskLevel
    public var riskReasons: [String]
    public var modelReason: String
    public var image: ImageData?
    public init(callId: String, tool: String, targetId: String, targetName: String, summary: String, detail: String,
                risk: RiskLevel, riskReasons: [String], modelReason: String, image: ImageData?) {
        self.callId = callId; self.tool = tool; self.targetId = targetId; self.targetName = targetName
        self.summary = summary; self.detail = detail; self.risk = risk; self.riskReasons = riskReasons
        self.modelReason = modelReason; self.image = image
    }
}

public struct ToolResultView: Equatable, Hashable {
    public var ok: Bool
    public var summary: String
    public var output: String
    public var image: ImageData?
    public var denied: Bool
    public init(ok: Bool, summary: String, output: String, image: ImageData?, denied: Bool) {
        self.ok = ok; self.summary = summary; self.output = output; self.image = image; self.denied = denied
    }
}

public enum AgentEvent: Equatable, Hashable {
    case assistantDelta(messageId: String, text: String)
    case assistantDone(messageId: String)
    case thinking(messageId: String, text: String)
    case toolProposed(call: ToolCallView, needsApproval: Bool)
    case toolStarted(callId: String)
    case toolOutput(callId: String, text: String)
    case toolFinished(callId: String, result: ToolResultView)
    case askUser(questionId: String, question: String, options: [String])
    case secretRequested(requestId: String, prompt: String, targetId: String)
    case usage(promptTokens: UInt64, completionTokens: UInt64)
    case status(running: Bool, step: UInt32)
    case error(message: String)
}

/// ffi.md 未定义字段：AgentConversation.messages() 的历史条目。
public enum AgentRole: Equatable, Hashable {
    case user, assistant, tool, system
}

public struct AgentMessage: Equatable, Hashable {
    public var id: String
    public var role: AgentRole
    public var text: String
    public var call: ToolCallView?
    public var result: ToolResultView?
    public var createdAt: Int64
    public init(id: String, role: AgentRole, text: String, call: ToolCallView?, result: ToolResultView?, createdAt: Int64) {
        self.id = id; self.role = role; self.text = text; self.call = call; self.result = result; self.createdAt = createdAt
    }
}

// MARK: - 5. HostService（ffi.md 未定义字段的记录都在这里）

public struct HostPolicy: Equatable, Hashable {
    public var permissions: Permissions
    /// 终端 / 文件 / 命令执行使用的本机系统用户
    public var systemUser: String
    public init(permissions: Permissions, systemUser: String) {
        self.permissions = permissions; self.systemUser = systemUser
    }
}

public struct HostAccount: Equatable, Hashable {
    public var userId: UInt64
    public var username: String
    public var displayName: String
    public var serverAddress: String
    public var policy: HostPolicy
    public init(userId: UInt64, username: String, displayName: String, serverAddress: String, policy: HostPolicy) {
        self.userId = userId; self.username = username; self.displayName = displayName
        self.serverAddress = serverAddress; self.policy = policy
    }
}

public struct HostSessionInfo: Equatable, Hashable {
    public var sessionId: String
    public var userId: UInt64?
    public var username: String
    public var displayName: String
    public var fromDevice: String
    public var fromPlatform: Platform
    /// 打开的功能："display" "terminal" "files" "exec" "assist" 等
    public var features: [String]
    public var ai: Bool
    public var startedAt: Int64
    public init(sessionId: String, userId: UInt64?, username: String, displayName: String, fromDevice: String,
                fromPlatform: Platform, features: [String], ai: Bool, startedAt: Int64) {
        self.sessionId = sessionId; self.userId = userId; self.username = username; self.displayName = displayName
        self.fromDevice = fromDevice; self.fromPlatform = fromPlatform; self.features = features; self.ai = ai
        self.startedAt = startedAt
    }
}

public struct AssistInfo: Equatable, Hashable {
    public var deviceCode: String
    public var verificationCode: String
    public var customCode: Bool
    public init(deviceCode: String, verificationCode: String, customCode: Bool) {
        self.deviceCode = deviceCode; self.verificationCode = verificationCode; self.customCode = customCode
    }
}

public struct HostStatus: Equatable, Hashable {
    public var remoteEnabled: Bool
    public var online: Bool
    public var accounts: [String]
    public var sessions: [HostSessionInfo]
    public var aiActive: Bool
    public var assist: AssistInfo?
    public init(remoteEnabled: Bool, online: Bool, accounts: [String], sessions: [HostSessionInfo], aiActive: Bool, assist: AssistInfo?) {
        self.remoteEnabled = remoteEnabled; self.online = online; self.accounts = accounts
        self.sessions = sessions; self.aiActive = aiActive; self.assist = assist
    }
}

public enum HostEventLevel: Equatable, Hashable {
    case info, warning, error
}

public struct HostEvent: Equatable, Hashable {
    public var time: Int64
    public var level: HostEventLevel
    /// "connect" "disconnect" "terminal" "file" "exec" "ai" "assist" "auth" "policy"
    public var category: String
    public var user: String?
    public var ai: Bool
    public var message: String
    public init(time: Int64, level: HostEventLevel, category: String, user: String?, ai: Bool, message: String) {
        self.time = time; self.level = level; self.category = category; self.user = user; self.ai = ai; self.message = message
    }
}

public struct CaptureOptions: Equatable, Hashable {
    public var maxFps: UInt32
    public var bitrateKbps: UInt32
    public var codec: VideoCodec
    public var maxWidth: UInt32
    public var maxHeight: UInt32
    public var showCursor: Bool
    public init(maxFps: UInt32, bitrateKbps: UInt32, codec: VideoCodec, maxWidth: UInt32, maxHeight: UInt32, showCursor: Bool) {
        self.maxFps = maxFps; self.bitrateKbps = bitrateKbps; self.codec = codec; self.maxWidth = maxWidth
        self.maxHeight = maxHeight; self.showCursor = showCursor
    }
}

// MARK: - 7. 错误

public enum GalaxyError: Error, Equatable, Hashable {
    case network(message: String)
    case auth(code: String, message: String)
    case permission(message: String)
    case notFound(message: String)
    case `protocol`(message: String)
    case cancelled
    case other(message: String)
}
#endif
