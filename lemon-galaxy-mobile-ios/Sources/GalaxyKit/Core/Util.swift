@_exported import GalaxyCore
import Foundation

extension NSLock {
    /// 作用域加锁（异步函数里不能直接调 lock()/unlock()）
    @discardableResult
    public func locked<R>(_ body: () throws -> R) rethrows -> R {
        lock()
        defer { unlock() }
        return try body()
    }
}

/// 统一的「现在」（毫秒）。测试里可以冻结时间。
public enum Clock {
    public static var frozenMs: Int64?
    public static var nowMs: Int64 { frozenMs ?? Int64(Date().timeIntervalSince1970 * 1000) }
}

/// 内核回调可能在任意线程触发：统一切回主线程再碰界面状态。
/// 用 DispatchQueue.main.async（编译器把它的闭包视为主线程隔离），保持回调的先后顺序；
/// 不用 MainActor.assumeIsolated：命令行进程（冒烟测试）里主队列不一定被运行时认成主执行器。
public func onMain(_ body: @escaping @MainActor () -> Void) {
    DispatchQueue.main.async { body() }
}

/// 错误 → 直接给用户看的文字（内核的错误文字已按 core_set_locale 本地化）。
public func describeError(_ error: Error) -> String {
    if let g = error as? GalaxyError {
        switch g {
        case .Network(let m), .Permission(let m), .NotFound(let m), .Protocol(let m), .Other(let m): return m
        case .Auth(_, let m): return m
        case .Cancelled: return L("common.cancelled")
        }
    }
    if error is CancellationError { return L("common.cancelled") }
    return error.localizedDescription
}

/// 是否是「已取消」（不需要提示用户）
public func isCancellation(_ error: Error) -> Bool {
    if let g = error as? GalaxyError, case .Cancelled = g { return true }
    return error is CancellationError
}
