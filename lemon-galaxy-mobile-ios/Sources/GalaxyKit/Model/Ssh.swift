import Foundation
import Combine

/// 终端、文件页的目标：LemonGalaxy 设备（端到端会话）或加密库里的 SSH 主机（内核直连）。
public enum RemoteTarget: Hashable, Identifiable, Sendable {
    case device(String)
    case ssh(String)

    public var id: String {
        switch self {
        case .device(let d): return "device:" + d
        case .ssh(let h): return "ssh:" + h
        }
    }

    public var deviceId: String? { if case .device(let d) = self { return d }; return nil }
    public var sshHostId: String? { if case .ssh(let h) = self { return h }; return nil }

    public var agentTarget: AgentTarget {
        switch self {
        case .device(let d): return AgentTarget(kind: .device, id: d)
        case .ssh(let h): return AgentTarget(kind: .sshHost, id: h)
        }
    }
}

/// 一条 SSH 直连会话（同一台主机的终端、文件共用）。
@MainActor
public final class SshConnection: ObservableObject {
    public let hostId: String
    public let session: SshSession
    public let info: SshInfo

    init(hostId: String, session: SshSession) {
        self.hostId = hostId
        self.session = session
        self.info = session.info()
    }

    public var isAlive: Bool { !session.isClosed() }
}

/// SSH 认证时内核要问用户的事：密码 / 私钥口令 / 键盘交互的回答，或首次连接、密钥变化时确认主机密钥。
public enum SecretRequest: Identifiable {
    case ask(id: String, title: String, prompt: String, echo: Bool, reply: (String?) -> Void)
    case hostKey(id: String, host: String, port: UInt16, keyType: String, fingerprint: String, changed: Bool, reply: (Bool) -> Void)

    public var id: String {
        switch self {
        case .ask(let id, _, _, _, _), .hostKey(let id, _, _, _, _, _, _): return id
        }
    }

    /// 界面关掉弹窗而没回答时：按取消处理
    public func cancel() {
        switch self {
        case .ask(_, _, _, _, let reply): reply(nil)
        case .hostKey(_, _, _, _, _, _, let reply): reply(false)
        }
    }
}

/// SecretPrompt 的实现：内核在自己的线程上同步调用，这里切到主线程交给界面弹窗，用信号量等回答。
/// 不能在主线程上等主线程：万一在主线程被调到，直接按取消处理。
public final class SecretPromptBridge: SecretPrompt, @unchecked Sendable {
    private let title: String
    private let present: @MainActor (SecretRequest) -> Void

    public init(title: String, present: @escaping @MainActor (SecretRequest) -> Void) {
        self.title = title
        self.present = present
    }

    public func ask(prompt: String, echo: Bool) -> String? {
        if Thread.isMainThread { return nil }
        let sem = DispatchSemaphore(value: 0)
        let box = OnceBox<String?>(nil)
        let title = self.title
        DispatchQueue.main.async {
            self.present(.ask(id: UUID().uuidString, title: title, prompt: prompt, echo: echo) { v in
                if box.set(v) { sem.signal() }
            })
        }
        sem.wait()
        return box.value
    }

    public func confirmHostKey(host: String, port: UInt16, keyType: String, fingerprint: String, changed: Bool) -> Bool {
        if Thread.isMainThread { return false }
        let sem = DispatchSemaphore(value: 0)
        let box = OnceBox<Bool>(false)
        DispatchQueue.main.async {
            self.present(.hostKey(id: UUID().uuidString, host: host, port: port, keyType: keyType, fingerprint: fingerprint, changed: changed) { ok in
                if box.set(ok) { sem.signal() }
            })
        }
        sem.wait()
        return box.value
    }
}

/// 只接受第一次写入（弹窗的回答只算一次）
final class OnceBox<T>: @unchecked Sendable {
    private let lock = NSLock()
    private var done = false
    private(set) var value: T
    init(_ initial: T) { value = initial }
    func set(_ v: T) -> Bool {
        lock.lock(); defer { lock.unlock() }
        guard !done else { return false }
        done = true
        value = v
        return true
    }
}

extension GalaxyAccount {
    /// 终端 / 文件 / AI 的目标显示名
    public func targetName(_ t: RemoteTarget) -> String {
        switch t {
        case .device(let d): return deviceName(d)
        case .ssh(let h): return hosts.sshLabel(h)
        }
    }

    /// 打开终端：设备支持接续被控端保留的会话；SSH 没有断线继承
    public func openTerminal(_ t: RemoteTarget, options: TerminalOptions, sink: TerminalSink) async throws -> TerminalChannel {
        switch t {
        case .device(let d): return try await peer(d).session.openTerminal(options: options, sink: sink)
        case .ssh(let h): return try await ssh(h).session.openTerminal(options: options, sink: sink)
        }
    }

    /// 文件客户端：设备走端到端会话，SSH 主机走 SFTP，接口完全相同
    public func files(_ t: RemoteTarget) async throws -> FileClient {
        switch t {
        case .device(let d): return try await peer(d).session.files()
        case .ssh(let h): return try await ssh(h).session.files()
        }
    }

    public func exec(_ t: RemoteTarget, _ request: ExecRequest) async throws -> ExecResult {
        switch t {
        case .device(let d): return try await peer(d).session.exec(request: request)
        case .ssh(let h): return try await ssh(h).session.exec(request: request)
        }
    }
}
