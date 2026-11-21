import SwiftUI
import LocalAuthentication
import GalaxyKit

struct SettingsView: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var account: GalaxyAccount
    @EnvironmentObject var settings: AppSettings
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var confirmLogout = false
    @State private var changePassword = false
    @State private var totpSheet = false
    @State private var shareLog: URL?

    var body: some View {
        Form {
            accountSection
            appearanceSection
            Section(loc("settings.terminal")) {
                NavigationLink {
                    TerminalThemePicker()
                } label: {
                    LabeledContent(loc("settings.terminalTheme"), value: TerminalTheme.named(settings.terminalTheme).name)
                }
                Stepper(value: $settings.terminalFontSize, in: AppSettings.fontRange, step: 1) {
                    LabeledContent(loc("settings.fontSize"), value: "\(Int(settings.terminalFontSize)) pt")
                }
                Toggle(loc("terminal.inputBox"), isOn: $settings.terminalInputBox)
                Toggle(loc("settings.askResume"), isOn: $settings.askResumeTerminal)
            }
            .galaxyRow()
            Section(loc("settings.language")) {
                Picker(loc("settings.language"), selection: Binding(get: { settings.language }, set: { app.setLanguage($0) })) {
                    Text(loc("lang.system")).tag("system")
                    ForEach(Localization.shared.packs) { pack in Text(pack.name).tag(pack.code) }
                }
            }
            .galaxyRow()
            Section {
                Toggle(isOn: Binding(get: { settings.appLock }, set: setAppLock)) {
                    Label(loc("settings.appLock"), systemImage: "faceid")
                }
            } header: {
                Text(loc("settings.security"))
            } footer: {
                HStack { Spacer(); InfoButton(text: loc("settings.appLockInfo"), size: 13) }
            }
            .galaxyRow()
            Section(loc("settings.defaults")) {
                Picker(loc("settings.defaultQuality"), selection: $settings.defaultQuality) {
                    ForEach(VideoQuality.allCases) { Text(loc("quality." + $0.rawValue)).tag($0) }
                }
                Picker(loc("settings.defaultInput"), selection: $settings.remoteMouseMode) {
                    Text(loc("remote.modeTouch")).tag(false)
                    Text(loc("remote.modeMouse")).tag(true)
                }
                Picker(loc("settings.defaultAiMode"), selection: $settings.defaultAiMode) {
                    ForEach(AiMode.selectable(maxMode: .fullTrust), id: \.self) { Text(loc("mode." + $0.key)).tag($0) }
                }
            }
            .galaxyRow()
            aboutSection
        }
        .galaxyList()
        .navigationTitle(loc("nav.settings"))
        .sheet(isPresented: $changePassword) { ChangePasswordSheet().environmentObject(app).environmentObject(account) }
        .sheet(isPresented: $totpSheet) { TotpSheet().environmentObject(app).environmentObject(account) }
        .sheet(item: Binding(get: { shareLog.map { ShareItem(url: $0) } }, set: { if $0 == nil { shareLog = nil } })) { s in ShareSheet(items: [s.url]) }
        .confirmationDialog(loc("settings.logout"), isPresented: $confirmLogout, titleVisibility: .visible) {
            Button(loc("settings.logout"), role: .destructive) { Task { await account.logout() } }
        } message: { Text(loc("logout.message")) }
    }

    private var accountSection: some View {
        Section(loc("settings.account")) {
            if let u = account.user {
                HStack(spacing: 14) {
                    Circle().fill(p.accent.opacity(0.18)).frame(width: 46, height: 46)
                        .overlay(Text(String(u.displayName.prefix(1)).uppercased()).font(.title3.weight(.semibold)).foregroundStyle(p.accent))
                    VStack(alignment: .leading, spacing: 3) {
                        Text(u.displayName.isEmpty ? u.username : u.displayName).font(.headline).foregroundStyle(p.text)
                        Text(u.username).font(.subheadline).foregroundStyle(p.secondaryText)
                    }
                }
                .padding(.vertical, 4)
            }
            if let s = account.server {
                LabeledContent(loc("settings.server")) {
                    VStack(alignment: .trailing, spacing: 2) {
                        Text(s.name.isEmpty ? (ServerAddress.parse(s.address)?.hostPort ?? s.address) : s.name)
                        if !s.version.isEmpty { Text(s.version).font(.caption).foregroundStyle(p.tertiaryText) }
                    }
                }
            }
            Button { changePassword = true } label: { Label(loc("settings.changePassword"), systemImage: "key") }
            Button { totpSheet = true } label: {
                HStack {
                    Label(loc("settings.twoFactor"), systemImage: "lock.shield")
                    Spacer()
                    Text(loc(account.user?.totpEnabled == true ? "tf.on" : "tf.off")).foregroundStyle(p.secondaryText)
                }
            }
            Button(role: .destructive) { confirmLogout = true } label: { Label(loc("settings.logout"), systemImage: "rectangle.portrait.and.arrow.right") }
        }
        .galaxyRow()
    }

    private var appearanceSection: some View {
        Section(loc("settings.appearance")) {
            Picker(loc("settings.theme"), selection: $settings.themeMode) {
                ForEach(ThemeMode.allCases) { Text(loc("theme." + $0.rawValue)).tag($0) }
            }
            .pickerStyle(.segmented)
            VStack(alignment: .leading, spacing: 10) {
                Text(loc("settings.palette")).font(.subheadline).foregroundStyle(p.secondaryText)
                HStack(spacing: 14) {
                    ForEach(Palettes.families) { f in
                        Button { settings.palette = f.id } label: {
                            Circle().fill(Color(hex: f.swatch)).frame(width: 30, height: 30)
                                .overlay(Circle().stroke(p.text, lineWidth: settings.palette == f.id ? 2.5 : 0).padding(-4))
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(loc(f.nameKey))
                    }
                }
                .padding(.vertical, 4)
            }
        }
        .galaxyRow()
    }

    private var aboutSection: some View {
        Section(loc("settings.about")) {
            LabeledContent(loc("about.version"), value: "\(AppInfo.version) (\(AppInfo.build))")
            LabeledContent(loc("about.core")) {
                Text(app.isMock ? loc("about.coreMock", app.account.core.coreVersion) : app.account.core.coreVersion)
            }
            if let path = CoreProvider.logPath(isMock: app.isMock) {
                Button { shareLog = URL(fileURLWithPath: path) } label: { Label(loc("about.exportLog"), systemImage: "doc.text.magnifyingglass") }
            }
            NavigationLink { LicensesView() } label: { Label(loc("about.licenses"), systemImage: "doc.plaintext") }
            Link(destination: AppInfo.sourceURL) { Label(loc("about.source"), systemImage: "chevron.left.forwardslash.chevron.right") }
        }
        .galaxyRow()
    }

    private func setAppLock(_ on: Bool) {
        let ctx = LAContext()
        var err: NSError?
        guard ctx.canEvaluatePolicy(.deviceOwnerAuthentication, error: &err) else {
            app.showToast(loc("appLock.unavailable"), error: true)
            return
        }
        if on {
            ctx.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: loc("lock.prompt")) { ok, _ in
                DispatchQueue.main.async { if ok { settings.appLock = true } }
            }
        } else {
            settings.appLock = false
        }
    }
}

struct TerminalThemePicker: View {
    @EnvironmentObject var settings: AppSettings
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    var body: some View {
        List(TerminalTheme.all) { t in
            Button { settings.terminalTheme = t.id } label: {
                HStack(spacing: 14) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("~ $ ls -la").font(.system(size: 11, design: .monospaced)).foregroundStyle(t.fgColor)
                        HStack(spacing: 3) {
                            ForEach(1..<7) { i in RoundedRectangle(cornerRadius: 2).fill(Color(hex: t.ansi[i])).frame(width: 10, height: 6) }
                        }
                    }
                    .padding(8)
                    .frame(width: 96, alignment: .leading)
                    .background(RoundedRectangle(cornerRadius: 8).fill(t.bgColor))
                    Text(t.name).foregroundStyle(p.text)
                    Spacer()
                    if settings.terminalTheme == t.id { Image(systemName: "checkmark").foregroundStyle(p.accent) }
                }
            }
            .galaxyRow()
        }
        .galaxyList()
        .navigationTitle(loc("settings.terminalTheme"))
    }
}

struct LicensesView: View {
    @Environment(\.loc) private var loc
    @Environment(\.palette) private var p
    var body: some View {
        List {
            ForEach(["LemonGalaxy", "SwiftTerm"], id: \.self) { name in
                Section(name) {
                    Text(Self.text(name)).font(.caption.monospaced()).foregroundStyle(p.secondaryText).textSelection(.enabled)
                }
                .galaxyRow()
            }
        }
        .galaxyList()
        .navigationTitle(loc("about.licenses"))
    }

    static func text(_ name: String) -> String {
        guard let url = AppResources.files(in: "Licenses", ext: "txt").first(where: { $0.lastPathComponent.hasPrefix(name) }),
              let s = try? String(contentsOf: url, encoding: .utf8) else { return "MIT License" }
        return s
    }
}

struct ChangePasswordSheet: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.dismiss) private var dismiss
    @Environment(\.loc) private var loc
    @State private var current = ""
    @State private var new = ""
    @State private var confirm = ""
    @State private var totp = ""
    @State private var error: String?
    @State private var saving = false

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    SecretField(title: loc("cp.current"), text: $current)
                    SecretField(title: loc("cp.new"), text: $new, contentType: .newPassword)
                    SecretField(title: loc("cp.confirm"), text: $confirm, contentType: .newPassword)
                    if account.user?.totpEnabled == true {
                        TextField(loc("login.totpCode"), text: $totp).keyboardType(.numberPad).textContentType(.oneTimeCode)
                    }
                } footer: {
                    if let msg = validation { Text(msg).foregroundStyle(.red) }
                }
                .galaxyRow()
                if let error { Section { Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(.red) }.galaxyRow() }
            }
            .galaxyList()
            .navigationTitle(loc("settings.changePassword"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(loc("common.cancel")) { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if saving { ProgressView() } else {
                        Button(loc("common.save")) {
                            saving = true
                            Task {
                                do {
                                    try await account.changePassword(old: current, new: new, totp: totp.isEmpty ? nil : totp)
                                    app.showToast(loc("cp.done"))
                                    dismiss()
                                } catch { self.error = describeError(error) }
                                saving = false
                            }
                        }
                        .disabled(current.isEmpty || new.isEmpty || validation != nil)
                    }
                }
            }
        }
    }

    private var validation: String? {
        if !new.isEmpty && new.count < 8 { return loc("password.tooShort") }
        if !confirm.isEmpty && confirm != new { return loc("password.mismatch") }
        return nil
    }
}

struct TotpSheet: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.dismiss) private var dismiss
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var setup: TotpSetup?
    @State private var code = ""
    @State private var error: String?
    @State private var busy = false

    var enabled: Bool { account.user?.totpEnabled == true }

    var body: some View {
        NavigationStack {
            Form {
                if enabled {
                    Section {
                        TextField(loc("login.totpCode"), text: $code).keyboardType(.numberPad).textContentType(.oneTimeCode).font(.title3.monospacedDigit())
                    } footer: { Text(loc("totp.disablePrompt")) }
                    .galaxyRow()
                } else if let setup {
                    Section {
                        LabeledContent(loc("totp.secret")) {
                            Text(setup.secret).font(.footnote.monospaced()).textSelection(.enabled)
                        }
                        Button { UIPasteboard.general.string = setup.secret; app.showToast(loc("common.copied")) } label: { Label(loc("common.copy"), systemImage: "doc.on.doc") }
                        if let url = URL(string: setup.otpauthUri), UIApplication.shared.canOpenURL(url) {
                            Link(destination: url) { Label(loc("totp.openAuthenticator"), systemImage: "arrow.up.forward.app") }
                        }
                    } footer: {
                        HStack { Spacer(); InfoButton(text: loc("totp.setupInfo"), size: 13) }
                    }
                    .galaxyRow()
                    Section {
                        TextField(loc("login.totpCode"), text: $code).keyboardType(.numberPad).textContentType(.oneTimeCode).font(.title3.monospacedDigit())
                    }
                    .galaxyRow()
                } else {
                    Section { HStack { ProgressView(); Text(loc("common.loading")).foregroundStyle(p.secondaryText) } }.galaxyRow()
                }
                if let error { Section { Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(.red) }.galaxyRow() }
            }
            .galaxyList()
            .navigationTitle(loc(enabled ? "totp.disableTitle" : "totp.setupTitle"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(loc("common.cancel")) { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if busy { ProgressView() } else {
                        Button(loc(enabled ? "totp.disable" : "totp.enable"), action: submit).disabled(code.count != 6)
                    }
                }
            }
            .task {
                guard !enabled, setup == nil else { return }
                do { setup = try await account.totpSetup() } catch { self.error = describeError(error) }
            }
        }
    }

    private func submit() {
        busy = true
        Task {
            do {
                if enabled {
                    try await account.totpDisable(code)
                    app.showToast(loc("totp.disabledToast"))
                } else {
                    try await account.totpVerify(code)
                    app.showToast(loc("totp.enabledToast"))
                }
                dismiss()
            } catch { self.error = describeError(error) }
            busy = false
        }
    }
}
