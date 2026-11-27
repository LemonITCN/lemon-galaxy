import Foundation

/// 主控端远程升级一台被控端的进度（设备详情里的「升级被控端」，不用先开远程桌面；docs/ffi.md 5.0）。
/// 点了直接开始，不再弹确认；对方自己走完整个升级（下载、核对、替换、以新版本重新启动），全程不用人操作。
/// 进度存在 GalaxyAccount.upgrades 里，离开详情再回来还能看到。
public enum DeviceUpgrade: Equatable {
    /// 已发出请求，等对方开始（参数是服务器报的新版本）
    case requesting(String)
    /// 对方开始升级到这个版本
    case installing(String)
    /// 对方已断开，正在替换、重启
    case restarting(String)
    /// 已以新版本重新上线
    case done(String)
    /// 对方本来就是最新（列表里的「可升级」已过时）；参数是对方现在的版本，可能为空
    case upToDate(String)
    /// 失败原因（对方版本太旧不支持远程升级、没有控制权限、超时没以新版本上线等）
    case failed(String)

    public var inProgress: Bool {
        switch self {
        case .requesting, .installing, .restarting: return true
        default: return false
        }
    }

    /// 对方已经开始升级（之后的「结束」才算升级成功，否则是本来就最新）
    public var started: Bool {
        switch self {
        case .installing, .restarting: return true
        default: return false
        }
    }

    public var targetVersion: String? {
        switch self {
        case .requesting(let v), .installing(let v), .restarting(let v): return v.isEmpty ? nil : v
        default: return nil
        }
    }

    /// 能远程升级的设备：被控端，或带「本机被控」的 Mac 客户端；不含本机自己与手机
    public static func supported(_ d: DeviceInfo) -> Bool {
        guard !d.isSelf else { return false }
        return d.kind == .worker || (d.kind == .control && d.platform == .macos)
    }
}

/// 远程升级进度回调 → 闭包（内核线程，由使用者切主线程）
public final class UpgradeListenerAdapter: UpgradeListener, @unchecked Sendable {
    public var started: ((String) -> Void)?
    public var offline: (() -> Void)?
    public var finished: ((String?, String?) -> Void)?
    public init() {}
    public func onStarted(version: String) { started?(version) }
    public func onOffline() { offline?() }
    public func onFinished(version: String?, error: String?) { finished?(version, error) }
}
