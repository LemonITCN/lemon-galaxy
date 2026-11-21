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

// MARK: - 记录与枚举

public struct AgentTarget {
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

extension AgentTarget: Equatable, Hashable {
    public static func ==(lhs: AgentTarget, rhs: AgentTarget) -> Bool {
        if lhs.kind != rhs.kind {
            return false
        }
        if lhs.id != rhs.id {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(kind)
        hasher.combine(id)
    }
}

public struct AiModel {
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

extension AiModel: Equatable, Hashable {
    public static func ==(lhs: AiModel, rhs: AiModel) -> Bool {
        if lhs.providerId != rhs.providerId {
            return false
        }
        if lhs.providerName != rhs.providerName {
            return false
        }
        if lhs.id != rhs.id {
            return false
        }
        if lhs.name != rhs.name {
            return false
        }
        if lhs.vision != rhs.vision {
            return false
        }
        if lhs.tools != rhs.tools {
            return false
        }
        if lhs.contextLength != rhs.contextLength {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(providerId)
        hasher.combine(providerName)
        hasher.combine(id)
        hasher.combine(name)
        hasher.combine(vision)
        hasher.combine(tools)
        hasher.combine(contextLength)
    }
}

public struct AiModels {
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

extension AiModels: Equatable, Hashable {
    public static func ==(lhs: AiModels, rhs: AiModels) -> Bool {
        if lhs.enabled != rhs.enabled {
            return false
        }
        if lhs.maxMode != rhs.maxMode {
            return false
        }
        if lhs.items != rhs.items {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(enabled)
        hasher.combine(maxMode)
        hasher.combine(items)
    }
}

public struct AssistInfo {
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

extension AssistInfo: Equatable, Hashable {
    public static func ==(lhs: AssistInfo, rhs: AssistInfo) -> Bool {
        if lhs.deviceCode != rhs.deviceCode {
            return false
        }
        if lhs.code != rhs.code {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(deviceCode)
        hasher.combine(code)
    }
}

public struct CaptureOptions {
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

extension CaptureOptions: Equatable, Hashable {
    public static func ==(lhs: CaptureOptions, rhs: CaptureOptions) -> Bool {
        if lhs.maxFps != rhs.maxFps {
            return false
        }
        if lhs.bitrateKbps != rhs.bitrateKbps {
            return false
        }
        if lhs.codecs != rhs.codecs {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(maxFps)
        hasher.combine(bitrateKbps)
        hasher.combine(codecs)
    }
}

public struct CaptureStarted {
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

extension CaptureStarted: Equatable, Hashable {
    public static func ==(lhs: CaptureStarted, rhs: CaptureStarted) -> Bool {
        if lhs.width != rhs.width {
            return false
        }
        if lhs.height != rhs.height {
            return false
        }
        if lhs.codec != rhs.codec {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(width)
        hasher.combine(height)
        hasher.combine(codec)
    }
}

public struct ConversationConfig {
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

extension ConversationConfig: Equatable, Hashable {
    public static func ==(lhs: ConversationConfig, rhs: ConversationConfig) -> Bool {
        if lhs.providerId != rhs.providerId {
            return false
        }
        if lhs.modelId != rhs.modelId {
            return false
        }
        if lhs.targets != rhs.targets {
            return false
        }
        if lhs.mode != rhs.mode {
            return false
        }
        if lhs.planFirst != rhs.planFirst {
            return false
        }
        if lhs.trustMinutes != rhs.trustMinutes {
            return false
        }
        if lhs.title != rhs.title {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(providerId)
        hasher.combine(modelId)
        hasher.combine(targets)
        hasher.combine(mode)
        hasher.combine(planFirst)
        hasher.combine(trustMinutes)
        hasher.combine(title)
    }
}

public struct ConversationSummary {
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

extension ConversationSummary: Equatable, Hashable {
    public static func ==(lhs: ConversationSummary, rhs: ConversationSummary) -> Bool {
        if lhs.id != rhs.id {
            return false
        }
        if lhs.title != rhs.title {
            return false
        }
        if lhs.modelId != rhs.modelId {
            return false
        }
        if lhs.targetNames != rhs.targetNames {
            return false
        }
        if lhs.updatedAt != rhs.updatedAt {
            return false
        }
        if lhs.config != rhs.config {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(id)
        hasher.combine(title)
        hasher.combine(modelId)
        hasher.combine(targetNames)
        hasher.combine(updatedAt)
        hasher.combine(config)
    }
}

public struct CoreConfig {
    public var dataDir: String
    public var cacheDir: String
    public var logLevel: String
    public var platform: Platform
    public var deviceKind: DeviceKind
    public var appVersion: String
    public var osVersion: String
    public var hostname: String
    public var locale: String

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(dataDir: String, cacheDir: String, logLevel: String, platform: Platform, deviceKind: DeviceKind, appVersion: String, osVersion: String, hostname: String, locale: String) {
        self.dataDir = dataDir
        self.cacheDir = cacheDir
        self.logLevel = logLevel
        self.platform = platform
        self.deviceKind = deviceKind
        self.appVersion = appVersion
        self.osVersion = osVersion
        self.hostname = hostname
        self.locale = locale
    }
}

#if compiler(>=6)
extension CoreConfig: Sendable {}
#endif

extension CoreConfig: Equatable, Hashable {
    public static func ==(lhs: CoreConfig, rhs: CoreConfig) -> Bool {
        if lhs.dataDir != rhs.dataDir {
            return false
        }
        if lhs.cacheDir != rhs.cacheDir {
            return false
        }
        if lhs.logLevel != rhs.logLevel {
            return false
        }
        if lhs.platform != rhs.platform {
            return false
        }
        if lhs.deviceKind != rhs.deviceKind {
            return false
        }
        if lhs.appVersion != rhs.appVersion {
            return false
        }
        if lhs.osVersion != rhs.osVersion {
            return false
        }
        if lhs.hostname != rhs.hostname {
            return false
        }
        if lhs.locale != rhs.locale {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(dataDir)
        hasher.combine(cacheDir)
        hasher.combine(logLevel)
        hasher.combine(platform)
        hasher.combine(deviceKind)
        hasher.combine(appVersion)
        hasher.combine(osVersion)
        hasher.combine(hostname)
        hasher.combine(locale)
    }
}

public struct DeviceInfo {
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

extension DeviceInfo: Equatable, Hashable {
    public static func ==(lhs: DeviceInfo, rhs: DeviceInfo) -> Bool {
        if lhs.id != rhs.id {
            return false
        }
        if lhs.kind != rhs.kind {
            return false
        }
        if lhs.platform != rhs.platform {
            return false
        }
        if lhs.hostname != rhs.hostname {
            return false
        }
        if lhs.osVersion != rhs.osVersion {
            return false
        }
        if lhs.arch != rhs.arch {
            return false
        }
        if lhs.appVersion != rhs.appVersion {
            return false
        }
        if lhs.online != rhs.online {
            return false
        }
        if lhs.lanSeen != rhs.lanSeen {
            return false
        }
        if lhs.lastSeenAt != rhs.lastSeenAt {
            return false
        }
        if lhs.isSelf != rhs.isSelf {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(id)
        hasher.combine(kind)
        hasher.combine(platform)
        hasher.combine(hostname)
        hasher.combine(osVersion)
        hasher.combine(arch)
        hasher.combine(appVersion)
        hasher.combine(online)
        hasher.combine(lanSeen)
        hasher.combine(lastSeenAt)
        hasher.combine(isSelf)
    }
}

public struct DisplayInfo {
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

extension DisplayInfo: Equatable, Hashable {
    public static func ==(lhs: DisplayInfo, rhs: DisplayInfo) -> Bool {
        if lhs.id != rhs.id {
            return false
        }
        if lhs.name != rhs.name {
            return false
        }
        if lhs.width != rhs.width {
            return false
        }
        if lhs.height != rhs.height {
            return false
        }
        if lhs.scale != rhs.scale {
            return false
        }
        if lhs.primary != rhs.primary {
            return false
        }
        if lhs.isVirtual != rhs.isVirtual {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(id)
        hasher.combine(name)
        hasher.combine(width)
        hasher.combine(height)
        hasher.combine(scale)
        hasher.combine(primary)
        hasher.combine(isVirtual)
    }
}

public struct DisplayOptions {
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

extension DisplayOptions: Equatable, Hashable {
    public static func ==(lhs: DisplayOptions, rhs: DisplayOptions) -> Bool {
        if lhs.displayId != rhs.displayId {
            return false
        }
        if lhs.maxFps != rhs.maxFps {
            return false
        }
        if lhs.bitrateKbps != rhs.bitrateKbps {
            return false
        }
        if lhs.codecs != rhs.codecs {
            return false
        }
        if lhs.viewOnly != rhs.viewOnly {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(displayId)
        hasher.combine(maxFps)
        hasher.combine(bitrateKbps)
        hasher.combine(codecs)
        hasher.combine(viewOnly)
    }
}

public struct EnvVar {
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

extension EnvVar: Equatable, Hashable {
    public static func ==(lhs: EnvVar, rhs: EnvVar) -> Bool {
        if lhs.name != rhs.name {
            return false
        }
        if lhs.value != rhs.value {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(name)
        hasher.combine(value)
    }
}

public struct ExecRequest {
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

extension ExecRequest: Equatable, Hashable {
    public static func ==(lhs: ExecRequest, rhs: ExecRequest) -> Bool {
        if lhs.command != rhs.command {
            return false
        }
        if lhs.shell != rhs.shell {
            return false
        }
        if lhs.cwd != rhs.cwd {
            return false
        }
        if lhs.timeoutMs != rhs.timeoutMs {
            return false
        }
        if lhs.maxOutput != rhs.maxOutput {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(command)
        hasher.combine(shell)
        hasher.combine(cwd)
        hasher.combine(timeoutMs)
        hasher.combine(maxOutput)
    }
}

public struct ExecResult {
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

extension ExecResult: Equatable, Hashable {
    public static func ==(lhs: ExecResult, rhs: ExecResult) -> Bool {
        if lhs.code != rhs.code {
            return false
        }
        if lhs.stdout != rhs.stdout {
            return false
        }
        if lhs.stderr != rhs.stderr {
            return false
        }
        if lhs.timedOut != rhs.timedOut {
            return false
        }
        if lhs.truncated != rhs.truncated {
            return false
        }
        if lhs.durationMs != rhs.durationMs {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(code)
        hasher.combine(stdout)
        hasher.combine(stderr)
        hasher.combine(timedOut)
        hasher.combine(truncated)
        hasher.combine(durationMs)
    }
}

public struct FileEntry {
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

extension FileEntry: Equatable, Hashable {
    public static func ==(lhs: FileEntry, rhs: FileEntry) -> Bool {
        if lhs.name != rhs.name {
            return false
        }
        if lhs.path != rhs.path {
            return false
        }
        if lhs.kind != rhs.kind {
            return false
        }
        if lhs.size != rhs.size {
            return false
        }
        if lhs.mode != rhs.mode {
            return false
        }
        if lhs.modifiedAt != rhs.modifiedAt {
            return false
        }
        if lhs.owner != rhs.owner {
            return false
        }
        if lhs.group != rhs.group {
            return false
        }
        if lhs.linkTarget != rhs.linkTarget {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(name)
        hasher.combine(path)
        hasher.combine(kind)
        hasher.combine(size)
        hasher.combine(mode)
        hasher.combine(modifiedAt)
        hasher.combine(owner)
        hasher.combine(group)
        hasher.combine(linkTarget)
    }
}

public struct HostAccount {
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

extension HostAccount: Equatable, Hashable {
    public static func ==(lhs: HostAccount, rhs: HostAccount) -> Bool {
        if lhs.userId != rhs.userId {
            return false
        }
        if lhs.username != rhs.username {
            return false
        }
        if lhs.displayName != rhs.displayName {
            return false
        }
        if lhs.status != rhs.status {
            return false
        }
        if lhs.policy != rhs.policy {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(userId)
        hasher.combine(username)
        hasher.combine(displayName)
        hasher.combine(status)
        hasher.combine(policy)
    }
}

public struct HostEvent {
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

extension HostEvent: Equatable, Hashable {
    public static func ==(lhs: HostEvent, rhs: HostEvent) -> Bool {
        if lhs.at != rhs.at {
            return false
        }
        if lhs.session != rhs.session {
            return false
        }
        if lhs.user != rhs.user {
            return false
        }
        if lhs.kind != rhs.kind {
            return false
        }
        if lhs.detail != rhs.detail {
            return false
        }
        if lhs.ai != rhs.ai {
            return false
        }
        if lhs.level != rhs.level {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(at)
        hasher.combine(session)
        hasher.combine(user)
        hasher.combine(kind)
        hasher.combine(detail)
        hasher.combine(ai)
        hasher.combine(level)
    }
}

public struct HostPolicy {
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

extension HostPolicy: Equatable, Hashable {
    public static func ==(lhs: HostPolicy, rhs: HostPolicy) -> Bool {
        if lhs.systemUser != rhs.systemUser {
            return false
        }
        if lhs.view != rhs.view {
            return false
        }
        if lhs.control != rhs.control {
            return false
        }
        if lhs.files != rhs.files {
            return false
        }
        if lhs.terminal != rhs.terminal {
            return false
        }
        if lhs.clipboard != rhs.clipboard {
            return false
        }
        if lhs.audio != rhs.audio {
            return false
        }
        if lhs.aiCap != rhs.aiCap {
            return false
        }
        if lhs.notify != rhs.notify {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(systemUser)
        hasher.combine(view)
        hasher.combine(control)
        hasher.combine(files)
        hasher.combine(terminal)
        hasher.combine(clipboard)
        hasher.combine(audio)
        hasher.combine(aiCap)
        hasher.combine(notify)
    }
}

public struct HostSessionInfo {
    public var id: String
    public var fromUser: String
    public var fromDevice: String
    public var mode: ConnMode
    public var path: PathKind
    public var startedAt: Int64
    public var aiActive: Bool

    // Default memberwise initializers are never public by default, so we
    // declare one manually.
    public init(id: String, fromUser: String, fromDevice: String, mode: ConnMode, path: PathKind, startedAt: Int64, aiActive: Bool) {
        self.id = id
        self.fromUser = fromUser
        self.fromDevice = fromDevice
        self.mode = mode
        self.path = path
        self.startedAt = startedAt
        self.aiActive = aiActive
    }
}

#if compiler(>=6)
extension HostSessionInfo: Sendable {}
#endif

extension HostSessionInfo: Equatable, Hashable {
    public static func ==(lhs: HostSessionInfo, rhs: HostSessionInfo) -> Bool {
        if lhs.id != rhs.id {
            return false
        }
        if lhs.fromUser != rhs.fromUser {
            return false
        }
        if lhs.fromDevice != rhs.fromDevice {
            return false
        }
        if lhs.mode != rhs.mode {
            return false
        }
        if lhs.path != rhs.path {
            return false
        }
        if lhs.startedAt != rhs.startedAt {
            return false
        }
        if lhs.aiActive != rhs.aiActive {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(id)
        hasher.combine(fromUser)
        hasher.combine(fromDevice)
        hasher.combine(mode)
        hasher.combine(path)
        hasher.combine(startedAt)
        hasher.combine(aiActive)
    }
}

public struct HostStatus {
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

extension HostStatus: Equatable, Hashable {
    public static func ==(lhs: HostStatus, rhs: HostStatus) -> Bool {
        if lhs.online != rhs.online {
            return false
        }
        if lhs.remoteEnabled != rhs.remoteEnabled {
            return false
        }
        if lhs.deviceId != rhs.deviceId {
            return false
        }
        if lhs.publicIp != rhs.publicIp {
            return false
        }
        if lhs.accounts != rhs.accounts {
            return false
        }
        if lhs.sessions != rhs.sessions {
            return false
        }
        if lhs.assist != rhs.assist {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(online)
        hasher.combine(remoteEnabled)
        hasher.combine(deviceId)
        hasher.combine(publicIp)
        hasher.combine(accounts)
        hasher.combine(sessions)
        hasher.combine(assist)
    }
}

public struct ImageData {
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

extension ImageData: Equatable, Hashable {
    public static func ==(lhs: ImageData, rhs: ImageData) -> Bool {
        if lhs.mime != rhs.mime {
            return false
        }
        if lhs.data != rhs.data {
            return false
        }
        if lhs.width != rhs.width {
            return false
        }
        if lhs.height != rhs.height {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(mime)
        hasher.combine(data)
        hasher.combine(width)
        hasher.combine(height)
    }
}

public struct PathInfo {
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

extension PathInfo: Equatable, Hashable {
    public static func ==(lhs: PathInfo, rhs: PathInfo) -> Bool {
        if lhs.kind != rhs.kind {
            return false
        }
        if lhs.rttMs != rhs.rttMs {
            return false
        }
        if lhs.remoteAddr != rhs.remoteAddr {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(kind)
        hasher.combine(rttMs)
        hasher.combine(remoteAddr)
    }
}

public struct PeerInfo {
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

extension PeerInfo: Equatable, Hashable {
    public static func ==(lhs: PeerInfo, rhs: PeerInfo) -> Bool {
        if lhs.deviceId != rhs.deviceId {
            return false
        }
        if lhs.hostname != rhs.hostname {
            return false
        }
        if lhs.platform != rhs.platform {
            return false
        }
        if lhs.osVersion != rhs.osVersion {
            return false
        }
        if lhs.arch != rhs.arch {
            return false
        }
        if lhs.systemUser != rhs.systemUser {
            return false
        }
        if lhs.shell != rhs.shell {
            return false
        }
        if lhs.permissions != rhs.permissions {
            return false
        }
        if lhs.capabilities != rhs.capabilities {
            return false
        }
        if lhs.path != rhs.path {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(deviceId)
        hasher.combine(hostname)
        hasher.combine(platform)
        hasher.combine(osVersion)
        hasher.combine(arch)
        hasher.combine(systemUser)
        hasher.combine(shell)
        hasher.combine(permissions)
        hasher.combine(capabilities)
        hasher.combine(path)
    }
}

public struct Permissions {
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

extension Permissions: Equatable, Hashable {
    public static func ==(lhs: Permissions, rhs: Permissions) -> Bool {
        if lhs.view != rhs.view {
            return false
        }
        if lhs.control != rhs.control {
            return false
        }
        if lhs.files != rhs.files {
            return false
        }
        if lhs.terminal != rhs.terminal {
            return false
        }
        if lhs.clipboard != rhs.clipboard {
            return false
        }
        if lhs.audio != rhs.audio {
            return false
        }
        if lhs.aiCap != rhs.aiCap {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(view)
        hasher.combine(control)
        hasher.combine(files)
        hasher.combine(terminal)
        hasher.combine(clipboard)
        hasher.combine(audio)
        hasher.combine(aiCap)
    }
}

public struct ServerInfo {
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

extension ServerInfo: Equatable, Hashable {
    public static func ==(lhs: ServerInfo, rhs: ServerInfo) -> Bool {
        if lhs.address != rhs.address {
            return false
        }
        if lhs.name != rhs.name {
            return false
        }
        if lhs.version != rhs.version {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(address)
        hasher.combine(name)
        hasher.combine(version)
    }
}

public struct TerminalOptions {
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

extension TerminalOptions: Equatable, Hashable {
    public static func ==(lhs: TerminalOptions, rhs: TerminalOptions) -> Bool {
        if lhs.attachSession != rhs.attachSession {
            return false
        }
        if lhs.shell != rhs.shell {
            return false
        }
        if lhs.cols != rhs.cols {
            return false
        }
        if lhs.rows != rhs.rows {
            return false
        }
        if lhs.env != rhs.env {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(attachSession)
        hasher.combine(shell)
        hasher.combine(cols)
        hasher.combine(rows)
        hasher.combine(env)
    }
}

public struct TerminalSessionInfo {
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

extension TerminalSessionInfo: Equatable, Hashable {
    public static func ==(lhs: TerminalSessionInfo, rhs: TerminalSessionInfo) -> Bool {
        if lhs.id != rhs.id {
            return false
        }
        if lhs.title != rhs.title {
            return false
        }
        if lhs.createdAt != rhs.createdAt {
            return false
        }
        if lhs.attached != rhs.attached {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(id)
        hasher.combine(title)
        hasher.combine(createdAt)
        hasher.combine(attached)
    }
}

public struct ToolCallView {
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

extension ToolCallView: Equatable, Hashable {
    public static func ==(lhs: ToolCallView, rhs: ToolCallView) -> Bool {
        if lhs.callId != rhs.callId {
            return false
        }
        if lhs.tool != rhs.tool {
            return false
        }
        if lhs.targetId != rhs.targetId {
            return false
        }
        if lhs.targetName != rhs.targetName {
            return false
        }
        if lhs.summary != rhs.summary {
            return false
        }
        if lhs.detail != rhs.detail {
            return false
        }
        if lhs.risk != rhs.risk {
            return false
        }
        if lhs.riskReasons != rhs.riskReasons {
            return false
        }
        if lhs.modelReason != rhs.modelReason {
            return false
        }
        if lhs.image != rhs.image {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(callId)
        hasher.combine(tool)
        hasher.combine(targetId)
        hasher.combine(targetName)
        hasher.combine(summary)
        hasher.combine(detail)
        hasher.combine(risk)
        hasher.combine(riskReasons)
        hasher.combine(modelReason)
        hasher.combine(image)
    }
}

public struct ToolResultView {
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

extension ToolResultView: Equatable, Hashable {
    public static func ==(lhs: ToolResultView, rhs: ToolResultView) -> Bool {
        if lhs.ok != rhs.ok {
            return false
        }
        if lhs.summary != rhs.summary {
            return false
        }
        if lhs.output != rhs.output {
            return false
        }
        if lhs.image != rhs.image {
            return false
        }
        if lhs.denied != rhs.denied {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(ok)
        hasher.combine(summary)
        hasher.combine(output)
        hasher.combine(image)
        hasher.combine(denied)
    }
}

public struct TotpSetup {
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

extension TotpSetup: Equatable, Hashable {
    public static func ==(lhs: TotpSetup, rhs: TotpSetup) -> Bool {
        if lhs.secret != rhs.secret {
            return false
        }
        if lhs.otpauthUri != rhs.otpauthUri {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(secret)
        hasher.combine(otpauthUri)
    }
}

public struct UserInfo {
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

extension UserInfo: Equatable, Hashable {
    public static func ==(lhs: UserInfo, rhs: UserInfo) -> Bool {
        if lhs.id != rhs.id {
            return false
        }
        if lhs.username != rhs.username {
            return false
        }
        if lhs.displayName != rhs.displayName {
            return false
        }
        if lhs.aiEnabled != rhs.aiEnabled {
            return false
        }
        if lhs.totpEnabled != rhs.totpEnabled {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(id)
        hasher.combine(username)
        hasher.combine(displayName)
        hasher.combine(aiEnabled)
        hasher.combine(totpEnabled)
    }
}

public struct VaultEntry {
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

extension VaultEntry: Equatable, Hashable {
    public static func ==(lhs: VaultEntry, rhs: VaultEntry) -> Bool {
        if lhs.id != rhs.id {
            return false
        }
        if lhs.folderId != rhs.folderId {
            return false
        }
        if lhs.sort != rhs.sort {
            return false
        }
        if lhs.data != rhs.data {
            return false
        }
        if lhs.updatedAt != rhs.updatedAt {
            return false
        }
        return true
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(id)
        hasher.combine(folderId)
        hasher.combine(sort)
        hasher.combine(data)
        hasher.combine(updatedAt)
    }
}

public enum AccountStatus {
    
    case active
    case needsRelogin
}

#if compiler(>=6)
extension AccountStatus: Sendable {}
#endif

extension AccountStatus: Equatable, Hashable {}

public enum AgentEvent {
    
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

extension AgentEvent: Equatable, Hashable {}

public enum AgentMessage {
    
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

extension AgentMessage: Equatable, Hashable {}

public enum AgentTargetKind {
    
    case device
    case sshHost
}

#if compiler(>=6)
extension AgentTargetKind: Sendable {}
#endif

extension AgentTargetKind: Equatable, Hashable {}

public enum AiMode {
    
    case off
    case readOnly
    case confirmAll
    case confirmDangerous
    case fullTrust
}

#if compiler(>=6)
extension AiMode: Sendable {}
#endif

extension AiMode: Equatable, Hashable {}

public enum ApprovalDecision {
    
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

extension ApprovalDecision: Equatable, Hashable {}

public enum ClientState {
    
    case loggedOut
    case connecting
    case online
    case offline
}

#if compiler(>=6)
extension ClientState: Sendable {}
#endif

extension ClientState: Equatable, Hashable {}

public enum ClipboardData {
    
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

extension ClipboardData: Equatable, Hashable {}

public enum ComboKey {
    
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

extension ComboKey: Equatable, Hashable {}

public enum ConnMode {
    
    case control
    case assist
}

#if compiler(>=6)
extension ConnMode: Sendable {}
#endif

extension ConnMode: Equatable, Hashable {}

public enum DeviceKind {
    
    case control
    case worker
    case mobile
}

#if compiler(>=6)
extension DeviceKind: Sendable {}
#endif

extension DeviceKind: Equatable, Hashable {}

public enum EventLevel {
    
    case info
    /**
     * 被拒绝的连接、被拦下的 AI 操作等，界面应醒目标出
     */
    case warning
}

#if compiler(>=6)
extension EventLevel: Sendable {}
#endif

extension EventLevel: Equatable, Hashable {}

public enum FileKind {
    
    case file
    case dir
    case symlink
    case other
}

#if compiler(>=6)
extension FileKind: Sendable {}
#endif

extension FileKind: Equatable, Hashable {}

public enum GalaxyError: Swift.Error {

    
    
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
}

extension GalaxyError: Equatable, Hashable {}

extension GalaxyError: Foundation.LocalizedError {
    public var errorDescription: String? {
        String(reflecting: self)
    }
}

public enum InputEvent {
    
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

extension InputEvent: Equatable, Hashable {}

public enum LoginResult {
    
    case success
    case needTotp
    case mustChangePassword
}

#if compiler(>=6)
extension LoginResult: Sendable {}
#endif

extension LoginResult: Equatable, Hashable {}

public enum PathKind {
    
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

extension PathKind: Equatable, Hashable {}

public enum Platform {
    
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

extension Platform: Equatable, Hashable {}

public enum PointerButton {
    
    case left
    case right
    case middle
    case back
    case forward
}

#if compiler(>=6)
extension PointerButton: Sendable {}
#endif

extension PointerButton: Equatable, Hashable {}

public enum RiskLevel {
    
    case read
    case normal
    case dangerous
    case unknown
    case forbidden
}

#if compiler(>=6)
extension RiskLevel: Sendable {}
#endif

extension RiskLevel: Equatable, Hashable {}

public enum SshAuth {
    
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

extension SshAuth: Equatable, Hashable {}

public enum TouchPhase {
    
    case began
    case moved
    case ended
    case cancelled
}

#if compiler(>=6)
extension TouchPhase: Sendable {}
#endif

extension TouchPhase: Equatable, Hashable {}

public enum VaultData {
    
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
}

#if compiler(>=6)
extension VaultData: Sendable {}
#endif

extension VaultData: Equatable, Hashable {}

public enum VideoCodec {
    
    case h264
    case h265
}

#if compiler(>=6)
extension VideoCodec: Sendable {}
#endif

extension VideoCodec: Equatable, Hashable {}

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

public protocol ClientListener: AnyObject, Sendable {
    
    func onState(state: ClientState) 
    
    func onDevicesChanged() 
    
    func onVaultChanged() 
    
    func onSessionRevoked() 
    
}

public protocol DisplayChannelProtocol: AnyObject, Sendable {
    
    func requestKeyframe() 
    
    func sendInput(event: InputEvent) 
    
    func setBitrate(kbps: UInt32) 
    
    func stop() 
    
}

public protocol EncodedFrameSinkProtocol: AnyObject, Sendable {
    
    func push(data: Data, ptsUs: UInt64, keyframe: Bool) 
    
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
    
    func changePassword(oldPassword: String, newPassword: String, totp: String?) async throws 
    
    /**
     * 首次登录设置新密码（会生成账号密钥）。
     */
    func completeFirstLogin(newPassword: String) async throws 
    
    func connect(deviceId: String) async throws  -> PeerSession
    
    func connectAssist(deviceCode: String, verificationCode: String) async throws  -> PeerSession
    
    func currentUser()  -> UserInfo?
    
    func deleteConversation(id: String) 
    
    func deviceId()  -> String
    
    func devices()  -> [DeviceInfo]
    
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
     * 坐标是 `display_id` 这块屏幕的像素。
     */
    func inject(displayId: UInt32, event: InputEvent) 
    
    func screenshot(displayId: UInt32, maxWidth: UInt32, maxHeight: UInt32)  -> ImageData?
    
    func uiTree(displayId: UInt32, maxNodes: UInt32)  -> String?
    
    func clipboardGet()  -> ClipboardData?
    
    func clipboardSet(data: ClipboardData) 
    
    /**
     * 已获得的系统授权对应的能力："display" "screenshot" "input" "ui_tree" "clipboard"
     */
    func capabilities()  -> [String]
    
}

public protocol HostServiceProtocol: AnyObject, Sendable {
    
    func accounts()  -> [HostAccount]
    
    func disconnectAll() 
    
    func recentEvents(limit: UInt32)  -> [HostEvent]
    
    func respondAssist(requestId: String, allow: Bool) 
    
    /**
     * 自定义验证码（8 到 16 位字母数字），None 为每次随机。
     */
    func setAssistCode(code: String?) 
    
    func setListener(listener: HostListener) 
    
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
    
    func clipboardGet() async throws  -> ClipboardData?
    
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
    
    func screenshot(displayId: UInt32, maxWidth: UInt32) async throws  -> ImageData
    
    func setListener(listener: PeerListener) 
    
    func wakeOnLan(mac: String) async throws 
    
}

public protocol PortForwardProtocol: AnyObject, Sendable {
    
    func localPort()  -> UInt16
    
    func stop() 
    
}

public protocol SecureStore: AnyObject, Sendable {
    
    func get(key: String)  -> Data?
    
    func set(key: String, value: Data) 
    
    func remove(key: String) 
    
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

// MARK: - 对象（占位）

open class AgentConversation: AgentConversationProtocol, @unchecked Sendable {
    public struct NoPointer { public init() {} }
    public init(noPointer: NoPointer) {}
    required public init(unsafeFromRawPointer pointer: UnsafeMutableRawPointer) {}
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

open class DisplayChannel: DisplayChannelProtocol, @unchecked Sendable {
    public struct NoPointer { public init() {} }
    public init(noPointer: NoPointer) {}
    required public init(unsafeFromRawPointer pointer: UnsafeMutableRawPointer) {}
    open func requestKeyframe() { bridgeUnimplemented() }
    open func sendInput(event: InputEvent) { bridgeUnimplemented() }
    open func setBitrate(kbps: UInt32) { bridgeUnimplemented() }
    open func stop() { bridgeUnimplemented() }
}

open class EncodedFrameSink: EncodedFrameSinkProtocol, @unchecked Sendable {
    public struct NoPointer { public init() {} }
    public init(noPointer: NoPointer) {}
    required public init(unsafeFromRawPointer pointer: UnsafeMutableRawPointer) {}
    open func push(data: Data, ptsUs: UInt64, keyframe: Bool) { bridgeUnimplemented() }
    open func setResolution(width: UInt32, height: UInt32) { bridgeUnimplemented() }
}

open class FileClient: FileClientProtocol, @unchecked Sendable {
    public struct NoPointer { public init() {} }
    public init(noPointer: NoPointer) {}
    required public init(unsafeFromRawPointer pointer: UnsafeMutableRawPointer) {}
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
    public struct NoPointer { public init() {} }
    public init(noPointer: NoPointer) {}
    required public init(unsafeFromRawPointer pointer: UnsafeMutableRawPointer) {}
    open func aiModels() async throws  -> AiModels { bridgeUnimplemented() }
    open func changePassword(oldPassword: String, newPassword: String, totp: String?) async throws { bridgeUnimplemented() }
    open func completeFirstLogin(newPassword: String) async throws { bridgeUnimplemented() }
    open func connect(deviceId: String) async throws  -> PeerSession { bridgeUnimplemented() }
    open func connectAssist(deviceCode: String, verificationCode: String) async throws  -> PeerSession { bridgeUnimplemented() }
    open func currentUser()  -> UserInfo? { bridgeUnimplemented() }
    open func deleteConversation(id: String) { bridgeUnimplemented() }
    open func deviceId()  -> String { bridgeUnimplemented() }
    open func devices()  -> [DeviceInfo] { bridgeUnimplemented() }
    open func host()  -> HostService? { bridgeUnimplemented() }
    open func listConversations()  -> [ConversationSummary] { bridgeUnimplemented() }
    open func login(server: String, username: String, password: String, totp: String?) async throws  -> LoginResult { bridgeUnimplemented() }
    open func logout() async throws { bridgeUnimplemented() }
    open func newConversation(config: ConversationConfig, listener: AgentListener) async throws  -> AgentConversation { bridgeUnimplemented() }
    open func openConversation(id: String, config: ConversationConfig?, listener: AgentListener) async throws  -> AgentConversation { bridgeUnimplemented() }
    open func refreshDevices() async throws { bridgeUnimplemented() }
    open func removeDevice(deviceId: String) async throws { bridgeUnimplemented() }
    open func restore()  -> Bool { bridgeUnimplemented() }
    open func server()  -> ServerInfo? { bridgeUnimplemented() }
    open func setLanDiscovery(active: Bool) { bridgeUnimplemented() }
    open func state()  -> ClientState { bridgeUnimplemented() }
    open func totpDisable(code: String) async throws { bridgeUnimplemented() }
    open func totpSetup() async throws  -> TotpSetup { bridgeUnimplemented() }
    open func totpVerify(code: String) async throws { bridgeUnimplemented() }
    open func vault()  -> [VaultEntry] { bridgeUnimplemented() }
    open func vaultDelete(id: String) async throws { bridgeUnimplemented() }
    open func vaultSave(entry: VaultEntry) async throws  -> VaultEntry { bridgeUnimplemented() }
}

open class HostService: HostServiceProtocol, @unchecked Sendable {
    public struct NoPointer { public init() {} }
    public init(noPointer: NoPointer) {}
    required public init(unsafeFromRawPointer pointer: UnsafeMutableRawPointer) {}
    open func accounts()  -> [HostAccount] { bridgeUnimplemented() }
    open func disconnectAll() { bridgeUnimplemented() }
    open func recentEvents(limit: UInt32)  -> [HostEvent] { bridgeUnimplemented() }
    open func respondAssist(requestId: String, allow: Bool) { bridgeUnimplemented() }
    open func setAssistCode(code: String?) { bridgeUnimplemented() }
    open func setListener(listener: HostListener) { bridgeUnimplemented() }
    open func setPlatform(platform: HostPlatform) { bridgeUnimplemented() }
    open func setRemoteEnabled(enabled: Bool) { bridgeUnimplemented() }
    open func startAssist() async throws  -> AssistInfo { bridgeUnimplemented() }
    open func status()  -> HostStatus { bridgeUnimplemented() }
    open func stopAi() { bridgeUnimplemented() }
    open func stopAssist() { bridgeUnimplemented() }
    open func updateAccountPolicy(userId: UInt64, policy: HostPolicy) { bridgeUnimplemented() }
}

open class PeerSession: PeerSessionProtocol, @unchecked Sendable {
    public struct NoPointer { public init() {} }
    public init(noPointer: NoPointer) {}
    required public init(unsafeFromRawPointer pointer: UnsafeMutableRawPointer) {}
    open func clipboardGet() async throws  -> ClipboardData? { bridgeUnimplemented() }
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
    open func screenshot(displayId: UInt32, maxWidth: UInt32) async throws  -> ImageData { bridgeUnimplemented() }
    open func setListener(listener: PeerListener) { bridgeUnimplemented() }
    open func wakeOnLan(mac: String) async throws { bridgeUnimplemented() }
}

open class PortForward: PortForwardProtocol, @unchecked Sendable {
    public struct NoPointer { public init() {} }
    public init(noPointer: NoPointer) {}
    required public init(unsafeFromRawPointer pointer: UnsafeMutableRawPointer) {}
    open func localPort()  -> UInt16 { bridgeUnimplemented() }
    open func stop() { bridgeUnimplemented() }
}

open class TerminalChannel: TerminalChannelProtocol, @unchecked Sendable {
    public struct NoPointer { public init() {} }
    public init(noPointer: NoPointer) {}
    required public init(unsafeFromRawPointer pointer: UnsafeMutableRawPointer) {}
    open func detach() { bridgeUnimplemented() }
    open func kill() { bridgeUnimplemented() }
    open func resize(cols: UInt16, rows: UInt16) { bridgeUnimplemented() }
    open func sessionId()  -> String { bridgeUnimplemented() }
    open func title()  -> String { bridgeUnimplemented() }
    open func write(data: Data) { bridgeUnimplemented() }
}

open class Transfer: TransferProtocol, @unchecked Sendable {
    public struct NoPointer { public init() {} }
    public init(noPointer: NoPointer) {}
    required public init(unsafeFromRawPointer pointer: UnsafeMutableRawPointer) {}
    open func cancel() { bridgeUnimplemented() }
    open func pause() { bridgeUnimplemented() }
    open func resume() { bridgeUnimplemented() }
}
#endif
