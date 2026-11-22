import Foundation
import SwiftUI
import UIKit
import GalaxyKit

enum MainTab: String, CaseIterable, Identifiable {
    case hosts, files, ai, settings
    var id: String { rawValue }
    var icon: String {
        switch self {
        case .hosts: return "server.rack"
        case .files: return "folder"
        case .ai: return "sparkles"
        case .settings: return "gearshape"
        }
    }
    var titleKey: String { "nav." + rawValue }
}

/// 应用的根状态：账号（内核）、AI、传输、打开中的会话、导航与提示。
@MainActor
final class AppModel: ObservableObject {
    let settings: AppSettings
    let account: GalaxyAccount
    let ai: AgentHub
    let transfers: TransferCenter
    let sessions: SessionStore

    @Published var tab: MainTab = .hosts
    /// iPhone：会话全屏层是否展开（收起后会话照常运行）
    @Published var sessionsShown = false
    @Published var toast: Toast?
    /// SSH 认证要用户回答的问题（排队，一次弹一个）
    @Published private(set) var secretQueue: [SecretRequest] = []
    /// 新建 AI 对话（预先放进输入框的引用，如从主机页点「AI」）
    @Published var newChatRefs: [ContextRef]?
    /// AI 页要打开的对话
    @Published var openConversationId: String?
    /// 文件页要打开的目标
    @Published var filesTarget: RemoteTarget?
    @Published var locked = false
    @Published var assistShown = false
    /// 经设备转发的 RDP：先提示转发会被挂起，再交给 Windows App
    @Published var rdpLaunch: RdpTarget?
    /// 没装 Windows App
    @Published var rdpAppMissing = false

    init() {
        let settings = AppSettings()
        self.settings = settings
        Localization.shared.apply(setting: settings.language)
        let forceMock = CoreProvider.wantsMock()
        // 模拟内核不碰钥匙串
        let store: SecureStore = forceMock ? MemorySecureStore() : KeychainStore()
        let config = CoreEnvironment.config(appVersion: AppInfo.version, osVersion: AppInfo.osVersion,
                                            hostname: AppInfo.deviceName, locale: Localization.shared.strings.pack.coreLocale)
        let account = GalaxyAccount(config: config, store: store, forceMock: forceMock)
        self.account = account
        ai = AgentHub(client: { account.client }, hosts: { account.hosts })
        transfers = TransferCenter()
        sessions = SessionStore()
        ai.defaultMode = { settings.defaultAiMode }
        sessions.app = self
        account.secretPresenter = { [weak self] req in self?.secretQueue.append(req) }
        account.onLoggedIn = { [weak self] in
            guard let self else { return }
            Task { await self.ai.loadModels() }
        }
        account.onLoggedOut = { [weak self] in
            guard let self else { return }
            self.sessions.closeAll()
            self.ai.reset()
            self.transfers.cancelAll()
            self.sessionsShown = false
            self.tab = .hosts
        }
        account.onSessionRevoked = { [weak self] in self?.showToast(L("login.revoked"), error: true) }
        transfers.onFinished = { [weak self] item in
            guard let self else { return }
            switch item.status {
            case .done:
                self.showToast(L(item.direction == .upload ? "transfer.uploaded" : "transfer.downloaded", item.name))
            case .failed(let e):
                self.showToast(L("transfer.failed", e), error: true)
            default: break
            }
        }
        if settings.appLock { locked = true }
        account.restore()
    }

    var isMock: Bool { account.isMock }

    func setLanguage(_ code: String) {
        settings.language = code
        Localization.shared.apply(setting: code)
        CoreProvider.setLocale(Localization.shared.strings.pack.coreLocale, isMock: isMock)
    }

    // MARK: 提示

    func showToast(_ text: String, error: Bool = false) {
        let t = Toast(text: text, isError: error)
        withAnimation(.spring(response: 0.3)) { toast = t }
        DispatchQueue.main.asyncAfter(deadline: .now() + (error ? 4.5 : 2.5)) { [weak self] in
            guard let self, self.toast == t else { return }
            withAnimation(.easeOut(duration: 0.2)) { self.toast = nil }
        }
    }

    func showError(_ error: Error) {
        guard !isCancellation(error) else { return }
        showToast(describeError(error), error: true)
    }

    // MARK: SSH 认证弹窗

    var currentSecret: SecretRequest? { secretQueue.first }

    func answerSecret(_ value: String?) {
        guard let r = secretQueue.first else { return }
        secretQueue.removeFirst()
        if case .ask(_, _, _, _, let reply) = r { reply(value) } else { r.cancel() }
    }

    func answerHostKey(_ ok: Bool) {
        guard let r = secretQueue.first else { return }
        secretQueue.removeFirst()
        if case .hostKey(_, _, _, _, _, _, let reply) = r { reply(ok) } else { r.cancel() }
    }

    // MARK: 打开连接

    func targetName(_ t: RemoteTarget) -> String { account.targetName(t) }

    /// 设备离线时不发起连接（局域网内发现的除外）
    func canConnect(_ t: RemoteTarget) -> Bool {
        switch t {
        case .device(let id):
            if account.isAssist(id) { return true }
            guard let d = account.device(id) else { return false }
            return d.online || d.lanSeen
        case .ssh: return true
        }
    }

    // MARK: 远程开机

    /// 正在发送唤醒包的设备（防重复点击、详情页显示进度）
    @Published private(set) var waking: Set<String> = []

    /// 远程开机，结果用提示条告知；返回提示文字与是否成功（设备详情是弹出页，提示条被盖住时就地显示）
    @discardableResult
    func wake(_ deviceId: String) async -> (ok: Bool, message: String) {
        guard !waking.contains(deviceId) else { return (false, "") }
        waking.insert(deviceId)
        defer { waking.remove(deviceId) }
        do {
            let via = try await account.wake(deviceId)
            let m = WakeText.sent(via)
            showToast(m)
            return (true, m)
        } catch {
            let m = L("wake.failed", describeError(error))
            showToast(m, error: true)
            return (false, m)
        }
    }

    /// 打开终端：设备上有保留的会话时先问要不要接续
    func openTerminal(_ t: RemoteTarget, attach: TerminalSessionInfo? = nil, ask: Bool = true) {
        if attach == nil, ask, settings.askResumeTerminal, case .device(let id) = t {
            Task {
                do {
                    let p = try await account.peer(id)
                    let kept = try await p.session.listTerminals().filter { !$0.attached && !sessions.isOpen(remoteSessionId: $0.id) }
                    if kept.isEmpty {
                        presentTerminal(t, attach: nil)
                    } else {
                        resumeChoice = ResumeChoice(target: t, sessions: kept)
                    }
                } catch {
                    showError(error)
                }
            }
            return
        }
        presentTerminal(t, attach: attach)
    }

    struct ResumeChoice: Identifiable {
        let id = UUID()
        let target: RemoteTarget
        let sessions: [TerminalSessionInfo]
    }
    @Published var resumeChoice: ResumeChoice?

    private func presentTerminal(_ t: RemoteTarget, attach: TerminalSessionInfo?) {
        let s = TerminalSession(app: self, target: t, title: targetName(t))
        sessions.add(.terminal(s))
        sessionsShown = true
        s.start(attach: attach?.id)
    }

    func openRemoteDesktop(_ deviceId: String) {
        if let existing = sessions.remote(for: deviceId) {
            sessions.activeId = existing.id
            sessionsShown = true
            return
        }
        let platform = account.device(deviceId)?.platform ?? account.assistPlatform(deviceId) ?? .windows
        let s = RemoteSession(app: self, deviceId: deviceId, title: account.deviceName(deviceId), platform: platform)
        sessions.add(.remote(s))
        sessionsShown = true
        s.start()
    }

    func openFiles(_ t: RemoteTarget) {
        filesTarget = t
        tab = .files
        sessionsShown = false
    }

    func openAI(_ targets: [AgentTarget]) {
        let wasCovered = sessionsShown
        tab = .ai
        sessionsShown = false
        // 全屏层收起的动画结束后再弹新建对话，否则弹不出来
        DispatchQueue.main.asyncAfter(deadline: .now() + (wasCovered ? 0.6 : 0.05)) { [weak self] in
            guard let self else { return }
            self.newChatRefs = targets.map { ContextRef.agentTarget($0, label: self.account.hosts.targetName($0)) }
        }
    }

    /// RDP 主机：交给微软 Windows App（rdp:// 链接，不带密码）。经设备转发时先弹提示与「停止转发」
    func openRdp(_ hostId: String) {
        Task {
            do {
                let t = try await account.openRdp(hostId)
                if t.forwarded {
                    sessionsShown = false
                    rdpLaunch = t
                } else {
                    launchRdp(t)
                }
            } catch {
                showError(error)
            }
        }
    }

    /// 打开 Windows App；没装时提示去 App Store，并收掉刚开的转发
    func launchRdp(_ t: RdpTarget) {
        guard let url = RdpLink.url(for: t), UIApplication.shared.canOpenURL(url) else {
            if t.forwarded { account.closeRdp(t.forwardId) }
            rdpAppMissing = true
            return
        }
        UIApplication.shared.open(url) { [weak self] ok in
            guard !ok else { return }
            if t.forwarded { self?.account.closeRdp(t.forwardId) }
            self?.rdpAppMissing = true
        }
    }

    /// 临时协助：连上后让用户选远程桌面 / 终端 / 文件
    func connectAssist(deviceCode: String, code: String) async throws -> String {
        try await account.connectAssist(deviceCode: deviceCode, verificationCode: code)
    }

    /// 会话关掉后，没有界面在用这台设备 / 主机就断开端到端会话
    func releaseIfUnused(_ t: RemoteTarget) {
        guard !sessions.uses(t), filesTarget != t else { return }
        account.release(t)
    }

    // MARK: 前后台

    func scenePhaseChanged(_ phase: ScenePhase) {
        switch phase {
        case .background:
            // 切后台：AI 任务暂停（ai-agent.md 第 1 节）；远程画面停解码
            ai.setForeground(false)
            sessions.setForeground(false)
            account.setLanDiscovery(false)
            if settings.appLock, account.phase == .main { locked = true }
        case .active:
            ai.setForeground(true)
            sessions.setForeground(true)
            if tab == .hosts, account.phase == .main { account.setLanDiscovery(true) }
        default:
            break
        }
    }
}

/// 打开中的会话：终端与远程桌面（文件页不算会话）。
enum SessionItem: Identifiable {
    case terminal(TerminalSession)
    case remote(RemoteSession)

    var id: String {
        switch self {
        case .terminal(let t): return t.id
        case .remote(let r): return r.id
        }
    }

    var target: RemoteTarget {
        switch self {
        case .terminal(let t): return t.target
        case .remote(let r): return .device(r.deviceId)
        }
    }

    var icon: String {
        switch self {
        case .terminal: return "terminal"
        case .remote: return "display"
        }
    }
}

@MainActor
final class SessionStore: ObservableObject {
    @Published private(set) var items: [SessionItem] = []
    @Published var activeId: String?
    weak var app: AppModel?

    var active: SessionItem? { items.first { $0.id == activeId } ?? items.last }

    func add(_ item: SessionItem) {
        items.append(item)
        activeId = item.id
    }

    func title(_ item: SessionItem) -> String {
        switch item {
        case .terminal(let t): return t.title
        case .remote(let r): return r.title
        }
    }

    func remote(for deviceId: String) -> RemoteSession? {
        for i in items { if case .remote(let r) = i, r.deviceId == deviceId { return r } }
        return nil
    }

    func isOpen(remoteSessionId: String) -> Bool {
        items.contains { if case .terminal(let t) = $0 { return t.remoteSessionId == remoteSessionId }; return false }
    }

    func uses(_ t: RemoteTarget) -> Bool { items.contains { $0.target == t } }

    /// 关闭：终端默认分离（被控端保留会话，之后可以接续）；kill 为真时结束远端会话
    func close(_ id: String, kill: Bool = false) {
        guard let i = items.firstIndex(where: { $0.id == id }) else { return }
        let item = items.remove(at: i)
        switch item {
        case .terminal(let t): kill ? t.kill() : t.detach()
        case .remote(let r): r.stop()
        }
        if activeId == id { activeId = items.isEmpty ? nil : items[min(i, items.count - 1)].id }
        if items.isEmpty { app?.sessionsShown = false }
        app?.releaseIfUnused(item.target)
    }

    func closeAll() {
        for i in items {
            switch i {
            case .terminal(let t): t.detach()
            case .remote(let r): r.stop()
            }
        }
        items.removeAll()
        activeId = nil
    }

    func setForeground(_ fg: Bool) {
        for i in items { if case .remote(let r) = i { r.setForeground(fg) } }
    }
}
