import Foundation

/// 被控一侧的远程升级（docs/ffi.md 5.0）：主控端在设备详情里点了「升级被控端」，内核查到新版本（已验发布签名）后调这里。
/// 必须立即返回：交给零交互升级的唯一入口 AppUpdateEntry（App/UpdateModel.swift），在后台下载、核对、替换、
/// 以新版本重新打开，中间不弹任何要点的东西。
final class RemoteUpdateRequestHandler: UpdateRequestHandler, @unchecked Sendable {
    func onUpdateRequested(info: UpdateInfo) {
        NSLog("LemonGalaxy: 主控端要求升级到 %@", info.version)
        AppUpdateEntry.startUpdateNow(info: info)
    }

    /// 应用启动时登记一次（真内核初始化后、本机被控服务启动前；已经在跑的被控服务内核也会补上）。
    /// 模拟内核与自测不登记：内核不报远程升级能力，也就不会真的去升级
    static func register(isMock: Bool, selfTest: Bool) {
        guard !isMock, !selfTest else { return }
        setUpdateRequestHandler(handler: RemoteUpdateRequestHandler())
    }
}
