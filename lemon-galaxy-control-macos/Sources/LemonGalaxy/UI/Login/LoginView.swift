import SwiftUI

/// 服务器地址：host:port#sha256-指纹（security.md 第 7 节）。host 可以是域名、IPv4 或 [IPv6]。
struct ServerAddress: Equatable {
    let host: String
    let port: Int
    let fingerprint: String

    static func parse(_ raw: String) -> ServerAddress? {
        let s = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let hashIdx = s.firstIndex(of: "#") else { return nil }
        let hostPort = String(s[..<hashIdx])
        let fp = String(s[s.index(after: hashIdx)...])
        guard fp.hasPrefix("sha256-") else { return nil }
        let body = fp.dropFirst("sha256-".count)
        // SHA-256 摘要 base64 / base64url 编码：43 或 44 个字符
        let allowed = CharacterSet(charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/-_=")
        guard (43...44).contains(body.count), body.unicodeScalars.allSatisfy(allowed.contains) else { return nil }
        var host = ""
        var portStr = ""
        if hostPort.hasPrefix("[") {
            guard let close = hostPort.firstIndex(of: "]") else { return nil }
            host = String(hostPort[hostPort.index(after: hostPort.startIndex)..<close])
            let rest = hostPort[hostPort.index(after: close)...]
            guard rest.hasPrefix(":") else { return nil }
            portStr = String(rest.dropFirst())
            guard host.contains(":") else { return nil }
        } else {
            guard let colon = hostPort.lastIndex(of: ":") else { return nil }
            host = String(hostPort[..<colon])
            portStr = String(hostPort[hostPort.index(after: colon)...])
            let hostChars = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-")
            guard !host.isEmpty, host.unicodeScalars.allSatisfy(hostChars.contains), !host.hasPrefix("."), !host.hasPrefix("-") else { return nil }
        }
        guard let port = Int(portStr), (1...65535).contains(port) else { return nil }
        return ServerAddress(host: host, port: port, fingerprint: fp)
    }
}

@MainActor
final class LoginModel: ObservableObject {
    enum Step { case credentials, totp, changePassword }
    @Published var step: Step = .credentials
    @Published var server = ""
    @Published var username = ""
    @Published var password = ""
    @Published var totp = ""
    @Published var newPassword = ""
    @Published var confirmPassword = ""
    @Published var busy = false
    @Published var error: String?
    @Published var rememberServer = true
}

struct LoginView: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var settings: AppSettings
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @StateObject var model = LoginModel()
    var preset: ((LoginModel) -> Void)? = nil

    var body: some View {
        ZStack {
            background
            VStack(spacing: 22) {
                brand
                VStack(alignment: .leading, spacing: 16) {
                    switch model.step {
                    case .credentials: credentials
                    case .totp: totpStep
                    case .changePassword: changePasswordStep
                    }
                    if let e = model.error {
                        HStack(alignment: .top, spacing: 6) {
                            Image(systemName: "exclamationmark.circle.fill").foregroundStyle(p.danger)
                            Text(e).font(.system(size: 12)).foregroundStyle(p.danger).fixedSize(horizontal: false, vertical: true)
                        }
                    }
                }
                .padding(24)
                .frame(width: 380)
                .background(RoundedRectangle(cornerRadius: 14).fill(p.card).shadow(color: .black.opacity(p.isDark ? 0.4 : 0.08), radius: 20, y: 6))
                .overlay(RoundedRectangle(cornerRadius: 14).stroke(p.border, lineWidth: 0.5))
                footer
            }
        }
        .onAppear {
            if model.server.isEmpty { model.server = settings.lastServer }
            if model.username.isEmpty { model.username = settings.lastUsername }
            // 模拟内核：预填演示地址，方便直接体验（任意密码可登录，用户名含 totp / new 可演示两步验证与首次改密码）
            if app.core.isMock && model.server.isEmpty && !app.selfTest {
                model.server = "demo.lemongalaxy.test:7443#sha256-TW9ja0NvcmVEZW1vU2VydmVyRmluZ2VycHJpbnQwMDA"
                if model.username.isEmpty { model.username = "alice" }
            }
            preset?(model)
        }
    }

    private var background: some View {
        ZStack {
            p.content
            // 克制的背景：极淡的强调色径向渐变
            RadialGradient(colors: [p.accent.opacity(p.isDark ? 0.16 : 0.10), .clear], center: .topLeading, startRadius: 0, endRadius: 700)
            RadialGradient(colors: [p.accent.opacity(p.isDark ? 0.08 : 0.05), .clear], center: .bottomTrailing, startRadius: 0, endRadius: 600)
        }
    }

    private var brand: some View {
        VStack(spacing: 10) {
            AppMark(size: 52)
            Text("LemonGalaxy").font(.system(size: 22, weight: .semibold)).foregroundStyle(p.text)
        }
    }

    private var footer: some View {
        HStack(spacing: 14) {
            Menu {
                Button(loc("settings.language.system")) { settings.language = "system" }
                ForEach(Localization.shared.packs) { pack in
                    Button(pack.name) { settings.language = pack.code }
                }
            } label: {
                HStack(spacing: 4) {
                    Image(systemName: "globe")
                    Text(Localization.shared.strings.pack.name)
                }
                .font(.system(size: 12)).foregroundStyle(p.secondaryText)
            }
            .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
            if app.core.isMock {
                Text(loc("about.mock")).font(.system(size: 11)).foregroundStyle(p.warning)
                    .padding(.horizontal, 6).padding(.vertical, 2)
                    .background(Capsule().fill(p.warning.opacity(0.12)))
            }
        }
    }

    // MARK: 第一步：服务器、用户名、密码

    private var serverValid: Bool { ServerAddress.parse(model.server) != nil }

    private var credentials: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(loc("login.title")).font(.system(size: 16, weight: .semibold))
            field(loc("login.server"), info: loc("login.server.info")) {
                TextField("galaxy.example.com:7443#sha256-…", text: $model.server)
                    .fieldStyle(invalid: !model.server.isEmpty && !serverValid)
                    .font(.system(size: 12.5, design: .monospaced))
            }
            if !model.server.isEmpty && !serverValid {
                Text(loc("login.server.invalid")).font(.system(size: 11.5)).foregroundStyle(p.danger)
            }
            field(loc("login.username"), info: nil) {
                TextField("", text: $model.username).fieldStyle()
            }
            field(loc("login.password"), info: nil) {
                SecureField("", text: $model.password).fieldStyle().onSubmit(login)
            }
            Toggle(loc("login.remember"), isOn: $model.rememberServer).toggleStyle(.checkbox).font(.system(size: 12))
            Button(action: login) {
                HStack {
                    if model.busy { ProgressView().controlSize(.small).tint(p.accentText) }
                    Text(loc("login.signIn"))
                }
                .frame(maxWidth: .infinity)
            }
            .buttonStyle(PrimaryButtonStyle())
            .keyboardShortcut(.defaultAction)
            .disabled(model.busy || !serverValid || model.username.isEmpty || model.password.isEmpty)
        }
    }

    // MARK: 第二步：两步验证

    private var totpStep: some View {
        VStack(alignment: .leading, spacing: 14) {
            stepHeader(loc("login.totp.title"), icon: "lock.shield")
            Text(loc("login.totp.prompt")).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
            TextField("000000", text: $model.totp)
                .fieldStyle()
                .font(.system(size: 20, weight: .medium, design: .monospaced))
                .multilineTextAlignment(.center)
                .onSubmit(login)
            HStack {
                Button(loc("common.back")) { model.step = .credentials; model.totp = ""; model.error = nil }
                    .buttonStyle(SecondaryButtonStyle())
                Spacer()
                Button(loc("login.verify"), action: login)
                    .buttonStyle(PrimaryButtonStyle())
                    .keyboardShortcut(.defaultAction)
                    .disabled(model.busy || model.totp.filter(\.isNumber).count != 6)
            }
        }
    }

    // MARK: 第三步：首次登录改密码

    private var passwordStrength: Int {
        let s = model.newPassword
        var score = 0
        if s.count >= 8 { score += 1 }
        if s.count >= 12 { score += 1 }
        if s.contains(where: \.isNumber) && s.contains(where: \.isLetter) { score += 1 }
        if s.contains(where: { !$0.isLetter && !$0.isNumber }) { score += 1 }
        return score
    }

    private var changePasswordStep: some View {
        VStack(alignment: .leading, spacing: 14) {
            stepHeader(loc("login.change.title"), icon: "key")
            HStack(spacing: 6) {
                Text(loc("login.change.prompt")).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                InfoButton(text: loc("login.change.info"))
            }
            field(loc("login.change.new"), info: nil) {
                SecureField("", text: $model.newPassword).fieldStyle()
            }
            HStack(spacing: 4) {
                ForEach(0..<4) { i in
                    RoundedRectangle(cornerRadius: 2).fill(i < passwordStrength ? strengthColor : p.border).frame(height: 4)
                }
            }
            field(loc("login.change.confirm"), info: nil) {
                SecureField("", text: $model.confirmPassword).fieldStyle(invalid: !model.confirmPassword.isEmpty && model.confirmPassword != model.newPassword)
                    .onSubmit(changePassword)
            }
            Button(action: changePassword) {
                HStack {
                    if model.busy { ProgressView().controlSize(.small) }
                    Text(loc("login.change.submit"))
                }
                .frame(maxWidth: .infinity)
            }
            .buttonStyle(PrimaryButtonStyle())
            .keyboardShortcut(.defaultAction)
            .disabled(model.busy || model.newPassword.count < 8 || model.newPassword != model.confirmPassword)
        }
    }

    private var strengthColor: Color {
        switch passwordStrength {
        case 0, 1: return p.danger
        case 2: return p.warning
        default: return p.success
        }
    }

    private func stepHeader(_ title: String, icon: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: icon).font(.system(size: 16)).foregroundStyle(p.accent)
            Text(title).font(.system(size: 16, weight: .semibold))
        }
    }

    private func field<C: View>(_ label: String, info: String?, @ViewBuilder _ content: () -> C) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 4) {
                Text(label).font(.system(size: 12)).foregroundStyle(p.secondaryText)
                if let info { InfoButton(text: info) }
            }
            content()
        }
    }

    // MARK: 动作

    private func login() {
        guard !model.busy else { return }
        model.busy = true
        model.error = nil
        let totp = model.step == .totp ? model.totp.filter(\.isNumber) : nil
        Task {
            defer { model.busy = false }
            do {
                let r = try await app.client.login(server: model.server.trimmingCharacters(in: .whitespaces),
                                                   username: model.username, password: model.password, totp: totp)
                switch r {
                case .success:
                    remember()
                    app.didLogin()
                case .needTotp:
                    model.step = .totp
                case .mustChangePassword:
                    model.step = .changePassword
                }
            } catch {
                model.error = AppModel.describe(error)
            }
        }
    }

    private func changePassword() {
        guard model.newPassword == model.confirmPassword, model.newPassword.count >= 8 else { return }
        model.busy = true
        model.error = nil
        Task {
            defer { model.busy = false }
            do {
                try await app.client.completeFirstLogin(newPassword: model.newPassword)
                remember()
                app.didLogin()
            } catch {
                model.error = AppModel.describe(error)
            }
        }
    }

    private func remember() {
        settings.lastUsername = model.username
        settings.lastServer = model.rememberServer ? model.server : ""
        model.password = ""
        model.newPassword = ""
        model.confirmPassword = ""
    }
}

/// 应用标志：深色圆角方块 + 黄绿渐变的星轨弧线（矢量自绘）
struct AppMark: View {
    var size: CGFloat = 40
    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: size * 0.24)
                .fill(LinearGradient(colors: [Color(hex: "#1B1F2A"), Color(hex: "#0E1016")], startPoint: .top, endPoint: .bottom))
            GalaxyArcs()
                .stroke(LinearGradient(colors: [Color(hex: "#F5D90A"), Color(hex: "#7BD35A")], startPoint: .topLeading, endPoint: .bottomTrailing),
                        style: StrokeStyle(lineWidth: size * 0.055, lineCap: .round))
                .padding(size * 0.2)
            Circle().fill(Color(hex: "#F5D90A")).frame(width: size * 0.13, height: size * 0.13)
        }
        .frame(width: size, height: size)
    }
}

struct GalaxyArcs: Shape {
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
