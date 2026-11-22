import SwiftUI
import AppKit

/// 根视图：登录页或主界面。配色与语言从这里放进环境，变化时整个界面即时重绘。
struct RootView: View {
    @ObservedObject var app: AppModel
    @ObservedObject var settings: AppSettings

    var body: some View {
        ThemedRoot(app: app, settings: settings) {
            switch app.phase {
            case .login: LoginView()
            case .main: MainView()
            }
        }
    }
}

/// 给任意内容套上与主窗口相同的环境（配色、语言、模型对象）。selftest 单独渲染某个视图时也用它。
struct ThemedRoot<Content: View>: View {
    @ObservedObject var app: AppModel
    @ObservedObject var settings: AppSettings
    @ObservedObject var localization = Localization.shared
    @ViewBuilder var content: Content
    @Environment(\.colorScheme) var scheme

    var body: some View {
        let palette = Palettes.make(settings.palette, dark: scheme == .dark)
        content
            .environmentObject(app)
            .environmentObject(settings)
            .environmentObject(app.host)
            .environmentObject(app.ai)
            .environment(\.palette, palette)
            .environment(\.loc, localization.strings)
            .tint(palette.accent)
            .accentColor(palette.accent)
            .foregroundStyle(palette.text)
            .preferredColorScheme(settings.appearance == "light" ? .light : settings.appearance == "dark" ? .dark : nil)
    }
}

struct MainView: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var settings: AppSettings
    @Environment(\.palette) var p

    var body: some View {
        HStack(spacing: 0) {
            Sidebar()
            Rectangle().fill(p.separator).frame(width: 1)
            if !settings.listHidden && !(settings.autoHideList && app.activeTabId != nil) {
                MiddleColumn()
                    .frame(width: app.section == .ai ? 240 : 284)
                    .background(p.list)
                Rectangle().fill(p.separator).frame(width: 1)
            }
            VStack(spacing: 0) {
                MainArea()
                if !app.rdpForwards.isEmpty { RdpForwardBar() }
            }
            .frame(minWidth: 520, maxWidth: .infinity)
            .background(p.content)
        }
        .background(p.content)
        .overlay(ModalOverlay())
        .overlay(alignment: .bottom) { ToastView() }
    }
}

// MARK: - 左侧导航

struct Sidebar: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var settings: AppSettings
    @EnvironmentObject var host: LocalHostModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var compact: Bool { settings.sidebarCompact }

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            // 窗口左上角留给红绿灯
            HStack {
                Spacer()
                if !compact {
                    IconButton(icon: "sidebar.left", help: loc("nav.collapse")) { settings.sidebarCompact = true }
                }
            }
            .frame(height: 38)
            .padding(.trailing, 8)
            if compact {
                IconButton(icon: "sidebar.left", help: loc("nav.expand")) { settings.sidebarCompact = false }
                    .frame(maxWidth: .infinity).padding(.bottom, 4)
            }
            ForEach(NavSection.allCases) { s in
                item(s)
            }
            Spacer()
            accountChip
        }
        .padding(.horizontal, compact ? 6 : 10)
        .padding(.bottom, 10)
        .frame(width: compact ? 56 : 196)
        .background(p.sidebar)
    }

    @ViewBuilder
    private func item(_ s: NavSection) -> some View {
        let selected = app.section == s && app.activeTabId == nil
        Button {
            app.selectHome(s)
        } label: {
            HStack(spacing: 10) {
                Image(systemName: s.icon)
                    .font(.system(size: 14, weight: .medium))
                    .frame(width: 20)
                    // 有新版本：设置图标右上角小红点
                    .overlay(alignment: .topTrailing) {
                        if s == .settings, app.update.available != nil { UpdateDot().offset(x: 3, y: -2) }
                    }
                if !compact {
                    Text(loc(s.titleKey)).font(.system(size: 13, weight: selected ? .semibold : .regular)).lineLimit(1)
                    Spacer(minLength: 0)
                    badge(s)
                }
            }
            .foregroundStyle(selected ? p.accent : p.sidebarText)
            .padding(.horizontal, compact ? 0 : 10)
            .frame(maxWidth: .infinity, minHeight: 32, alignment: compact ? .center : .leading)
            .background(RoundedRectangle(cornerRadius: 7).fill(selected ? p.sidebarSelection : (app.section == s ? p.hover : .clear)))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(loc(s.titleKey))
    }

    @ViewBuilder
    private func badge(_ s: NavSection) -> some View {
        if s == .local {
            if !host.status.sessions.isEmpty {
                StatusDot(state: host.aiActive ? .busy : .online, size: 7)
            } else if !host.status.remoteEnabled {
                Image(systemName: "nosign").font(.system(size: 10)).foregroundStyle(p.tertiaryText)
            }
        } else if s == .hosts {
            let n = app.devices.filter { $0.online && !$0.isSelf && $0.kind != .mobile }.count
            if n > 0 { Text("\(n)").font(.system(size: 10.5, weight: .medium).monospacedDigit()).foregroundStyle(p.tertiaryText) }
        } else if s == .forwards {
            // 运行中的端口映射条数（绿色）
            let n = app.forwards.count
            if n > 0 { Text("\(n)").font(.system(size: 10.5, weight: .semibold).monospacedDigit()).foregroundStyle(p.success) }
        }
    }

    private var accountChip: some View {
        Button { app.settingsPage = .account; app.selectHome(.settings) } label: {
            HStack(spacing: 8) {
                ZStack(alignment: .bottomTrailing) {
                    Circle().fill(p.accent.opacity(0.2)).frame(width: 26, height: 26)
                        .overlay(Text(String((app.user?.displayName ?? "?").prefix(1)).uppercased())
                            .font(.system(size: 12, weight: .semibold)).foregroundStyle(p.accent))
                    StatusDot(state: app.clientState == .online ? .online : (app.clientState == .offline ? .busy : .offline), size: 7)
                }
                if !compact {
                    VStack(alignment: .leading, spacing: 1) {
                        Text(app.user?.displayName ?? "").font(.system(size: 12, weight: .medium)).foregroundStyle(p.text).lineLimit(1)
                        Text(stateText).font(.system(size: 10.5)).foregroundStyle(p.tertiaryText).lineLimit(1)
                    }
                    Spacer(minLength: 0)
                }
            }
            .padding(.horizontal, compact ? 0 : 6).padding(.vertical, 5)
            .frame(maxWidth: .infinity, alignment: compact ? .center : .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(loc("account.settings"))
    }

    private var stateText: String {
        switch app.clientState {
        case .online: return app.server?.name ?? loc("state.online")
        case .offline: return loc("state.offline")
        case .connecting: return loc("state.connecting")
        case .loggedOut: return loc("state.loggedOut")
        }
    }
}

// MARK: - 中间列

struct MiddleColumn: View {
    @EnvironmentObject var app: AppModel
    var body: some View {
        Group {
            switch app.section {
            case .hosts: HostsList()
            case .sftp: SFTPTargetsList()
            case .forwards: PortForwardList()
            case .ai: AIConversationList()
            case .snippets: SnippetsList()
            case .local: LocalHostNav()
            case .settings: SettingsNav()
            }
        }
    }
}

/// 中间列的标题栏（与主区标签栏同高）
struct ColumnHeader<Trailing: View>: View {
    let title: String
    @ViewBuilder var trailing: Trailing
    @Environment(\.palette) var p
    var body: some View {
        HStack(spacing: 6) {
            Text(title).font(.system(size: 14, weight: .semibold)).foregroundStyle(p.text)
            Spacer()
            trailing
        }
        .padding(.leading, 14).padding(.trailing, 8)
        .frame(height: 38)
    }
}

// MARK: - 主区：标签栏 + 内容

struct MainArea: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p

    var body: some View {
        VStack(spacing: 0) {
            TabBar()
            // 所有标签的视图都保留在层级里，切换时只改可见性：终端、远程画面不会重建。
            // 每个子视图都钉死成可用区域的尺寸，某个标签内容偏宽时只会被裁掉，不会把整层撑大。
            GeometryReader { g in
                ZStack {
                    HomePage()
                        .frame(width: g.size.width, height: g.size.height)
                        .opacity(app.activeTabId == nil ? 1 : 0).allowsHitTesting(app.activeTabId == nil)
                    ForEach(app.tabs) { tab in
                        TabContent(tab: tab)
                            .frame(width: g.size.width, height: g.size.height)
                            .clipped()
                            .opacity(app.activeTabId == tab.id ? 1 : 0)
                            .allowsHitTesting(app.activeTabId == tab.id)
                    }
                }
            }
        }
    }
}

struct TabContent: View {
    let tab: TabModel
    var body: some View {
        switch tab.kind {
        case .terminal: TerminalTabView(tab: tab as! TerminalTabModel)
        case .sftp: SFTPTabView(tab: tab as! SFTPTabModel)
        case .remote: RemoteDesktopView(tab: tab as! RemoteDesktopModel)
        }
    }
}

/// 首页：当前导航分区的页面
struct HomePage: View {
    @EnvironmentObject var app: AppModel
    var body: some View {
        switch app.section {
        case .hosts: HostsHome()
        case .sftp: SFTPHome()
        case .forwards: PortForwardHome()
        case .ai: AIHomePage()
        case .snippets: SnippetHome()
        case .local: LocalHostPageView()
        case .settings: SettingsPageView()
        }
    }
}

struct TabBar: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var settings: AppSettings
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        HStack(spacing: 0) {
            IconButton(icon: settings.listHidden ? "sidebar.squares.left" : "rectangle.leadinghalf.inset.filled",
                       help: loc("nav.toggleList")) { settings.listHidden.toggle() }
                .padding(.horizontal, 6)
            homeTab
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 0) {
                    ForEach(app.tabs) { tab in TabItem(tab: tab) }
                }
            }
            Spacer(minLength: 0)
        }
        .frame(height: 38)
        .background(p.tabBar)
        .overlay(Rectangle().fill(p.separator).frame(height: 1), alignment: .bottom)
    }

    private var homeTab: some View {
        let active = app.activeTabId == nil
        return Button { app.activeTabId = nil } label: {
            HStack(spacing: 7) {
                Image(systemName: app.section.icon).font(.system(size: 12))
                Text(loc(app.section.titleKey)).font(.system(size: 12.5, weight: active ? .semibold : .regular)).lineLimit(1)
            }
            .foregroundStyle(active ? p.text : p.secondaryText)
            .padding(.horizontal, 14)
            .frame(height: 38)
            // 颜色背景默认会铺进窗口顶部安全区，选中标签的底色会越过顶部高亮线一直到窗口上沿：限定在标签本身
            .background(active ? p.tabActive : .clear, ignoresSafeAreaEdges: [])
            .overlay(Rectangle().fill(active ? p.accent : .clear).frame(height: 2), alignment: .top)
        }
        .buttonStyle(.plain)
    }
}

struct TabItem: View {
    @ObservedObject var tab: TabModel
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @State private var hover = false

    var body: some View {
        let active = app.activeTabId == tab.id
        HStack(spacing: 7) {
            ZStack(alignment: .bottomTrailing) {
                Image(systemName: tab.icon).font(.system(size: 11.5))
                if let live = tab.live, !live {
                    Circle().fill(p.danger).frame(width: 6, height: 6).offset(x: 3, y: 2)
                }
            }
            Text(tab.title).font(.system(size: 12.5, weight: active ? .medium : .regular)).lineLimit(1).frame(maxWidth: 150, alignment: .leading)
            Button { app.closeTab(tab.id) } label: {
                Image(systemName: "xmark").font(.system(size: 8.5, weight: .bold))
                    .frame(width: 16, height: 16)
                    .background(Circle().fill(hover ? p.hover.opacity(3) : .clear))
            }
            .buttonStyle(.plain)
            .opacity(hover || active ? 1 : 0)
        }
        .foregroundStyle(active ? p.text : p.secondaryText)
        .padding(.leading, 12).padding(.trailing, 6)
        .frame(height: 38)
        .background(active ? p.tabActive : (hover ? p.hover : .clear), ignoresSafeAreaEdges: [])
        .overlay(Rectangle().fill(active ? p.accent : .clear).frame(height: 2), alignment: .top)
        .overlay(Rectangle().fill(p.separator).frame(width: 1), alignment: .leading)
        .contentShape(Rectangle())
        .onTapGesture { app.activeTabId = tab.id }
        .onHover { hover = $0 }
        .contextMenu {
            Button(L("tab.close")) { app.closeTab(tab.id) }
            Button(L("tab.closeOthers")) { for t in app.tabs where t.id != tab.id { app.closeTab(t.id) } }
        }
    }
}

// MARK: - 提示条

struct ToastView: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    var body: some View {
        if let t = app.toast {
            HStack(spacing: 8) {
                Image(systemName: t.isError ? "exclamationmark.triangle.fill" : "checkmark.circle.fill")
                    .foregroundStyle(t.isError ? p.danger : p.success)
                Text(t.text).font(.system(size: 12.5)).foregroundStyle(p.text).lineLimit(2)
                if let a = t.action {
                    Rectangle().fill(p.separator).frame(width: 1, height: 14)
                    Button {
                        a.run()
                        app.toast = nil
                    } label: {
                        Text(a.title).font(.system(size: 12.5, weight: .semibold)).foregroundStyle(p.accent)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 14).padding(.vertical, 9)
            .background(RoundedRectangle(cornerRadius: 9).fill(p.elevated).shadow(color: .black.opacity(0.18), radius: 10, y: 3))
            .overlay(RoundedRectangle(cornerRadius: 9).stroke(p.border, lineWidth: 0.5))
            .padding(.bottom, 18)
            .onTapGesture { app.toast = nil }
        }
    }
}

/// 有新版本的小红点
struct UpdateDot: View {
    @Environment(\.palette) var p
    var body: some View {
        Circle().fill(p.danger).frame(width: 7, height: 7)
            .overlay(Circle().stroke(p.sidebar, lineWidth: 1.5))
    }
}

/// 主区底部：经设备转发中的 RDP。远程桌面应用连的是本机端口，用完在这里停止转发
struct RdpForwardBar: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "arrow.left.arrow.right.circle.fill").font(.system(size: 13)).foregroundStyle(p.info)
            Text(loc("rdp.bar.title")).font(.system(size: 12, weight: .semibold))
            InfoButton(text: loc("rdp.bar.info"))
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(app.rdpForwards) { f in
                        HStack(spacing: 6) {
                            Text(f.label).font(.system(size: 12, weight: .medium)).lineLimit(1)
                            Text(verbatim: "127.0.0.1:\(f.port)").font(.system(size: 11.5, design: .monospaced)).foregroundStyle(p.secondaryText)
                            IconButton(icon: "doc.on.doc", help: loc("rdp.bar.copy"), size: 10.5) {
                                NSPasteboard.general.clearContents()
                                NSPasteboard.general.setString("127.0.0.1:\(f.port)", forType: .string)
                                app.showToast(loc("common.copied"))
                            }
                            Button(loc("rdp.bar.stop")) { app.closeRdp(f.id) }
                                .buttonStyle(.plain)
                                .font(.system(size: 11.5, weight: .medium)).foregroundStyle(p.danger)
                        }
                        .padding(.leading, 10).padding(.trailing, 8).frame(height: 26)
                        .background(Capsule().fill(p.elevated))
                        .overlay(Capsule().stroke(p.border, lineWidth: 0.6))
                    }
                }
            }
            if app.rdpForwards.count > 1 {
                Button(loc("rdp.bar.stopAll")) { app.closeAllRdp() }.buttonStyle(SecondaryButtonStyle(destructive: true))
            }
        }
        .padding(.horizontal, 12)
        .frame(height: 38)
        .background(p.card)
        .overlay(Rectangle().fill(p.separator).frame(height: 1), alignment: .top)
    }
}
