import Foundation

extension NSLock {
    /// 作用域加锁（异步函数里不能直接调 lock()/unlock()）
    @discardableResult
    func locked<R>(_ body: () throws -> R) rethrows -> R {
        lock()
        defer { unlock() }
        return try body()
    }
}

/// 统一的「现在」（毫秒）。selftest 冻结时间，截图里的时间戳才稳定。
enum Clock {
    static var frozenMs: Int64?
    static var nowMs: Int64 { frozenMs ?? Int64(Date().timeIntervalSince1970 * 1000) }
}
