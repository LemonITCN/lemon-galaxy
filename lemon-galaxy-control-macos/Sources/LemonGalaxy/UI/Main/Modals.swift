import SwiftUI

/// 主窗口内的模态卡片层
struct ModalOverlay: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p

    var body: some View {
        if let m = app.modal {
            ZStack {
                Color.black.opacity(p.isDark ? 0.45 : 0.25)
                    // 内核在等回答的卡片（密码、主机密钥）不能点空白处关掉
                    .onTapGesture { if m.dismissOnBackground { app.modal = nil } }
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
        case .hostKey(let r): HostKeyCard(request: r).frame(width: 460)
        case .update: UpdateCard(model: app.update).frame(width: 480)
        case .assistConnect: AssistConnectCard().frame(width: 400)
        case .terminalSessions(let id, let list): TerminalSessionsCard(deviceId: id, sessions: list).frame(width: 440)
        case .confirm(let c): ConfirmCard(request: c).frame(width: 380)
        case .portForwardEditor(let d): PortForwardEditorCard(draft: d).frame(width: 480)
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
                    Spacer()
                    if app.canWake(deviceId) {
                        WakeButton(deviceId: deviceId, compact: true)
                    }
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

/// SSH 的密码 / 私钥口令 / 键盘交互问题（echo 为假时遮挡输入）
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
                Button(loc("common.cancel")) { request.respond(nil); app.modal = nil }
                    .buttonStyle(SecondaryButtonStyle()).keyboardShortcut(.cancelAction)
                Button(loc("common.ok"), action: submit).buttonStyle(PrimaryButtonStyle()).keyboardShortcut(.defaultAction)
            }
        }
        .padding(18)
    }

    private func submit() {
        request.respond(value)
        app.modal = nil
        value = ""
    }
}

/// SSH 主机密钥确认：显示密钥类型与 SHA256 指纹。
/// 首次连接是普通确认；与加密库记录不一致（changed）时用警告样式，回车与 Esc 都是「取消」，信任要明确点按钮。
struct HostKeyCard: View {
    let request: HostKeyRequest
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        let changed = request.changed
        let address = request.port == 22 ? request.host : "\(request.host):\(request.port)"
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                Image(systemName: changed ? "exclamationmark.triangle.fill" : "key.horizontal")
                    .font(.system(size: 18)).foregroundStyle(changed ? p.danger : p.accent)
                VStack(alignment: .leading, spacing: 2) {
                    Text(loc(changed ? "hostkey.changed.title" : "hostkey.new.title"))
                        .font(.system(size: 14, weight: .semibold)).foregroundStyle(changed ? p.danger : p.text)
                    Text(request.title).font(.system(size: 11.5)).foregroundStyle(p.secondaryText)
                }
            }
            Text(loc(changed ? "hostkey.changed.message" : "hostkey.new.message", address))
                .font(.system(size: 12.5)).foregroundStyle(changed ? p.danger : p.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
            VStack(alignment: .leading, spacing: 9) {
                field(loc("hostkey.host"), address)
                field(loc("hostkey.type"), request.keyType)
                field(loc("hostkey.fingerprint"), request.fingerprint.hasPrefix("SHA256:") ? String(request.fingerprint.dropFirst(7)) : request.fingerprint)
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 8).fill(changed ? p.danger.opacity(0.08) : p.elevated))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(changed ? p.danger.opacity(0.55) : p.border, lineWidth: changed ? 1 : 0.6))
            if changed {
                Label(loc("hostkey.changed.hint"), systemImage: "exclamationmark.shield")
                    .font(.system(size: 12, weight: .medium)).foregroundStyle(p.danger)
            }
            HStack {
                Spacer()
                if changed {
                    Button(loc("hostkey.trustAnyway")) { answer(true) }.buttonStyle(SecondaryButtonStyle(destructive: true))
                    Button(loc("common.cancel")) { answer(false) }
                        .buttonStyle(PrimaryButtonStyle()).keyboardShortcut(.defaultAction)
                } else {
                    Button(loc("common.cancel")) { answer(false) }.buttonStyle(SecondaryButtonStyle()).keyboardShortcut(.cancelAction)
                    Button(loc("hostkey.trust")) { answer(true) }.buttonStyle(PrimaryButtonStyle()).keyboardShortcut(.defaultAction)
                }
            }
        }
        .padding(18)
        .onExitCommand { answer(false) }
    }

    private func field(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.system(size: 11)).foregroundStyle(p.tertiaryText)
            Text(value).font(.system(size: 12.5, design: .monospaced)).foregroundStyle(p.text)
                .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
        }
    }

    private func answer(_ trust: Bool) {
        request.respond(trust)
        app.modal = nil
    }
}

/// 新版本：版本号、更新说明、下载进度；下载完打开 DMG，提示拖进「应用程序」替换
struct UpdateCard: View {
    @ObservedObject var model: UpdateModel
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                Image(systemName: "arrow.down.circle.fill").font(.system(size: 22)).foregroundStyle(p.accent)
                VStack(alignment: .leading, spacing: 2) {
                    Text(loc("update.title")).font(.system(size: 14, weight: .semibold))
                    if let info {
                        Text(loc("update.versions", AppInfo.version, info.version) + " · " + Fmt.bytes(info.size))
                            .font(.system(size: 11.5)).foregroundStyle(p.secondaryText)
                    }
                }
                Spacer()
                if !model.busy { IconButton(icon: "xmark", size: 11) { close() } }
            }
            if let info {
                VStack(alignment: .leading, spacing: 6) {
                    SectionLabel(text: loc("update.notes"))
                    ScrollView {
                        Text(info.notes.isEmpty ? loc("update.noNotes") : info.notes)
                            .font(.system(size: 12.5)).foregroundStyle(info.notes.isEmpty ? p.tertiaryText : p.text)
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .frame(maxHeight: 180)
                    .padding(10)
                    .background(RoundedRectangle(cornerRadius: 8).fill(p.elevated))
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(p.border, lineWidth: 0.5))
                }
            }
            status
            HStack {
                Spacer()
                buttons
            }
        }
        .padding(18)
    }

    private var info: UpdateInfo? {
        switch model.state {
        case .available(let i), .downloading(let i, _, _), .downloaded(let i, _): return i
        default: return model.available
        }
    }

    @ViewBuilder
    private var status: some View {
        switch model.state {
        case .downloading(_, let done, let total):
            VStack(alignment: .leading, spacing: 6) {
                ProgressView(value: total == 0 ? 0 : min(1, Double(done) / Double(total)))
                Text(loc("update.downloading", Fmt.bytes(done), Fmt.bytes(total)))
                    .font(.system(size: 11.5).monospacedDigit()).foregroundStyle(p.secondaryText)
            }
        case .downloaded(_, let path):
            HStack(alignment: .top, spacing: 8) {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(p.success)
                Text(loc("update.downloaded", (path as NSString).lastPathComponent))
                    .font(.system(size: 12.5)).fixedSize(horizontal: false, vertical: true)
            }
        case .failed(let msg):
            HStack(alignment: .top, spacing: 8) {
                Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(p.danger)
                Text(msg).font(.system(size: 12.5)).foregroundStyle(p.danger).fixedSize(horizontal: false, vertical: true)
            }
        default:
            Text(loc("update.verified")).font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
        }
    }

    @ViewBuilder
    private var buttons: some View {
        switch model.state {
        case .available:
            Button(loc("update.later")) { close() }.buttonStyle(SecondaryButtonStyle()).keyboardShortcut(.cancelAction)
            Button(loc("update.download")) { model.download() }.buttonStyle(PrimaryButtonStyle()).keyboardShortcut(.defaultAction)
        case .downloading:
            Button(loc("update.background")) { app.modal = nil }.buttonStyle(SecondaryButtonStyle())
        case .downloaded:
            Button(loc("update.reveal")) { model.revealDownloaded() }.buttonStyle(SecondaryButtonStyle())
            Button(loc("update.done")) { close() }.buttonStyle(PrimaryButtonStyle()).keyboardShortcut(.defaultAction)
        case .failed:
            Button(loc("common.close")) { close() }.buttonStyle(SecondaryButtonStyle()).keyboardShortcut(.cancelAction)
            if model.available != nil {
                Button(loc("common.retry")) { model.reset(); model.download() }.buttonStyle(PrimaryButtonStyle())
            }
        default:
            Button(loc("common.close")) { close() }.buttonStyle(SecondaryButtonStyle())
        }
    }

    private func close() {
        app.modal = nil
        if case .downloaded = model.state { return }
        model.reset()
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
