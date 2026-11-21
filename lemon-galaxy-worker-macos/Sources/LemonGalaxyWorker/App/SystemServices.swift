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
enum Notifier {
    static func requestAuthorization() {
        guard AppInfo.isBundled else { return }
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }

    static func post(title: String, body: String) {
        guard AppInfo.isBundled else { return }
        let c = UNMutableNotificationContent()
        c.title = title
        c.body = body
        c.sound = .default
        let req = UNNotificationRequest(identifier: UUID().uuidString, content: c, trigger: nil)
        UNUserNotificationCenter.current().add(req) { _ in }
    }
}
