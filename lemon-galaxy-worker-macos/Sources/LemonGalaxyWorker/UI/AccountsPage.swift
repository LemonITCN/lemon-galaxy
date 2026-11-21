import SwiftUI
import AppKit

// MARK: - 账号与权限

struct AccountsPage: View {
    @EnvironmentObject var model: WorkerModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            PageTitle(text: loc("page.accounts"), info: loc("accounts.info")) {
                if model.hasAccounts {
                    Button { model.openLogin() } label: { Label(loc("accounts.add"), systemImage: "plus") }
                        .buttonStyle(PrimaryButtonStyle())
                        .disabled(model.serviceStarting)
                }
            }
            if let server = model.serverAddress {
                serverCard(server)
            }
            if model.accounts.isEmpty {
                Card(padding: 24) {
                    VStack(spacing: 12) {
                        Image(systemName: "person.crop.circle.badge.plus").font(.system(size: 30, weight: .light)).foregroundStyle(p.tertiaryText)
                        Text(loc("accounts.empty")).font(.system(size: 13)).foregroundStyle(p.secondaryText)
                        Button(loc("accounts.signIn")) { model.openLogin() }.buttonStyle(PrimaryButtonStyle())
                    }
                    .frame(maxWidth: .infinity)
                }
            }
            ForEach(model.accounts, id: \.userId) { a in AccountCard(account: a) }
        }
    }

    private func serverCard(_ server: String) -> some View {
        Card {
            HStack(spacing: 12) {
                Image(systemName: "server.rack").font(.system(size: 16)).foregroundStyle(p.secondaryText).frame(width: 22)
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 5) {
                        Text(loc("accounts.server")).font(.system(size: 12)).foregroundStyle(p.secondaryText)
                        InfoButton(text: loc("accounts.server.info"))
                    }
                    Text(server).font(.system(size: 12, design: .monospaced)).lineLimit(1).truncationMode(.middle).textSelection(.enabled)
                }
                Spacer()
                IconButton(icon: "doc.on.doc", help: loc("common.copy"), size: 11) { copyToPasteboard(server) }
            }
        }
    }
}

struct AccountCard: View {
    let account: HostAccount
    @EnvironmentObject var model: WorkerModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        let a = account
        Card {
            VStack(alignment: .leading, spacing: 14) {
                HStack(spacing: 10) {
                    Circle().fill(p.accent.opacity(0.18)).frame(width: 34, height: 34)
                        .overlay(Text(String((a.displayName.isEmpty ? a.username : a.displayName).prefix(1)).uppercased())
                            .font(.system(size: 14, weight: .semibold)).foregroundStyle(p.accent))
                    VStack(alignment: .leading, spacing: 2) {
                        Text(a.displayName.isEmpty || a.displayName == a.username ? a.username : "\(a.displayName) (\(a.username))")
                            .font(.system(size: 13.5, weight: .semibold))
                        if a.status == .needsRelogin {
                            Label(loc("accounts.needsRelogin"), systemImage: "exclamationmark.triangle.fill").font(.system(size: 11.5)).foregroundStyle(p.warning)
                        } else {
                            Text(loc("accounts.active", liveCount)).font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
                        }
                    }
                    Spacer()
                    if a.status == .needsRelogin {
                        Button(loc("accounts.relogin")) { model.openLogin(username: a.username) }.buttonStyle(PrimaryButtonStyle())
                    }
                    if model.loggingOut.contains(a.userId) {
                        ProgressView().controlSize(.small)
                    } else {
                        Button(loc("accounts.signOut")) { model.confirmLogout = a }.buttonStyle(SecondaryButtonStyle(destructive: true))
                    }
                }
                Rectangle().fill(p.separator).frame(height: 1)
                HStack(alignment: .top, spacing: 28) {
                    VStack(spacing: 12) {
                        perm("view", \.view)
                        perm("files", \.files)
                        perm("clipboard", \.clipboard)
                    }
                    VStack(spacing: 12) {
                        perm("control", \.control)
                        perm("terminal", \.terminal)
                        perm("audio", \.audio)
                    }
                }
                Rectangle().fill(p.separator).frame(height: 1)
                HStack(spacing: 10) {
                    Image(systemName: "wand.and.stars").font(.system(size: 12)).foregroundStyle(p.secondaryText).frame(width: 18)
                    Text(loc("perm.aiCap")).font(.system(size: 13))
                    InfoButton(text: loc("perm.aiCap.info"))
                    Spacer()
                    Picker("", selection: Binding(get: { a.policy.aiCap }, set: { v in model.updatePolicy(a) { $0.aiCap = v } })) {
                        ForEach(AiMode.capChoices, id: \.self) { m in Text(loc(m.labelKey)).tag(m) }
                    }
                    .labelsHidden().fixedSize()
                }
                HStack(spacing: 10) {
                    Image(systemName: "bell").font(.system(size: 12)).foregroundStyle(p.secondaryText).frame(width: 18)
                    Text(loc("perm.notify")).font(.system(size: 13))
                    InfoButton(text: loc("perm.notify.info"))
                    Spacer()
                    Toggle("", isOn: Binding(get: { a.policy.notify }, set: { v in model.updatePolicy(a) { $0.notify = v } }))
                        .toggleStyle(PaletteSwitchStyle(small: true)).labelsHidden()
                }
                HStack(spacing: 10) {
                    Image(systemName: "person.badge.key").font(.system(size: 12)).foregroundStyle(p.secondaryText).frame(width: 18)
                    Text(loc("perm.systemUser")).font(.system(size: 13))
                    InfoButton(text: loc("perm.systemUser.info"))
                    Spacer()
                    Text(a.policy.systemUser.isEmpty ? AppInfo.userName : a.policy.systemUser)
                        .font(.system(size: 12.5, design: .monospaced)).foregroundStyle(p.secondaryText)
                }
            }
        }
        .opacity(model.loggingOut.contains(a.userId) ? 0.55 : 1)
    }

    /// 这个账号当前有几个连接
    private var liveCount: Int { model.status.sessions.filter { model.account(for: $0)?.userId == account.userId }.count }

    private func perm(_ key: String, _ kp: WritableKeyPath<HostPolicy, Bool>) -> some View {
        HStack {
            Image(systemName: Self.permIcon(key)).font(.system(size: 12)).foregroundStyle(p.secondaryText).frame(width: 18)
            Text(loc("perm." + key)).font(.system(size: 13))
            Spacer()
            Toggle("", isOn: Binding(get: { account.policy[keyPath: kp] }, set: { v in model.updatePolicy(account) { $0[keyPath: kp] = v } }))
                .toggleStyle(PaletteSwitchStyle(small: true)).labelsHidden()
        }
    }

    static func permIcon(_ k: String) -> String {
        ["view": "eye", "control": "cursorarrow.motionlines", "files": "folder", "terminal": "terminal",
         "clipboard": "doc.on.clipboard", "audio": "speaker.wave.2"][k] ?? "circle"
    }
}

// MARK: - 登录（窗口内浮层）

struct LoginFormView: View {
    @ObservedObject var form: LoginForm
    @EnvironmentObject var model: WorkerModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 10) {
                Image(systemName: stepIcon).font(.system(size: 18)).foregroundStyle(p.accent)
                Text(stepTitle).font(.system(size: 16, weight: .semibold))
                Spacer()
            }
            switch form.step {
            case .credentials: credentials
            case .totp: totpStep
            case .changePassword: changeStep
            }
            if let e = form.error {
                HStack(alignment: .top, spacing: 6) {
                    Image(systemName: "exclamationmark.circle.fill").foregroundStyle(p.danger)
                    Text(e).font(.system(size: 12)).foregroundStyle(p.danger).fixedSize(horizontal: false, vertical: true)
                }
            }
            HStack {
                if form.busy { ProgressView().controlSize(.small) }
                Spacer()
                Button(loc("common.cancel")) { model.loginForm = nil }
                    .buttonStyle(SecondaryButtonStyle()).keyboardShortcut(.cancelAction)
                Button(submitTitle) { model.submitLogin(form) }
                    .buttonStyle(PrimaryButtonStyle()).keyboardShortcut(.defaultAction)
                    .disabled(!form.canSubmit || form.busy)
            }
        }
        .padding(22)
        .frame(width: 420)
    }

    private var stepIcon: String {
        switch form.step {
        case .credentials: return "person.crop.circle.badge.plus"
        case .totp: return "lock.rotation"
        case .changePassword: return "key"
        }
    }

    private var stepTitle: String {
        switch form.step {
        case .credentials: return loc("login.title")
        case .totp: return loc("login.totp.title")
        case .changePassword: return loc("login.change.title")
        }
    }

    private var submitTitle: String {
        switch form.step {
        case .credentials: return loc("login.signIn")
        case .totp: return loc("login.verify")
        case .changePassword: return loc("login.change.submit")
        }
    }

    private func label(_ text: String, info: String? = nil) -> some View {
        HStack(spacing: 4) {
            Text(text).font(.system(size: 12)).foregroundStyle(p.secondaryText)
            if let info { InfoButton(text: info) }
        }
    }

    private var credentials: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 5) {
                label(loc("login.server"), info: loc(form.serverLocked ? "login.server.locked" : form.serverInUseOnly ? "login.server.inUse" : "login.server.info"))
                TextField("host:7443#sha256-…", text: $form.server)
                    .font(.system(size: 12, design: .monospaced))
                    .fieldStyle(invalid: !form.server.isEmpty && !LoginForm.looksLikeServer(form.server.trimmingCharacters(in: .whitespacesAndNewlines)))
                    .disabled(form.serverLocked)
                    .opacity(form.serverLocked ? 0.7 : 1)
            }
            VStack(alignment: .leading, spacing: 5) {
                label(loc("login.username"))
                TextField("", text: $form.username).fieldStyle()
            }
            VStack(alignment: .leading, spacing: 5) {
                label(loc("login.password"))
                SecureField("", text: $form.password).fieldStyle()
            }
            HStack(spacing: 6) {
                Image(systemName: "person.badge.key").font(.system(size: 11)).foregroundStyle(p.tertiaryText)
                Text(loc("login.systemUser", AppInfo.userName)).font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
                InfoButton(text: loc("perm.systemUser.info"))
            }
        }
    }

    private var totpStep: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(loc("login.totp.prompt", form.username)).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
            TextField("000000", text: $form.totp)
                .font(.system(size: 15, weight: .medium, design: .monospaced))
                .fieldStyle()
                .frame(width: 160)
                .onChange(of: form.totp) { v in
                    let digits = String(v.filter(\.isNumber).prefix(8))
                    if digits != v { form.totp = digits }
                }
        }
    }

    private var changeStep: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 4) {
                Text(loc("login.change.prompt")).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                InfoButton(text: loc("login.change.info"))
            }
            VStack(alignment: .leading, spacing: 5) {
                label(loc("login.change.new"))
                SecureField("", text: $form.newPassword).fieldStyle(invalid: !form.newPassword.isEmpty && form.newPassword.count < 8)
            }
            VStack(alignment: .leading, spacing: 5) {
                label(loc("login.change.confirm"))
                SecureField("", text: $form.confirmPassword)
                    .fieldStyle(invalid: !form.confirmPassword.isEmpty && form.confirmPassword != form.newPassword)
            }
        }
    }
}

struct LogoutConfirmView: View {
    let account: HostAccount
    @EnvironmentObject var model: WorkerModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                Image(systemName: "rectangle.portrait.and.arrow.right").font(.system(size: 18)).foregroundStyle(p.danger)
                Text(loc("accounts.signOut.title", account.username)).font(.system(size: 15, weight: .semibold))
            }
            Text(loc("accounts.signOut.body")).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button(loc("common.cancel")) { model.confirmLogout = nil }.buttonStyle(SecondaryButtonStyle()).keyboardShortcut(.cancelAction)
                Button(loc("accounts.signOut")) { model.logout(account) }.buttonStyle(PrimaryButtonStyle(destructive: true))
            }
        }
        .padding(22)
        .frame(width: 400)
    }
}
