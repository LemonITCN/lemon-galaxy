import Foundation
import AppKit
import ServiceManagement
import UserNotifications

/// 开机启动：SMAppService.mainApp（macOS 13 起）。登记的是当前这份 .app，挪动位置后要重新打开开关。
enum LoginItem {
    enum State: Equatable { case enabled, requiresApproval, notRegistered, unavailable }

    static var state: State {
        guard AppInfo.isBundled else { return .unavailable }
        switch SMAppService.mainApp.status {
        case .enabled: return .enabled
        case .requiresApproval: return .requiresApproval
        case .notRegistered: return .notRegistered
        case .notFound: return .unavailable
        @unknown default: return .notRegistered
        }
    }

    /// 返回错误文字；成功为 nil
    static func set(_ on: Bool) -> String? {
        guard AppInfo.isBundled else { return L("general.loginItem.unavailable") }
        do {
            if on { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
            return nil
        } catch {
            return L("general.loginItem.failed", error.localizedDescription)
        }
    }

    /// 需要用户在「系统设置 → 通用 → 登录项」里批准
    static func openSettings() {
        SMAppService.openSystemSettingsLoginItems()
    }
}

/// 系统通知（有人连入、有人请求协助）。只在正式 .app 里可用：裸可执行文件调用通知中心会直接崩溃。
/// 有人连入的通知带「断开」「查看」两个按钮（与 Linux 被控端一致），会话结束后收回。
enum Notifier {
    static let sessionCategory = "cn.lemonit.galaxy.worker.session"
    private static let handler = NotificationHandler()

    static func requestAuthorization() {
        guard AppInfo.isBundled else { return }
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }

    /// 启动时调用：接管通知的点击，登记连入通知的按钮（换语言后再调一次）
    @MainActor
    static func install(_ model: WorkerModel) {
        guard AppInfo.isBundled else { return }
        handler.model = model
        let c = UNUserNotificationCenter.current()
        c.delegate = handler
        let disconnect = UNNotificationAction(identifier: "disconnect", title: L("notify.action.disconnect"), options: [.destructive])
        let show = UNNotificationAction(identifier: "show", title: L("notify.action.show"), options: [.foreground])
        c.setNotificationCategories([UNNotificationCategory(identifier: sessionCategory, actions: [disconnect, show], intentIdentifiers: [], options: [])])
    }

    static func post(title: String, body: String, id: String? = nil, category: String? = nil, session: String? = nil) {
        guard AppInfo.isBundled else { return }
        let c = UNMutableNotificationContent()
        c.title = title
        c.body = body
        c.sound = .default
        if let category { c.categoryIdentifier = category }
        if let session { c.userInfo = ["session": session] }
        let req = UNNotificationRequest(identifier: id ?? UUID().uuidString, content: c, trigger: nil)
        UNUserNotificationCenter.current().add(req) { _ in }
    }

    /// 收回已经发出的通知（会话结束了，「断开」按钮没有意义了）
    static func remove(id: String) {
        guard AppInfo.isBundled else { return }
        UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: [id])
    }
}

/// 通知的点击：「断开」断开那个会话；「查看」或点通知本身打开状态页
final class NotificationHandler: NSObject, UNUserNotificationCenterDelegate, @unchecked Sendable {
    @MainActor weak var model: WorkerModel?

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse, withCompletionHandler done: @escaping () -> Void) {
        let session = response.notification.request.content.userInfo["session"] as? String
        let action = response.actionIdentifier
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                guard let m = self.model else { return }
                if action == "disconnect", let session, let s = m.status.sessions.first(where: { $0.id == session }) {
                    m.disconnect(s)
                } else if action != UNNotificationDismissActionIdentifier {
                    m.page = .status
                    (NSApp.delegate as? AppDelegate)?.showSettings()
                }
            }
            done()
        }
    }

    /// 设置窗口在前台时也照样弹横幅（默认前台应用的通知不显示）
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification, withCompletionHandler done: @escaping (UNNotificationPresentationOptions) -> Void) {
        done([.banner, .sound])
    }
}
