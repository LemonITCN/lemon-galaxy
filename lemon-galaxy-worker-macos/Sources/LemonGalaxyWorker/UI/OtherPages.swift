import SwiftUI
import AppKit

// MARK: - 临时协助

struct AssistPage: View {
    @EnvironmentObject var model: WorkerModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            PageTitle(text: loc("page.assist"), info: loc("assist.info"))
            if let r = model.pendingRequest { AssistRequestView(request: r) }
            if model.service == nil {
                // 全新安装、没登录过账号：先填服务器地址，set_server 后再开协助
                Card {
                    VStack(alignment: .leading, spacing: 12) {
                        HStack(spacing: 10) {
                            Image(systemName: "person.line.dotted.person").font(.system(size: 22, weight: .light)).foregroundStyle(p.accent)
                            Text(loc("assist.noAccount")).font(.system(size: 13)).foregroundStyle(p.secondaryText)
                            InfoButton(text: loc("assist.noAccount.info"))
                            Spacer()
                        }
                        HStack(spacing: 8) {
                            Text(loc("login.server")).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                            InfoButton(text: loc("login.server.info"))
                            TextField("host:7443#sha256-…", text: $model.assistServer)
                                .font(.system(size: 12, design: .monospaced))
                                .fieldStyle(invalid: !model.assistServer.isEmpty && !LoginForm.looksLikeServer(model.assistServer.trimmingCharacters(in: .whitespacesAndNewlines)))
                                .onSubmit { model.startAssist() }
                            if model.assistBusy { ProgressView().controlSize(.small) }
                            Button(loc("assist.start")) { model.startAssist() }.buttonStyle(PrimaryButtonStyle())
                                .disabled(model.assistBusy || model.serviceStarting || model.assistServer.trimmingCharacters(in: .whitespaces).isEmpty)
                        }
                        assistErrorLine
                    }
                }
            } else if let a = model.status.assist {
                Card {
                    VStack(alignment: .leading, spacing: 18) {
                        HStack(alignment: .top, spacing: 40) {
                            codeBlock(loc("assist.deviceCode"), Self.group(a.deviceCode, by: 3))
                            codeBlock(loc("assist.verificationCode"), a.code)
                            Spacer()
                        }
                        HStack(spacing: 8) {
                            Button { model.refreshCode() } label: { Label(loc("assist.refresh"), systemImage: "arrow.clockwise") }
                                .buttonStyle(SecondaryButtonStyle())
                                .disabled(model.pendingRequest != nil)
                            Button { copyToPasteboard("\(a.deviceCode) \(a.code)") } label: { Label(loc("common.copy"), systemImage: "doc.on.doc") }
                                .buttonStyle(SecondaryButtonStyle())
                            Spacer()
                            Button(loc("assist.stop")) { model.stopAssist() }.buttonStyle(SecondaryButtonStyle(destructive: true))
                        }
                        Rectangle().fill(p.separator).frame(height: 1)
                        HStack(spacing: 8) {
                            Text(loc("assist.custom")).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                            InfoButton(text: loc("assist.custom.info"))
                            TextField("8–16", text: $model.customCode)
                                .font(.system(size: 13, design: .monospaced))
                                .fieldStyle(invalid: !model.customCode.isEmpty && !model.customCodeValid)
                                .frame(width: 180)
                            Button(loc("common.apply")) { model.applyCustomCode() }.buttonStyle(SecondaryButtonStyle())
                                .disabled(!model.customCodeValid)
                        }
                    }
                }
            } else {
                Card {
                    HStack(spacing: 14) {
                        Image(systemName: "person.line.dotted.person").font(.system(size: 26, weight: .light)).foregroundStyle(p.accent)
                        Text(loc("assist.idle")).font(.system(size: 13)).foregroundStyle(p.secondaryText)
                        Spacer()
                        if model.assistBusy { ProgressView().controlSize(.small) }
                        Button(loc("assist.start")) { model.startAssist() }.buttonStyle(PrimaryButtonStyle())
                            .disabled(model.assistBusy || !model.status.remoteEnabled)
                    }
                }
                if !model.hasAccounts, let server = model.serverAddress {
                    HStack(spacing: 6) {
                        Image(systemName: "server.rack").font(.system(size: 11)).foregroundStyle(p.tertiaryText)
                        Text(loc("assist.viaServer", server.split(separator: "#").first.map(String.init) ?? server))
                            .font(.system(size: 12)).foregroundStyle(p.tertiaryText)
                        InfoButton(text: loc("assist.viaServer.info"))
                    }
                }
                assistErrorLine
                if !model.status.remoteEnabled {
                    Label(loc("assist.remoteDisabled"), systemImage: "hand.raised.fill").font(.system(size: 12)).foregroundStyle(p.danger)
                }
            }
        }
    }

    @ViewBuilder
    private var assistErrorLine: some View {
        if let e = model.assistError {
            HStack(alignment: .top, spacing: 6) {
                Image(systemName: "exclamationmark.circle.fill").foregroundStyle(p.danger)
                Text(e).font(.system(size: 12)).foregroundStyle(p.danger).fixedSize(horizontal: false, vertical: true)
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

/// 协助请求：允许 / 拒绝（窗口里与浮动面板里共用）
struct AssistRequestView: View {
    let request: WorkerModel.AssistRequest
    @EnvironmentObject var model: WorkerModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                Image(systemName: "person.crop.circle.badge.questionmark").font(.system(size: 24)).foregroundStyle(p.warning)
                VStack(alignment: .leading, spacing: 3) {
                    Text(loc("assist.request.title")).font(.system(size: 14, weight: .semibold))
                    Text(loc("assist.request.body", request.user.isEmpty ? loc("status.anonymous") : request.user, WorkerModel.shortDevice(request.device)))
                        .font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            HStack {
                InfoButton(text: loc("assist.request.info"))
                Spacer()
                Button(loc("assist.request.deny")) { model.respond(false) }.buttonStyle(SecondaryButtonStyle()).keyboardShortcut(.cancelAction)
                Button(loc("assist.request.allow")) { model.respond(true) }.buttonStyle(PrimaryButtonStyle())
            }
        }
        .padding(16)
        .background(RoundedRectangle(cornerRadius: 12).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(p.warning.opacity(0.5), lineWidth: 1))
    }
}

// MARK: - 系统授权

struct PermissionsPage: View {
    @EnvironmentObject var model: WorkerModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            PageTitle(text: loc("page.permissions"), info: loc("perm.info"))
            permRow(icon: "rectangle.dashed.badge.record", title: loc("perm.screen"), purpose: loc("perm.screen.purpose"),
                    granted: model.screenGranted) { SystemPermissions.requestScreenRecording() }
            permRow(icon: "accessibility", title: loc("perm.accessibility"), purpose: loc("perm.accessibility.purpose"),
                    granted: model.accessibilityGranted) { SystemPermissions.requestAccessibility() }
            if !model.needsPermissions {
                Label(loc("perm.allSet"), systemImage: "checkmark.seal.fill").font(.system(size: 13)).foregroundStyle(p.success)
            }
        }
        .onAppear { model.refreshPermissions(); model.watchPermissions(true) }
        .onDisappear { model.watchPermissions(false) }
    }

    private func permRow(icon: String, title: String, purpose: String, granted: Bool, action: @escaping () -> Void) -> some View {
        Card {
            HStack(spacing: 14) {
                Image(systemName: icon).font(.system(size: 22, weight: .light)).foregroundStyle(granted ? p.success : p.warning).frame(width: 34)
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 5) {
                        Text(title).font(.system(size: 13.5, weight: .semibold))
                        InfoButton(text: purpose)
                    }
                    Text(granted ? loc("perm.granted") : loc("perm.notGranted"))
                        .font(.system(size: 12)).foregroundStyle(granted ? p.success : p.warning)
                }
                Spacer()
                if granted {
                    Image(systemName: "checkmark.circle.fill").font(.system(size: 18)).foregroundStyle(p.success)
                } else {
                    Button(loc("perm.open"), action: action).buttonStyle(PrimaryButtonStyle())
                }
            }
        }
    }
}

// MARK: - 通用

struct GeneralPage: View {
    @EnvironmentObject var model: WorkerModel
    @EnvironmentObject var settings: WorkerSettings
    @ObservedObject var localization = Localization.shared
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            PageTitle(text: loc("page.general"))
            SectionLabel(text: loc("general.unattended"))
            UnattendedCard(ctl: model.unattended)
            SectionLabel(text: loc("general.startup"))
            Card {
                VStack(alignment: .leading, spacing: 10) {
                    HStack(spacing: 6) {
                        Text(loc("general.loginItem")).font(.system(size: 13))
                        InfoButton(text: loc("general.loginItem.info"))
                        Spacer()
                        Toggle("", isOn: Binding(get: { model.loginItem == .enabled || model.loginItem == .requiresApproval },
                                                 set: { model.setLoginItem($0) }))
                            .toggleStyle(PaletteSwitchStyle(small: true)).labelsHidden()
                            .disabled(model.loginItem == .unavailable)
                    }
                    switch model.loginItem {
                    case .requiresApproval:
                        HStack(spacing: 6) {
                            Image(systemName: "exclamationmark.triangle.fill").font(.system(size: 11)).foregroundStyle(p.warning)
                            Text(loc("general.loginItem.approval")).font(.system(size: 12)).foregroundStyle(p.secondaryText)
                            Button(loc("general.loginItem.open")) { LoginItem.openSettings() }.buttonStyle(.plain).font(.system(size: 12)).foregroundStyle(p.accent)
                        }
                    case .unavailable:
                        Text(loc("general.loginItem.unavailable")).font(.system(size: 12)).foregroundStyle(p.tertiaryText)
                    default: EmptyView()
                    }
                }
            }
            SectionLabel(text: loc("general.security"))
            Card {
                ToggleRow(label: loc("general.lockOnEnd"), isOn: Binding(get: { model.lockOnEnd }, set: { model.setLockOnEnd($0) }),
                          info: loc("general.lockOnEnd.info"))
            }
            .disabled(model.service == nil)
            HStack(spacing: 4) {
                SectionLabel(text: loc("general.notify"))
                InfoButton(text: loc("general.menuIconHint"))
            }
            Card {
                VStack(spacing: 12) {
                    ToggleRow(label: loc("general.systemNotifications"),
                              isOn: Binding(get: { settings.systemNotifications }, set: { model.setSystemNotifications($0) }),
                              info: loc("general.systemNotifications.info"))
                    ToggleRow(label: loc("general.assistSound"), isOn: $settings.assistSound)
                }
            }
            SectionLabel(text: loc("general.appearanceSection"))
            Card {
                VStack(spacing: 12) {
                    HStack {
                        Text(loc("general.language")).font(.system(size: 13))
                        InfoButton(text: loc("general.language.info"))
                        Spacer()
                        Picker("", selection: $settings.language) {
                            Text(loc("general.language.system")).tag("system")
                            ForEach(localization.packs) { pack in Text(pack.name).tag(pack.code) }
                        }
                        .labelsHidden().fixedSize()
                    }
                    HStack {
                        Text(loc("general.appearance")).font(.system(size: 13))
                        Spacer()
                        Picker("", selection: $settings.appearance) {
                            Text(loc("general.appearance.system")).tag("system")
                            Text(loc("general.appearance.light")).tag("light")
                            Text(loc("general.appearance.dark")).tag("dark")
                        }
                        .pickerStyle(.segmented).labelsHidden().frame(width: 240)
                    }
                }
            }
        }
    }
}

// MARK: - 关于

struct AboutPage: View {
    @EnvironmentObject var model: WorkerModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            PageTitle(text: loc("page.about"))
            Card(padding: 20) {
                HStack(spacing: 16) {
                    WorkerMark(size: 56)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("LemonGalaxyWorker").font(.system(size: 17, weight: .semibold))
                        Text(loc("about.tagline")).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                        Text(loc("about.version", AppInfo.version, AppInfo.build)).font(.system(size: 12).monospacedDigit()).foregroundStyle(p.tertiaryText)
                    }
                    Spacer()
                    if model.core.isMock {
                        Text(loc("about.mock")).font(.system(size: 11)).foregroundStyle(p.warning)
                            .padding(.horizontal, 7).padding(.vertical, 2).background(Capsule().fill(p.warning.opacity(0.12)))
                    }
                }
            }
            Card(padding: 0) {
                VStack(spacing: 0) {
                    row(loc("about.core"), model.coreVersionText)
                    row(loc("status.deviceId"), groupedId(model.deviceId), mono: true)
                    row(loc("about.system"), AppInfo.osVersion)
                    row(loc("about.dataDir"), model.dataDirText, last: true)
                }
            }
            HStack(spacing: 4) {
                SectionLabel(text: loc("about.update"))
                InfoButton(text: loc("update.info"))
            }
            UpdateCard(updater: model.updater)
            HStack(spacing: 14) {
                Button(loc("about.source")) { NSWorkspace.shared.open(AppInfo.repoURL) }.buttonStyle(.plain).foregroundStyle(p.accent)
                Text("·").foregroundStyle(p.tertiaryText)
                Text(loc("about.license")).foregroundStyle(p.tertiaryText)
                Text("·").foregroundStyle(p.tertiaryText)
                Text("Copyright © 2026 LemonIT.CN").foregroundStyle(p.tertiaryText)
            }
            .font(.system(size: 12))
        }
    }

    private func row(_ label: String, _ value: String, mono: Bool = false, last: Bool = false) -> some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Text(label).font(.system(size: 12.5)).foregroundStyle(p.secondaryText).frame(width: 110, alignment: .leading)
                Text(value).font(mono ? .system(size: 12.5, design: .monospaced) : .system(size: 12.5)).lineLimit(1).truncationMode(.middle).textSelection(.enabled)
                Spacer()
            }
            .padding(.horizontal, 16).frame(height: 36)
            if !last { Rectangle().fill(p.separator).frame(height: 1).padding(.leading, 16) }
        }
    }
}

// MARK: - 软件更新卡片

struct UpdateCard: View {
    @ObservedObject var updater: WorkerUpdater
    @EnvironmentObject var model: WorkerModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        Card {
            switch updater.state {
            case .available(let info): available(info)
            case .downloading(let info, let done, let total): downloading(info, done: done, total: total)
            case .downloaded(let info, _): downloaded(info)
            default: summary
            }
        }
    }

    /// 当前版本 + 检查按钮（含检查中、已是最新、失败）
    private var summary: some View {
        HStack(spacing: 12) {
            Image(systemName: icon).font(.system(size: 18)).foregroundStyle(iconColor)
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.system(size: 13)).fixedSize(horizontal: false, vertical: true)
                    .foregroundStyle(isFailed ? p.danger : p.text)
                if let d = updater.lastChecked {
                    Text(loc("update.lastChecked", Fmt.date(Int64(d.timeIntervalSince1970 * 1000), zh: loc.isChinese)))
                        .font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
                }
            }
            Spacer()
            if updater.state == .checking { ProgressView().controlSize(.small) }
            Button(loc("update.releases")) { NSWorkspace.shared.open(AppInfo.releasesURL) }.buttonStyle(.plain)
                .font(.system(size: 12)).foregroundStyle(p.accent)
            Button(loc("update.check")) { Task { await updater.check(manual: true) } }
                .buttonStyle(SecondaryButtonStyle())
                .disabled(updater.busy || !model.hasActiveAccount)
                .help(model.hasActiveAccount ? "" : loc("update.needAccount"))
        }
    }

    private var isFailed: Bool { if case .failed = updater.state { return true }; return false }

    private var title: String {
        switch updater.state {
        case .checking: return loc("update.checking")
        case .upToDate: return loc("update.upToDate", AppInfo.version)
        case .failed(let m): return m
        default: return model.hasActiveAccount ? loc("update.idle", AppInfo.version) : loc("update.needAccount")
        }
    }

    private var icon: String {
        switch updater.state {
        case .upToDate: return "checkmark.circle"
        case .failed: return "exclamationmark.circle"
        default: return "arrow.down.circle"
        }
    }

    private var iconColor: Color {
        switch updater.state {
        case .upToDate: return p.success
        case .failed: return p.danger
        default: return p.accent
        }
    }

    private func header(_ info: UpdateInfo) -> some View {
        HStack(spacing: 12) {
            Image(systemName: "arrow.down.circle.fill").font(.system(size: 20)).foregroundStyle(p.accent)
            VStack(alignment: .leading, spacing: 2) {
                Text(loc("update.title")).font(.system(size: 13.5, weight: .semibold))
                Text(loc("update.versions", AppInfo.version, info.version)).font(.system(size: 12).monospacedDigit()).foregroundStyle(p.secondaryText)
            }
            Spacer()
            Text(Fmt.bytes(info.size)).font(.system(size: 11.5).monospacedDigit()).foregroundStyle(p.tertiaryText)
        }
    }

    private func available(_ info: UpdateInfo) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            header(info)
            VStack(alignment: .leading, spacing: 6) {
                Text(loc("update.notes")).font(.system(size: 11.5, weight: .medium)).foregroundStyle(p.secondaryText)
                Text(info.notes.isEmpty ? loc("update.noNotes") : info.notes)
                    .font(.system(size: 12.5)).foregroundStyle(info.notes.isEmpty ? p.tertiaryText : p.text)
                    .fixedSize(horizontal: false, vertical: true).lineLimit(12).textSelection(.enabled)
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 8).fill(p.elevated))
            HStack(spacing: 8) {
                Label(loc("update.verified"), systemImage: "checkmark.shield").font(.system(size: 11.5)).foregroundStyle(p.success)
                Spacer()
                Button(loc("update.recheck")) { Task { await updater.check(manual: true) } }.buttonStyle(SecondaryButtonStyle())
                Button(loc("update.download")) { updater.download() }.buttonStyle(PrimaryButtonStyle())
            }
        }
    }

    private func downloading(_ info: UpdateInfo, done: UInt64, total: UInt64) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            header(info)
            ProgressView(value: Double(done), total: Double(max(total, 1))).progressViewStyle(.linear).tint(p.accent)
            Text(loc("update.downloading", Fmt.bytes(done), Fmt.bytes(total))).font(.system(size: 11.5).monospacedDigit()).foregroundStyle(p.secondaryText)
        }
    }

    private func downloaded(_ info: UpdateInfo) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            header(info)
            HStack(alignment: .top, spacing: 8) {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(p.success)
                Text(loc("update.downloaded", info.fileName)).font(.system(size: 12.5)).fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Spacer()
                Button(loc("update.reveal")) { updater.revealDownloaded() }.buttonStyle(SecondaryButtonStyle())
                Button(loc("update.done")) { updater.reset() }.buttonStyle(PrimaryButtonStyle())
            }
        }
    }
}
