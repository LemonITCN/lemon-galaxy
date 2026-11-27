import SwiftUI
import AppKit
import CoreImage.CIFilterBuiltins

extension SettingsPage {
    var icon: String {
        switch self {
        case .account: return "person.crop.circle"
        case .appearance: return "paintpalette"
        case .terminal: return "terminal"
        case .connection: return "network"
        case .ai: return "wand.and.stars"
        case .about: return "info.circle"
        }
    }
}

struct SettingsNav: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    var body: some View {
        VStack(spacing: 0) {
            ColumnHeader(title: loc("nav.settings")) { EmptyView() }
            Rectangle().fill(p.separator).frame(height: 1)
            VStack(spacing: 2) {
                ForEach(SettingsPage.allCases, id: \.self) { page in
                    let sel = app.settingsPage == page && app.activeTabId == nil
                    Button { app.settingsPage = page; app.activeTabId = nil } label: {
                        HStack(spacing: 10) {
                            Image(systemName: page.icon).font(.system(size: 13)).frame(width: 20).foregroundStyle(sel ? p.accent : p.secondaryText)
                            Text(loc("settings.page." + page.rawValue)).font(.system(size: 13, weight: sel ? .semibold : .regular))
                            Spacer()
                            if page == .about, app.update.available != nil { UpdateDot() }
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
        }
    }
}

struct SettingsPageView: View {
    @EnvironmentObject var app: AppModel
    var body: some View {
        PageScroll(maxWidth: 780) {
            switch app.settingsPage {
            case .account: AccountSettings()
            case .appearance: AppearanceSettings()
            case .terminal: TerminalSettings()
            case .connection: ConnectionSettings()
            case .ai: AISettings()
            case .about: AboutSettings()
            }
        }
    }
}

struct SettingsSection<Content: View>: View {
    let title: String
    var info: String? = nil
    @ViewBuilder let content: Content
    @Environment(\.palette) var p
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 5) {
                SectionLabel(text: title)
                if let info { InfoButton(text: info) }
            }
            Card { VStack(alignment: .leading, spacing: 14) { content } }
        }
    }
}

// MARK: 账号

struct AccountSettings: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var oldPw = ""
    @State private var newPw = ""
    @State private var confirmPw = ""
    @State private var busy = false
    @State private var message: (String, Bool)?
    @State private var totpSetup: TotpSetup?
    @State private var totpCode = ""
    @State private var totpEnabled = false
    @State private var pwTotp = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            PageTitle(text: loc("settings.page.account"))
            HStack(spacing: 14) {
                Circle().fill(p.accent.opacity(0.18)).frame(width: 48, height: 48)
                    .overlay(Text(String((app.user?.displayName ?? "?").prefix(1)).uppercased()).font(.system(size: 20, weight: .semibold)).foregroundStyle(p.accent))
                VStack(alignment: .leading, spacing: 3) {
                    Text(app.user?.displayName ?? "").font(.system(size: 15, weight: .semibold))
                    Text("@" + (app.user?.username ?? "")).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                }
                Spacer()
                Button(loc("account.logout")) {
                    app.confirm(loc("account.logout"), loc("account.logout.message"), button: loc("account.logout")) { Task { await app.logout() } }
                }
                .buttonStyle(SecondaryButtonStyle(destructive: true))
            }
            InfoGrid(rows: [
                (loc("account.server"), app.server?.name ?? "—"),
                (loc("account.address"), app.server?.address ?? "—"),
                (loc("account.serverVersion"), app.server?.version ?? "—"),
                (loc("account.ai"), app.user?.aiEnabled == true ? loc("common.on") : loc("common.off")),
            ])
            SettingsSection(title: loc("account.changePassword"), info: loc("account.changePassword.info")) {
                VStack(alignment: .leading, spacing: 10) {
                    FormRow(label: loc("account.oldPassword"), labelWidth: 160) { SecureField("", text: $oldPw).fieldStyle().frame(width: 260) }
                    FormRow(label: loc("account.newPassword"), labelWidth: 160) { SecureField("", text: $newPw).fieldStyle().frame(width: 260) }
                    FormRow(label: loc("account.confirmPassword"), labelWidth: 160) {
                        SecureField("", text: $confirmPw).fieldStyle(invalid: !confirmPw.isEmpty && confirmPw != newPw).frame(width: 260)
                    }
                    if totpEnabled {
                        FormRow(label: loc("account.totpCode"), labelWidth: 160) {
                            TextField("000000", text: $pwTotp).fieldStyle().frame(width: 120).font(.system(size: 13, design: .monospaced))
                        }
                    }
                    HStack {
                        Spacer().frame(width: 172)
                        Button(loc("account.changePassword")) { changePassword() }
                            .buttonStyle(PrimaryButtonStyle())
                            .disabled(busy || oldPw.isEmpty || newPw.count < 8 || newPw != confirmPw)
                        if let (m, ok) = message { Text(m).font(.system(size: 12)).foregroundStyle(ok ? p.success : p.danger) }
                    }
                }
            }
            totpSection
        }
        .onAppear { totpEnabled = app.client.currentUser()?.totpEnabled ?? false }
    }

    @ViewBuilder
    private var totpSection: some View {
        let client = app.client
        SettingsSection(title: loc("account.totp"), info: loc("account.totp.info")) {
            if let setup = totpSetup {
                HStack(alignment: .top, spacing: 18) {
                    if let qr = QRCode.image(setup.otpauthUri) {
                        Image(nsImage: qr).interpolation(.none).resizable().frame(width: 132, height: 132)
                            .padding(6).background(RoundedRectangle(cornerRadius: 8).fill(.white))
                    }
                    VStack(alignment: .leading, spacing: 10) {
                        Text(loc("account.totp.scan")).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                        Text(setup.secret).font(.system(size: 13, design: .monospaced)).textSelection(.enabled)
                        HStack {
                            TextField("000000", text: $totpCode).fieldStyle().frame(width: 110).font(.system(size: 14, design: .monospaced))
                            Button(loc("account.totp.confirm")) {
                                Task {
                                    do { try await client.totpVerify(code: totpCode); totpEnabled = true; totpSetup = nil; totpCode = "" }
                                    catch { app.showError(error) }
                                }
                            }
                            .buttonStyle(PrimaryButtonStyle()).disabled(totpCode.count != 6)
                            Button(loc("common.cancel")) { totpSetup = nil }.buttonStyle(SecondaryButtonStyle())
                        }
                    }
                }
            } else {
                HStack {
                    Label(totpEnabled ? loc("account.totp.on") : loc("account.totp.off"), systemImage: totpEnabled ? "checkmark.shield.fill" : "shield")
                        .foregroundStyle(totpEnabled ? p.success : p.secondaryText).font(.system(size: 13))
                    Spacer()
                    if totpEnabled {
                        TextField(loc("account.totp.codeToDisable"), text: $totpCode).fieldStyle().frame(width: 150)
                        Button(loc("account.totp.disable")) {
                            Task {
                                do { try await client.totpDisable(code: totpCode); totpEnabled = false; totpCode = "" } catch { app.showError(error) }
                            }
                        }
                        .buttonStyle(SecondaryButtonStyle(destructive: true)).disabled(totpCode.count != 6)
                    } else {
                        Button(loc("account.totp.enable")) {
                            Task { do { totpSetup = try await client.totpSetup() } catch { app.showError(error) } }
                        }
                        .buttonStyle(PrimaryButtonStyle())
                    }
                }
            }
        }
    }

    private func changePassword() {
        busy = true
        Task {
            do {
                try await app.client.changePassword(oldPassword: oldPw, newPassword: newPw, totp: pwTotp.isEmpty ? nil : pwTotp)
                message = (loc("account.passwordChanged"), true)
                oldPw = ""; newPw = ""; confirmPw = ""; pwTotp = ""
            } catch {
                message = (AppModel.describe(error), false)
            }
            busy = false
        }
    }
}

enum QRCode {
    static func image(_ text: String) -> NSImage? {
        let f = CIFilter.qrCodeGenerator()
        f.message = Data(text.utf8)
        f.correctionLevel = "M"
        guard let out = f.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)) else { return nil }
        let rep = NSCIImageRep(ciImage: out)
        let img = NSImage(size: rep.size)
        img.addRepresentation(rep)
        return img
    }
}

// MARK: 外观

struct AppearanceSettings: View {
    @EnvironmentObject var settings: AppSettings
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            PageTitle(text: loc("settings.page.appearance"))
            SettingsSection(title: loc("settings.theme")) {
                HStack(spacing: 14) {
                    modeTile("system", loc("settings.theme.system"))
                    modeTile("light", loc("settings.theme.light"))
                    modeTile("dark", loc("settings.theme.dark"))
                }
            }
            SettingsSection(title: loc("settings.palette")) {
                HStack(spacing: 16) {
                    ForEach(Palettes.families) { f in
                        VStack(spacing: 6) {
                            Circle().fill(Color(hex: f.swatch)).frame(width: 28, height: 28)
                                .overlay(Circle().stroke(p.text, lineWidth: settings.palette == f.id ? 2 : 0).padding(-4))
                                .overlay(Image(systemName: "checkmark").font(.system(size: 11, weight: .bold)).foregroundStyle(.white).opacity(settings.palette == f.id ? 1 : 0))
                            Text(loc(f.nameKey)).font(.system(size: 11.5)).foregroundStyle(settings.palette == f.id ? p.text : p.secondaryText)
                        }
                        .onTapGesture { settings.palette = f.id }
                    }
                }
                .padding(.vertical, 4)
            }
            SettingsSection(title: loc("settings.language"), info: loc("settings.language.info")) {
                HStack {
                    Text(loc("settings.language")).font(.system(size: 13))
                    Spacer()
                    Picker("", selection: $settings.language) {
                        Text(loc("settings.language.system")).tag("system")
                        ForEach(Localization.shared.packs) { pk in Text(pk.name).tag(pk.code) }
                    }
                    .labelsHidden().fixedSize()
                }
            }
            SettingsSection(title: loc("settings.layout")) {
                ToggleRow(label: loc("settings.compactSidebar"), isOn: $settings.sidebarCompact)
                ToggleRow(label: loc("settings.hideList"), isOn: $settings.listHidden)
                ToggleRow(label: loc("settings.autoHideList"), isOn: $settings.autoHideList, info: loc("settings.autoHideList.info"))
            }
        }
    }

    private func modeTile(_ mode: String, _ title: String) -> some View {
        let sel = settings.appearance == mode
        return VStack(spacing: 8) {
            ThemePreviewTile(mode: mode, palette: settings.palette)
                .frame(width: 150, height: 92)
                .clipShape(RoundedRectangle(cornerRadius: 8))
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(sel ? p.accent : p.border, lineWidth: sel ? 2 : 0.6))
            Text(title).font(.system(size: 12, weight: sel ? .semibold : .regular))
        }
        .onTapGesture { settings.appearance = mode }
    }
}

/// 主题缩略图：画一个迷你窗口
struct ThemePreviewTile: View {
    let mode: String
    let palette: String
    var body: some View {
        GeometryReader { g in
            if mode == "system" {
                HStack(spacing: 0) {
                    mini(Palettes.make(palette, dark: false)).frame(width: g.size.width / 2).clipped()
                    mini(Palettes.make(palette, dark: true)).frame(width: g.size.width / 2).clipped()
                }
            } else {
                mini(Palettes.make(palette, dark: mode == "dark"))
            }
        }
    }

    private func mini(_ q: Palette) -> some View {
        HStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 5) {
                ForEach(0..<4) { i in
                    RoundedRectangle(cornerRadius: 2).fill(i == 0 ? q.accent : q.sidebarText.opacity(0.3)).frame(width: 22, height: 4)
                }
                Spacer()
            }
            .padding(8).frame(width: 40).frame(maxHeight: .infinity).background(q.sidebar)
            VStack(alignment: .leading, spacing: 5) {
                ForEach(0..<3) { i in
                    RoundedRectangle(cornerRadius: 3).fill(i == 1 ? q.selection : q.card).frame(height: 12)
                        .overlay(RoundedRectangle(cornerRadius: 3).stroke(q.border, lineWidth: 0.5))
                }
                Spacer()
            }
            .padding(8).frame(maxWidth: .infinity, maxHeight: .infinity).background(q.content)
        }
    }
}

// MARK: 终端

struct TerminalSettings: View {
    @EnvironmentObject var settings: AppSettings
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            PageTitle(text: loc("settings.page.terminal"))
            TerminalPreview(theme: TerminalTheme.named(settings.terminalTheme), font: settings.terminalFont, size: settings.terminalFontSize)
            SettingsSection(title: loc("settings.terminal.theme"), info: loc("settings.terminal.theme.info")) {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 150), spacing: 10)], spacing: 10) {
                    ForEach(TerminalTheme.all) { t in
                        ThemeSwatch(theme: t, selected: settings.terminalTheme == t.id).onTapGesture { settings.terminalTheme = t.id }
                    }
                }
            }
            SettingsSection(title: loc("settings.terminal.font")) {
                HStack {
                    Text(loc("settings.terminal.family")).font(.system(size: 13))
                    Spacer()
                    Picker("", selection: $settings.terminalFont) {
                        ForEach(TerminalFonts.families, id: \.self) { f in Text(f == "SF Mono" ? loc("settings.terminal.systemMono") : f).tag(f) }
                    }
                    .labelsHidden().frame(width: 220)
                }
                HStack {
                    Text(loc("settings.terminal.size")).font(.system(size: 13))
                    Spacer()
                    Slider(value: $settings.terminalFontSize, in: 9...24, step: 1).frame(width: 180)
                    Text("\(Int(settings.terminalFontSize)) pt").font(.system(size: 12).monospacedDigit()).frame(width: 44, alignment: .trailing)
                }
                ToggleRow(label: loc("settings.terminal.blink"), isOn: $settings.terminalCursorBlink)
                ToggleRow(label: loc("settings.terminal.optionMeta"), isOn: $settings.terminalOptionAsMeta, info: loc("settings.terminal.optionMeta.info"))
            }
        }
    }
}

struct TerminalPreview: View {
    let theme: TerminalTheme
    let font: String
    let size: Double
    var body: some View {
        let f = Font(TerminalFonts.font(family: font, size: CGFloat(size)) as CTFont)
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 0) {
                Text("deploy@web-01").foregroundStyle(c(10))
                Text(":").foregroundStyle(theme.fgColor)
                Text("~/app").foregroundStyle(c(12))
                Text("$ ls --color").foregroundStyle(theme.fgColor)
            }
            HStack(spacing: 14) {
                Text("config.yaml").foregroundStyle(theme.fgColor)
                Text("deploy.sh").foregroundStyle(c(10))
                Text("logs").foregroundStyle(c(12))
                Text("backup.tar.gz").foregroundStyle(c(1))
            }
            HStack(spacing: 3) {
                ForEach(0..<16) { i in RoundedRectangle(cornerRadius: 2).fill(c(i)).frame(width: 16, height: 12) }
            }
            .padding(.top, 4)
        }
        .font(f)
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 10).fill(theme.bgColor))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(Color.gray.opacity(0.25), lineWidth: 0.6))
    }
    private func c(_ i: Int) -> Color { Color(hex: theme.ansi[i]) }
}

struct ThemeSwatch: View {
    let theme: TerminalTheme
    let selected: Bool
    @Environment(\.palette) var p
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            VStack(alignment: .leading, spacing: 4) {
                Text("$ ls").font(.system(size: 10, design: .monospaced)).foregroundStyle(theme.fgColor)
                HStack(spacing: 2) {
                    ForEach([1, 2, 3, 4, 5, 6], id: \.self) { i in RoundedRectangle(cornerRadius: 1.5).fill(Color(hex: theme.ansi[i])).frame(width: 12, height: 8) }
                }
            }
            .padding(8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 6).fill(theme.bgColor))
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(p.border, lineWidth: 0.6))
            Text(theme.name).font(.system(size: 11.5, weight: selected ? .semibold : .regular)).foregroundStyle(selected ? p.text : p.secondaryText)
        }
        .padding(6)
        .background(RoundedRectangle(cornerRadius: 9).stroke(selected ? p.accent : Color.clear, lineWidth: 2))
        .contentShape(Rectangle())
    }
}

// MARK: 连接

struct ConnectionSettings: View {
    @EnvironmentObject var settings: AppSettings
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            PageTitle(text: loc("settings.page.connection"))
            SettingsSection(title: loc("settings.connection.display"), info: loc("settings.connection.display.info")) {
                HStack {
                    Text(loc("remote.quality")).font(.system(size: 13))
                    Spacer()
                    Picker("", selection: Binding(get: { settings.displayQuality }, set: { q in
                        settings.displayQuality = q
                        let v = AppSettings.quality(q)
                        settings.displayFps = v.fps
                        settings.displayBitrateKbps = v.kbps
                    })) {
                        Text(loc("remote.quality.smooth")).tag("smooth")
                        Text(loc("remote.quality.balanced")).tag("balanced")
                        Text(loc("remote.quality.sharp")).tag("sharp")
                        Text(loc("remote.quality.custom")).tag("custom")
                    }
                    .pickerStyle(.segmented).labelsHidden().frame(width: 320)
                }
                HStack {
                    Text(loc("remote.fps")).font(.system(size: 13))
                    Spacer()
                    Picker("", selection: Binding(get: { settings.displayFps }, set: { settings.displayFps = $0; settings.displayQuality = "custom" })) {
                        ForEach([30, 60, 90, 120, 144], id: \.self) { Text("\($0) fps").tag($0) }
                    }
                    .labelsHidden().fixedSize()
                }
                HStack {
                    Text(loc("remote.bitrate")).font(.system(size: 13))
                    Spacer()
                    Slider(value: Binding(get: { Double(settings.displayBitrateKbps) }, set: { settings.displayBitrateKbps = Int($0); settings.displayQuality = "custom" }),
                           in: 2_000...50_000, step: 1_000).frame(width: 200)
                    Text("\(settings.displayBitrateKbps / 1000) Mbps").font(.system(size: 12).monospacedDigit()).frame(width: 64, alignment: .trailing)
                }
                ToggleRow(label: loc("settings.connection.h265"), isOn: $settings.preferH265, info: loc("settings.connection.h265.info"))
                ToggleRow(label: loc("remote.clipboard"), isOn: $settings.clipboardSync)
                ToggleRow(label: loc("settings.connection.audio"), isOn: $settings.remoteAudio, info: loc("settings.connection.audio.info"))
            }
            SettingsSection(title: loc("settings.connection.terminal")) {
                ToggleRow(label: loc("settings.connection.reattach"), isOn: $settings.reattachTerminals, info: loc("settings.connection.reattach.info"))
            }
        }
    }
}

// MARK: AI

struct AISettings: View {
    @EnvironmentObject var settings: AppSettings
    @Environment(\.loc) var loc
    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            PageTitle(text: loc("settings.page.ai"))
            SettingsSection(title: loc("settings.ai.defaultMode"), info: loc("ai.mode.info")) {
                ModePicker(mode: Binding(get: { settings.aiMode }, set: { settings.aiMode = $0 }))
            }
            SettingsSection(title: loc("settings.ai.behavior")) {
                ToggleRow(label: loc("ai.planFirst"), isOn: $settings.aiPlanFirst, info: loc("ai.planFirst.info"))
            }
        }
    }
}

// MARK: 关于

struct AboutSettings: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var license: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            HStack(spacing: 16) {
                AppMark(size: 64)
                VStack(alignment: .leading, spacing: 4) {
                    Text("LemonGalaxy").font(.system(size: 22, weight: .semibold))
                    Text(loc("about.version", AppInfo.version, AppInfo.build)).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                    HStack(spacing: 6) {
                        Text(loc("about.core", app.core.coreVersion)).font(.system(size: 12)).foregroundStyle(p.tertiaryText)
                        if app.core.isMock {
                            Text(loc("about.mock")).font(.system(size: 11, weight: .medium)).foregroundStyle(p.warning)
                                .padding(.horizontal, 6).padding(.vertical, 1).background(Capsule().fill(p.warning.opacity(0.13)))
                            InfoButton(text: loc("about.mock.info"))
                        }
                    }
                }
            }
            UpdateSection(model: app.update)
            SettingsSection(title: loc("about.licenses")) {
                ForEach(Self.licenses, id: \.0) { item in
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(item.0).font(.system(size: 13, weight: .medium))
                            Text(item.1).font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
                        }
                        Spacer()
                        Button(loc("about.viewLicense")) {
                            license = license == item.2 ? nil : item.2
                        }
                        .buttonStyle(SecondaryButtonStyle())
                    }
                }
                if let l = license, let text = try? String(contentsOf: AppResources.url("Licenses/" + l), encoding: .utf8) {
                    Text(text).font(.system(size: 11, design: .monospaced)).foregroundStyle(p.secondaryText).textSelection(.enabled)
                        .padding(10).frame(maxWidth: .infinity, alignment: .leading)
                        .background(RoundedRectangle(cornerRadius: 6).fill(p.elevated))
                }
            }
            Text("Copyright © 2026 LemonIT.CN · MIT License").font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
        }
    }

    static let licenses: [(String, String, String)] = [
        ("LemonGalaxy", "MIT · github.com/LemonITCN/lemon-galaxy", "LemonGalaxy-LICENSE.txt"),
        ("SwiftTerm", "MIT · github.com/migueldeicaza/SwiftTerm", "SwiftTerm-LICENSE.txt"),
    ]
}

/// 关于页的「软件更新」：检查、立即更新（一路自动做完）、下载 / 安装进度、自动更新开关、不能自动替换时的原因
struct UpdateSection: View {
    @ObservedObject var model: UpdateModel
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var settings: AppSettings
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        SettingsSection(title: loc("update.section"), info: loc("update.section.info")) {
            HStack(spacing: 10) {
                statusIcon
                VStack(alignment: .leading, spacing: 2) {
                    Text(statusText).font(.system(size: 13, weight: .medium))
                        .foregroundStyle(isFailed ? p.danger : p.text)
                        .fixedSize(horizontal: false, vertical: true)
                    if let t = model.lastChecked {
                        Text(loc("update.lastChecked", Fmt.time(Int64(t.timeIntervalSince1970 * 1000))))
                            .font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
                    }
                }
                Spacer()
                if let a = model.available, !model.isInstalling {
                    Button(loc("update.view", a.version)) { app.modal = .update }.buttonStyle(SecondaryButtonStyle())
                    if !model.installWhenReady {
                        Button(loc("update.installNow")) { model.installNow() }
                            .buttonStyle(PrimaryButtonStyle())
                            .disabled(model.blocker != nil)
                    }
                }
                Button(loc("update.check")) { Task { await model.check(manual: true) } }
                    .buttonStyle(SecondaryButtonStyle())
                    .disabled(model.busy)
            }
            if case .downloading(_, let done, let total) = model.state {
                ProgressView(value: total == 0 ? 0 : min(1, Double(done) / Double(total)))
            }
            if let b = model.blocker {
                HStack(alignment: .top, spacing: 6) {
                    Image(systemName: "info.circle").foregroundStyle(p.warning)
                    Text(b.message).font(.system(size: 12)).foregroundStyle(p.secondaryText).fixedSize(horizontal: false, vertical: true)
                }
            }
            ToggleRow(label: loc("update.auto"), isOn: $settings.autoUpdate, info: loc("update.auto.info"))
        }
    }

    private var isFailed: Bool { if case .failed = model.state { return true }; return false }

    @ViewBuilder
    private var statusIcon: some View {
        switch model.state {
        case .checking, .installing: ProgressView().controlSize(.small).frame(width: 18)
        case .failed: Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(p.danger).frame(width: 18)
        case .upToDate: Image(systemName: "checkmark.circle.fill").foregroundStyle(p.success).frame(width: 18)
        default:
            if model.available != nil {
                Image(systemName: "arrow.down.circle.fill").foregroundStyle(p.accent).frame(width: 18)
            } else {
                Image(systemName: "arrow.triangle.2.circlepath").foregroundStyle(p.secondaryText).frame(width: 18)
            }
        }
    }

    private var statusText: String {
        switch model.state {
        case .checking: return loc("update.checking")
        case .upToDate: return loc("update.upToDate", AppInfo.version)
        case .failed(let m): return m
        case .downloading(let i, let d, let t):
            let s = loc("update.downloadingShort", i.version, t == 0 ? 0 : Int(Double(d) * 100 / Double(t)))
            return model.installWhenReady ? s + " · " + loc("update.installWhenReady") : s
        case .ready(let i, _): return loc("update.readyShort", i.version)
        case .installing(let i, let step): return loc("update.installingShort", i.version) + " · " + UpdateCard.stepText(step, loc)
        case .available(let i): return loc("update.availableShort", i.version)
        case .idle: return model.available.map { loc("update.availableShort", $0.version) } ?? loc("update.idle", AppInfo.version)
        }
    }
}
