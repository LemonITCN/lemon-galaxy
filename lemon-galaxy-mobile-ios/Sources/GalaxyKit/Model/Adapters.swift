import Foundation

// 内核回调 → 闭包。回调可能在任意内核线程触发；这里不切线程，由使用者决定（界面状态一律 onMain）。

public final class ClientListenerAdapter: ClientListener, @unchecked Sendable {
    public var onStateChange: ((ClientState) -> Void)?
    public var onDevices: (() -> Void)?
    public var onVault: (() -> Void)?
    public var onRevoked: (() -> Void)?
    public init() {}
    public func onState(state: ClientState) { onStateChange?(state) }
    public func onDevicesChanged() { onDevices?() }
    public func onVaultChanged() { onVault?() }
    public func onSessionRevoked() { onRevoked?() }
}

public final class PeerListenerAdapter: PeerListener, @unchecked Sendable {
    public var onPath: ((PathInfo) -> Void)?
    public var onClose: ((String) -> Void)?
    public init() {}
    public func onPathChanged(path: PathInfo) { onPath?(path) }
    public func onClosed(reason: String) { onClose?(reason) }
}

/// 终端输出：按到达顺序交给 onData（内核线程）。
public final class TerminalSinkAdapter: TerminalSink, @unchecked Sendable {
    private let lock = NSLock()
    private var _onData: ((Data) -> Void)?
    private var _onExitCode: ((Int32?) -> Void)?
    public init(onData: @escaping (Data) -> Void, onExit: @escaping (Int32?) -> Void) {
        _onData = onData
        _onExitCode = onExit
    }
    /// 标签关掉后不再转发
    public func detach() { lock.locked { _onData = nil; _onExitCode = nil } }
    public func onOutput(data: Data) { lock.locked { _onData }?(data) }
    public func onExit(code: Int32?) { lock.locked { _onExitCode }?(code) }
}

public final class VideoSinkAdapter: VideoSink, @unchecked Sendable {
    public var onConfigure: ((VideoCodec, UInt32, UInt32) -> Void)?
    public var onData: ((Data, UInt64, Bool) -> Void)?
    public var onCursorMove: ((Int32, Int32, Bool) -> Void)?
    public var onClose: ((String) -> Void)?
    public init() {}
    public func onConfig(codec: VideoCodec, width: UInt32, height: UInt32) { onConfigure?(codec, width, height) }
    public func onFrame(data: Data, ptsUs: UInt64, keyframe: Bool) { onData?(data, ptsUs, keyframe) }
    public func onCursor(x: Int32, y: Int32, visible: Bool) { onCursorMove?(x, y, visible) }
    public func onClosed(reason: String) { onClose?(reason) }
}

public final class TransferListenerAdapter: TransferListener, @unchecked Sendable {
    public var progress: ((UInt64, UInt64) -> Void)?
    public var finished: ((String?) -> Void)?
    public init() {}
    public func onProgress(done: UInt64, total: UInt64) { progress?(done, total) }
    public func onFinished(error: String?) { finished?(error) }
}

public final class AgentListenerAdapter: AgentListener, @unchecked Sendable {
    public var handler: ((AgentEvent) -> Void)?
    public init() {}
    public func onEvent(event: AgentEvent) { handler?(event) }
}
