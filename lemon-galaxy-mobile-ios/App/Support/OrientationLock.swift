import UIKit

/// 远程桌面菜单里的「横屏 / 竖屏」：在 iPhone 上锁定界面方向（锁定后在两个横向或两个竖向之间仍跟随重力），离开远程画面时解除。
/// 默认不锁，跟随设备方向。iPad 不提供（分屏、台前调度下请求方向不一定生效）。
@MainActor
enum OrientationLock {
    private static var locked: UIInterfaceOrientationMask?

    static var defaultMask: UIInterfaceOrientationMask {
        UIDevice.current.userInterfaceIdiom == .pad ? .all : .allButUpsideDown
    }

    /// AppDelegate 回答系统「支持哪些方向」
    static var mask: UIInterfaceOrientationMask { locked ?? defaultMask }

    static var supported: Bool { UIDevice.current.userInterfaceIdiom == .phone }

    /// 当前是竖屏就转到横屏，反之亦然
    static func toggle(portraitNow: Bool) {
        apply(portraitNow ? .landscape : .portrait)
    }

    /// 解除锁定（离开远程画面）
    static func release() {
        guard locked != nil else { return }
        apply(nil)
    }

    private static func apply(_ m: UIInterfaceOrientationMask?) {
        locked = m
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        for scene in scenes {
            for w in scene.windows {
                // 全屏层（fullScreenCover）是弹出的控制器：整条弹出链都要刷新
                var vc = w.rootViewController
                while let c = vc {
                    c.setNeedsUpdateOfSupportedInterfaceOrientations()
                    vc = c.presentedViewController
                }
            }
            if let m { scene.requestGeometryUpdate(.iOS(interfaceOrientations: m)) { _ in } }
        }
    }
}

final class AppDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication, supportedInterfaceOrientationsFor window: UIWindow?) -> UIInterfaceOrientationMask {
        OrientationLock.mask
    }
}
