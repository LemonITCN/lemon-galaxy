// 内核接口层：对象（object → open class + XxxProtocol）、回调（callback → protocol）、顶层函数。
// 与 uniffi 0.28 生成的 Swift 外形一致：
//   - 每个对象生成 `XxxProtocol` 协议和同名 `open class Xxx: XxxProtocol`，方法都是 `open`；
//   - 有 `public init(noPointer: NoPointer)`，供测试替身（这里的 MockCore）继承后覆盖方法；
//   - 回调接口生成为 `protocol Xxx: AnyObject`。
// 所以 MockCore 里的 `MockPeerSession: PeerSession` 等子类，换成真生成代码后仍能原样编译。
// 本文件里的方法体只是占位：没有真内核时不会被调用（CoreProvider 只会创建 Mock 子类）。
#if !LEMONGALAXY_REAL_CORE
import Foundation

/// uniffi 生成代码里用于「不带 Rust 指针」构造对象的标记（测试替身专用）。
public struct NoPointer { public init() {} }

@inline(never)
func bridgeUnimplemented(_ function: String = #function) -> Never {
    fatalError("内核接口 \(function) 没有实现：当前没有接入 Rust 内核，只能用 MockCore 的子类")
}

// MARK: - 1. 初始化（顶层函数）

public func coreVersion() -> String { "0.0.0-bridge" }
public func coreInit(config: CoreConfig) {}

// MARK: 1.1 安全存储（界面层实现）

public protocol SecureStore: AnyObject {
    func get(key: String) -> Data?
    func set(key: String, value: Data)
    func remove(key: String)
}

// MARK: - 2. GalaxyClient

public protocol ClientListener: AnyObject {
    func onState(state: ClientState)
    func onDevicesChanged()
    func onVaultChanged()
    func onSessionRevoked()
}

public protocol GalaxyClientProtocol: AnyObject {
    func state() -> ClientState
    func currentUser() -> UserInfo?
    func server() -> ServerInfo?
    func login(server: String, username: String, password: String, totp: String?) async throws -> LoginResult
    func completeFirstLogin(newPassword: String) async throws
    func changePassword(oldPassword: String, newPassword: String) async throws
    func logout() async throws
    func restore() -> Bool
    func devices() -> [DeviceInfo]
    func refreshDevices() async throws
    func removeDevice(deviceId: String) async throws
    func vault() -> [VaultEntry]
    func vaultSave(entry: VaultEntry) async throws -> VaultEntry
    func vaultDelete(id: String) async throws
    func connect(deviceId: String) async throws -> PeerSession
    func connectAssist(deviceCode: String, verificationCode: String) async throws -> PeerSession
    func connectSsh(hostId: String, secretPrompt: SecretPrompt) async throws -> SshSession
    func aiModels() async throws -> AiModels
    func newConversation(config: ConversationConfig, listener: AgentListener) -> AgentConversation
    func host() -> HostService
}

open class GalaxyClient: GalaxyClientProtocol {
    public init(noPointer: NoPointer) {}
    open func state() -> ClientState { bridgeUnimplemented() }
    open func currentUser() -> UserInfo? { bridgeUnimplemented() }
    open func server() -> ServerInfo? { bridgeUnimplemented() }
    open func login(server: String, username: String, password: String, totp: String?) async throws -> LoginResult { bridgeUnimplemented() }
    open func completeFirstLogin(newPassword: String) async throws { bridgeUnimplemented() }
    open func changePassword(oldPassword: String, newPassword: String) async throws { bridgeUnimplemented() }
    open func logout() async throws { bridgeUnimplemented() }
    open func restore() -> Bool { bridgeUnimplemented() }
    open func devices() -> [DeviceInfo] { bridgeUnimplemented() }
    open func refreshDevices() async throws { bridgeUnimplemented() }
    open func removeDevice(deviceId: String) async throws { bridgeUnimplemented() }
    open func vault() -> [VaultEntry] { bridgeUnimplemented() }
    open func vaultSave(entry: VaultEntry) async throws -> VaultEntry { bridgeUnimplemented() }
    open func vaultDelete(id: String) async throws { bridgeUnimplemented() }
    open func connect(deviceId: String) async throws -> PeerSession { bridgeUnimplemented() }
    open func connectAssist(deviceCode: String, verificationCode: String) async throws -> PeerSession { bridgeUnimplemented() }
    open func connectSsh(hostId: String, secretPrompt: SecretPrompt) async throws -> SshSession { bridgeUnimplemented() }
    open func aiModels() async throws -> AiModels { bridgeUnimplemented() }
    open func newConversation(config: ConversationConfig, listener: AgentListener) -> AgentConversation { bridgeUnimplemented() }
    open func host() -> HostService { bridgeUnimplemented() }
}

// MARK: - 3. PeerSession

public protocol PeerListener: AnyObject {
    func onPathChanged(path: PathInfo)
    func onClosed(reason: String)
}

public protocol PeerSessionProtocol: AnyObject {
    func info() -> PeerInfo
    func path() -> PathInfo
    func setListener(listener: PeerListener)
    func close()
    func openTerminal(options: TerminalOptions, sink: TerminalSink) async throws -> TerminalChannel
    func listTerminals() async throws -> [TerminalSessionInfo]
    func killTerminal(sessionId: String) async throws
    func files() -> FileClient
    func exec(request: ExecRequest) async throws -> ExecResult
    func listDisplays() async throws -> [DisplayInfo]
    func openDisplay(options: DisplayOptions, sink: VideoSink) async throws -> DisplayChannel
    func screenshot(displayId: UInt32, maxWidth: UInt32) async throws -> ImageData
    func clipboardGet() async throws -> ClipboardData
    func clipboardSet(data: ClipboardData) async throws
    func forwardPort(localPort: UInt16, remoteHost: String, remotePort: UInt16) async throws -> PortForward
}

open class PeerSession: PeerSessionProtocol {
    public init(noPointer: NoPointer) {}
    open func info() -> PeerInfo { bridgeUnimplemented() }
    open func path() -> PathInfo { bridgeUnimplemented() }
    open func setListener(listener: PeerListener) { bridgeUnimplemented() }
    open func close() { bridgeUnimplemented() }
    open func openTerminal(options: TerminalOptions, sink: TerminalSink) async throws -> TerminalChannel { bridgeUnimplemented() }
    open func listTerminals() async throws -> [TerminalSessionInfo] { bridgeUnimplemented() }
    open func killTerminal(sessionId: String) async throws { bridgeUnimplemented() }
    open func files() -> FileClient { bridgeUnimplemented() }
    open func exec(request: ExecRequest) async throws -> ExecResult { bridgeUnimplemented() }
    open func listDisplays() async throws -> [DisplayInfo] { bridgeUnimplemented() }
    open func openDisplay(options: DisplayOptions, sink: VideoSink) async throws -> DisplayChannel { bridgeUnimplemented() }
    open func screenshot(displayId: UInt32, maxWidth: UInt32) async throws -> ImageData { bridgeUnimplemented() }
    open func clipboardGet() async throws -> ClipboardData { bridgeUnimplemented() }
    open func clipboardSet(data: ClipboardData) async throws { bridgeUnimplemented() }
    open func forwardPort(localPort: UInt16, remoteHost: String, remotePort: UInt16) async throws -> PortForward { bridgeUnimplemented() }
}

// MARK: 端口映射（ffi.md 未定义对象接口，这里给最小形态）

public protocol PortForwardProtocol: AnyObject {
    func localPort() -> UInt16
    func close()
}

open class PortForward: PortForwardProtocol {
    public init(noPointer: NoPointer) {}
    open func localPort() -> UInt16 { bridgeUnimplemented() }
    open func close() { bridgeUnimplemented() }
}

// MARK: 3.1 终端

public protocol TerminalSink: AnyObject {
    func onOutput(data: Data)
    func onExit(code: Int32?)
}

public protocol TerminalChannelProtocol: AnyObject {
    func sessionId() -> String
    func write(data: Data)
    func resize(cols: UInt16, rows: UInt16)
    func detach()
    func kill()
}

open class TerminalChannel: TerminalChannelProtocol {
    public init(noPointer: NoPointer) {}
    open func sessionId() -> String { bridgeUnimplemented() }
    open func write(data: Data) { bridgeUnimplemented() }
    open func resize(cols: UInt16, rows: UInt16) { bridgeUnimplemented() }
    open func detach() { bridgeUnimplemented() }
    open func kill() { bridgeUnimplemented() }
}

// MARK: 3.2 文件

public protocol TransferListener: AnyObject {
    func onProgress(done: UInt64, total: UInt64)
    func onFinished(error: String?)
}

public protocol FileClientProtocol: AnyObject {
    func home() async throws -> String
    func list(path: String) async throws -> [FileEntry]
    func stat(path: String) async throws -> FileEntry
    func mkdir(path: String) async throws
    func rename(from: String, to: String) async throws
    func remove(path: String, recursive: Bool) async throws
    func chmod(path: String, mode: UInt32) async throws
    func readText(path: String, maxBytes: UInt64) async throws -> String
    func writeText(path: String, text: String) async throws
    func download(remote: String, local: String, listener: TransferListener) -> Transfer
    func upload(local: String, remote: String, listener: TransferListener) -> Transfer
}

open class FileClient: FileClientProtocol {
    public init(noPointer: NoPointer) {}
    open func home() async throws -> String { bridgeUnimplemented() }
    open func list(path: String) async throws -> [FileEntry] { bridgeUnimplemented() }
    open func stat(path: String) async throws -> FileEntry { bridgeUnimplemented() }
    open func mkdir(path: String) async throws { bridgeUnimplemented() }
    open func rename(from: String, to: String) async throws { bridgeUnimplemented() }
    open func remove(path: String, recursive: Bool) async throws { bridgeUnimplemented() }
    open func chmod(path: String, mode: UInt32) async throws { bridgeUnimplemented() }
    open func readText(path: String, maxBytes: UInt64) async throws -> String { bridgeUnimplemented() }
    open func writeText(path: String, text: String) async throws { bridgeUnimplemented() }
    open func download(remote: String, local: String, listener: TransferListener) -> Transfer { bridgeUnimplemented() }
    open func upload(local: String, remote: String, listener: TransferListener) -> Transfer { bridgeUnimplemented() }
}

public protocol TransferProtocol: AnyObject {
    func cancel()
    func pause()
    func resume()
}

open class Transfer: TransferProtocol {
    public init(noPointer: NoPointer) {}
    open func cancel() { bridgeUnimplemented() }
    open func pause() { bridgeUnimplemented() }
    open func resume() { bridgeUnimplemented() }
}

// MARK: 3.3 远程画面

public protocol VideoSink: AnyObject {
    func onConfig(codec: VideoCodec, width: UInt32, height: UInt32)
    func onFrame(data: Data, ptsUs: UInt64, keyframe: Bool)
    func onCursor(x: Int32, y: Int32, visible: Bool)
}

public protocol DisplayChannelProtocol: AnyObject {
    func sendInput(event: InputEvent)
    func requestKeyframe()
    func setBitrate(kbps: UInt32)
    func stop()
}

open class DisplayChannel: DisplayChannelProtocol {
    public init(noPointer: NoPointer) {}
    open func sendInput(event: InputEvent) { bridgeUnimplemented() }
    open func requestKeyframe() { bridgeUnimplemented() }
    open func setBitrate(kbps: UInt32) { bridgeUnimplemented() }
    open func stop() { bridgeUnimplemented() }
}

// MARK: - 4. AI 智能体

public protocol AgentListener: AnyObject {
    func onEvent(event: AgentEvent)
}

public protocol AgentConversationProtocol: AnyObject {
    func id() -> String
    func send(text: String) async throws
    func decide(callId: String, decision: ApprovalDecision)
    func answer(questionId: String, answer: String)
    func provideSecret(requestId: String, secret: String)
    func setMode(mode: AiMode, trustMinutes: UInt32?)
    func stop()
    func messages() -> [AgentMessage]
}

open class AgentConversation: AgentConversationProtocol {
    public init(noPointer: NoPointer) {}
    open func id() -> String { bridgeUnimplemented() }
    open func send(text: String) async throws { bridgeUnimplemented() }
    open func decide(callId: String, decision: ApprovalDecision) { bridgeUnimplemented() }
    open func answer(questionId: String, answer: String) { bridgeUnimplemented() }
    open func provideSecret(requestId: String, secret: String) { bridgeUnimplemented() }
    open func setMode(mode: AiMode, trustMinutes: UInt32?) { bridgeUnimplemented() }
    open func stop() { bridgeUnimplemented() }
    open func messages() -> [AgentMessage] { bridgeUnimplemented() }
}

// MARK: - 5. HostService

public protocol HostPlatform: AnyObject {
    func listDisplays() -> [DisplayInfo]
    func startCapture(displayId: UInt32, options: CaptureOptions, sink: EncodedFrameSink) -> Bool
    func stopCapture(displayId: UInt32)
    func requestKeyframe(displayId: UInt32)
    func setBitrate(displayId: UInt32, kbps: UInt32)
    func inject(event: InputEvent)
    func screenshot(displayId: UInt32, maxWidth: UInt32) -> ImageData?
    func uiTree(displayId: UInt32, maxNodes: UInt32) -> String?
    func clipboardGet() -> ClipboardData?
    func clipboardSet(data: ClipboardData)
}

public protocol HostListener: AnyObject {
    func onStatus(status: HostStatus)
    func onAssistRequest(requestId: String, fromUser: String, fromDevice: String)
    func onSessionStarted(info: HostSessionInfo)
    func onSessionEnded(sessionId: String)
    func onAiActivity(sessionId: String, description: String)
}

public protocol EncodedFrameSinkProtocol: AnyObject {
    func push(data: Data, ptsUs: UInt64, keyframe: Bool)
    func setConfig(codec: VideoCodec, width: UInt32, height: UInt32)
}

/// 内核实现：平台层编码后把 Annex B 码流推进来。
open class EncodedFrameSink: EncodedFrameSinkProtocol {
    public init(noPointer: NoPointer) {}
    open func push(data: Data, ptsUs: UInt64, keyframe: Bool) { bridgeUnimplemented() }
    open func setConfig(codec: VideoCodec, width: UInt32, height: UInt32) { bridgeUnimplemented() }
}

public protocol HostServiceProtocol: AnyObject {
    func status() -> HostStatus
    func setPlatform(platform: HostPlatform)
    func setListener(listener: HostListener)
    func accounts() -> [HostAccount]
    func updateAccountPolicy(userId: UInt64, policy: HostPolicy)
    func setRemoteEnabled(enabled: Bool)
    func disconnectAll()
    func stopAi()
    func startAssist() -> AssistInfo
    func stopAssist()
    func setAssistCode(code: String?)
    func respondAssist(requestId: String, allow: Bool)
    func recentEvents() -> [HostEvent]
}

open class HostService: HostServiceProtocol {
    public init(noPointer: NoPointer) {}
    open func status() -> HostStatus { bridgeUnimplemented() }
    open func setPlatform(platform: HostPlatform) { bridgeUnimplemented() }
    open func setListener(listener: HostListener) { bridgeUnimplemented() }
    open func accounts() -> [HostAccount] { bridgeUnimplemented() }
    open func updateAccountPolicy(userId: UInt64, policy: HostPolicy) { bridgeUnimplemented() }
    open func setRemoteEnabled(enabled: Bool) { bridgeUnimplemented() }
    open func disconnectAll() { bridgeUnimplemented() }
    open func stopAi() { bridgeUnimplemented() }
    open func startAssist() -> AssistInfo { bridgeUnimplemented() }
    open func stopAssist() { bridgeUnimplemented() }
    open func setAssistCode(code: String?) { bridgeUnimplemented() }
    open func respondAssist(requestId: String, allow: Bool) { bridgeUnimplemented() }
    open func recentEvents() -> [HostEvent] { bridgeUnimplemented() }
}

// MARK: - 6. SSH 直连

public protocol SecretPrompt: AnyObject {
    func ask(prompt: String, echo: Bool) -> String?
}

public protocol SshSessionProtocol: AnyObject {
    func openTerminal(cols: UInt16, rows: UInt16, sink: TerminalSink) async throws -> TerminalChannel
    func files() -> FileClient
    func exec(request: ExecRequest) async throws -> ExecResult
    func forwardLocal(localPort: UInt16, remoteHost: String, remotePort: UInt16) async throws -> PortForward
    func close()
}

open class SshSession: SshSessionProtocol {
    public init(noPointer: NoPointer) {}
    open func openTerminal(cols: UInt16, rows: UInt16, sink: TerminalSink) async throws -> TerminalChannel { bridgeUnimplemented() }
    open func files() -> FileClient { bridgeUnimplemented() }
    open func exec(request: ExecRequest) async throws -> ExecResult { bridgeUnimplemented() }
    open func forwardLocal(localPort: UInt16, remoteHost: String, remotePort: UInt16) async throws -> PortForward { bridgeUnimplemented() }
    open func close() { bridgeUnimplemented() }
}
#endif
