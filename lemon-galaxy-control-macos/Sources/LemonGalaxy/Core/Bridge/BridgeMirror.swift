// 由 scripts/gen-bridge-mirror.py 从 Core/Generated/galaxy_ffi.swift 生成，不要手改。
// 没有接入 Rust 内核（无 Vendor/LemonGalaxyCore.xcframework）时用它编译：类型与真绑定逐字相同，对象方法是占位，
// 只有 MockCore 的子类会被创建。接入真内核后（编译条件 LEMONGALAXY_REAL_CORE）本文件整体不参与编译。
#if !LEMONGALAXY_REAL_CORE
import Foundation

@inline(never)
func bridgeUnimplemented(_ function: String = #function) -> Never {
    fatalError("内核接口 \(function) 没有实现：当前没有接入 Rust 内核，只能用 MockCore")
}

public func coreVersion() -> String { "0.0.0-mirror" }
public func coreInit(config: CoreConfig) {}
public func coreSetLocale(locale: String) {}
public func coreLogPath() -> String { "" }

// MARK: - 记录与枚举

public struct AgentTarget: Equatable, Hashable {
    public var kind: AgentTargetKind
    public var id: String

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(kind: AgentTargetKind, id: String) {
        self.kind = kind
        self.id = id
    }

    

    
}

#if compiler(>=6)
extension AgentTarget: Sendable {}
#endif

public struct AiModel: Equatable, Hashable {
    public var providerId: UInt64
    public var providerName: String
    public var id: String
    public var name: String
    public var vision: Bool
    public var tools: Bool
    public var contextLength: UInt32

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(providerId: UInt64, providerName: String, id: String, name: String, vision: Bool, tools: Bool, contextLength: UInt32) {
        self.providerId = providerId
        self.providerName = providerName
        self.id = id
        self.name = name
        self.vision = vision
        self.tools = tools
        self.contextLength = contextLength
    }

    

    
}

#if compiler(>=6)
extension AiModel: Sendable {}
#endif

public struct AiModels: Equatable, Hashable {
    public var enabled: Bool
    public var maxMode: AiMode
    public var items: [AiModel]

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(enabled: Bool, maxMode: AiMode, items: [AiModel]) {
        self.enabled = enabled
        self.maxMode = maxMode
        self.items = items
    }

    

    
}

#if compiler(>=6)
extension AiModels: Sendable {}
#endif

public struct AssistInfo: Equatable, Hashable {
    public var deviceCode: String
    public var code: String

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(deviceCode: String, code: String) {
        self.deviceCode = deviceCode
        self.code = code
    }

    

    
}

#if compiler(>=6)
extension AssistInfo: Sendable {}
#endif

public struct CaptureOptions: Equatable, Hashable {
    public var maxFps: UInt32
    public var bitrateKbps: UInt32
    public var codecs: [VideoCodec]

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(maxFps: UInt32, bitrateKbps: UInt32, codecs: [VideoCodec]) {
        self.maxFps = maxFps
        self.bitrateKbps = bitrateKbps
        self.codecs = codecs
    }

    

    
}

#if compiler(>=6)
extension CaptureOptions: Sendable {}
#endif

public struct CaptureStarted: Equatable, Hashable {
    public var width: UInt32
    public var height: UInt32
    public var codec: VideoCodec

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(width: UInt32, height: UInt32, codec: VideoCodec) {
        self.width = width
        self.height = height
        self.codec = codec
    }

    

    
}

#if compiler(>=6)
extension CaptureStarted: Sendable {}
#endif

public struct ConversationConfig: Equatable, Hashable {
    public var providerId: UInt64
    public var modelId: String
    public var targets: [AgentTarget]
    public var mode: AiMode
    public var planFirst: Bool
    /**
     * 完全信任的时限（分钟），None 为不限
     */
    public var trustMinutes: UInt32?
    public var title: String?

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(providerId: UInt64, modelId: String, targets: [AgentTarget], mode: AiMode, planFirst: Bool, 
        /**
         * 完全信任的时限（分钟），None 为不限
         */trustMinutes: UInt32?, title: String?) {
        self.providerId = providerId
        self.modelId = modelId
        self.targets = targets
        self.mode = mode
        self.planFirst = planFirst
        self.trustMinutes = trustMinutes
        self.title = title
    }

    

    
}

#if compiler(>=6)
extension ConversationConfig: Sendable {}
#endif

public struct ConversationSummary: Equatable, Hashable {
    public var id: String
    public var title: String
    public var modelId: String
    public var targetNames: [String]
    public var updatedAt: Int64
    /**
     * 创建时的配置（模型、目标、档位）；老记录没有
     */
    public var config: ConversationConfig?

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(id: String, title: String, modelId: String, targetNames: [String], updatedAt: Int64, 
        /**
         * 创建时的配置（模型、目标、档位）；老记录没有
         */config: ConversationConfig?) {
        self.id = id
        self.title = title
        self.modelId = modelId
        self.targetNames = targetNames
        self.updatedAt = updatedAt
        self.config = config
    }

    

    
}

#if compiler(>=6)
extension ConversationSummary: Sendable {}
#endif

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
    /**
     * control 客户端是否在进程内跑本机被控（Mac 是；Windows 主控端由系统服务被控，设为 false）
     */
    public var hostEnabled: Bool

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(dataDir: String, cacheDir: String, logLevel: String, platform: Platform, deviceKind: DeviceKind, appVersion: String, osVersion: String, hostname: String, locale: String, 
        /**
         * control 客户端是否在进程内跑本机被控（Mac 是；Windows 主控端由系统服务被控，设为 false）
         */hostEnabled: Bool = true) {
        self.dataDir = dataDir
        self.cacheDir = cacheDir
        self.logLevel = logLevel
        self.platform = platform
        self.deviceKind = deviceKind
        self.appVersion = appVersion
        self.osVersion = osVersion
        self.hostname = hostname
        self.locale = locale
        self.hostEnabled = hostEnabled
    }

    

    
}

#if compiler(>=6)
extension CoreConfig: Sendable {}
#endif

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

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(id: String, kind: DeviceKind, platform: Platform, hostname: String, osVersion: String, arch: String, appVersion: String, online: Bool, lanSeen: Bool, lastSeenAt: Int64, isSelf: Bool) {
        self.id = id
        self.kind = kind
        self.platform = platform
        self.hostname = hostname
        self.osVersion = osVersion
        self.arch = arch
        self.appVersion = appVersion
        self.online = online
        self.lanSeen = lanSeen
        self.lastSeenAt = lastSeenAt
        self.isSelf = isSelf
    }

    

    
}

#if compiler(>=6)
extension DeviceInfo: Sendable {}
#endif

public struct DisplayInfo: Equatable, Hashable {
    public var id: UInt32
    public var name: String
    public var width: UInt32
    public var height: UInt32
    public var scale: Float
    public var primary: Bool
    public var isVirtual: Bool

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(id: UInt32, name: String, width: UInt32, height: UInt32, scale: Float, primary: Bool, isVirtual: Bool) {
        self.id = id
        self.name = name
        self.width = width
        self.height = height
        self.scale = scale
        self.primary = primary
        self.isVirtual = isVirtual
    }

    

    
}

#if compiler(>=6)
extension DisplayInfo: Sendable {}
#endif

public struct DisplayOptions: Equatable, Hashable {
    public var displayId: UInt32
    public var maxFps: UInt32
    public var bitrateKbps: UInt32
    public var codecs: [VideoCodec]
    public var viewOnly: Bool

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(displayId: UInt32, maxFps: UInt32, bitrateKbps: UInt32, codecs: [VideoCodec], viewOnly: Bool) {
        self.displayId = displayId
        self.maxFps = maxFps
        self.bitrateKbps = bitrateKbps
        self.codecs = codecs
        self.viewOnly = viewOnly
    }

    

    
}

#if compiler(>=6)
extension DisplayOptions: Sendable {}
#endif

public struct EnvVar: Equatable, Hashable {
    public var name: String
    public var value: String

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(name: String, value: String) {
        self.name = name
        self.value = value
    }

    

    
}

#if compiler(>=6)
extension EnvVar: Sendable {}
#endif

public struct ExecRequest: Equatable, Hashable {
    public var command: String
    public var shell: String?
    public var cwd: String?
    public var timeoutMs: UInt64
    public var maxOutput: UInt64

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(command: String, shell: String?, cwd: String?, timeoutMs: UInt64, maxOutput: UInt64) {
        self.command = command
        self.shell = shell
        self.cwd = cwd
        self.timeoutMs = timeoutMs
        self.maxOutput = maxOutput
    }

    

    
}

#if compiler(>=6)
extension ExecRequest: Sendable {}
#endif

public struct ExecResult: Equatable, Hashable {
    public var code: Int32?
    public var stdout: String
    public var stderr: String
    public var timedOut: Bool
    public var truncated: Bool
    public var durationMs: UInt64

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(code: Int32?, stdout: String, stderr: String, timedOut: Bool, truncated: Bool, durationMs: UInt64) {
        self.code = code
        self.stdout = stdout
        self.stderr = stderr
        self.timedOut = timedOut
        self.truncated = truncated
        self.durationMs = durationMs
    }

    

    
}

#if compiler(>=6)
extension ExecResult: Sendable {}
#endif

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

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(name: String, path: String, kind: FileKind, size: UInt64, mode: UInt32, modifiedAt: Int64, owner: String, group: String, linkTarget: String?) {
        self.name = name
        self.path = path
        self.kind = kind
        self.size = size
        self.mode = mode
        self.modifiedAt = modifiedAt
        self.owner = owner
        self.group = group
        self.linkTarget = linkTarget
    }

    

    
}

#if compiler(>=6)
extension FileEntry: Sendable {}
#endif

public struct HostAccount: Equatable, Hashable {
    public var userId: UInt64
    public var username: String
    public var displayName: String
    public var status: AccountStatus
    public var policy: HostPolicy

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(userId: UInt64, username: String, displayName: String, status: AccountStatus, policy: HostPolicy) {
        self.userId = userId
        self.username = username
        self.displayName = displayName
        self.status = status
        self.policy = policy
    }

    

    
}

#if compiler(>=6)
extension HostAccount: Sendable {}
#endif

public struct HostEvent: Equatable, Hashable {
    public var at: Int64
    public var session: String
    public var user: String
    /**
     * connect / disconnect / terminal / file / exec / display / forward / ai / reject / ai_denied
     */
    public var kind: String
    public var detail: String
    public var ai: Bool
    public var level: EventLevel

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(at: Int64, session: String, user: String, 
        /**
         * connect / disconnect / terminal / file / exec / display / forward / ai / reject / ai_denied
         */kind: String, detail: String, ai: Bool, level: EventLevel) {
        self.at = at
        self.session = session
        self.user = user
        self.kind = kind
        self.detail = detail
        self.ai = ai
        self.level = level
    }

    

    
}

#if compiler(>=6)
extension HostEvent: Sendable {}
#endif

public struct HostPolicy: Equatable, Hashable {
    public var systemUser: String
    public var view: Bool
    public var control: Bool
    public var files: Bool
    public var terminal: Bool
    public var clipboard: Bool
    public var audio: Bool
    public var aiCap: AiMode
    public var notify: Bool

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(systemUser: String, view: Bool, control: Bool, files: Bool, terminal: Bool, clipboard: Bool, audio: Bool, aiCap: AiMode, notify: Bool) {
        self.systemUser = systemUser
        self.view = view
        self.control = control
        self.files = files
        self.terminal = terminal
        self.clipboard = clipboard
        self.audio = audio
        self.aiCap = aiCap
        self.notify = notify
    }

    

    
}

#if compiler(>=6)
extension HostPolicy: Sendable {}
#endif

public struct HostSessionInfo: Equatable, Hashable {
    public var id: String
    public var fromUser: String
    public var fromDevice: String
    public var mode: ConnMode
    public var path: PathKind
    public var startedAt: Int64
    public var aiActive: Bool
    /**
     * 连进来的账号（临时协助时为空）
     */
    public var accountId: UInt64?

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(id: String, fromUser: String, fromDevice: String, mode: ConnMode, path: PathKind, startedAt: Int64, aiActive: Bool, 
        /**
         * 连进来的账号（临时协助时为空）
         */accountId: UInt64? = nil) {
        self.id = id
        self.fromUser = fromUser
        self.fromDevice = fromDevice
        self.mode = mode
        self.path = path
        self.startedAt = startedAt
        self.aiActive = aiActive
        self.accountId = accountId
    }

    

    
}

#if compiler(>=6)
extension HostSessionInfo: Sendable {}
#endif

public struct HostStatus: Equatable, Hashable {
    public var online: Bool
    public var remoteEnabled: Bool
    public var deviceId: String
    public var publicIp: String
    public var accounts: [HostAccount]
    public var sessions: [HostSessionInfo]
    public var assist: AssistInfo?

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(online: Bool, remoteEnabled: Bool, deviceId: String, publicIp: String, accounts: [HostAccount], sessions: [HostSessionInfo], assist: AssistInfo?) {
        self.online = online
        self.remoteEnabled = remoteEnabled
        self.deviceId = deviceId
        self.publicIp = publicIp
        self.accounts = accounts
        self.sessions = sessions
        self.assist = assist
    }

    

    
}

#if compiler(>=6)
extension HostStatus: Sendable {}
#endif

public struct ImageData: Equatable, Hashable {
    public var mime: String
    public var data: Data
    public var width: UInt32
    public var height: UInt32

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(mime: String, data: Data, width: UInt32, height: UInt32) {
        self.mime = mime
        self.data = data
        self.width = width
        self.height = height
    }

    

    
}

#if compiler(>=6)
extension ImageData: Sendable {}
#endif

public struct PathInfo: Equatable, Hashable {
    public var kind: PathKind
    public var rttMs: UInt32
    public var remoteAddr: String

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(kind: PathKind, rttMs: UInt32, remoteAddr: String) {
        self.kind = kind
        self.rttMs = rttMs
        self.remoteAddr = remoteAddr
    }

    

    
}

#if compiler(>=6)
extension PathInfo: Sendable {}
#endif

public struct PeerInfo: Equatable, Hashable {
    public var deviceId: String
    public var hostname: String
    public var platform: Platform
    public var osVersion: String
    public var arch: String
    public var systemUser: String
    public var shell: String
    public var permissions: Permissions
    public var capabilities: [String]
    public var path: PathInfo

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(deviceId: String, hostname: String, platform: Platform, osVersion: String, arch: String, systemUser: String, shell: String, permissions: Permissions, capabilities: [String], path: PathInfo) {
        self.deviceId = deviceId
        self.hostname = hostname
        self.platform = platform
        self.osVersion = osVersion
        self.arch = arch
        self.systemUser = systemUser
        self.shell = shell
        self.permissions = permissions
        self.capabilities = capabilities
        self.path = path
    }

    

    
}

#if compiler(>=6)
extension PeerInfo: Sendable {}
#endif

public struct Permissions: Equatable, Hashable {
    public var view: Bool
    public var control: Bool
    public var files: Bool
    public var terminal: Bool
    public var clipboard: Bool
    public var audio: Bool
    public var aiCap: AiMode

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(view: Bool, control: Bool, files: Bool, terminal: Bool, clipboard: Bool, audio: Bool, aiCap: AiMode) {
        self.view = view
        self.control = control
        self.files = files
        self.terminal = terminal
        self.clipboard = clipboard
        self.audio = audio
        self.aiCap = aiCap
    }

    

    
}

#if compiler(>=6)
extension Permissions: Sendable {}
#endif

public struct RdpTarget: Equatable, Hashable {
    public var label: String
    /**
     * 远程桌面客户端要连的地址与端口
     */
    public var host: String
    public var port: UInt16
    public var username: String
    public var domain: String?
    /**
     * 加密库里保存的密码（界面决定是否交给系统客户端，如 Windows 的 cmdkey）
     */
    public var password: String?
    /**
     * 经设备转发时为真；用完调 close_rdp 关掉转发
     */
    public var forwarded: Bool
    /**
     * 用于 close_rdp
     */
    public var forwardId: String

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(label: String, 
        /**
         * 远程桌面客户端要连的地址与端口
         */host: String, port: UInt16, username: String, domain: String?, 
        /**
         * 加密库里保存的密码（界面决定是否交给系统客户端，如 Windows 的 cmdkey）
         */password: String?, 
        /**
         * 经设备转发时为真；用完调 close_rdp 关掉转发
         */forwarded: Bool, 
        /**
         * 用于 close_rdp
         */forwardId: String) {
        self.label = label
        self.host = host
        self.port = port
        self.username = username
        self.domain = domain
        self.password = password
        self.forwarded = forwarded
        self.forwardId = forwardId
    }

    

    
}

#if compiler(>=6)
extension RdpTarget: Sendable {}
#endif

public struct RecordingInfo: Equatable, Hashable {
    /**
     * 写出的文件（录像中途被控端分辨率变了会分成几个文件）
     */
    public var paths: [String]
    public var durationMs: UInt64
    public var bytes: UInt64

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(
        /**
         * 写出的文件（录像中途被控端分辨率变了会分成几个文件）
         */paths: [String], durationMs: UInt64, bytes: UInt64) {
        self.paths = paths
        self.durationMs = durationMs
        self.bytes = bytes
    }

    

    
}

#if compiler(>=6)
extension RecordingInfo: Sendable {}
#endif

public struct ServerInfo: Equatable, Hashable {
    public var address: String
    public var name: String
    public var version: String

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(address: String, name: String, version: String) {
        self.address = address
        self.name = name
        self.version = version
    }

    

    
}

#if compiler(>=6)
extension ServerInfo: Sendable {}
#endif

public struct SshInfo: Equatable, Hashable {
    /**
     * 加密库条目 id
     */
    public var hostId: String
    public var label: String
    public var host: String
    public var port: UInt16
    public var username: String
    /**
     * 怎么连过去的：direct / device:<设备名> / ssh:<跳转主机名>
     */
    public var via: String

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(
        /**
         * 加密库条目 id
         */hostId: String, label: String, host: String, port: UInt16, username: String, 
        /**
         * 怎么连过去的：direct / device:<设备名> / ssh:<跳转主机名>
         */via: String) {
        self.hostId = hostId
        self.label = label
        self.host = host
        self.port = port
        self.username = username
        self.via = via
    }

    

    
}

#if compiler(>=6)
extension SshInfo: Sendable {}
#endif

public struct TermLine: Equatable, Hashable {
    public var spans: [TermSpan]
    /**
     * 这一行是上一行自动折行过来的（复制时不加换行）
     */
    public var wrapped: Bool

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(spans: [TermSpan], 
        /**
         * 这一行是上一行自动折行过来的（复制时不加换行）
         */wrapped: Bool) {
        self.spans = spans
        self.wrapped = wrapped
    }

    

    
}

#if compiler(>=6)
extension TermLine: Sendable {}
#endif

public struct TermScreenState: Equatable, Hashable {
    public var rows: UInt16
    public var cols: UInt16
    public var lines: [TermLine]
    public var cursorRow: UInt16
    public var cursorCol: UInt16
    /**
     * 程序隐藏了光标，或正在往上翻看历史
     */
    public var cursorVisible: Bool
    public var title: String
    public var alternateScreen: Bool
    public var applicationCursor: Bool
    public var applicationKeypad: Bool
    public var bracketedPaste: Bool
    public var mouseMode: TermMouseMode
    /**
     * 鼠标上报用 SGR 编码
     */
    public var mouseSgr: Bool
    /**
     * 当前往上翻看的行数（0 为最底部）
     */
    public var scrollOffset: UInt32
    /**
     * 历史里一共有多少行
     */
    public var scrollbackLen: UInt32
    /**
     * 响铃计数（变化时界面提示一下）
     */
    public var bellCount: UInt32

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(rows: UInt16, cols: UInt16, lines: [TermLine], cursorRow: UInt16, cursorCol: UInt16, 
        /**
         * 程序隐藏了光标，或正在往上翻看历史
         */cursorVisible: Bool, title: String, alternateScreen: Bool, applicationCursor: Bool, applicationKeypad: Bool, bracketedPaste: Bool, mouseMode: TermMouseMode, 
        /**
         * 鼠标上报用 SGR 编码
         */mouseSgr: Bool, 
        /**
         * 当前往上翻看的行数（0 为最底部）
         */scrollOffset: UInt32, 
        /**
         * 历史里一共有多少行
         */scrollbackLen: UInt32, 
        /**
         * 响铃计数（变化时界面提示一下）
         */bellCount: UInt32) {
        self.rows = rows
        self.cols = cols
        self.lines = lines
        self.cursorRow = cursorRow
        self.cursorCol = cursorCol
        self.cursorVisible = cursorVisible
        self.title = title
        self.alternateScreen = alternateScreen
        self.applicationCursor = applicationCursor
        self.applicationKeypad = applicationKeypad
        self.bracketedPaste = bracketedPaste
        self.mouseMode = mouseMode
        self.mouseSgr = mouseSgr
        self.scrollOffset = scrollOffset
        self.scrollbackLen = scrollbackLen
        self.bellCount = bellCount
    }

    

    
}

#if compiler(>=6)
extension TermScreenState: Sendable {}
#endif

public struct TermSpan: Equatable, Hashable {
    public var text: String
    public var cells: UInt16
    public var fg: UInt32
    public var bg: UInt32
    /**
     * 1 粗体、2 斜体、4 下划线、8 反色
     */
    public var flags: UInt8

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(text: String, cells: UInt16, fg: UInt32, bg: UInt32, 
        /**
         * 1 粗体、2 斜体、4 下划线、8 反色
         */flags: UInt8) {
        self.text = text
        self.cells = cells
        self.fg = fg
        self.bg = bg
        self.flags = flags
    }

    

    
}

#if compiler(>=6)
extension TermSpan: Sendable {}
#endif

public struct TerminalOptions: Equatable, Hashable {
    public var attachSession: String?
    public var shell: String?
    public var cols: UInt16
    public var rows: UInt16
    public var env: [EnvVar]

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(attachSession: String?, shell: String?, cols: UInt16, rows: UInt16, env: [EnvVar]) {
        self.attachSession = attachSession
        self.shell = shell
        self.cols = cols
        self.rows = rows
        self.env = env
    }

    

    
}

#if compiler(>=6)
extension TerminalOptions: Sendable {}
#endif

public struct TerminalSessionInfo: Equatable, Hashable {
    public var id: String
    public var title: String
    public var createdAt: Int64
    public var attached: Bool

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(id: String, title: String, createdAt: Int64, attached: Bool) {
        self.id = id
        self.title = title
        self.createdAt = createdAt
        self.attached = attached
    }

    

    
}

#if compiler(>=6)
extension TerminalSessionInfo: Sendable {}
#endif

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

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(callId: String, tool: String, targetId: String, targetName: String, summary: String, detail: String, risk: RiskLevel, riskReasons: [String], modelReason: String, image: ImageData?) {
        self.callId = callId
        self.tool = tool
        self.targetId = targetId
        self.targetName = targetName
        self.summary = summary
        self.detail = detail
        self.risk = risk
        self.riskReasons = riskReasons
        self.modelReason = modelReason
        self.image = image
    }

    

    
}

#if compiler(>=6)
extension ToolCallView: Sendable {}
#endif

public struct ToolResultView: Equatable, Hashable {
    public var ok: Bool
    public var summary: String
    public var output: String
    public var image: ImageData?
    public var denied: Bool

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(ok: Bool, summary: String, output: String, image: ImageData?, denied: Bool) {
        self.ok = ok
        self.summary = summary
        self.output = output
        self.image = image
        self.denied = denied
    }

    

    
}

#if compiler(>=6)
extension ToolResultView: Sendable {}
#endif

public struct TotpSetup: Equatable, Hashable {
    public var secret: String
    public var otpauthUri: String

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(secret: String, otpauthUri: String) {
        self.secret = secret
        self.otpauthUri = otpauthUri
    }

    

    
}

#if compiler(>=6)
extension TotpSetup: Sendable {}
#endif

public struct UpdateInfo: Equatable, Hashable {
    public var version: String
    public var fileName: String
    public var size: UInt64
    public var sha256: String
    /**
     * 服务器上的下载路径（交给 download_update）
     */
    public var url: String
    public var notes: String

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(version: String, fileName: String, size: UInt64, sha256: String, 
        /**
         * 服务器上的下载路径（交给 download_update）
         */url: String, notes: String) {
        self.version = version
        self.fileName = fileName
        self.size = size
        self.sha256 = sha256
        self.url = url
        self.notes = notes
    }

    

    
}

#if compiler(>=6)
extension UpdateInfo: Sendable {}
#endif

public struct UserInfo: Equatable, Hashable {
    public var id: UInt64
    public var username: String
    public var displayName: String
    public var aiEnabled: Bool
    public var totpEnabled: Bool

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(id: UInt64, username: String, displayName: String, aiEnabled: Bool, totpEnabled: Bool) {
        self.id = id
        self.username = username
        self.displayName = displayName
        self.aiEnabled = aiEnabled
        self.totpEnabled = totpEnabled
    }

    

    
}

#if compiler(>=6)
extension UserInfo: Sendable {}
#endif

public struct VaultEntry: Equatable, Hashable {
    public var id: String
    public var folderId: String?
    public var sort: Int64
    public var data: VaultData
    public var updatedAt: Int64

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(id: String, folderId: String?, sort: Int64, data: VaultData, updatedAt: Int64) {
        self.id = id
        self.folderId = folderId
        self.sort = sort
        self.data = data
        self.updatedAt = updatedAt
    }

    

    
}

#if compiler(>=6)
extension VaultEntry: Sendable {}
#endif

public enum AccountStatus: Equatable, Hashable {
    
    case active
    case needsRelogin





}

#if compiler(>=6)
extension AccountStatus: Sendable {}
#endif

public enum AgentEvent: Equatable, Hashable {
    
    case assistantDelta(messageId: String, text: String
    )
    case assistantDone(messageId: String
    )
    case thinking(messageId: String, text: String
    )
    case toolProposed(call: ToolCallView, needsApproval: Bool
    )
    case toolStarted(callId: String
    )
    case toolOutput(callId: String, text: String
    )
    case toolFinished(callId: String, result: ToolResultView
    )
    case askUser(questionId: String, question: String, options: [String]
    )
    case secretRequested(requestId: String, prompt: String, targetId: String
    )
    case usage(promptTokens: UInt64, completionTokens: UInt64
    )
    case status(running: Bool, step: UInt32
    )
    case modeChanged(mode: AiMode, effective: AiMode, untilMs: Int64?
    )
    case error(message: String
    )





}

#if compiler(>=6)
extension AgentEvent: Sendable {}
#endif

public enum AgentMessage: Equatable, Hashable {
    
    case user(text: String
    )
    case assistant(id: String, text: String, thinking: String
    )
    case tool(call: ToolCallView, result: ToolResultView?
    )





}

#if compiler(>=6)
extension AgentMessage: Sendable {}
#endif

public enum AgentTargetKind: Equatable, Hashable {
    
    case device
    case sshHost





}

#if compiler(>=6)
extension AgentTargetKind: Sendable {}
#endif

public enum AiMode: Equatable, Hashable {
    
    case off
    case readOnly
    case confirmAll
    case confirmDangerous
    case fullTrust





}

#if compiler(>=6)
extension AiMode: Sendable {}
#endif

public enum ApprovalDecision: Equatable, Hashable {
    
    case allowOnce
    case allowSimilar
    case deny(reason: String?
    )
    case editAndRun(command: String
    )





}

#if compiler(>=6)
extension ApprovalDecision: Sendable {}
#endif

public enum ClientState: Equatable, Hashable {
    
    case loggedOut
    case connecting
    case online
    case offline





}

#if compiler(>=6)
extension ClientState: Sendable {}
#endif

public enum ClipboardData: Equatable, Hashable {
    
    case text(text: String
    )
    case image(image: ImageData
    )
    case files(paths: [String]
    )





}

#if compiler(>=6)
extension ClipboardData: Sendable {}
#endif

public enum ComboKey: Equatable, Hashable {
    
    case ctrlAltDel
    case lockScreen
    case altTab
    case winKey
    case taskManager
    case showDesktop





}

#if compiler(>=6)
extension ComboKey: Sendable {}
#endif

public enum ConnMode: Equatable, Hashable {
    
    case control
    case assist





}

#if compiler(>=6)
extension ConnMode: Sendable {}
#endif

public enum DeviceKind: Equatable, Hashable {
    
    case control
    case worker
    case mobile





}

#if compiler(>=6)
extension DeviceKind: Sendable {}
#endif

public enum EventLevel: Equatable, Hashable {
    
    case info
    /**
     * 被拒绝的连接、被拦下的 AI 操作等，界面应醒目标出
     */
    case warning





}

#if compiler(>=6)
extension EventLevel: Sendable {}
#endif

public enum FileKind: Equatable, Hashable {
    
    case file
    case dir
    case symlink
    case other





}

#if compiler(>=6)
extension FileKind: Sendable {}
#endif

public enum GalaxyError: Swift.Error, Equatable, Hashable, Foundation.LocalizedError {

    
    
    case Network(detail: String
    )
    case Auth(code: String, detail: String
    )
    case Permission(detail: String
    )
    case NotFound(detail: String
    )
    case Protocol(detail: String
    )
    case Cancelled
    case Other(detail: String
    )

    

    

    
    public var errorDescription: String? {
        String(reflecting: self)
    }
    
}

#if compiler(>=6)
extension GalaxyError: Sendable {}
#endif

public enum InputEvent: Equatable, Hashable {
    
    case mouseMove(x: Int32, y: Int32
    )
    case mouseButton(button: PointerButton, down: Bool, x: Int32, y: Int32
    )
    case wheel(dx: Int32, dy: Int32
    )
    case key(hidUsage: UInt32, down: Bool
    )
    case text(text: String
    )
    case touch(id: UInt32, phase: TouchPhase, x: Int32, y: Int32
    )
    case specialCombo(combo: ComboKey
    )





}

#if compiler(>=6)
extension InputEvent: Sendable {}
#endif

public enum LoginResult: Equatable, Hashable {
    
    case success
    case needTotp
    case mustChangePassword





}

#if compiler(>=6)
extension LoginResult: Sendable {}
#endif

public enum PathKind: Equatable, Hashable {
    
    case connecting
    case lan
    case direct
    case relayUdp
    case relayTcp
    case closed





}

#if compiler(>=6)
extension PathKind: Sendable {}
#endif

public enum Platform: Equatable, Hashable {
    
    case macos
    case windows
    case linux
    case android
    case ios
    case harmonyos





}

#if compiler(>=6)
extension Platform: Sendable {}
#endif

public enum PointerButton: Equatable, Hashable {
    
    case left
    case right
    case middle
    case back
    case forward





}

#if compiler(>=6)
extension PointerButton: Sendable {}
#endif

public enum RiskLevel: Equatable, Hashable {
    
    case read
    case normal
    case dangerous
    case unknown
    case forbidden





}

#if compiler(>=6)
extension RiskLevel: Sendable {}
#endif

public enum SshAuth: Equatable, Hashable {
    
    case password(password: String?
    )
    case privateKey(credentialId: String
    )
    case agent
    case interactive





}

#if compiler(>=6)
extension SshAuth: Sendable {}
#endif

public enum TermMouseMode: Equatable, Hashable {
    
    case none
    case press
    case pressRelease
    case buttonMotion
    case anyMotion





}

#if compiler(>=6)
extension TermMouseMode: Sendable {}
#endif

public enum TouchPhase: Equatable, Hashable {
    
    case began
    case moved
    case ended
    case cancelled





}

#if compiler(>=6)
extension TouchPhase: Sendable {}
#endif

public enum VaultData: Equatable, Hashable {
    
    case folder(name: String, color: String?, expanded: Bool
    )
    case deviceMeta(deviceId: String, alias: String?, tags: [String], note: String, favorite: Bool
    )
    case sshHost(label: String, host: String, port: UInt16, username: String, auth: SshAuth, jumpDeviceId: String?, jumpHostId: String?, tags: [String], note: String, favorite: Bool, terminalTheme: String?
    )
    case rdpHost(label: String, host: String, port: UInt16, username: String, password: String?, domain: String?, jumpDeviceId: String?, tags: [String], note: String, favorite: Bool
    )
    case credential(label: String, username: String, password: String?, privateKey: String?, passphrase: String?
    )
    case snippet(label: String, command: String, tags: [String]
    )
    case knownHost(host: String, port: UInt16, keyType: String, fingerprint: String
    )
    case preferences(json: String
    )
    /**
     * 端口映射规则（经由设备或 SSH 主机，恰好一个有值；local_port 为 0 时自动选）
     */
    case portForward(label: String, viaDeviceId: String?, viaSshHostId: String?, localPort: UInt16, remoteHost: String, remotePort: UInt16, autoStart: Bool, note: String
    )





}

#if compiler(>=6)
extension VaultData: Sendable {}
#endif

public enum VideoCodec: Equatable, Hashable {
    
    case h264
    case h265





}

#if compiler(>=6)
extension VideoCodec: Sendable {}
#endif

public enum WorkerLoginResult: Equatable, Hashable {
    
    case success(userId: UInt64, username: String
    )
    /**
     * 需要两步验证码：带上 totp 再调一次
     */
    case needTotp
    /**
     * 首次登录或被管理员重置：带上 new_password 再调一次
     */
    case mustChangePassword





}

#if compiler(>=6)
extension WorkerLoginResult: Sendable {}
#endif

// MARK: - 协议（对象与回调）

public protocol AgentConversationProtocol: AnyObject, Sendable {
    
    func answer(questionId: String, answer: String) 
    
    func decide(callId: String, decision: ApprovalDecision) 
    
    func id()  -> String
    
    func messages()  -> [AgentMessage]
    
    /**
     * 回应 request_secret：内容直接送到被控端，不进模型上下文。
     */
    func provideSecret(requestId: String, secret: String) 
    
    /**
     * 发送一条消息；任务在内核后台运行，进度经监听器回调。
     */
    func send(text: String) 
    
    /**
     * 移动端切到后台时调用 false：正在进行的任务停止，回到前台后可继续发送。
     */
    func setForeground(foreground: Bool) 
    
    func setMode(mode: AiMode, trustMinutes: UInt32?) 
    
    func stop() 
    
    func title()  -> String
    
}

public protocol AgentListener: AnyObject, Sendable {
    
    func onEvent(event: AgentEvent) 
    
}

public protocol AudioPcmSinkProtocol: AnyObject, Sendable {
    
    /**
     * `pcm` 是交错排列的 32 位浮点样本（小端），采样率与声道数随系统。
     */
    func push(pcm: Data, sampleRate: UInt32, channels: UInt32) 
    
}

public protocol AudioPlaybackProtocol: AnyObject, Sendable {
    
    func isActive()  -> Bool
    
    func setMuted(muted: Bool) 
    
    func setVolume(volume: Float) 
    
    /**
     * 停止收听并关闭输出设备。
     */
    func stop() 
    
}

public protocol ClientListener: AnyObject, Sendable {
    
    func onState(state: ClientState) 
    
    func onDevicesChanged() 
    
    func onVaultChanged() 
    
    func onSessionRevoked() 
    
}

public protocol ClipboardTransferProtocol: AnyObject, Sendable {
    
    func cancel() 
    
    /**
     * 完成后的路径：推送时是被控端上的路径，拉取时是本机路径。
     */
    func paths()  -> [String]
    
}

public protocol DisplayChannelProtocol: AnyObject, Sendable {
    
    func isRecording()  -> Bool
    
    func requestKeyframe() 
    
    func sendInput(event: InputEvent) 
    
    func setBitrate(kbps: UInt32) 
    
    /**
     * 主控端在本地画光标（跟手）：被控端画面里不再画光标，位置变化经 VideoSink.on_cursor 告知。
     * 只在对端能力列表有 "client_cursor" 时调用。
     */
    func setClientCursor(on: Bool) 
    
    /**
     * 开始录像到 `path`（.mp4，分片 MP4，H.264 原样写入不重新编码）。`with_audio` 为真时正在收听的声音也一起录。
     * 从下一个关键帧开始写（会马上请求一个）。
     */
    func startRecording(path: String, withAudio: Bool) async throws 
    
    func stop() 
    
    /**
     * 停止录像。没在录时报错。
     */
    func stopRecording() throws  -> RecordingInfo
    
}

public protocol EncodedFrameSinkProtocol: AnyObject, Sendable {
    
    func push(data: Data, ptsUs: UInt64, keyframe: Bool) 
    
    /**
     * 光标位置（这块屏的画面像素）变了，位置变化时调用即可（约 30 次/秒以内）。
     */
    func setCursor(x: Int32, y: Int32, visible: Bool) 
    
    func setResolution(width: UInt32, height: UInt32) 
    
}

public protocol FileClientProtocol: AnyObject, Sendable {
    
    func chmod(path: String, mode: UInt32) async throws 
    
    /**
     * 下载到本地文件（本地已有部分则续传）。
     */
    func download(remote: String, local: String, listener: TransferListener)  -> Transfer
    
    func home() async throws  -> String
    
    func list(path: String) async throws  -> [FileEntry]
    
    func mkdir(path: String) async throws 
    
    func readText(path: String, maxBytes: UInt64) async throws  -> String
    
    func realpath(path: String) async throws  -> String
    
    func remove(path: String, recursive: Bool) async throws 
    
    func rename(from: String, to: String) async throws 
    
    func search(root: String, pattern: String, content: Bool) async throws  -> [FileEntry]
    
    func stat(path: String) async throws  -> FileEntry
    
    /**
     * 上传本地文件（远端已有部分则续传）。
     */
    func upload(local: String, remote: String, listener: TransferListener)  -> Transfer
    
    func writeText(path: String, text: String) async throws 
    
}

public protocol GalaxyClientProtocol: AnyObject, Sendable {
    
    func aiModels() async throws  -> AiModels
    
    /**
     * 能不能远程开机：设备离线、能被控、上报过网卡地址（界面据此显示「远程开机」）。
     */
    func canWake(deviceId: String)  -> Bool
    
    func changePassword(oldPassword: String, newPassword: String, totp: String?) async throws 
    
    /**
     * 向服务器查询新版本。`component`：control / mobile / worker；`platform`：macos / android / windows / ios；
     * `arch` 为空表示不限；`current` 是本机版本。返回前已验发布签名。
     */
    func checkUpdate(component: String, platform: String, arch: String, current: String) async throws  -> UpdateInfo?
    
    /**
     * 关掉 open_rdp 开的端口转发。
     */
    func closeRdp(forwardId: String) 
    
    /**
     * 首次登录设置新密码（会生成账号密钥）。
     */
    func completeFirstLogin(newPassword: String) async throws 
    
    func connect(deviceId: String) async throws  -> PeerSession
    
    func connectAssist(deviceCode: String, verificationCode: String) async throws  -> PeerSession
    
    /**
     * 连接加密库里的 ssh_host 条目。需要密码、口令或确认主机密钥时经 `prompt` 问用户。
     */
    func connectSsh(hostId: String, prompt: SecretPrompt) async throws  -> SshSession
    
    func currentUser()  -> UserInfo?
    
    func deleteConversation(id: String) 
    
    func deviceId()  -> String
    
    func devices()  -> [DeviceInfo]
    
    /**
     * 下载新版本到 `dest`（本地文件路径），核对 SHA-256。进度经 listener 的 on_progress 上报，完成或失败经 on_finished。
     */
    func downloadUpdate(info: UpdateInfo, dest: String, listener: TransferListener) 
    
    /**
     * control 客户端的本机被控服务；登录完成并上线后才有。
     */
    func host()  -> HostService?
    
    func listConversations()  -> [ConversationSummary]
    
    /**
     * 登录。server 为后台给出的完整地址 "host:port#sha256-指纹"。
     */
    func login(server: String, username: String, password: String, totp: String?) async throws  -> LoginResult
    
    func logout() async throws 
    
    /**
     * 新建对话：先连上所有目标设备。
     */
    func newConversation(config: ConversationConfig, listener: AgentListener) async throws  -> AgentConversation
    
    /**
     * 打开本机保存的对话（需要重新连接目标设备）。`config` 为空时用创建时保存的配置。
     */
    func openConversation(id: String, config: ConversationConfig?, listener: AgentListener) async throws  -> AgentConversation
    
    /**
     * 准备连接加密库里的 rdp_host：设了经由设备时在本机开端口转发穿过那台设备。
     * 界面拿返回的地址交给系统的远程桌面客户端（Windows 的 mstsc，其他平台的微软 Windows App）。
     */
    func openRdp(hostId: String) async throws  -> RdpTarget
    
    func refreshDevices() async throws 
    
    func removeDevice(deviceId: String) async throws 
    
    /**
     * 启动时用本机保存的会话恢复登录（不需要联网就能显示缓存的设备与主机）。
     */
    func restore()  -> Bool
    
    func server()  -> ServerInfo?
    
    /**
     * 设备列表页打开期间设为 true：每隔几秒查询局域网内的同账号设备。
     */
    func setLanDiscovery(active: Bool) 
    
    /**
     * 按加密库里的端口映射规则开启映射：经由设备时连上该设备转发，经由 SSH 主机时连上该主机做本地转发。
     * 返回的 PortForward 一直有效直到 stop()（期间保持与设备 / 主机的连接）。
     */
    func startPortForward(entryId: String, prompt: SecretPrompt) async throws  -> PortForward
    
    func state()  -> ClientState
    
    func totpDisable(code: String) async throws 
    
    func totpSetup() async throws  -> TotpSetup
    
    func totpVerify(code: String) async throws 
    
    func vault()  -> [VaultEntry]
    
    func vaultDelete(id: String) async throws 
    
    /**
     * 保存加密库条目（id 为空则新建），返回保存后的条目。
     */
    func vaultSave(entry: VaultEntry) async throws  -> VaultEntry
    
    /**
     * 远程开机：由本机（与目标同一网络时）或同一网络里在线的设备代发唤醒包。
     * 返回代发设备的名字，空字符串表示本机。目标要在主板 / 网卡设置里开启网络唤醒。
     */
    func wakeDevice(deviceId: String) async throws  -> [String]
    
}

public protocol HostListener: AnyObject, Sendable {
    
    func onStatus(status: HostStatus) 
    
    /**
     * 临时协助请求：界面弹窗后调用 HostService.respond_assist
     */
    func onAssistRequest(requestId: String, fromUser: String, fromDevice: String) 
    
    func onSessionStarted(info: HostSessionInfo) 
    
    func onSessionEnded(sessionId: String) 
    
    func onAiActivity(sessionId: String, description: String) 
    
}

public protocol HostPlatform: AnyObject, Sendable {
    
    func listDisplays()  -> [DisplayInfo]
    
    /**
     * 开始截屏编码，编码后的帧推给 `sink`；返回实际分辨率与编码，失败返回 None。
     */
    func startCapture(displayId: UInt32, options: CaptureOptions, sink: EncodedFrameSink)  -> CaptureStarted?
    
    func stopCapture(displayId: UInt32) 
    
    func requestKeyframe(displayId: UInt32) 
    
    func setBitrate(displayId: UInt32, kbps: UInt32) 
    
    /**
     * 画面里要不要画光标（主控端在本地画时关掉）；支持时在 capabilities 里报 "client_cursor"。
     */
    func setCursorInVideo(displayId: UInt32, show: Bool) 
    
    /**
     * 坐标是 `display_id` 这块屏幕的像素。
     */
    func inject(displayId: UInt32, event: InputEvent) 
    
    func screenshot(displayId: UInt32, maxWidth: UInt32, maxHeight: UInt32)  -> ImageData?
    
    func uiTree(displayId: UInt32, maxNodes: UInt32)  -> String?
    
    func clipboardGet()  -> ClipboardData?
    
    func clipboardSet(data: ClipboardData) 
    
    /**
     * 隐私屏：本机屏幕黑掉、本地键鼠失效，远端照常看到和操作。返回是否做到了（不支持返回 false）。
     */
    func setPrivacy(on: Bool)  -> Bool
    
    /**
     * 开始采集系统声音，样本推给 `sink`；不支持或没授权返回 false。同一时间只有一路。
     */
    func startAudio(sink: AudioPcmSink)  -> Bool
    
    func stopAudio() 
    
    /**
     * 已获得的系统授权对应的能力："display" "screenshot" "input" "ui_tree" "clipboard" "privacy" "audio"
     */
    func capabilities()  -> [String]
    
}

public protocol HostServiceProtocol: AnyObject, Sendable {
    
    func accounts()  -> [HostAccount]
    
    func disconnectAll() 
    
    /**
     * 断开一个会话（status().sessions 里的 id）。
     */
    func disconnectSession(id: String)  -> Bool
    
    func lockOnEnd()  -> Bool
    
    func recentEvents(limit: UInt32)  -> [HostEvent]
    
    func respondAssist(requestId: String, allow: Bool) 
    
    /**
     * 自定义验证码（8 到 16 位字母数字），None 为每次随机。
     */
    func setAssistCode(code: String?) 
    
    func setListener(listener: HostListener) 
    
    /**
     * 远程看过桌面的会话都结束后自动锁屏（默认关）。
     */
    func setLockOnEnd(on: Bool) 
    
    func setPlatform(platform: HostPlatform) 
    
    func setRemoteEnabled(enabled: Bool) 
    
    /**
     * 开启临时协助，等到服务器分配设备码后返回。
     */
    func startAssist() async throws  -> AssistInfo
    
    func status()  -> HostStatus
    
    func stopAi() 
    
    func stopAssist() 
    
    /**
     * 修改某账号在本机的权限（只能在本机修改）。
     */
    func updateAccountPolicy(userId: UInt64, policy: HostPolicy) 
    
}

public protocol PeerListener: AnyObject, Sendable {
    
    func onPathChanged(path: PathInfo) 
    
    func onClosed(reason: String) 
    
}

public protocol PeerSessionProtocol: AnyObject, Sendable {
    
    /**
     * 在被控端加一块虚拟屏（扩展桌面），返回它的屏幕信息（is_virtual 为真），之后用它的 id 调 open_display。
     * `refresh` 为 0 时用 60 Hz。虚拟屏属于本会话，断开时被控端自动删掉。需要控制权限。
     */
    func addVirtualDisplay(width: UInt32, height: UInt32, refresh: UInt32) async throws  -> DisplayInfo
    
    func clipboardGet() async throws  -> ClipboardData?
    
    /**
     * 复制粘贴文件（远端 → 本机）：把远端剪贴板里的文件（clipboard_get 得到的路径）下载到 `local_dir`。
     * 完成后 ClipboardTransfer.paths() 是本机上的顶层路径，界面把本机剪贴板设成它们。
     */
    func clipboardPullFiles(remotePaths: [String], localDir: String, listener: TransferListener)  -> ClipboardTransfer
    
    /**
     * 复制粘贴文件（本机 → 远端）：把本机文件 / 文件夹传到被控端暂存区，再把被控端剪贴板设成它们。
     * 进度与结果经 listener；超过 4 GB 或 1 万个文件时报错（请改用文件管理）。需要剪贴板与文件权限。
     */
    func clipboardPushFiles(localPaths: [String], listener: TransferListener)  -> ClipboardTransfer
    
    func clipboardSet(data: ClipboardData) async throws 
    
    /**
     * 断开会话（不叫 close，避免与绑定自动生成的释放方法冲突）。
     */
    func disconnect() 
    
    func exec(request: ExecRequest) async throws  -> ExecResult
    
    func files()  -> FileClient
    
    func forwardPort(localPort: UInt16, remoteHost: String, remotePort: UInt16) async throws  -> PortForward
    
    func info()  -> PeerInfo
    
    func isClosed()  -> Bool
    
    func killTerminal(sessionId: String) async throws 
    
    func listDisplays() async throws  -> [DisplayInfo]
    
    func listTerminals() async throws  -> [TerminalSessionInfo]
    
    func openDisplay(options: DisplayOptions, sink: VideoSink) async throws  -> DisplayChannel
    
    func openTerminal(options: TerminalOptions, sink: TerminalSink) async throws  -> TerminalChannel
    
    func path()  -> PathInfo
    
    /**
     * 删掉本会话加的虚拟屏。
     */
    func removeVirtualDisplay(displayId: UInt32) async throws 
    
    func screenshot(displayId: UInt32, maxWidth: UInt32) async throws  -> ImageData
    
    func setListener(listener: PeerListener) 
    
    /**
     * 隐私屏：被控端本地屏幕黑掉、本地键鼠失效，远端照常操作；本会话结束时自动关闭。能力列表里有 "privacy" 才支持。
     */
    func setPrivacy(on: Bool) async throws 
    
    /**
     * 开始收听被控端的系统声音，用本机默认输出设备播放。能力列表里有 "audio" 才支持。`volume` 0..1。
     */
    func startAudio(volume: Float) async throws  -> AudioPlayback
    
    /**
     * 主控最多能在被控端加几块虚拟屏（能力列表 "virtual_display" + "virtual_display_max=N"）；不支持或没有控制权限时为 0。
     */
    func virtualDisplayMax()  -> UInt32
    
    func wakeOnLan(mac: String) async throws 
    
}

public protocol PortForwardProtocol: AnyObject, Sendable {
    
    func localPort()  -> UInt16
    
    func stop() 
    
}

public protocol SecretPrompt: AnyObject, Sendable {
    
    /**
     * 密码、私钥口令、键盘交互的回答；返回 None 表示取消。`echo` 为假时输入框应遮挡。
     */
    func ask(prompt: String, echo: Bool)  -> String?
    
    /**
     * 首次连接或主机密钥变化时确认。`changed` 为真表示与记录的不一致（可能被冒充），界面应醒目警告。
     */
    func confirmHostKey(host: String, port: UInt16, keyType: String, fingerprint: String, changed: Bool)  -> Bool
    
}

public protocol SecureStore: AnyObject, Sendable {
    
    func get(key: String)  -> Data?
    
    func set(key: String, value: Data) 
    
    func remove(key: String) 
    
}

public protocol SshSessionProtocol: AnyObject, Sendable {
    
    func disconnect() 
    
    func exec(request: ExecRequest) async throws  -> ExecResult
    
    /**
     * 文件（SFTP）。
     */
    func files()  -> FileClient
    
    /**
     * 本地端口转发（在本机 127.0.0.1 监听，经 SSH 连到 remote_host:remote_port；local_port 为 0 时自动选）。
     */
    func forwardPort(localPort: UInt16, remoteHost: String, remotePort: UInt16) async throws  -> PortForward
    
    func info()  -> SshInfo
    
    func isClosed()  -> Bool
    
    /**
     * 打开终端。SSH 没有断线继承：detach 与 kill 都会结束这个终端。
     */
    func openTerminal(options: TerminalOptions, sink: TerminalSink) async throws  -> TerminalChannel
    
}

public protocol TerminalChannelProtocol: AnyObject, Sendable {
    
    /**
     * 断开但被控端保留会话
     */
    func detach() 
    
    func kill() 
    
    func resize(cols: UInt16, rows: UInt16) 
    
    func sessionId()  -> String
    
    func title()  -> String
    
    func write(data: Data) 
    
}

public protocol TerminalScreenProtocol: AnyObject, Sendable {
    
    /**
     * 喂入被控端终端的输出。
     */
    func feed(data: Data) 
    
    func resize(rows: UInt16, cols: UInt16) 
    
    /**
     * 取屏幕内容。`scroll_offset` 为往上翻看的行数，0 为最底部（超出历史长度时按最大值算）。
     */
    func state(scrollOffset: UInt32)  -> TermScreenState
    
    /**
     * 可见区域全部文字。
     */
    func text()  -> String
    
    /**
     * 可见区域里两点之间的文字（选中复制用），行列从 0 开始，含终点。
     */
    func textBetween(startRow: UInt16, startCol: UInt16, endRow: UInt16, endCol: UInt16)  -> String
    
}

public protocol TerminalSink: AnyObject, Sendable {
    
    func onOutput(data: Data) 
    
    func onExit(code: Int32?) 
    
}

public protocol TransferProtocol: AnyObject, Sendable {
    
    func cancel() 
    
    func pause() 
    
    func resume() 
    
}

public protocol TransferListener: AnyObject, Sendable {
    
    func onProgress(done: UInt64, total: UInt64) 
    
    func onFinished(error: String?) 
    
}

public protocol VideoSink: AnyObject, Sendable {
    
    func onConfig(codec: VideoCodec, width: UInt32, height: UInt32) 
    
    /**
     * Annex B 码流（关键帧前带参数集）
     */
    func onFrame(data: Data, ptsUs: UInt64, keyframe: Bool) 
    
    /**
     * 光标位置，被控屏幕像素
     */
    func onCursor(x: Int32, y: Int32, visible: Bool) 
    
    func onClosed(reason: String) 
    
}

public protocol WorkerHostProtocol: AnyObject, Sendable {
    
    /**
     * 检查被控端新版本（用任一有效账号的登录令牌访问服务器）。返回前已验发布签名。
     */
    func checkUpdate(platform: String, current: String) async throws  -> UpdateInfo?
    
    func deviceId()  -> String
    
    /**
     * 下载新版本到本地文件并核对 SHA-256，进度与结果经 listener。
     */
    func downloadUpdate(info: UpdateInfo, dest: String, listener: TransferListener) 
    
    /**
     * 登录一个账号。`system_user` 为空时用当前系统用户。所有账号必须在同一台服务器上。
     */
    func login(server: String, username: String, password: String, totp: String?, newPassword: String?, systemUser: String?) async throws  -> WorkerLoginResult
    
    /**
     * 退出某个账号（服务器上也作废这台设备的登录）。
     */
    func logout(userId: UInt64) async throws 
    
    /**
     * 已登录账号所在的服务器地址；还没登录过为空。
     */
    func server()  -> String?
    
    func service()  -> HostService?
    
    /**
     * 还没登录账号时先设服务器地址（全新安装也能开临时协助）。已有账号时只能是同一台服务器。
     */
    func setServer(server: String) async throws  -> HostService?
    
    /**
     * 启动被控服务（有账号时）。应用启动时调一次；之后的状态从返回的 HostService 读。
     */
    func start() async throws  -> HostService?
    
}

// MARK: - 对象（占位）

open class AgentConversation: AgentConversationProtocol, @unchecked Sendable {
    public struct NoHandle { public init() {} }
    public init(noHandle: NoHandle) {}
    required public init(unsafeFromHandle handle: UInt64) {}
    open func answer(questionId: String, answer: String) { bridgeUnimplemented() }
    open func decide(callId: String, decision: ApprovalDecision) { bridgeUnimplemented() }
    open func id()  -> String { bridgeUnimplemented() }
    open func messages()  -> [AgentMessage] { bridgeUnimplemented() }
    open func provideSecret(requestId: String, secret: String) { bridgeUnimplemented() }
    open func send(text: String) { bridgeUnimplemented() }
    open func setForeground(foreground: Bool) { bridgeUnimplemented() }
    open func setMode(mode: AiMode, trustMinutes: UInt32?) { bridgeUnimplemented() }
    open func stop() { bridgeUnimplemented() }
    open func title()  -> String { bridgeUnimplemented() }
}

open class AudioPcmSink: AudioPcmSinkProtocol, @unchecked Sendable {
    public struct NoHandle { public init() {} }
    public init(noHandle: NoHandle) {}
    required public init(unsafeFromHandle handle: UInt64) {}
    open func push(pcm: Data, sampleRate: UInt32, channels: UInt32) { bridgeUnimplemented() }
}

open class AudioPlayback: AudioPlaybackProtocol, @unchecked Sendable {
    public struct NoHandle { public init() {} }
    public init(noHandle: NoHandle) {}
    required public init(unsafeFromHandle handle: UInt64) {}
    open func isActive()  -> Bool { bridgeUnimplemented() }
    open func setMuted(muted: Bool) { bridgeUnimplemented() }
    open func setVolume(volume: Float) { bridgeUnimplemented() }
    open func stop() { bridgeUnimplemented() }
}

open class ClipboardTransfer: ClipboardTransferProtocol, @unchecked Sendable {
    public struct NoHandle { public init() {} }
    public init(noHandle: NoHandle) {}
    required public init(unsafeFromHandle handle: UInt64) {}
    open func cancel() { bridgeUnimplemented() }
    open func paths()  -> [String] { bridgeUnimplemented() }
}

open class DisplayChannel: DisplayChannelProtocol, @unchecked Sendable {
    public struct NoHandle { public init() {} }
    public init(noHandle: NoHandle) {}
    required public init(unsafeFromHandle handle: UInt64) {}
    open func isRecording()  -> Bool { bridgeUnimplemented() }
    open func requestKeyframe() { bridgeUnimplemented() }
    open func sendInput(event: InputEvent) { bridgeUnimplemented() }
    open func setBitrate(kbps: UInt32) { bridgeUnimplemented() }
    open func setClientCursor(on: Bool) { bridgeUnimplemented() }
    open func startRecording(path: String, withAudio: Bool) async throws { bridgeUnimplemented() }
    open func stop() { bridgeUnimplemented() }
    open func stopRecording() throws  -> RecordingInfo { bridgeUnimplemented() }
}

open class EncodedFrameSink: EncodedFrameSinkProtocol, @unchecked Sendable {
    public struct NoHandle { public init() {} }
    public init(noHandle: NoHandle) {}
    required public init(unsafeFromHandle handle: UInt64) {}
    open func push(data: Data, ptsUs: UInt64, keyframe: Bool) { bridgeUnimplemented() }
    open func setCursor(x: Int32, y: Int32, visible: Bool) { bridgeUnimplemented() }
    open func setResolution(width: UInt32, height: UInt32) { bridgeUnimplemented() }
}

open class FileClient: FileClientProtocol, @unchecked Sendable {
    public struct NoHandle { public init() {} }
    public init(noHandle: NoHandle) {}
    required public init(unsafeFromHandle handle: UInt64) {}
    open func chmod(path: String, mode: UInt32) async throws { bridgeUnimplemented() }
    open func download(remote: String, local: String, listener: TransferListener)  -> Transfer { bridgeUnimplemented() }
    open func home() async throws  -> String { bridgeUnimplemented() }
    open func list(path: String) async throws  -> [FileEntry] { bridgeUnimplemented() }
    open func mkdir(path: String) async throws { bridgeUnimplemented() }
    open func readText(path: String, maxBytes: UInt64) async throws  -> String { bridgeUnimplemented() }
    open func realpath(path: String) async throws  -> String { bridgeUnimplemented() }
    open func remove(path: String, recursive: Bool) async throws { bridgeUnimplemented() }
    open func rename(from: String, to: String) async throws { bridgeUnimplemented() }
    open func search(root: String, pattern: String, content: Bool) async throws  -> [FileEntry] { bridgeUnimplemented() }
    open func stat(path: String) async throws  -> FileEntry { bridgeUnimplemented() }
    open func upload(local: String, remote: String, listener: TransferListener)  -> Transfer { bridgeUnimplemented() }
    open func writeText(path: String, text: String) async throws { bridgeUnimplemented() }
}

open class GalaxyClient: GalaxyClientProtocol, @unchecked Sendable {
    public struct NoHandle { public init() {} }
    public init(noHandle: NoHandle) {}
    required public init(unsafeFromHandle handle: UInt64) {}
    open func aiModels() async throws  -> AiModels { bridgeUnimplemented() }
    open func canWake(deviceId: String)  -> Bool { bridgeUnimplemented() }
    open func changePassword(oldPassword: String, newPassword: String, totp: String?) async throws { bridgeUnimplemented() }
    open func checkUpdate(component: String, platform: String, arch: String, current: String) async throws  -> UpdateInfo? { bridgeUnimplemented() }
    open func closeRdp(forwardId: String) { bridgeUnimplemented() }
    open func completeFirstLogin(newPassword: String) async throws { bridgeUnimplemented() }
    open func connect(deviceId: String) async throws  -> PeerSession { bridgeUnimplemented() }
    open func connectAssist(deviceCode: String, verificationCode: String) async throws  -> PeerSession { bridgeUnimplemented() }
    open func connectSsh(hostId: String, prompt: SecretPrompt) async throws  -> SshSession { bridgeUnimplemented() }
    open func currentUser()  -> UserInfo? { bridgeUnimplemented() }
    open func deleteConversation(id: String) { bridgeUnimplemented() }
    open func deviceId()  -> String { bridgeUnimplemented() }
    open func devices()  -> [DeviceInfo] { bridgeUnimplemented() }
    open func downloadUpdate(info: UpdateInfo, dest: String, listener: TransferListener) { bridgeUnimplemented() }
    open func host()  -> HostService? { bridgeUnimplemented() }
    open func listConversations()  -> [ConversationSummary] { bridgeUnimplemented() }
    open func login(server: String, username: String, password: String, totp: String?) async throws  -> LoginResult { bridgeUnimplemented() }
    open func logout() async throws { bridgeUnimplemented() }
    open func newConversation(config: ConversationConfig, listener: AgentListener) async throws  -> AgentConversation { bridgeUnimplemented() }
    open func openConversation(id: String, config: ConversationConfig?, listener: AgentListener) async throws  -> AgentConversation { bridgeUnimplemented() }
    open func openRdp(hostId: String) async throws  -> RdpTarget { bridgeUnimplemented() }
    open func refreshDevices() async throws { bridgeUnimplemented() }
    open func removeDevice(deviceId: String) async throws { bridgeUnimplemented() }
    open func restore()  -> Bool { bridgeUnimplemented() }
    open func server()  -> ServerInfo? { bridgeUnimplemented() }
    open func setLanDiscovery(active: Bool) { bridgeUnimplemented() }
    open func startPortForward(entryId: String, prompt: SecretPrompt) async throws  -> PortForward { bridgeUnimplemented() }
    open func state()  -> ClientState { bridgeUnimplemented() }
    open func totpDisable(code: String) async throws { bridgeUnimplemented() }
    open func totpSetup() async throws  -> TotpSetup { bridgeUnimplemented() }
    open func totpVerify(code: String) async throws { bridgeUnimplemented() }
    open func vault()  -> [VaultEntry] { bridgeUnimplemented() }
    open func vaultDelete(id: String) async throws { bridgeUnimplemented() }
    open func vaultSave(entry: VaultEntry) async throws  -> VaultEntry { bridgeUnimplemented() }
    open func wakeDevice(deviceId: String) async throws  -> [String] { bridgeUnimplemented() }
}

open class HostService: HostServiceProtocol, @unchecked Sendable {
    public struct NoHandle { public init() {} }
    public init(noHandle: NoHandle) {}
    required public init(unsafeFromHandle handle: UInt64) {}
    open func accounts()  -> [HostAccount] { bridgeUnimplemented() }
    open func disconnectAll() { bridgeUnimplemented() }
    open func disconnectSession(id: String)  -> Bool { bridgeUnimplemented() }
    open func lockOnEnd()  -> Bool { bridgeUnimplemented() }
    open func recentEvents(limit: UInt32)  -> [HostEvent] { bridgeUnimplemented() }
    open func respondAssist(requestId: String, allow: Bool) { bridgeUnimplemented() }
    open func setAssistCode(code: String?) { bridgeUnimplemented() }
    open func setListener(listener: HostListener) { bridgeUnimplemented() }
    open func setLockOnEnd(on: Bool) { bridgeUnimplemented() }
    open func setPlatform(platform: HostPlatform) { bridgeUnimplemented() }
    open func setRemoteEnabled(enabled: Bool) { bridgeUnimplemented() }
    open func startAssist() async throws  -> AssistInfo { bridgeUnimplemented() }
    open func status()  -> HostStatus { bridgeUnimplemented() }
    open func stopAi() { bridgeUnimplemented() }
    open func stopAssist() { bridgeUnimplemented() }
    open func updateAccountPolicy(userId: UInt64, policy: HostPolicy) { bridgeUnimplemented() }
}

open class PeerSession: PeerSessionProtocol, @unchecked Sendable {
    public struct NoHandle { public init() {} }
    public init(noHandle: NoHandle) {}
    required public init(unsafeFromHandle handle: UInt64) {}
    open func addVirtualDisplay(width: UInt32, height: UInt32, refresh: UInt32) async throws  -> DisplayInfo { bridgeUnimplemented() }
    open func clipboardGet() async throws  -> ClipboardData? { bridgeUnimplemented() }
    open func clipboardPullFiles(remotePaths: [String], localDir: String, listener: TransferListener)  -> ClipboardTransfer { bridgeUnimplemented() }
    open func clipboardPushFiles(localPaths: [String], listener: TransferListener)  -> ClipboardTransfer { bridgeUnimplemented() }
    open func clipboardSet(data: ClipboardData) async throws { bridgeUnimplemented() }
    open func disconnect() { bridgeUnimplemented() }
    open func exec(request: ExecRequest) async throws  -> ExecResult { bridgeUnimplemented() }
    open func files()  -> FileClient { bridgeUnimplemented() }
    open func forwardPort(localPort: UInt16, remoteHost: String, remotePort: UInt16) async throws  -> PortForward { bridgeUnimplemented() }
    open func info()  -> PeerInfo { bridgeUnimplemented() }
    open func isClosed()  -> Bool { bridgeUnimplemented() }
    open func killTerminal(sessionId: String) async throws { bridgeUnimplemented() }
    open func listDisplays() async throws  -> [DisplayInfo] { bridgeUnimplemented() }
    open func listTerminals() async throws  -> [TerminalSessionInfo] { bridgeUnimplemented() }
    open func openDisplay(options: DisplayOptions, sink: VideoSink) async throws  -> DisplayChannel { bridgeUnimplemented() }
    open func openTerminal(options: TerminalOptions, sink: TerminalSink) async throws  -> TerminalChannel { bridgeUnimplemented() }
    open func path()  -> PathInfo { bridgeUnimplemented() }
    open func removeVirtualDisplay(displayId: UInt32) async throws { bridgeUnimplemented() }
    open func screenshot(displayId: UInt32, maxWidth: UInt32) async throws  -> ImageData { bridgeUnimplemented() }
    open func setListener(listener: PeerListener) { bridgeUnimplemented() }
    open func setPrivacy(on: Bool) async throws { bridgeUnimplemented() }
    open func startAudio(volume: Float) async throws  -> AudioPlayback { bridgeUnimplemented() }
    open func virtualDisplayMax()  -> UInt32 { bridgeUnimplemented() }
    open func wakeOnLan(mac: String) async throws { bridgeUnimplemented() }
}

open class PortForward: PortForwardProtocol, @unchecked Sendable {
    public struct NoHandle { public init() {} }
    public init(noHandle: NoHandle) {}
    required public init(unsafeFromHandle handle: UInt64) {}
    open func localPort()  -> UInt16 { bridgeUnimplemented() }
    open func stop() { bridgeUnimplemented() }
}

open class SshSession: SshSessionProtocol, @unchecked Sendable {
    public struct NoHandle { public init() {} }
    public init(noHandle: NoHandle) {}
    required public init(unsafeFromHandle handle: UInt64) {}
    open func disconnect() { bridgeUnimplemented() }
    open func exec(request: ExecRequest) async throws  -> ExecResult { bridgeUnimplemented() }
    open func files()  -> FileClient { bridgeUnimplemented() }
    open func forwardPort(localPort: UInt16, remoteHost: String, remotePort: UInt16) async throws  -> PortForward { bridgeUnimplemented() }
    open func info()  -> SshInfo { bridgeUnimplemented() }
    open func isClosed()  -> Bool { bridgeUnimplemented() }
    open func openTerminal(options: TerminalOptions, sink: TerminalSink) async throws  -> TerminalChannel { bridgeUnimplemented() }
}

open class TerminalChannel: TerminalChannelProtocol, @unchecked Sendable {
    public struct NoHandle { public init() {} }
    public init(noHandle: NoHandle) {}
    required public init(unsafeFromHandle handle: UInt64) {}
    open func detach() { bridgeUnimplemented() }
    open func kill() { bridgeUnimplemented() }
    open func resize(cols: UInt16, rows: UInt16) { bridgeUnimplemented() }
    open func sessionId()  -> String { bridgeUnimplemented() }
    open func title()  -> String { bridgeUnimplemented() }
    open func write(data: Data) { bridgeUnimplemented() }
}

open class TerminalScreen: TerminalScreenProtocol, @unchecked Sendable {
    public struct NoHandle { public init() {} }
    public init(noHandle: NoHandle) {}
    required public init(unsafeFromHandle handle: UInt64) {}
    open func feed(data: Data) { bridgeUnimplemented() }
    open func resize(rows: UInt16, cols: UInt16) { bridgeUnimplemented() }
    open func state(scrollOffset: UInt32)  -> TermScreenState { bridgeUnimplemented() }
    open func text()  -> String { bridgeUnimplemented() }
    open func textBetween(startRow: UInt16, startCol: UInt16, endRow: UInt16, endCol: UInt16)  -> String { bridgeUnimplemented() }
}

open class Transfer: TransferProtocol, @unchecked Sendable {
    public struct NoHandle { public init() {} }
    public init(noHandle: NoHandle) {}
    required public init(unsafeFromHandle handle: UInt64) {}
    open func cancel() { bridgeUnimplemented() }
    open func pause() { bridgeUnimplemented() }
    open func resume() { bridgeUnimplemented() }
}

open class WorkerHost: WorkerHostProtocol, @unchecked Sendable {
    public struct NoHandle { public init() {} }
    public init(noHandle: NoHandle) {}
    required public init(unsafeFromHandle handle: UInt64) {}
    open func checkUpdate(platform: String, current: String) async throws  -> UpdateInfo? { bridgeUnimplemented() }
    open func deviceId()  -> String { bridgeUnimplemented() }
    open func downloadUpdate(info: UpdateInfo, dest: String, listener: TransferListener) { bridgeUnimplemented() }
    open func login(server: String, username: String, password: String, totp: String?, newPassword: String?, systemUser: String?) async throws  -> WorkerLoginResult { bridgeUnimplemented() }
    open func logout(userId: UInt64) async throws { bridgeUnimplemented() }
    open func server()  -> String? { bridgeUnimplemented() }
    open func service()  -> HostService? { bridgeUnimplemented() }
    open func setServer(server: String) async throws  -> HostService? { bridgeUnimplemented() }
    open func start() async throws  -> HostService? { bridgeUnimplemented() }
}
#endif
