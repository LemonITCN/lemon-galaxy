import SwiftUI
import AppKit

extension LocalHostPage {
    var icon: String {
        switch self {
        case .overview: return "dot.radiowaves.left.and.right"
        case .accounts: return "person.badge.key"
        case .assist: return "person.line.dotted.person"
        case .events: return "list.bullet.rectangle"
        case .permissions: return "lock.shield"
        }
    }
}

struct LocalHostNav: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var host: LocalHostModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(spacing: 0) {
            ColumnHeader(title: loc("nav.local")) { EmptyView() }
            Rectangle().fill(p.separator).frame(height: 1)
            VStack(spacing: 2) {
                ForEach(LocalHostPage.allCases, id: \.self) { page in
                    let sel = app.localPage == page && app.activeTabId == nil
                    Button { app.localPage = page; app.activeTabId = nil } label: {
                        HStack(spacing: 10) {
                            Image(systemName: page.icon).font(.system(size: 13)).frame(width: 20).foregroundStyle(sel ? p.accent : p.secondaryText)
                            Text(loc("local.page." + page.rawValue)).font(.system(size: 13, weight: sel ? .semibold : .regular))
                            Spacer()
                            badge(page)
                        }
                        .padding(.horizontal, 10).frame(height: 32)
                        .background(RoundedRectangle(cornerRadius: 7).fill(sel ? p.selection : .clear))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(8)
            Spacer()
            remoteSwitch.padding(12)
        }
    }

    @ViewBuilder
    private func badge(_ page: LocalHostPage) -> some View {
        switch page {
        case .overview:
            if !host.status.sessions.isEmpty { Text("\(host.status.sessions.count)").font(.system(size: 11, weight: .semibold)).foregroundStyle(p.accentText).padding(.horizontal, 6).background(Capsule().fill(host.aiActive ? p.warning : p.success)) }
        case .assist:
            if host.status.assist != nil { StatusDot(state: .online, size: 7) }
        case .permissions:
            if host.needsPermissions { Image(systemName: "exclamationmark.circle.fill").font(.system(size: 12)).foregroundStyle(p.warning) }
        default: EmptyView()
        }
    }

    private var remoteSwitch: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(loc("local.remoteEnabled")).font(.system(size: 12.5, weight: .medium))
                Text(host.status.remoteEnabled ? (host.status.online ? loc("local.state.online") : loc("local.state.offline")) : loc("local.state.disabled"))
                    .font(.system(size: 11)).foregroundStyle(host.status.remoteEnabled ? p.success : p.tertiaryText)
            }
            Spacer()
            Toggle("", isOn: Binding(get: { host.status.remoteEnabled }, set: { host.setRemoteEnabled($0) }))
                .toggleStyle(PaletteSwitchStyle()).labelsHidden()
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 9).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 9).stroke(p.border, lineWidth: 0.6))
    }
}

struct LocalHostPageView: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var host: LocalHostModel
    var body: some View {
        PageScroll(maxWidth: 860) {
            switch app.localPage {
            case .overview: LocalOverview()
            case .accounts: LocalAccounts()
            case .assist: LocalAssist()
            case .events: LocalEvents()
            case .permissions: LocalPermissions()
            }
        }
        .onAppear { host.refresh(); host.watchPermissions(app.localPage == .permissions) }
        .onChange(of: app.localPage) { p in host.watchPermissions(p == .permissions) }
    }
}

struct PageTitle: View {
    let text: String
    var info: String? = nil
    @Environment(\.palette) var p
    var body: some View {
        HStack(spacing: 8) {
            Text(text).font(.system(size: 20, weight: .semibold))
            if let info { InfoButton(text: info) }
            Spacer()
        }
    }
}

// MARK: 概览

struct LocalOverview: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var host: LocalHostModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            PageTitle(text: loc("local.page.overview"), info: loc("local.overview.info"))
            if host.needsPermissions {
                HStack(spacing: 10) {
                    Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(p.warning)
                    Text(loc("local.perm.missing")).font(.system(size: 12.5))
                    Spacer()
                    Button(loc("local.perm.go")) { app.localPage = .permissions }.buttonStyle(PrimaryButtonStyle())
                }
                .padding(12)
                .background(RoundedRectangle(cornerRadius: 9).fill(p.warning.opacity(0.1)))
                .overlay(RoundedRectangle(cornerRadius: 9).stroke(p.warning.opacity(0.35), lineWidth: 0.8))
            }
            HStack(spacing: 12) {
                stat(icon: "power", title: loc("local.remoteEnabled"),
                     value: host.status.remoteEnabled ? loc("common.on") : loc("common.off"),
                     color: host.status.remoteEnabled ? p.success : p.tertiaryText)
                stat(icon: "antenna.radiowaves.left.and.right", title: loc("local.server"),
                     value: host.status.online ? loc("local.state.online") : loc("local.state.offline"),
                     color: host.status.online ? p.success : p.warning)
                stat(icon: "person.crop.circle", title: loc("local.account"),
                     value: host.status.accounts.first?.username ?? "—", color: p.text)
            }
            Card {
                HStack(spacing: 10) {
                    Image(systemName: "lock.display").font(.system(size: 14)).foregroundStyle(p.secondaryText).frame(width: 20)
                    ToggleRow(label: loc("local.lockOnEnd"), isOn: Binding(get: { host.lockOnEnd }, set: { host.setLockOnEnd($0) }),
                              info: loc("local.lockOnEnd.info"))
                }
            }
            .disabled(host.service == nil)
            Card {
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: "rectangle.dashed").font(.system(size: 14)).foregroundStyle(p.secondaryText).frame(width: 20)
                        .padding(.top, 4)
                    HeadlessDisplaySetting(value: host.headlessDisplay) { host.setHeadlessDisplay($0) }
                }
            }
            .disabled(host.service == nil)
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    SectionLabel(text: loc("local.sessions"))
                    Spacer()
                    if !host.status.sessions.isEmpty {
                        if host.aiActive {
                            Button(loc("local.stopAi")) { host.stopAi() }.buttonStyle(SecondaryButtonStyle(destructive: true))
                        }
                        Button(loc("local.disconnectAll")) { host.disconnectAll() }.buttonStyle(PrimaryButtonStyle(destructive: true))
                    }
                }
                if host.status.sessions.isEmpty {
                    Card { Text(loc("local.noSessions")).font(.system(size: 12.5)).foregroundStyle(p.secondaryText) }
                } else {
                    ForEach(host.status.sessions, id: \.id) { s in sessionRow(s) }
                }
            }
            VStack(alignment: .leading, spacing: 10) {
                SectionLabel(text: loc("local.recentEvents"))
                Card(padding: 0) {
                    VStack(spacing: 0) {
                        ForEach(Array(host.events.prefix(5).enumerated()), id: \.offset) { i, e in
                            EventRow(event: e)
                            if i < min(5, host.events.count) - 1 { Rectangle().fill(p.separator).frame(height: 1) }
                        }
                    }
                }
            }
        }
    }

    private func stat(icon: String, title: String, value: String, color: Color) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Image(systemName: icon).font(.system(size: 14)).foregroundStyle(p.secondaryText)
            Text(title).font(.system(size: 11.5)).foregroundStyle(p.secondaryText)
            Text(value).font(.system(size: 15, weight: .semibold)).foregroundStyle(color).lineLimit(1)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 10).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(p.border, lineWidth: 0.6))
    }

    private func sessionRow(_ s: HostSessionInfo) -> some View {
        HStack(spacing: 12) {
            ZStack {
                Circle().fill((s.aiActive ? p.warning : p.accent).opacity(0.15)).frame(width: 34, height: 34)
                Image(systemName: s.aiActive ? "wand.and.stars" : "person.fill").foregroundStyle(s.aiActive ? p.warning : p.accent)
            }
            VStack(alignment: .leading, spacing: 3) {
                Text(s.fromUser).font(.system(size: 13, weight: .medium))
                HStack(spacing: 6) {
                    Image(systemName: "desktopcomputer").font(.system(size: 10)).foregroundStyle(p.tertiaryText)
                    Text(s.fromDevice).font(.system(size: 11.5)).foregroundStyle(p.secondaryText)
                    Text("·").foregroundStyle(p.tertiaryText)
                    Text(loc(s.mode == .assist ? "local.mode.assist" : "local.mode.control")).font(.system(size: 11.5)).foregroundStyle(p.secondaryText)
                    Text("·").foregroundStyle(p.tertiaryText)
                    Text(PathBadge.describe(s.path, p: p, loc: loc).0).font(.system(size: 11.5)).foregroundStyle(p.secondaryText)
                }
                if let act = host.aiActivity[s.id] {
                    Label(act, systemImage: "wand.and.stars").font(.system(size: 11.5)).foregroundStyle(p.warning)
                }
            }
            Spacer()
            Text(Fmt.time(s.startedAt)).font(.system(size: 11).monospacedDigit()).foregroundStyle(p.tertiaryText)
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 10).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(p.border, lineWidth: 0.6))
    }
}

struct EventRow: View {
    let event: HostEvent
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: icon).font(.system(size: 11)).foregroundStyle(color).frame(width: 14)
            Text(Fmt.date(event.at, zh: loc.isChinese)).font(.system(size: 11.5).monospacedDigit()).foregroundStyle(p.tertiaryText).frame(width: 112, alignment: .leading)
            if event.ai {
                Text("AI").font(.system(size: 9.5, weight: .bold)).foregroundStyle(p.warning)
                    .padding(.horizontal, 4).padding(.vertical, 1).background(RoundedRectangle(cornerRadius: 3).fill(p.warning.opacity(0.14)))
            }
            Text(loc("local.event." + event.kind) == "local.event." + event.kind ? event.kind : loc("local.event." + event.kind))
                .font(.system(size: 11.5, weight: .medium)).foregroundStyle(p.secondaryText)
            Text(event.detail).font(.system(size: 12.5)).foregroundStyle(p.text).lineLimit(2)
            Spacer()
            if !event.user.isEmpty { Text(event.user).font(.system(size: 11.5)).foregroundStyle(p.secondaryText) }
        }
        .padding(.horizontal, 14).padding(.vertical, 8)
    }
    private var icon: String { LocalHostModel.isWarning(event) ? "exclamationmark.triangle.fill" : "circle.fill" }
    private var color: Color { LocalHostModel.isWarning(event) ? p.warning : p.tertiaryText.opacity(0.6) }
}

// MARK: 账号与权限

struct LocalAccounts: View {
    @EnvironmentObject var host: LocalHostModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            PageTitle(text: loc("local.page.accounts"), info: loc("local.accounts.info"))
            if host.accounts.isEmpty {
                Card { Text(loc("local.noAccounts")).foregroundStyle(p.secondaryText) }
            }
            ForEach(host.accounts, id: \.userId) { a in accountCard(a) }
        }
    }

    private func accountCard(_ a: HostAccount) -> some View {
        Card {
            VStack(alignment: .leading, spacing: 14) {
                HStack(spacing: 10) {
                    Circle().fill(p.accent.opacity(0.18)).frame(width: 34, height: 34)
                        .overlay(Text(String(a.displayName.prefix(1))).font(.system(size: 14, weight: .semibold)).foregroundStyle(p.accent))
                    VStack(alignment: .leading, spacing: 2) {
                        Text("\(a.displayName) (\(a.username))").font(.system(size: 13.5, weight: .semibold))
                        if a.status == .needsRelogin {
                            Label(loc("local.account.needsRelogin"), systemImage: "exclamationmark.triangle.fill").font(.system(size: 11.5)).foregroundStyle(p.warning)
                        } else {
                            Text(loc("local.account.active")).font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
                        }
                    }
                    Spacer()
                    Text(loc("local.controlOnlyOne")).font(.system(size: 11)).foregroundStyle(p.tertiaryText)
                    InfoButton(text: loc("local.controlOnlyOne.info"))
                }
                Rectangle().fill(p.separator).frame(height: 1)
                HStack(alignment: .top, spacing: 28) {
                    VStack(spacing: 12) {
                        perm(a, "view", \.view)
                        perm(a, "files", \.files)
                        perm(a, "clipboard", \.clipboard)
                    }
                    VStack(spacing: 12) {
                        perm(a, "control", \.control)
                        perm(a, "terminal", \.terminal)
                        perm(a, "audio", \.audio)
                    }
                }
                Rectangle().fill(p.separator).frame(height: 1)
                HStack(spacing: 10) {
                    Text(loc("local.perm.aiCap")).font(.system(size: 13))
                    InfoButton(text: loc("local.perm.aiCap.info"))
                    Spacer()
                    Picker("", selection: Binding(get: { a.policy.aiCap }, set: { v in host.updatePolicy(a) { $0.aiCap = v } })) {
                        ForEach([AiMode.off] + AiMode.selectable, id: \.self) { m in Text(loc(m.labelKey)).tag(m) }
                    }
                    .labelsHidden().fixedSize()
                }
                HStack(spacing: 10) {
                    Text(loc("local.perm.notify")).font(.system(size: 13))
                    InfoButton(text: loc("local.perm.notify.info"))
                    Spacer()
                    Toggle("", isOn: Binding(get: { a.policy.notify }, set: { v in host.updatePolicy(a) { $0.notify = v } }))
                        .toggleStyle(PaletteSwitchStyle(small: true)).labelsHidden()
                }
                HStack(spacing: 10) {
                    Text(loc("local.systemUser")).font(.system(size: 13))
                    InfoButton(text: loc("local.systemUser.info"))
                    Spacer()
                    SystemUserField(account: a)
                }
            }
        }
    }

    private func perm(_ a: HostAccount, _ key: String, _ kp: WritableKeyPath<HostPolicy, Bool>) -> some View {
        HStack {
            Image(systemName: permIcon(key)).font(.system(size: 12)).foregroundStyle(p.secondaryText).frame(width: 18)
            Text(loc("local.perm." + key)).font(.system(size: 13))
            Spacer()
            Toggle("", isOn: Binding(get: { a.policy[keyPath: kp] }, set: { v in host.updatePolicy(a) { $0[keyPath: kp] = v } }))
                .toggleStyle(PaletteSwitchStyle(small: true)).labelsHidden()
        }
    }

    private func permIcon(_ k: String) -> String {
        ["view": "eye", "control": "cursorarrow.motionlines", "files": "folder", "terminal": "terminal", "clipboard": "doc.on.clipboard", "audio": "speaker.wave.2"][k] ?? "circle"
    }
}

struct SystemUserField: View {
    let account: HostAccount
    @EnvironmentObject var host: LocalHostModel
    @State private var text = ""
    var body: some View {
        TextField("", text: $text)
            .fieldStyle()
            .frame(width: 160)
            .onAppear { text = account.policy.systemUser }
            .onSubmit { host.updatePolicy(account) { $0.systemUser = text } }
    }
}

// MARK: 临时协助

struct LocalAssist: View {
    @EnvironmentObject var host: LocalHostModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            PageTitle(text: loc("local.page.assist"), info: loc("local.assist.info"))
            if let a = host.status.assist {
                Card {
                    VStack(alignment: .leading, spacing: 18) {
                        HStack(alignment: .top, spacing: 40) {
                            codeBlock(loc("assist.deviceCode"), Self.group(a.deviceCode, by: 3))
                            codeBlock(loc("assist.verificationCode"), a.code)
                            Spacer()
                        }
                        HStack(spacing: 8) {
                            Button { host.refreshCode() } label: { Label(loc("local.assist.refresh"), systemImage: "arrow.clockwise") }
                                .buttonStyle(SecondaryButtonStyle())
                                .disabled(host.pendingRequest != nil)
                            Button {
                                NSPasteboard.general.clearContents()
                                NSPasteboard.general.setString("\(a.deviceCode) \(a.code)", forType: .string)
                            } label: { Label(loc("common.copy"), systemImage: "doc.on.doc") }
                            .buttonStyle(SecondaryButtonStyle())
                            Spacer()
                            Button(loc("local.assist.stop")) { host.stopAssist() }.buttonStyle(SecondaryButtonStyle(destructive: true))
                        }
                        Rectangle().fill(p.separator).frame(height: 1)
                        HStack(spacing: 8) {
                            Text(loc("local.assist.custom")).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                            InfoButton(text: loc("local.assist.custom.info"))
                            TextField("8–16", text: $host.customCode).fieldStyle(invalid: !host.customCode.isEmpty && !(8...16).contains(host.customCode.count))
                                .frame(width: 180).font(.system(size: 13, design: .monospaced))
                            Button(loc("common.apply")) { host.applyCustomCode() }.buttonStyle(SecondaryButtonStyle())
                                .disabled(!(8...16).contains(host.customCode.count))
                        }
                    }
                }
                if let r = host.pendingRequest { AssistRequestView(request: r).frame(maxWidth: 460) }
            } else {
                Card {
                    HStack(spacing: 14) {
                        Image(systemName: "person.line.dotted.person").font(.system(size: 26, weight: .light)).foregroundStyle(p.accent)
                        Text(loc("local.assist.idle")).font(.system(size: 13)).foregroundStyle(p.secondaryText)
                        Spacer()
                        Button(loc("local.assist.start")) { host.startAssist() }.buttonStyle(PrimaryButtonStyle())
                    }
                }
            }
        }
    }

    static func group(_ s: String, by n: Int) -> String {
        var out = ""
        for (i, c) in s.enumerated() {
            if i > 0 && i % n == 0 { out += " " }
            out.append(c)
        }
        return out
    }

    private func codeBlock(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).font(.system(size: 12)).foregroundStyle(p.secondaryText)
            Text(value).font(.system(size: 28, weight: .semibold, design: .monospaced)).foregroundStyle(p.text).textSelection(.enabled)
        }
    }
}

/// 协助请求：允许 / 拒绝（本机窗口里与浮动面板里共用）
struct AssistRequestView: View {
    let request: LocalHostModel.AssistRequest
    @EnvironmentObject var host: LocalHostModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                Image(systemName: "person.crop.circle.badge.questionmark").font(.system(size: 24)).foregroundStyle(p.warning)
                VStack(alignment: .leading, spacing: 3) {
                    Text(loc("assist.request.title")).font(.system(size: 14, weight: .semibold))
                    Text(loc("assist.request.body", request.user, request.device)).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            HStack {
                InfoButton(text: loc("assist.request.info"))
                Spacer()
                Button(loc("assist.request.deny")) { host.respond(false) }.buttonStyle(SecondaryButtonStyle()).keyboardShortcut(.cancelAction)
                Button(loc("assist.request.allow")) { host.respond(true) }.buttonStyle(PrimaryButtonStyle())
            }
        }
        .padding(16)
        .background(RoundedRectangle(cornerRadius: 12).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(p.warning.opacity(0.5), lineWidth: 1))
    }
}

// MARK: 事件日志

struct LocalEvents: View {
    @EnvironmentObject var host: LocalHostModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                PageTitle(text: loc("local.page.events"), info: loc("local.events.info"))
                Picker("", selection: $host.eventFilter) {
                    Text(loc("local.events.all")).tag("all")
                    Text("AI").tag("ai")
                    Text(loc("local.events.warnings")).tag("warning")
                    Text(loc("local.events.connect")).tag("connect")
                    Text(loc("local.events.file")).tag("file")
                }
                .pickerStyle(.segmented).labelsHidden().frame(width: 330)
                IconButton(icon: "arrow.clockwise", help: loc("common.refresh")) { host.refresh() }
            }
            Card(padding: 0) {
                VStack(spacing: 0) {
                    let list = host.filteredEvents
                    ForEach(Array(list.enumerated()), id: \.offset) { i, e in
                        EventRow(event: e)
                        if i < list.count - 1 { Rectangle().fill(p.separator).frame(height: 1) }
                    }
                    if list.isEmpty { Text(loc("local.events.empty")).font(.system(size: 12.5)).foregroundStyle(p.tertiaryText).padding(16) }
                }
            }
        }
    }
}

// MARK: 系统授权

struct LocalPermissions: View {
    @EnvironmentObject var host: LocalHostModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            PageTitle(text: loc("local.page.permissions"), info: loc("local.perm.info"))
            permRow(icon: "rectangle.dashed.badge.record", title: loc("local.perm.screen"), info: loc("local.perm.screen.info"),
                    granted: host.screenGranted) { SystemPermissions.requestScreenRecording() }
            permRow(icon: "accessibility", title: loc("local.perm.accessibility"), info: loc("local.perm.accessibility.info"),
                    granted: host.accessibilityGranted) { SystemPermissions.requestAccessibility() }
            if !host.needsPermissions {
                Label(loc("local.perm.allSet"), systemImage: "checkmark.seal.fill").font(.system(size: 13)).foregroundStyle(p.success)
            }
        }
    }

    private func permRow(icon: String, title: String, info: String, granted: Bool, action: @escaping () -> Void) -> some View {
        Card {
            HStack(spacing: 14) {
                Image(systemName: icon).font(.system(size: 22, weight: .light)).foregroundStyle(granted ? p.success : p.warning).frame(width: 34)
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 5) {
                        Text(title).font(.system(size: 13.5, weight: .semibold))
                        InfoButton(text: info)
                    }
                    Text(granted ? loc("local.perm.granted") : loc("local.perm.notGranted"))
                        .font(.system(size: 12)).foregroundStyle(granted ? p.success : p.warning)
                }
                Spacer()
                if granted {
                    Image(systemName: "checkmark.circle.fill").font(.system(size: 18)).foregroundStyle(p.success)
                } else {
                    Button(loc("local.perm.open"), action: action).buttonStyle(PrimaryButtonStyle())
                }
            }
        }
    }
}

// MARK: - 被控提示条（屏幕右上角悬浮）

struct ControlBannerView: View {
    @ObservedObject var host: LocalHostModel
    @ObservedObject var settings: AppSettings
    @ObservedObject var localization = Localization.shared
    @Environment(\.colorScheme) var scheme

    var body: some View {
        let p = Palettes.make(settings.palette, dark: true)
        let loc = localization.strings
        let sessions = host.status.sessions
        let ai = host.aiActive
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Circle().fill(ai ? p.warning : p.danger).frame(width: 8, height: 8)
                    .shadow(color: (ai ? p.warning : p.danger).opacity(0.8), radius: 4)
                VStack(alignment: .leading, spacing: 1) {
                    Text(sessions.count == 1 ? loc("banner.controlledBy", sessions[0].fromUser) : loc("banner.controlledByMany", sessions.count))
                        .font(.system(size: 12.5, weight: .semibold)).foregroundStyle(.white).lineLimit(1)
                    if sessions.count == 1 {
                        Text(sessions[0].fromDevice).font(.system(size: 11)).foregroundStyle(.white.opacity(0.6)).lineLimit(1)
                    }
                }
                Spacer(minLength: 8)
                Button { host.disconnectAll() } label: {
                    Text(loc("banner.disconnect")).font(.system(size: 11.5, weight: .semibold)).foregroundStyle(.white)
                        .padding(.horizontal, 9).frame(height: 22)
                        .background(RoundedRectangle(cornerRadius: 6).fill(p.danger))
                }
                .buttonStyle(.plain)
            }
            if ai {
                HStack(spacing: 6) {
                    Image(systemName: "wand.and.stars").font(.system(size: 11)).foregroundStyle(p.warning)
                    Text(host.aiActivity.values.first ?? loc("banner.aiActive")).font(.system(size: 11.5)).foregroundStyle(.white.opacity(0.85)).lineLimit(1)
                    Spacer(minLength: 8)
                    Button { host.stopAi() } label: {
                        Text(loc("banner.stopAi")).font(.system(size: 11, weight: .medium)).foregroundStyle(p.warning)
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
    var enabled = true

    func update(_ host: LocalHostModel) {
        guard enabled, !host.selfTest else { return }
        if host.status.sessions.isEmpty {
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
            p.contentView = NSHostingView(rootView: ControlBannerView(host: host, settings: host.app?.settings ?? AppSettings.shared))
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

    func show(_ host: LocalHostModel) {
        guard let r = host.pendingRequest else { return }
        close()
        let settings = host.app?.settings ?? AppSettings.shared
        let view = AssistRequestView(request: r)
            .environmentObject(host)
            .environment(\.palette, Palettes.make(settings.palette, dark: NSApp.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua))
            .environment(\.loc, Localization.shared.strings)
            .frame(width: 420)
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
        NSSound.beep()
    }

    func close() {
        panel?.orderOut(nil)
        panel = nil
    }
}
