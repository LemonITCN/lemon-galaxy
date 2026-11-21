import SwiftUI
import AppKit

// MARK: - 状态

struct StatusPage: View {
    @EnvironmentObject var model: WorkerModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    static func stateKey(_ o: WorkerModel.Overall) -> String {
        switch o {
        case .starting: return "status.state.starting"
        case .noAccount: return "status.state.noAccount"
        case .offline: return "status.state.offline"
        case .online: return "status.state.online"
        case .controlled: return "status.state.controlled"
        case .ai: return "status.state.ai"
        case .disabled: return "status.state.disabled"
        }
    }

    static func stateColor(_ o: WorkerModel.Overall, _ p: Palette) -> Color {
        switch o {
        case .online: return p.success
        case .controlled: return p.danger
        case .ai: return p.warning
        case .offline, .disabled: return p.warning
        case .starting, .noAccount: return p.tertiaryText
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            PageTitle(text: loc("page.status"), info: loc("status.info"))
            if let e = model.startError {
                warning(icon: "exclamationmark.octagon.fill", text: loc("status.startFailed", e), color: p.danger, action: nil)
            }
            if !model.hasAccounts && !model.serviceStarting {
                welcome
            } else {
                if model.needsPermissions {
                    warning(icon: "exclamationmark.triangle.fill", text: loc("status.permMissing"), color: p.warning,
                            action: (loc("status.permGo"), { model.page = .permissions }))
                }
                if model.needsRelogin {
                    warning(icon: "person.crop.circle.badge.exclamationmark", text: loc("status.needsRelogin"), color: p.warning,
                            action: (loc("status.goAccounts"), { model.page = .accounts }))
                }
                if !model.status.remoteEnabled {
                    warning(icon: "hand.raised.fill", text: loc("status.remoteDisabled"), color: p.danger,
                            action: (loc("status.enableRemote"), { model.setRemoteEnabled(true) }))
                }
            }
            hero
            if model.service != nil {
                sessions
                recent
            }
        }
    }

    // 第一次打开：还没登录账号
    private var welcome: some View {
        Card(padding: 20) {
            HStack(spacing: 16) {
                WorkerMark(size: 46)
                HStack(spacing: 6) {
                    Text(loc("status.welcome.title")).font(.system(size: 15, weight: .semibold))
                    InfoButton(text: loc("status.welcome.body"))
                }
                Spacer()
                Button(loc("accounts.signIn")) { model.openLogin() }.buttonStyle(PrimaryButtonStyle())
            }
        }
    }

    private func warning(icon: String, text: String, color: Color, action: (String, () -> Void)?) -> some View {
        HStack(spacing: 10) {
            Image(systemName: icon).foregroundStyle(color)
            Text(text).font(.system(size: 12.5)).fixedSize(horizontal: false, vertical: true)
            Spacer()
            if let action { Button(action.0, action: action.1).buttonStyle(SecondaryButtonStyle()) }
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 9).fill(color.opacity(0.1)))
        .overlay(RoundedRectangle(cornerRadius: 9).stroke(color.opacity(0.35), lineWidth: 0.8))
    }

    private var hero: some View {
        Card(padding: 0) {
            VStack(spacing: 0) {
                if model.hasAccounts || model.service != nil || model.serviceStarting { heroHeader }
                infoRow(loc("status.deviceId"), groupedId(model.deviceId), mono: true, copy: model.deviceId)
                infoRow(loc("status.publicIp"), model.status.publicIp.isEmpty ? "—" : model.status.publicIp, mono: true, copy: model.status.publicIp.isEmpty ? nil : model.status.publicIp)
                infoRow(loc("status.server"), model.serverAddress.map(serverHost) ?? "—", mono: false, copy: model.serverAddress)
                infoRow(loc("status.accounts"), model.accounts.isEmpty ? "—" : model.accounts.map(\.username).joined(separator: ", "), mono: false, copy: nil, last: true)
            }
        }
    }

    private var heroHeader: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                ZStack {
                    Circle().fill(StatusPage.stateColor(model.overall, p).opacity(0.16)).frame(width: 38, height: 38)
                    Image(systemName: heroIcon).font(.system(size: 16, weight: .medium)).foregroundStyle(StatusPage.stateColor(model.overall, p))
                }
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 8) {
                        Text(loc(StatusPage.stateKey(model.overall))).font(.system(size: 15, weight: .semibold))
                        if model.unattended.clientMode {
                            Text(loc("unattended.badge")).font(.system(size: 10.5, weight: .medium)).foregroundStyle(p.accent)
                                .padding(.horizontal, 7).padding(.vertical, 2).background(Capsule().fill(p.accent.opacity(0.12)))
                        }
                    }
                    Text(AppInfo.computerName + " · " + (model.unattended.clientMode ? model.unattended.screenDescription : AppInfo.userName))
                        .font(.system(size: 11.5)).foregroundStyle(p.secondaryText)
                }
                Spacer()
                if model.serviceStarting { ProgressView().controlSize(.small) }
            }
            .padding(16)
            Rectangle().fill(p.separator).frame(height: 1)
        }
    }

    private var heroIcon: String {
        switch model.overall {
        case .online: return "checkmark"
        case .controlled: return "person.fill"
        case .ai: return "wand.and.stars"
        case .disabled: return "hand.raised.fill"
        case .offline: return "wifi.exclamationmark"
        case .starting: return "ellipsis"
        case .noAccount: return "person.crop.circle.badge.plus"
        }
    }

    /// 只显示 主机:端口，指纹放在复制里
    private func serverHost(_ s: String) -> String { s.split(separator: "#").first.map(String.init) ?? s }

    private func infoRow(_ label: String, _ value: String, mono: Bool, copy: String?, last: Bool = false) -> some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Text(label).font(.system(size: 12.5)).foregroundStyle(p.secondaryText).frame(width: 110, alignment: .leading)
                Text(value).font(mono ? .system(size: 12.5, design: .monospaced) : .system(size: 12.5)).lineLimit(1).truncationMode(.middle)
                    .textSelection(.enabled)
                Spacer()
                if let copy { IconButton(icon: "doc.on.doc", help: loc("common.copy"), size: 11) { copyToPasteboard(copy) } }
            }
            .padding(.horizontal, 16).frame(height: 36)
            if !last { Rectangle().fill(p.separator).frame(height: 1).padding(.leading, 16) }
        }
    }

    private var sessions: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                SectionLabel(text: loc("status.sessions"))
                Spacer()
                if !model.status.sessions.isEmpty {
                    if model.aiActive {
                        Button(loc("status.stopAi")) { model.stopAi() }.buttonStyle(SecondaryButtonStyle(destructive: true))
                    }
                    Button(loc("status.disconnectAll")) { model.disconnectAll() }.buttonStyle(PrimaryButtonStyle(destructive: true))
                }
            }
            if model.status.sessions.isEmpty {
                Card { Text(loc("status.noSessions")).font(.system(size: 12.5)).foregroundStyle(p.secondaryText) }
            } else {
                ForEach(model.status.sessions, id: \.id) { s in sessionRow(s) }
            }
        }
    }

    private func sessionRow(_ s: HostSessionInfo) -> some View {
        HStack(spacing: 12) {
            ZStack {
                Circle().fill((s.aiActive ? p.warning : p.accent).opacity(0.15)).frame(width: 34, height: 34)
                Image(systemName: s.aiActive ? "wand.and.stars" : (s.mode == .assist ? "person.line.dotted.person" : "person.fill"))
                    .foregroundStyle(s.aiActive ? p.warning : p.accent)
            }
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(s.fromUser.isEmpty ? loc("status.anonymous") : s.fromUser).font(.system(size: 13, weight: .medium))
                    if let a = model.account(for: s), !a.policy.notify {
                        Image(systemName: "bell.slash").font(.system(size: 10.5)).foregroundStyle(p.tertiaryText).help(loc("status.quietAccount"))
                    }
                }
                HStack(spacing: 6) {
                    Image(systemName: "desktopcomputer").font(.system(size: 10)).foregroundStyle(p.tertiaryText)
                    Text(WorkerModel.shortDevice(s.fromDevice)).font(.system(size: 11.5, design: .monospaced)).foregroundStyle(p.secondaryText)
                    Text("·").foregroundStyle(p.tertiaryText)
                    Text(loc(s.mode == .assist ? "session.mode.assist" : "session.mode.control")).font(.system(size: 11.5)).foregroundStyle(p.secondaryText)
                    Text("·").foregroundStyle(p.tertiaryText)
                    Text(loc(Self.pathKey(s.path))).font(.system(size: 11.5)).foregroundStyle(p.secondaryText)
                }
                if let act = model.aiActivity[s.id] {
                    Label(act, systemImage: "wand.and.stars").font(.system(size: 11.5)).foregroundStyle(p.warning).lineLimit(1)
                }
            }
            Spacer()
            VStack(alignment: .trailing, spacing: 6) {
                Text(loc("status.since", Fmt.time(s.startedAt))).font(.system(size: 11).monospacedDigit()).foregroundStyle(p.tertiaryText)
                Button(loc("status.disconnect")) { model.disconnect(s) }
                    .buttonStyle(SecondaryButtonStyle(destructive: true))
                    .disabled(model.disconnecting.contains(s.id))
            }
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 10).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(s.aiActive ? p.warning.opacity(0.5) : p.border, lineWidth: s.aiActive ? 1 : 0.6))
    }

    static func pathKey(_ k: PathKind) -> String {
        switch k {
        case .lan: return "path.lan"
        case .direct: return "path.direct"
        case .relayUdp: return "path.relayUdp"
        case .relayTcp: return "path.relayTcp"
        case .connecting: return "path.connecting"
        case .closed: return "path.closed"
        }
    }

    private var recent: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                SectionLabel(text: loc("status.recentEvents"))
                Spacer()
                if model.events.count > 5 {
                    Button(loc("status.allEvents")) { model.page = .events }.buttonStyle(.plain)
                        .font(.system(size: 12)).foregroundStyle(p.accent)
                }
            }
            Card(padding: 0) {
                VStack(spacing: 0) {
                    let list = Array(model.events.prefix(5))
                    ForEach(Array(list.enumerated()), id: \.offset) { i, e in
                        EventRow(event: e)
                        if i < list.count - 1 { Rectangle().fill(p.separator).frame(height: 1) }
                    }
                    if list.isEmpty { Text(loc("events.empty")).font(.system(size: 12.5)).foregroundStyle(p.tertiaryText).padding(14) }
                }
            }
        }
    }
}

// MARK: - 事件日志

struct EventRow: View {
    let event: HostEvent
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: warn ? "exclamationmark.triangle.fill" : "circle.fill")
                .font(.system(size: warn ? 11 : 6)).foregroundStyle(warn ? p.warning : p.tertiaryText.opacity(0.6)).frame(width: 14)
            Text(Fmt.date(event.at, zh: loc.isChinese)).font(.system(size: 11.5).monospacedDigit()).foregroundStyle(p.tertiaryText)
                .frame(width: 108, alignment: .leading)
            if event.ai {
                Text("AI").font(.system(size: 9.5, weight: .bold)).foregroundStyle(p.warning)
                    .padding(.horizontal, 4).padding(.vertical, 1).background(RoundedRectangle(cornerRadius: 3).fill(p.warning.opacity(0.14)))
            }
            Text(kindLabel).font(.system(size: 11.5, weight: .medium)).foregroundStyle(warn ? p.warning : p.secondaryText)
            Text(event.detail).font(.system(size: 12.5)).foregroundStyle(p.text).lineLimit(2)
            Spacer(minLength: 6)
            if !event.user.isEmpty { Text(event.user).font(.system(size: 11.5)).foregroundStyle(p.secondaryText).lineLimit(1) }
        }
        .padding(.horizontal, 14).padding(.vertical, 8)
    }
    private var warn: Bool { event.level == .warning }
    private var kindLabel: String {
        let key = "event." + event.kind
        let s = loc(key)
        return s == key ? event.kind : s
    }
}

struct EventsPage: View {
    @EnvironmentObject var model: WorkerModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            PageTitle(text: loc("page.events"), info: loc("events.info")) {
                IconButton(icon: "arrow.clockwise", help: loc("common.refresh")) { model.refresh() }
            }
            Picker("", selection: $model.eventFilter) {
                Text(loc("events.all")).tag("all")
                Text("AI").tag("ai")
                Text(loc("events.warnings")).tag("warning")
                Text(loc("events.connect")).tag("connect")
                Text(loc("events.file")).tag("file")
                Text(loc("events.terminal")).tag("terminal")
            }
            .pickerStyle(.segmented).labelsHidden().frame(maxWidth: 440)
            Card(padding: 0) {
                VStack(spacing: 0) {
                    let list = model.filteredEvents
                    ForEach(Array(list.enumerated()), id: \.offset) { i, e in
                        EventRow(event: e)
                        if i < list.count - 1 { Rectangle().fill(p.separator).frame(height: 1) }
                    }
                    if list.isEmpty { Text(loc("events.empty")).font(.system(size: 12.5)).foregroundStyle(p.tertiaryText).padding(16) }
                }
            }
        }
    }
}
