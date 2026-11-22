import SwiftUI
import LocalAuthentication
import GalaxyKit

struct RootView: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var account: GalaxyAccount
    @EnvironmentObject var settings: AppSettings
    @ObservedObject private var localization = Localization.shared
    @Environment(\.colorScheme) private var systemScheme
    @Environment(\.horizontalSizeClass) private var sizeClass

    var body: some View {
        let dark = (settings.themeMode.colorScheme ?? systemScheme) == .dark
        let palette = Palettes.make(settings.palette, dark: dark)
        // iPhone 上会话是全屏层：那时提示与 SSH 认证弹窗由全屏层自己弹（根视图正在展示全屏层，弹不出别的）
        let coverShown = sizeClass == .compact && app.sessionsShown && account.phase == .main
        ZStack {
            if account.phase == .login {
                LoginView()
            } else {
                MainView()
            }
            if app.locked { LockView() }
        }
        .modifier(AppOverlays(active: !coverShown))
        .environment(\.loc, localization.strings)
        .environment(\.palette, palette)
        .environment(\.locale, Locale(identifier: localization.strings.code))
        .tint(palette.accent)
        .preferredColorScheme(settings.themeMode.colorScheme)
        .sheet(isPresented: $app.assistShown) {
            AssistConnectView()
                .environmentObject(app)
                .environment(\.loc, localization.strings)
                .environment(\.palette, palette)
        }
        .sheet(item: $app.rdpLaunch) { t in
            RdpLaunchSheet(target: t)
                .environmentObject(app)
                .environmentObject(account)
                .environment(\.loc, localization.strings)
                .environment(\.palette, palette)
        }
        .alert(localization.strings("rdp.appMissingTitle"), isPresented: $app.rdpAppMissing) {
            Link(localization.strings("rdp.openAppStore"), destination: RdpLink.appStoreURL)
            Button(localization.strings("common.cancel"), role: .cancel) {}
        } message: {
            Text(localization.strings("rdp.appMissingMessage"))
        }
        .sheet(item: $app.resumeChoice) { choice in
            RemoteSessionsSheet(target: choice.target, sessions: choice.sessions)
                .environmentObject(app)
                .environment(\.loc, localization.strings)
                .environment(\.palette, palette)
        }
    }
}

struct MainView: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.horizontalSizeClass) private var sizeClass

    var body: some View {
        if sizeClass == .regular {
            SplitMainView()
        } else {
            TabMainView()
        }
    }
}

/// 页面内容（iPhone 的标签页与 iPad 的详情区共用）
struct PageView: View {
    let tab: MainTab
    var body: some View {
        switch tab {
        case .hosts: NavigationStack { HostsView() }
        case .files: NavigationStack { FilesView() }
        case .ai: AIView()
        case .settings: NavigationStack { SettingsView() }
        }
    }
}

/// iPhone：底部标签栏；会话是全屏层，收起后底部有个「会话」胶囊可以回去。
struct TabMainView: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var sessions: SessionStore
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        TabView(selection: $app.tab) {
            ForEach(MainTab.allCases) { t in
                PageView(tab: t)
                    .tabItem { Label(loc(t.titleKey), systemImage: t.icon) }
                    .tag(t)
            }
        }
        .overlay(alignment: .bottom) {
            if !sessions.items.isEmpty && !app.sessionsShown {
                Button { app.sessionsShown = true } label: {
                    HStack(spacing: 8) {
                        Image(systemName: sessions.active?.icon ?? "terminal")
                        Text(sessions.active.map { sessions.title($0) } ?? "").lineLimit(1)
                        if sessions.items.count > 1 {
                            Text("\(sessions.items.count)").font(.caption.weight(.bold))
                                .padding(.horizontal, 6).padding(.vertical, 1)
                                .background(Capsule().fill(p.accentText.opacity(0.25)))
                        }
                        Image(systemName: "chevron.up").font(.caption.weight(.semibold))
                    }
                    .font(.subheadline.weight(.semibold))
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                    .background(Capsule().fill(p.accent))
                    .foregroundStyle(p.accentText)
                    .shadow(color: .black.opacity(0.2), radius: 8, y: 3)
                }
                .padding(.bottom, 58)
                .transition(.move(edge: .bottom).combined(with: .opacity))
                .accessibilityLabel(loc("session.show"))
            }
        }
        .fullScreenCover(isPresented: $app.sessionsShown) {
            SessionsView(store: sessions, onMinimize: { app.sessionsShown = false })
                .modifier(AppOverlays(active: true))
                .environmentObject(app)
                .environmentObject(app.account)
                .environmentObject(app.settings)
                .environmentObject(sessions)
                .environment(\.loc, Localization.shared.strings)
                .environment(\.palette, p)
        }
    }
}

/// iPad：左侧栏（主机 / 文件 / AI / 设置 + 打开中的会话），右侧详情。
struct SplitMainView: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var sessions: SessionStore
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var columns = NavigationSplitViewVisibility.all

    var body: some View {
        NavigationSplitView(columnVisibility: $columns) {
            List {
                Section {
                    ForEach(MainTab.allCases) { t in
                        sidebarRow(selected: !app.sessionsShown && app.tab == t) {
                            Label(loc(t.titleKey), systemImage: t.icon)
                        } action: {
                            app.tab = t
                            app.sessionsShown = false
                        }
                    }
                }
                if !sessions.items.isEmpty {
                    Section(loc("session.title")) {
                        ForEach(sessions.items) { item in
                            sidebarRow(selected: app.sessionsShown && sessions.active?.id == item.id) {
                                Label(sessions.title(item), systemImage: item.icon).lineLimit(1)
                            } action: {
                                sessions.activeId = item.id
                                app.sessionsShown = true
                            }
                            .contextMenu {
                                Button(role: .destructive) { sessions.close(item.id) } label: { Label(loc("common.close"), systemImage: "xmark") }
                            }
                        }
                    }
                }
            }
            .listStyle(.sidebar)
            .navigationTitle("LemonGalaxy")
            .safeAreaInset(edge: .bottom) {
                if let u = account.user {
                    HStack(spacing: 8) {
                        StatusDot(color: account.clientState == .online ? p.success : p.warning, size: 7)
                        Text(u.displayName.isEmpty ? u.username : u.displayName).font(.footnote).foregroundStyle(p.secondaryText)
                        Spacer()
                    }
                    .padding(.horizontal, 20)
                    .padding(.vertical, 10)
                }
            }
        } detail: {
            if app.sessionsShown && !sessions.items.isEmpty {
                SessionsView(store: sessions)
            } else {
                PageView(tab: app.tab)
            }
        }
        .navigationSplitViewStyle(.balanced)
    }

    private func sidebarRow<L: View>(selected: Bool, @ViewBuilder label: () -> L, action: @escaping () -> Void) -> some View {
        Button(action: action) { label().frame(maxWidth: .infinity, alignment: .leading) }
            .buttonStyle(.plain)
            .listRowBackground(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(selected ? p.accent.opacity(0.18) : Color.clear).padding(.horizontal, 8))
            .foregroundStyle(selected ? p.accent : p.text)
    }
}

/// 应用锁：回到前台要先过 Face ID / Touch ID / 锁屏密码
struct LockView: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @Environment(\.scenePhase) private var phase
    @State private var trying = false

    var body: some View {
        VStack(spacing: 18) {
            Image(systemName: "lock.fill").font(.system(size: 40)).foregroundStyle(p.accent)
            Text(loc("lock.title")).font(.title3.weight(.semibold)).foregroundStyle(p.text)
            Button(loc("lock.unlock"), action: unlock).buttonStyle(PrimaryButtonStyle()).frame(maxWidth: 260)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(p.background.ignoresSafeArea())
        // 整个锁屏都接住触摸，点击、长按都不会穿到下面的界面
        .contentShape(Rectangle())
        .onTapGesture {}
        .onLongPressGesture {}
        .onAppear { if phase == .active { unlock() } }
        .onChange(of: phase) { if $0 == .active { unlock() } }
    }

    private func unlock() {
        guard !trying else { return }
        trying = true
        let ctx = LAContext()
        ctx.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: loc("lock.prompt")) { ok, _ in
            DispatchQueue.main.async {
                trying = false
                if ok { app.locked = false }
            }
        }
    }
}

/// 全局提示条与 SSH 认证弹窗（密码 / 主机密钥）。内核在自己的线程上等回答，弹窗一定要弹得出来。
struct AppOverlays: ViewModifier {
    let active: Bool
    @EnvironmentObject var app: AppModel
    @Environment(\.loc) private var loc
    @Environment(\.palette) private var palette

    func body(content: Content) -> some View {
        content
            .overlay(alignment: .top) {
                if active, let t = app.toast {
                    ToastView(toast: t)
                        .padding(.top, 8)
                        .transition(.move(edge: .top).combined(with: .opacity))
                        .onTapGesture { app.toast = nil }
                }
            }
            .sheet(item: Binding(get: { active ? app.currentSecret : nil }, set: { _ in })) { req in
                SecretPromptSheet(request: req)
                    .environmentObject(app)
                    .environment(\.loc, loc)
                    .environment(\.palette, palette)
            }
    }
}
