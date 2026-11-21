import SwiftUI
import AppKit

/// 套上配色、语言、主题环境
struct Themed<Content: View>: View {
    @ObservedObject var model: WorkerModel
    @ObservedObject var settings: WorkerSettings
    @ObservedObject var localization = Localization.shared
    @ViewBuilder var content: Content
    @Environment(\.colorScheme) var scheme

    var body: some View {
        let palette = Palettes.make(settings.palette, dark: scheme == .dark)
        content
            .environmentObject(model)
            .environmentObject(settings)
            .environment(\.palette, palette)
            .environment(\.loc, localization.strings)
            .tint(palette.accent)
            .accentColor(palette.accent)
            .foregroundStyle(palette.text)
            .preferredColorScheme(settings.appearance == "light" ? .light : settings.appearance == "dark" ? .dark : nil)
    }
}

/// 设置窗口：左侧导航 + 右侧页面；登录与确认以窗口内浮层显示
struct WorkerRootView: View {
    @ObservedObject var model: WorkerModel
    @ObservedObject var settings: WorkerSettings

    var body: some View {
        Themed(model: model, settings: settings) {
            WorkerMain()
        }
    }
}

struct WorkerMain: View {
    @EnvironmentObject var model: WorkerModel
    @Environment(\.palette) var p

    var body: some View {
        HStack(spacing: 0) {
            Sidebar().frame(width: 212)
            Rectangle().fill(p.separator).frame(width: 1)
            ZStack(alignment: .top) {
                p.content
                PageScroll(maxWidth: 720) {
                    page
                }
                if let n = model.notice { NoticeBar(text: n).padding(.top, 14) }
            }
        }
        .background(p.content)
        .overlay { modal }
        .onChange(of: model.page) { pg in model.watchPermissions(pg == .permissions) }
    }

    @ViewBuilder
    private var page: some View {
        switch model.page {
        case .status: StatusPage()
        case .accounts: AccountsPage()
        case .assist: AssistPage()
        case .events: EventsPage()
        case .permissions: PermissionsPage()
        case .general: GeneralPage()
        case .about: AboutPage()
        }
    }

    @ViewBuilder
    private var modal: some View {
        if let f = model.loginForm {
            ModalLayer { LoginFormView(form: f) }
        } else if let a = model.confirmLogout {
            ModalLayer { LogoutConfirmView(account: a) }
        } else if model.unattended.confirmDisable {
            ModalLayer { UnattendedDisableView(ctl: model.unattended) }
        }
    }
}

/// 窗口内模态浮层：半透明遮罩 + 居中卡片
struct ModalLayer<Content: View>: View {
    @ViewBuilder let content: Content
    @Environment(\.palette) var p
    var body: some View {
        ZStack {
            Color.black.opacity(p.isDark ? 0.45 : 0.25).ignoresSafeArea()
            content
                .background(RoundedRectangle(cornerRadius: 14).fill(p.card).shadow(color: .black.opacity(p.isDark ? 0.45 : 0.15), radius: 24, y: 8))
                .overlay(RoundedRectangle(cornerRadius: 14).stroke(p.border, lineWidth: 0.5))
        }
    }
}

struct NoticeBar: View {
    let text: String
    @EnvironmentObject var model: WorkerModel
    @Environment(\.palette) var p
    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "info.circle.fill").foregroundStyle(p.accent)
            Text(text).font(.system(size: 12.5)).lineLimit(2)
            Button { model.notice = nil } label: {
                Image(systemName: "xmark").font(.system(size: 10, weight: .semibold)).foregroundStyle(p.tertiaryText)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 14).padding(.vertical, 9)
        .background(Capsule().fill(p.elevated).shadow(color: .black.opacity(p.isDark ? 0.4 : 0.12), radius: 10, y: 3))
        .overlay(Capsule().stroke(p.border, lineWidth: 0.5))
    }
}

// MARK: - 左侧导航

struct Sidebar: View {
    @EnvironmentObject var model: WorkerModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 10) {
                WorkerMark(size: 30)
                VStack(alignment: .leading, spacing: 1) {
                    Text("LemonGalaxyWorker").font(.system(size: 13, weight: .semibold)).lineLimit(1)
                    HStack(spacing: 5) {
                        Circle().fill(stateColor).frame(width: 6, height: 6)
                        Text(stateText).font(.system(size: 11)).foregroundStyle(p.secondaryText).lineLimit(1)
                    }
                }
            }
            .padding(.horizontal, 16).padding(.top, 34).padding(.bottom, 14)
            VStack(spacing: 2) {
                ForEach(WorkerPage.allCases, id: \.self) { page in
                    navItem(page)
                    if page == .events { Rectangle().fill(p.separator).frame(height: 1).padding(.vertical, 6).padding(.horizontal, 6) }
                }
            }
            .padding(.horizontal, 8)
            Spacer()
            remoteSwitch.padding(12)
        }
        .frame(maxHeight: .infinity)
        .background(p.sidebar)
    }

    private func navItem(_ page: WorkerPage) -> some View {
        let sel = model.page == page
        return Button { model.page = page } label: {
            HStack(spacing: 10) {
                Image(systemName: page.icon).font(.system(size: 13)).frame(width: 20).foregroundStyle(sel ? p.accent : p.secondaryText)
                Text(loc("page." + page.rawValue)).font(.system(size: 13, weight: sel ? .semibold : .regular)).foregroundStyle(p.sidebarText)
                Spacer()
                badge(page)
            }
            .padding(.horizontal, 10).frame(height: 30)
            .background(RoundedRectangle(cornerRadius: 7).fill(sel ? p.sidebarSelection : .clear))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    @ViewBuilder
    private func badge(_ page: WorkerPage) -> some View {
        switch page {
        case .status:
            if !model.status.sessions.isEmpty {
                Text("\(model.status.sessions.count)").font(.system(size: 11, weight: .semibold)).foregroundStyle(.white)
                    .padding(.horizontal, 6).background(Capsule().fill(model.aiActive ? p.warning : p.danger))
            }
        case .accounts:
            if model.needsRelogin { Image(systemName: "exclamationmark.circle.fill").font(.system(size: 12)).foregroundStyle(p.warning) }
            else if model.hasAccounts { Text("\(model.accounts.count)").font(.system(size: 11)).foregroundStyle(p.tertiaryText) }
        case .assist:
            if model.pendingRequest != nil { Image(systemName: "bell.badge.fill").font(.system(size: 11)).foregroundStyle(p.warning) }
            else if model.status.assist != nil { Circle().fill(p.success).frame(width: 7, height: 7) }
        case .permissions:
            if model.needsPermissions { Image(systemName: "exclamationmark.circle.fill").font(.system(size: 12)).foregroundStyle(p.warning) }
        case .about:
            if model.updater.available != nil { Circle().fill(p.danger).frame(width: 7, height: 7) }
        default: EmptyView()
        }
    }

    private var stateText: String { loc(StatusPage.stateKey(model.overall)) }
    private var stateColor: Color { StatusPage.stateColor(model.overall, p) }

    private var remoteSwitch: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(loc("status.allowRemote")).font(.system(size: 12.5, weight: .medium))
                Text(model.status.remoteEnabled ? loc("common.on") : loc("common.off"))
                    .font(.system(size: 11)).foregroundStyle(model.status.remoteEnabled ? p.success : p.danger)
            }
            Spacer()
            Toggle("", isOn: Binding(get: { model.status.remoteEnabled }, set: { model.setRemoteEnabled($0) }))
                .toggleStyle(PaletteSwitchStyle()).labelsHidden()
                .disabled(model.service == nil)
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 9).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 9).stroke(p.border, lineWidth: 0.6))
    }
}

/// 页面标题：大标题 + i 说明
struct PageTitle<Trailing: View>: View {
    let text: String
    var info: String? = nil
    @ViewBuilder var trailing: Trailing
    var body: some View {
        HStack(spacing: 8) {
            Text(text).font(.system(size: 20, weight: .semibold))
            if let info { InfoButton(text: info) }
            Spacer()
            trailing
        }
    }
}

extension PageTitle where Trailing == EmptyView {
    init(text: String, info: String? = nil) {
        self.text = text
        self.info = info
        self.trailing = EmptyView()
    }
}

/// 应用标志：与主控端同一套星系弧线，底色换成深青，便于在应用列表里区分被控端
struct WorkerMark: View {
    var size: CGFloat = 40
    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: size * 0.24)
                .fill(LinearGradient(colors: [Color(hex: "#15363A"), Color(hex: "#0A1719")], startPoint: .top, endPoint: .bottom))
            MarkArcs()
                .stroke(LinearGradient(colors: [Color(hex: "#F5D90A"), Color(hex: "#7BD35A")], startPoint: .topLeading, endPoint: .bottomTrailing),
                        style: StrokeStyle(lineWidth: size * 0.055, lineCap: .round))
                .padding(size * 0.2)
            Circle().fill(Color(hex: "#F5D90A")).frame(width: size * 0.13, height: size * 0.13)
        }
        .frame(width: size, height: size)
    }
}

struct MarkArcs: Shape {
    func path(in r: CGRect) -> Path {
        var p = Path()
        let c = CGPoint(x: r.midX, y: r.midY)
        p.addArc(center: c, radius: r.width * 0.5, startAngle: .degrees(200), endAngle: .degrees(340), clockwise: false)
        p.move(to: CGPoint(x: c.x + r.width * 0.5 * cos(.pi * 20 / 180), y: c.y + r.width * 0.5 * sin(.pi * 20 / 180)))
        p.addArc(center: c, radius: r.width * 0.5, startAngle: .degrees(20), endAngle: .degrees(160), clockwise: false)
        p.move(to: CGPoint(x: c.x + r.width * 0.3 * cos(.pi * 290 / 180), y: c.y + r.width * 0.3 * sin(.pi * 290 / 180)))
        p.addArc(center: c, radius: r.width * 0.3, startAngle: .degrees(290), endAngle: .degrees(70), clockwise: false)
        return p
    }
}
