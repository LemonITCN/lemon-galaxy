import SwiftUI
import GalaxyKit

/// 登录：服务器地址（可扫码）+ 用户名 + 密码 → 需要时两步验证 → 首次登录改密码。
struct LoginView: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var account: GalaxyAccount
    @EnvironmentObject var settings: AppSettings
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    enum Step { case credentials, totp, changePassword }
    @State private var step: Step = .credentials
    @State private var server = ""
    @State private var username = ""
    @State private var password = ""
    @State private var totp = ""
    @State private var newPassword = ""
    @State private var confirmPassword = ""
    @State private var busy = false
    @State private var error: String?
    @State private var scanning = false
    @FocusState private var focus: Field?
    enum Field { case server, username, password, totp, newPassword, confirm }

    var body: some View {
        ScrollView {
            VStack(spacing: 26) {
                header
                VStack(spacing: 14) {
                    switch step {
                    case .credentials: credentials
                    case .totp: totpStep
                    case .changePassword: changeStep
                    }
                    if let error {
                        Label(error, systemImage: "exclamationmark.triangle.fill")
                            .font(.callout)
                            .foregroundStyle(p.danger)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
                .padding(20)
                .background(RoundedRectangle(cornerRadius: 20, style: .continuous).fill(p.card))
                .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous).stroke(p.border.opacity(0.6), lineWidth: 0.5))
                if app.isMock { mockBadge }
            }
            .padding(.horizontal, 22)
            .padding(.vertical, 40)
            .frame(maxWidth: 440)
            .frame(maxWidth: .infinity)
        }
        .scrollDismissesKeyboard(.interactively)
        .background(p.background.ignoresSafeArea())
        .onAppear {
            if server.isEmpty { server = settings.lastServer }
            if username.isEmpty { username = settings.lastUsername }
        }
        .sheet(isPresented: $scanning) {
            QRScannerSheet { text in
                scanning = false
                if let s = ServerAddress.parseScanned(text) {
                    server = s
                    error = nil
                    focus = .username
                } else {
                    error = loc("login.scanInvalid")
                }
            }
        }
    }

    private var header: some View {
        VStack(spacing: 12) {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(LinearGradient(colors: [Color(hex: "#0B0D12"), Color(hex: "#1A2133")], startPoint: .topLeading, endPoint: .bottomTrailing))
                .frame(width: 72, height: 72)
                .overlay(Image(systemName: "sparkles").font(.system(size: 30, weight: .medium)).foregroundStyle(Color(hex: "#C6E35A")))
                .shadow(color: .black.opacity(0.18), radius: 12, y: 6)
            Text("LemonGalaxy").font(.largeTitle.weight(.bold)).foregroundStyle(p.text)
            Text(loc(step == .credentials ? "login.title" : (step == .totp ? "login.totpTitle" : "login.changeTitle")))
                .font(.headline).foregroundStyle(p.secondaryText)
        }
    }

    private func field<V: View>(_ icon: String, @ViewBuilder _ content: () -> V) -> some View {
        HStack(spacing: 10) {
            Image(systemName: icon).foregroundStyle(p.tertiaryText).frame(width: 20)
            content()
        }
        .padding(.horizontal, 14)
        .frame(minHeight: 48)
        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(p.elevated))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(p.border, lineWidth: 0.5))
    }

    private var serverValid: Bool { ServerAddress.isValid(server) }

    @ViewBuilder private var credentials: some View {
        VStack(alignment: .leading, spacing: 6) {
            field("server.rack") {
                TextField(loc("login.serverPlaceholder"), text: $server)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.URL)
                    .font(.callout.monospaced())
                    .focused($focus, equals: .server)
                    .submitLabel(.next)
                    .onSubmit { focus = .username }
                Button { scanning = true } label: { Image(systemName: "qrcode.viewfinder").font(.title3) }
                    .accessibilityLabel(loc("login.scan"))
                InfoButton(text: loc("login.serverInfo"))
            }
            if !server.isEmpty && !serverValid {
                Text(loc("login.serverInvalid")).font(.caption).foregroundStyle(p.danger).padding(.leading, 4)
            }
        }
        field("person") {
            TextField(loc("login.username"), text: $username)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textContentType(.username)
                .focused($focus, equals: .username)
                .submitLabel(.next)
                .onSubmit { focus = .password }
        }
        field("lock") {
            SecretField(title: loc("login.password"), text: $password)
                .focused($focus, equals: .password)
                .submitLabel(.go)
                .onSubmit(signIn)
        }
        Button(action: signIn) {
            if busy { ProgressView().tint(p.accentText) } else { Text(loc("login.signIn")) }
        }
        .buttonStyle(PrimaryButtonStyle())
        .disabled(busy || !serverValid || username.isEmpty || password.isEmpty)
        .padding(.top, 4)
    }

    @ViewBuilder private var totpStep: some View {
        HStack {
            Text(loc("login.totpPrompt")).font(.callout).foregroundStyle(p.secondaryText)
            Spacer()
            InfoButton(text: loc("login.totpInfo"))
        }
        field("number") {
            TextField(loc("login.totpCode"), text: $totp)
                .keyboardType(.numberPad)
                .textContentType(.oneTimeCode)
                .font(.title3.monospacedDigit())
                .focused($focus, equals: .totp)
                .onChange(of: totp) { v in if v.count == 6, v.allSatisfy(\.isNumber) { signIn() } }
        }
        Button(action: signIn) {
            if busy { ProgressView().tint(p.accentText) } else { Text(loc("login.verify")) }
        }
        .buttonStyle(PrimaryButtonStyle())
        .disabled(busy || totp.count != 6)
        backButton
    }

    @ViewBuilder private var changeStep: some View {
        HStack {
            Text(loc("login.changePrompt")).font(.callout).foregroundStyle(p.secondaryText)
            Spacer()
            InfoButton(text: loc("login.changeInfo"))
        }
        field("lock.rotation") { SecretField(title: loc("login.newPassword"), text: $newPassword, contentType: .newPassword).focused($focus, equals: .newPassword) }
        field("lock.rotation") { SecretField(title: loc("login.confirmPassword"), text: $confirmPassword, contentType: .newPassword).focused($focus, equals: .confirm) }
        if let v = passwordValidation {
            Text(v).font(.caption).foregroundStyle(p.danger).frame(maxWidth: .infinity, alignment: .leading)
        }
        Button(action: completeFirstLogin) {
            if busy { ProgressView().tint(p.accentText) } else { Text(loc("login.changeSubmit")) }
        }
        .buttonStyle(PrimaryButtonStyle())
        .disabled(busy || newPassword.isEmpty || passwordValidation != nil || confirmPassword.isEmpty)
        backButton
    }

    private var backButton: some View {
        Button(loc("common.back")) {
            step = .credentials
            error = nil
            totp = ""
        }
        .font(.callout)
        .foregroundStyle(p.secondaryText)
    }

    private var passwordValidation: String? {
        if !newPassword.isEmpty && newPassword.count < 8 { return loc("password.tooShort") }
        if !confirmPassword.isEmpty && confirmPassword != newPassword { return loc("password.mismatch") }
        return nil
    }

    private var mockBadge: some View {
        HStack(spacing: 8) {
            Label(loc("login.mockBadge"), systemImage: "testtube.2").font(.caption.weight(.semibold)).foregroundStyle(p.warning)
            InfoButton(text: loc("login.mockInfo"), size: 13)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(Capsule().fill(p.warning.opacity(0.12)))
    }

    private func signIn() {
        guard !busy, let addr = ServerAddress.parse(server) else { return }
        busy = true
        error = nil
        let s = addr.text
        let u = username.trimmingCharacters(in: .whitespaces)
        Task {
            do {
                let r = try await account.login(server: s, username: u, password: password, totp: step == .totp ? totp : nil)
                settings.lastServer = s
                settings.lastUsername = u
                switch r {
                case .success: password = ""
                case .needTotp:
                    step = .totp
                    focus = .totp
                case .mustChangePassword:
                    step = .changePassword
                    focus = .newPassword
                }
            } catch {
                self.error = describeError(error)
                if step == .totp { totp = "" }
            }
            busy = false
        }
    }

    private func completeFirstLogin() {
        busy = true
        error = nil
        Task {
            do {
                try await account.completeFirstLogin(newPassword: newPassword)
                password = ""
                newPassword = ""
                confirmPassword = ""
            } catch { self.error = describeError(error) }
            busy = false
        }
    }
}
