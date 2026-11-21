import SwiftUI
import AppKit

/// 本机被控：HostService 的界面状态。注册平台能力（MacHostPlatform），接收被控提示、协助请求。
@MainActor
final class LocalHostModel: ObservableObject {
    weak var app: AppModel?
    let selfTest: Bool
    @Published var status = HostStatus(remoteEnabled: true, online: false, accounts: [], sessions: [], aiActive: false, assist: nil)
    @Published var accounts: [HostAccount] = []
    @Published var events: [HostEvent] = []
    @Published var aiActivity: [String: String] = [:]        // session_id → 正在做什么
    @Published var pendingRequest: AssistRequest?
    @Published var screenGranted = false
    @Published var accessibilityGranted = false
    @Published var customCode = ""
    @Published var eventFilter = "all"
    private(set) var service: HostService?
    private var adapter: HostListenerAdapter?
    private var platform: MacHostPlatform?
    private var permTimer: Timer?

    struct AssistRequest: Identifiable, Equatable {
        let id: String
        let user: String
        let device: String
        let at = Date()
    }

    init(selfTest: Bool) { self.selfTest = selfTest }

    func attach(_ s: HostService) {
        service = s
        let a = HostListenerAdapter()
        a.owner = self
        adapter = a
        s.setListener(listener: a)
        if !selfTest {
            let p = MacHostPlatform()
            platform = p
            s.setPlatform(platform: p)
        }
        refresh()
        refreshPermissions()
    }

    func refresh() {
        guard let s = service else { return }
        status = s.status()
        accounts = s.accounts()
        events = s.recentEvents()
        ControlBanner.shared.update(self)
        StatusMenu.shared.update(self)
    }

    func refreshPermissions() {
        screenGranted = SystemPermissions.screenRecordingGranted
        accessibilityGranted = SystemPermissions.accessibilityGranted
    }

    /// 权限页打开时每 2 秒刷新授权状态（用户在系统设置里勾选后自动变绿）
    func watchPermissions(_ on: Bool) {
        permTimer?.invalidate()
        permTimer = nil
        guard on, !selfTest else { return }
        permTimer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refreshPermissions() }
        }
    }

    var needsPermissions: Bool { !screenGranted || !accessibilityGranted }

    // MARK: 操作

    func setRemoteEnabled(_ on: Bool) {
        if on, needsPermissions, let app {
            app.localPage = .permissions
            app.selectHome(.local)
        }
        service?.setRemoteEnabled(enabled: on)
        refresh()
    }

    func disconnectAll() { service?.disconnectAll(); aiActivity = [:]; refresh() }
    func stopAi() { service?.stopAi(); aiActivity = [:]; refresh() }

    func startAssist() { _ = service?.startAssist(); refresh() }
    func stopAssist() { service?.stopAssist(); refresh() }
    func refreshCode() { service?.setAssistCode(code: nil); refresh() }
    func applyCustomCode() {
        let c = customCode.trimmingCharacters(in: .whitespaces)
        guard (8...16).contains(c.count) else { return }
        service?.setAssistCode(code: c)
        refresh()
    }

    func respond(_ allow: Bool) {
        guard let r = pendingRequest else { return }
        service?.respondAssist(requestId: r.id, allow: allow)
        pendingRequest = nil
        AssistRequestPanel.shared.close()
        refresh()
    }

    func updatePolicy(_ account: HostAccount, _ change: (inout HostPolicy) -> Void) {
        var p = account.policy
        change(&p)
        service?.updateAccountPolicy(userId: account.userId, policy: p)
        refresh()
    }

    // MARK: 回调

    func received(status s: HostStatus) {
        status = s
        events = service?.recentEvents() ?? events
        ControlBanner.shared.update(self)
        StatusMenu.shared.update(self)
    }

    func assistRequested(id: String, user: String, device: String) {
        pendingRequest = AssistRequest(id: id, user: user, device: device)
        if !selfTest { AssistRequestPanel.shared.show(self) }
    }

    func sessionStarted(_ info: HostSessionInfo) { refresh() }

    func sessionEnded(_ id: String) {
        aiActivity[id] = nil
        refresh()
    }

    func aiActivity(_ id: String, _ text: String) {
        aiActivity[id] = text
        ControlBanner.shared.update(self)
        StatusMenu.shared.update(self)
    }

    var filteredEvents: [HostEvent] {
        switch eventFilter {
        case "ai": return events.filter(\.ai)
        case "warning": return events.filter { $0.level != .info }
        case "all": return events
        default: return events.filter { $0.category == eventFilter }
        }
    }
}

final class HostListenerAdapter: HostListener {
    weak var owner: LocalHostModel?
    private func main(_ f: @escaping @MainActor (LocalHostModel) -> Void) {
        DispatchQueue.main.async { [weak self] in
            guard let o = self?.owner else { return }
            MainActor.assumeIsolated { f(o) }
        }
    }
    func onStatus(status: HostStatus) { main { $0.received(status: status) } }
    func onAssistRequest(requestId: String, fromUser: String, fromDevice: String) { main { $0.assistRequested(id: requestId, user: fromUser, device: fromDevice) } }
    func onSessionStarted(info: HostSessionInfo) { main { $0.sessionStarted(info) } }
    func onSessionEnded(sessionId: String) { main { $0.sessionEnded(sessionId) } }
    func onAiActivity(sessionId: String, description: String) { main { $0.aiActivity(sessionId, description) } }
}
