import SwiftUI

/// 主窗口内的模态卡片层
struct ModalOverlay: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p

    var body: some View {
        if let m = app.modal {
            ZStack {
                Color.black.opacity(p.isDark ? 0.45 : 0.25)
                    .onTapGesture { if case .secretPrompt = m {} else { app.modal = nil } }
                card(m)
                    .background(RoundedRectangle(cornerRadius: 12).fill(p.card).shadow(color: .black.opacity(0.25), radius: 24, y: 8))
                    .overlay(RoundedRectangle(cornerRadius: 12).stroke(p.border, lineWidth: 0.5))
            }
        }
    }

    @ViewBuilder
    private func card(_ m: ModalKind) -> some View {
        switch m {
        case .connectChooser(let id): ConnectChooser(deviceId: id).frame(width: 440)
        case .secretPrompt(let r): SecretPromptCard(request: r).frame(width: 380)
        case .assistConnect: AssistConnectCard().frame(width: 400)
        case .terminalSessions(let id, let list): TerminalSessionsCard(deviceId: id, sessions: list).frame(width: 440)
        case .confirm(let c): ConfirmCard(request: c).frame(width: 380)
        }
    }
}

struct ModalHeader: View {
    let title: String
    var subtitle: String? = nil
    var icon: AnyView? = nil
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    var body: some View {
        HStack(spacing: 10) {
            if let icon { icon }
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.system(size: 14, weight: .semibold)).foregroundStyle(p.text)
                if let subtitle { Text(subtitle).font(.system(size: 11.5)).foregroundStyle(p.secondaryText) }
            }
            Spacer()
            IconButton(icon: "xmark", size: 11) { app.modal = nil }
        }
    }
}

/// 双击设备后的连接方式选择：远程桌面 / 终端 / 文件 / AI
struct ConnectChooser: View {
    let deviceId: String
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        let d = app.device(deviceId)
        let gui = d.map { $0.platform == .windows || $0.platform == .macos } ?? false
        VStack(alignment: .leading, spacing: 16) {
            ModalHeader(title: app.deviceName(deviceId),
                        subtitle: d.map { "\($0.hostname) · \($0.osVersion)" },
                        icon: AnyView(PlatformGlyph(platform: d?.platform ?? .linux, size: 22).foregroundStyle(p.secondaryText)))
            HStack(spacing: 10) {
                choice("display", loc("connect.desktop"), enabled: gui) { app.openRemoteDesktop(deviceId: deviceId) }
                choice("terminal", loc("connect.terminal"), enabled: true) { app.openTerminalAsking(deviceId: deviceId) }
                choice("folder", loc("connect.files"), enabled: true) { app.openSFTP(deviceId: deviceId) }
                choice("wand.and.stars", loc("connect.ai"), enabled: app.user?.aiEnabled ?? false) {
                    app.openAI(targets: [AgentTarget(kind: .device, id: deviceId)])
                }
            }
            if let d, !d.online {
                HStack(spacing: 6) {
                    StatusDot(state: StatusDot.of(d), size: 7)
                    Text(d.lanSeen ? loc("device.lanOnly") : loc("device.offline")).font(.system(size: 11.5)).foregroundStyle(p.secondaryText)
                }
            }
        }
        .padding(18)
    }

    private func choice(_ icon: String, _ title: String, enabled: Bool, _ action: @escaping () -> Void) -> some View {
        Button {
            app.modal = nil
            action()
        } label: {
            VStack(spacing: 9) {
                Image(systemName: icon).font(.system(size: 22, weight: .light)).frame(height: 26)
                Text(title).font(.system(size: 12, weight: .medium))
            }
            .foregroundStyle(enabled ? p.text : p.tertiaryText)
            .frame(maxWidth: .infinity).frame(height: 84)
            .background(RoundedRectangle(cornerRadius: 9).fill(p.elevated))
            .overlay(RoundedRectangle(cornerRadius: 9).stroke(p.border, lineWidth: 0.7))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
    }
}

/// SSH 的密码 / 口令 / 键盘交互问题，以及首次连接主机指纹确认
struct SecretPromptCard: View {
    let request: SecretPromptRequest
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var value = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                Image(systemName: "key.horizontal").font(.system(size: 18)).foregroundStyle(p.accent)
                Text(request.title).font(.system(size: 14, weight: .semibold))
            }
            Text(request.prompt).font(.system(size: 12.5)).foregroundStyle(p.secondaryText).fixedSize(horizontal: false, vertical: true)
            Group {
                if request.echo { TextField("", text: $value) } else { SecureField("", text: $value) }
            }
            .fieldStyle()
            .onSubmit(submit)
            HStack {
                Spacer()
                Button(loc("common.cancel")) { app.modal = nil; request.respond(nil) }.buttonStyle(SecondaryButtonStyle())
                Button(loc("common.ok"), action: submit).buttonStyle(PrimaryButtonStyle()).keyboardShortcut(.defaultAction)
            }
        }
        .padding(18)
    }

    private func submit() {
        app.modal = nil
        request.respond(value)
        value = ""
    }
}

/// 临时协助：输入对方屏幕上的设备码与验证码
struct AssistConnectCard: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var deviceCode = ""
    @State private var code = ""
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 6) {
                Text(loc("assist.connect.title")).font(.system(size: 14, weight: .semibold))
                InfoButton(text: loc("assist.connect.info"))
                Spacer()
                IconButton(icon: "xmark", size: 11) { app.modal = nil }
            }
            VStack(alignment: .leading, spacing: 6) {
                Text(loc("assist.deviceCode")).font(.system(size: 12)).foregroundStyle(p.secondaryText)
                TextField("123 456 789", text: $deviceCode).fieldStyle().font(.system(size: 15, design: .monospaced))
            }
            VStack(alignment: .leading, spacing: 6) {
                Text(loc("assist.verificationCode")).font(.system(size: 12)).foregroundStyle(p.secondaryText)
                TextField("", text: $code).fieldStyle().font(.system(size: 15, design: .monospaced))
            }
            if let error { Text(error).font(.system(size: 12)).foregroundStyle(p.danger) }
            HStack {
                Spacer()
                Button(loc("common.cancel")) { app.modal = nil }.buttonStyle(SecondaryButtonStyle())
                Button(loc("assist.connect.go")) { connect() }
                    .buttonStyle(PrimaryButtonStyle())
                    .disabled(busy || deviceCode.filter(\.isNumber).count != 9 || code.count < 6)
            }
        }
        .padding(18)
    }

    private func connect() {
        busy = true
        error = nil
        Task {
            do {
                let s = try await app.client.connectAssist(deviceCode: deviceCode, verificationCode: code)
                let id = app.registerAssistPeer(s)
                app.modal = nil
                app.openRemoteDesktop(deviceId: id)
            } catch {
                self.error = AppModel.describe(error)
            }
            busy = false
        }
    }
}

/// 被控端保留的终端会话：选一个接续，或新开
struct TerminalSessionsCard: View {
    let deviceId: String
    let sessions: [TerminalSessionInfo]
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            ModalHeader(title: loc("terminal.sessions.title"), subtitle: app.deviceName(deviceId))
            VStack(spacing: 0) {
                ForEach(sessions, id: \.id) { s in
                    HStack(spacing: 10) {
                        Image(systemName: "terminal").foregroundStyle(p.secondaryText)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(s.title).font(.system(size: 12.5, weight: .medium)).lineLimit(1)
                            Text(loc("terminal.sessions.started", Fmt.date(s.createdAt, zh: loc.isChinese)))
                                .font(.system(size: 11)).foregroundStyle(p.tertiaryText)
                        }
                        Spacer()
                        Button(loc("terminal.sessions.attach")) {
                            app.modal = nil
                            if let tab = app.activeTab as? TerminalTabModel, tab.target == .device(deviceId), tab.phase != .running {
                                tab.reconnect(attachTo: s)
                            } else {
                                app.openTerminal(deviceId: deviceId, attach: s)
                            }
                        }
                        .buttonStyle(SecondaryButtonStyle())
                    }
                    .padding(.vertical, 8)
                    if s.id != sessions.last?.id { Divider() }
                }
            }
            .padding(.horizontal, 12)
            .background(RoundedRectangle(cornerRadius: 8).fill(p.elevated))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(p.border, lineWidth: 0.5))
            HStack {
                Spacer()
                Button(loc("terminal.sessions.new")) { app.modal = nil; app.openTerminal(deviceId: deviceId) }
                    .buttonStyle(PrimaryButtonStyle())
            }
        }
        .padding(18)
    }
}

struct ConfirmCard: View {
    let request: ConfirmRequest
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(request.title).font(.system(size: 14, weight: .semibold))
            Text(request.message).font(.system(size: 12.5)).foregroundStyle(p.secondaryText).fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button(loc("common.cancel")) { app.modal = nil }.buttonStyle(SecondaryButtonStyle()).keyboardShortcut(.cancelAction)
                Button(request.confirmTitle) { app.modal = nil; request.action() }
                    .buttonStyle(PrimaryButtonStyle(destructive: request.destructive))
            }
        }
        .padding(18)
    }
}

extension AppModel {
    func confirm(_ title: String, _ message: String, button: String, destructive: Bool = true, _ action: @escaping () -> Void) {
        modal = .confirm(ConfirmRequest(title: title, message: message, confirmTitle: button, destructive: destructive, action: action))
    }
}
