import Foundation

/// 远程桌面断了自动重连的节奏（Mac / Windows / 安卓 / iOS 一致）：
/// 第一次等 1 秒，之后 2、4、8、15 秒，再往后每 15 秒一次；一共 10 次（约 2 分钟）还不行就放弃，
/// 回到「连不上」（原因 + 重试，重试重新开始一轮）。后台不重连，回前台次数归零、从 1 秒那次重新开始。
/// 新画面显示出第一帧才算重连成功（成功才把次数清零）：打开了但一帧没来就又断了（内核看门狗判定没有响应）
/// 接着用剩下的次数，免得无限循环。
public enum ReconnectSchedule {
    /// 前几次尝试之前的等待（秒）
    public static let ramp: [TimeInterval] = [1, 2, 4, 8, 15]
    /// 之后每次之间的等待（秒）
    public static let steady: TimeInterval = 15
    /// 一轮最多试几次
    public static let maxAttempts = 10
    /// 新画面打开后等第一帧最多多久（秒），没来算这次失败（与内核刚打开画面时的看门狗一致）
    public static let firstFrameTimeout: TimeInterval = 20

    /// 第 attempt 次尝试（从 0 起）之前要等多久；次数用完返回 nil（放弃）
    public static func delay(before attempt: Int) -> TimeInterval? {
        guard attempt >= 0, attempt < maxAttempts else { return nil }
        return attempt < ramp.count ? ramp[attempt] : steady
    }

    /// 一轮里所有等待加起来（不含每次尝试本身的耗时）
    public static var totalWait: TimeInterval {
        (0..<maxAttempts).compactMap { delay(before: $0) }.reduce(0, +)
    }
}
