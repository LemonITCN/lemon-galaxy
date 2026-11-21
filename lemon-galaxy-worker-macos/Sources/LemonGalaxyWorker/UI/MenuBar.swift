import AppKit
import SwiftUI

// MARK: - 菜单栏图标

@MainActor
final class StatusMenu: NSObject, NSMenuDelegate {
    static let shared = StatusMenu()
    private var item: NSStatusItem?
    private weak var model: WorkerModel?
    private var lastState: IconState?

    enum IconState: CaseIterable { case idle, online, controlled, ai, disabled, attention }

    func install(model: WorkerModel) {
        self.model = model
        let it = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        it.button?.image = Self.icon(state: .idle)
        it.button?.toolTip = "LemonGalaxyWorker"
        let menu = NSMenu()
        menu.delegate = self
        it.menu = menu
        item = it
        update(model)
    }

    static func iconState(_ m: WorkerModel) -> IconState {
        if m.pendingRequest != nil { return .attention }
        switch m.overall {
        case .disabled: return .disabled
        case .ai: return .ai
        case .controlled: return .controlled
        case .online: return .online
        case .starting, .noAccount, .offline: return .idle
        }
    }

    func update(_ m: WorkerModel) {
        guard let item else { return }
        let s = Self.iconState(m)
        if s != lastState {
            item.button?.image = Self.icon(state: s)
            lastState = s
        }
        item.button?.toolTip = "LemonGalaxyWorker · " + L(StatusPage.stateKey(m.overall))
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        guard let m = model else { return }
        let s = m.status
        var head = "LemonGalaxyWorker · " + L(StatusPage.stateKey(m.overall))
        if m.overall == .controlled || m.overall == .ai {
            head = "LemonGalaxyWorker · " + L("menu.controlledBy", s.sessions.map(\.fromUser).joined(separator: ", "))
        }
        menu.addItem(disabled(head))
        menu.addItem(disabled(m.hasAccounts ? L("menu.accounts", m.accounts.map(\.username).joined(separator: ", ")) : L("status.state.noAccount")))
        if let act = m.aiActivity.values.first { menu.addItem(disabled(act)) }
        if let r = m.pendingRequest {
            menu.addItem(.separator())
            menu.addItem(disabled(L("assist.request.body", r.user, WorkerModel.shortDevice(r.device))))
            menu.addItem(make(L("assist.request.allow"), #selector(allowAssist)))
            menu.addItem(make(L("assist.request.deny"), #selector(denyAssist)))
        }
        menu.addItem(.separator())
        if !s.sessions.isEmpty { menu.addItem(make(L("status.disconnectAll"), #selector(disconnect))) }
        if m.aiActive { menu.addItem(make(L("status.stopAi"), #selector(stopAi))) }
        let toggle = make(L("status.allowRemote"), #selector(toggleRemote))
        toggle.state = s.remoteEnabled ? .on : .off
        toggle.isEnabled = m.service != nil
        menu.addItem(toggle)
        if let a = s.assist {
            menu.addItem(make(L("menu.assistCode", AssistPage.group(a.deviceCode, by: 3), a.code), #selector(copyAssist)))
            menu.addItem(make(L("assist.stop"), #selector(stopAssist)))
        } else if m.service != nil {
            menu.addItem(make(L("menu.startAssist"), #selector(startAssist)))
        }
        menu.addItem(.separator())
        let settings = make(L("menu.settings"), #selector(openSettings))
        settings.keyEquivalent = ","
        menu.addItem(settings)
        if !m.hasAccounts { menu.addItem(make(L("accounts.signIn") + "…", #selector(openLogin))) }
        menu.addItem(make(L("menu.about"), #selector(openAbout)))
        if m.core.isMock, let mock = (m.worker as? MockWorkerHost)?.mockService {
            let sim = NSMenuItem(title: L("menu.simulate"), action: nil, keyEquivalent: "")
            let sub = NSMenu()
            sub.addItem(SimItem(L("menu.sim.assist")) { mock.simulateAssistRequest() })
            sub.addItem(SimItem(L("menu.sim.session")) { mock.simulateSession(ai: false) })
            sub.addItem(SimItem(L("menu.sim.ai")) { mock.simulateSession(ai: true) })
            sim.submenu = sub
            menu.addItem(sim)
        }
        menu.addItem(.separator())
        menu.addItem(NSMenuItem(title: L("menu.quit"), action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q"))
    }

    private func disabled(_ title: String) -> NSMenuItem {
        let i = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        i.isEnabled = false
        return i
    }

    private func make(_ title: String, _ sel: Selector) -> NSMenuItem {
        let i = NSMenuItem(title: title, action: sel, keyEquivalent: "")
        i.target = self
        return i
    }

    private var delegate: AppDelegate? { NSApp.delegate as? AppDelegate }

    @objc private func disconnect() { model?.disconnectAll() }
    @objc private func stopAi() { model?.stopAi() }
    @objc private func toggleRemote() { guard let m = model else { return }; m.setRemoteEnabled(!m.status.remoteEnabled) }
    @objc private func allowAssist() { model?.respond(true) }
    @objc private func denyAssist() { model?.respond(false) }
    @objc private func copyAssist() { if let a = model?.status.assist { copyToPasteboard("\(a.deviceCode) \(a.code)") } }
    @objc private func stopAssist() { model?.stopAssist() }
    @objc private func startAssist() { model?.page = .assist; model?.startAssist(); delegate?.showSettings() }
    @objc private func openSettings() { delegate?.showSettings() }
    @objc private func openLogin() { model?.openLogin(); delegate?.showSettings() }
    @objc private func openAbout() { model?.page = .about; delegate?.showSettings() }

    /// 菜单栏图标：模板图（跟随菜单栏深浅）；被控 / AI 操作 / 有协助请求时右下角加彩色圆点，禁止远程时加斜线
    static func icon(state: IconState) -> NSImage {
        let size = NSSize(width: 18, height: 18)
        let colored = state == .controlled || state == .ai || state == .attention
        let img = NSImage(size: size, flipped: false) { _ in
            // 模板图用黑色由系统着色；带彩色圆点的图不能是模板，线条用 labelColor 跟随菜单栏深浅
            let ink: NSColor = colored ? .labelColor : .black
            ink.setStroke()
            let ring = NSBezierPath()
            ring.appendArc(withCenter: NSPoint(x: 9, y: 9), radius: 6.5, startAngle: 20, endAngle: 160)
            ring.move(to: NSPoint(x: 9 + 6.5 * cos(.pi * 200 / 180), y: 9 + 6.5 * sin(.pi * 200 / 180)))
            ring.appendArc(withCenter: NSPoint(x: 9, y: 9), radius: 6.5, startAngle: 200, endAngle: 340)
            ring.lineWidth = 1.6
            ring.lineCapStyle = .round
            ring.stroke()
            ink.setFill()
            if state == .idle {
                // 未上线：中心点空心
                let dot = NSBezierPath(ovalIn: NSRect(x: 7.1, y: 7.1, width: 3.8, height: 3.8))
                dot.lineWidth = 1.2
                dot.stroke()
            } else {
                NSBezierPath(ovalIn: NSRect(x: 6.8, y: 6.8, width: 4.4, height: 4.4)).fill()
            }
            if state == .disabled {
                let slash = NSBezierPath()
                slash.move(to: NSPoint(x: 3, y: 15)); slash.line(to: NSPoint(x: 15, y: 3))
                slash.lineWidth = 1.6
                slash.lineCapStyle = .round
                slash.stroke()
            }
            if colored {
                let c: NSColor = state == .ai || state == .attention ? .systemOrange : .systemRed
                c.setFill()
                NSBezierPath(ovalIn: NSRect(x: 11.5, y: 0.5, width: 6.5, height: 6.5)).fill()
            }
            return true
        }
        img.isTemplate = !colored
        return img
    }
}

final class SimItem: NSMenuItem {
    private let handler: () -> Void
    init(_ title: String, _ handler: @escaping () -> Void) {
        self.handler = handler
        super.init(title: title, action: #selector(fire), keyEquivalent: "")
        target = self
    }
    required init(coder: NSCoder) { fatalError() }
    @objc private func fire() { handler() }
}

// MARK: - 被控提示条（屏幕右上角悬浮）

struct ControlBannerView: View {
    @ObservedObject var model: WorkerModel
    @ObservedObject var localization = Localization.shared

    var body: some View {
        let p = Palettes.make("galaxy", dark: true)
        let loc = localization.strings
        let sessions = model.noticeSessions
        let ai = sessions.contains { $0.aiActive }
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Circle().fill(ai ? p.warning : p.danger).frame(width: 8, height: 8)
                    .shadow(color: (ai ? p.warning : p.danger).opacity(0.8), radius: 4)
                VStack(alignment: .leading, spacing: 1) {
                    Text(sessions.count == 1 ? loc("banner.controlledBy", sessions[0].fromUser) : loc("banner.controlledByMany", sessions.count))
                        .font(.system(size: 12.5, weight: .semibold)).foregroundStyle(.white).lineLimit(1)
                    if sessions.count == 1 {
                        Text(loc(sessions[0].mode == .assist ? "session.mode.assist" : "session.mode.control") + " · " + WorkerModel.shortDevice(sessions[0].fromDevice))
                            .font(.system(size: 11)).foregroundStyle(.white.opacity(0.6)).lineLimit(1)
                    }
                }
                Spacer(minLength: 8)
                Button { model.disconnectAll() } label: {
                    Text(loc("banner.disconnect")).font(.system(size: 11.5, weight: .semibold)).foregroundStyle(.white)
                        .padding(.horizontal, 9).frame(height: 22)
                        .background(RoundedRectangle(cornerRadius: 6).fill(p.danger))
                }
                .buttonStyle(.plain)
            }
            if ai {
                HStack(spacing: 6) {
                    Image(systemName: "wand.and.stars").font(.system(size: 11)).foregroundStyle(p.warning)
                    Text(model.aiActivity.values.first ?? loc("banner.aiActive")).font(.system(size: 11.5)).foregroundStyle(.white.opacity(0.85)).lineLimit(1)
                    Spacer(minLength: 8)
                    Button { model.stopAi() } label: {
                        Text(loc("status.stopAi")).font(.system(size: 11, weight: .medium)).foregroundStyle(p.warning)
                            .padding(.horizontal, 8).frame(height: 20)
                            .background(RoundedRectangle(cornerRadius: 5).stroke(p.warning.opacity(0.7), lineWidth: 1))
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .padding(.horizontal, 12).padding(.vertical, 10)
        .frame(width: 340)
        .background(RoundedRectangle(cornerRadius: 12).fill(Color(hex: "#1B1E25").opacity(0.96)))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(Color.white.opacity(0.12), lineWidth: 0.6))
        .environment(\.palette, p)
        .environment(\.loc, loc)
    }
}

/// 悬浮提示条窗口：不抢焦点、所有桌面可见、不被自己的截屏录进去（截屏时排除本进程窗口）
@MainActor
final class ControlBanner {
    static let shared = ControlBanner()
    private var panel: NSPanel?

    func update(_ model: WorkerModel) {
        guard !model.selfTest else { return }
        if model.noticeSessions.isEmpty {
            panel?.orderOut(nil)
            return
        }
        if panel == nil {
            let p = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 340, height: 80), styleMask: [.borderless, .nonactivatingPanel],
                            backing: .buffered, defer: false)
            p.isFloatingPanel = true
            p.level = .statusBar
            p.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
            p.backgroundColor = .clear
            p.isOpaque = false
            p.hasShadow = true
            p.hidesOnDeactivate = false
            p.contentView = NSHostingView(rootView: ControlBannerView(model: model))
            panel = p
        }
        guard let panel, let screen = NSScreen.main else { return }
        panel.contentView?.layoutSubtreeIfNeeded()
        let size = panel.contentView?.fittingSize ?? NSSize(width: 340, height: 60)
        let f = screen.visibleFrame
        panel.setFrame(NSRect(x: f.maxX - size.width - 16, y: f.maxY - size.height - 12, width: size.width, height: size.height), display: true)
        panel.orderFrontRegardless()
    }
}

/// 协助请求浮动面板（屏幕中央，置顶）
@MainActor
final class AssistRequestPanel {
    static let shared = AssistRequestPanel()
    private var panel: NSPanel?

    func show(_ model: WorkerModel) {
        guard let r = model.pendingRequest else { return }
        close()
        let view = Themed(model: model, settings: model.settings) {
            AssistRequestView(request: r).frame(width: 420)
        }
        let p = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 420, height: 150), styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        p.level = .modalPanel
        p.isFloatingPanel = true
        p.backgroundColor = .clear
        p.isOpaque = false
        p.hasShadow = true
        p.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        p.contentView = NSHostingView(rootView: view)
        let size = p.contentView?.fittingSize ?? NSSize(width: 420, height: 150)
        if let s = NSScreen.main {
            p.setFrame(NSRect(x: s.frame.midX - size.width / 2, y: s.frame.midY - size.height / 2 + 120, width: size.width, height: size.height), display: true)
        }
        p.orderFrontRegardless()
        panel = p
        if model.settings.assistSound {
            if let s = NSSound(named: "Glass") { s.play() } else { NSSound.beep() }
        }
    }

    func close() {
        panel?.orderOut(nil)
        panel = nil
    }
}
