import SwiftUI

/// 通用页「无人值守」：开关 + 状态（未开启 / 等待批准 / 正在启动 / 正在移交 / 运行中 / 出错），说明都放在 i 里
struct UnattendedCard: View {
    @ObservedObject var ctl: UnattendedController
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        Card {
            VStack(alignment: .leading, spacing: 10) {
                HStack(spacing: 6) {
                    Text(loc("unattended.title")).font(.system(size: 13))
                    InfoButton(text: loc("unattended.info"))
                    Spacer()
                    if ctl.busy { ProgressView().controlSize(.small).padding(.trailing, 4) }
                    chip
                    Toggle("", isOn: Binding(get: { ctl.isOn }, set: { $0 ? ctl.enable() : ctl.requestDisable() }))
                        .toggleStyle(PaletteSwitchStyle(small: true)).labelsHidden()
                        .disabled(ctl.busy || unavailable)
                }
                detail
            }
        }
    }

    private var unavailable: Bool { if case .unavailable = ctl.phase { return true }; return false }

    private var chip: some View {
        let (key, color): (String, Color) = {
            switch ctl.phase {
            case .unavailable: return ("unattended.state.unavailable", p.tertiaryText)
            case .off: return ("unattended.state.off", p.tertiaryText)
            case .needsApproval: return ("unattended.state.approval", p.warning)
            case .starting: return ("unattended.state.starting", p.accent)
            case .migrating: return ("unattended.state.migrating", p.accent)
            case .running: return ("unattended.state.running", p.success)
            case .error: return ("unattended.state.error", p.danger)
            }
        }()
        return Text(loc(key)).font(.system(size: 11, weight: .medium)).foregroundStyle(color)
            .padding(.horizontal, 8).padding(.vertical, 2.5).background(Capsule().fill(color.opacity(0.12)))
    }

    @ViewBuilder
    private var detail: some View {
        switch ctl.phase {
        case .unavailable(let why):
            Text(why).font(.system(size: 12)).foregroundStyle(p.tertiaryText)
        case .off:
            if AppInfo.isBundled && !ctl.appInApplications {
                hint(icon: "folder.badge.questionmark", color: p.warning, text: loc("unattended.locationHint"))
            }
        case .needsApproval:
            HStack(spacing: 6) {
                Image(systemName: "exclamationmark.triangle.fill").font(.system(size: 11)).foregroundStyle(p.warning)
                Text(loc("unattended.approval")).font(.system(size: 12)).foregroundStyle(p.secondaryText)
                InfoButton(text: loc("unattended.approval.info"))
                Spacer()
                Button(loc("unattended.openSettings")) { ctl.openLoginItems() }.buttonStyle(SecondaryButtonStyle())
            }
        case .starting:
            progress(loc("unattended.starting"))
        case .migrating:
            progress(loc("unattended.migrating"))
        case .running:
            VStack(spacing: 0) {
                Rectangle().fill(p.separator).frame(height: 1).padding(.bottom, 8)
                row(loc("unattended.screen"), info: loc("unattended.screen.info"), value: ctl.screenDescription)
                row(loc("unattended.service"), info: loc("unattended.service.info"),
                    value: ctl.daemon.map { loc("unattended.service.version", $0.version, $0.build) } ?? "—")
                if let d = ctl.daemon, !d.peerAdmin {
                    hint(icon: "person.badge.key", color: p.tertiaryText, text: loc("unattended.nonAdmin")).padding(.top, 6)
                }
            }
        case .error(let msg):
            HStack(spacing: 6) {
                Image(systemName: "exclamationmark.octagon.fill").font(.system(size: 11)).foregroundStyle(p.danger)
                Text(msg).font(.system(size: 12)).foregroundStyle(p.secondaryText).fixedSize(horizontal: false, vertical: true)
                Spacer()
                if ctl.clientMode {
                    Button(loc("unattended.openSettings")) { ctl.openLoginItems() }.buttonStyle(SecondaryButtonStyle())
                }
            }
        }
    }

    private func progress(_ text: String) -> some View {
        HStack(spacing: 8) {
            ProgressView().controlSize(.small)
            Text(text).font(.system(size: 12)).foregroundStyle(p.secondaryText)
        }
    }

    private func hint(icon: String, color: Color, text: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Image(systemName: icon).font(.system(size: 11)).foregroundStyle(color)
            Text(text).font(.system(size: 12)).foregroundStyle(p.secondaryText).fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
    }

    private func row(_ label: String, info: String, value: String) -> some View {
        HStack(spacing: 6) {
            Text(label).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
            InfoButton(text: info)
            Spacer()
            Text(value).font(.system(size: 12.5)).lineLimit(1).truncationMode(.middle)
        }
        .padding(.vertical, 4)
    }
}

/// 关闭无人值守前确认（窗口内浮层）
struct UnattendedDisableView: View {
    @ObservedObject var ctl: UnattendedController
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                Image(systemName: "moon.zzz").font(.system(size: 18)).foregroundStyle(p.warning)
                Text(loc("unattended.disable.title")).font(.system(size: 15, weight: .semibold))
            }
            Text(loc(ctl.daemon?.peerAdmin == false ? "unattended.disable.bodyNonAdmin" : "unattended.disable.body"))
                .font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button(loc("common.cancel")) { ctl.confirmDisable = false }.buttonStyle(SecondaryButtonStyle()).keyboardShortcut(.cancelAction)
                Button(loc("unattended.disable.confirm")) { Task { await ctl.disable() } }.buttonStyle(PrimaryButtonStyle(destructive: true))
            }
        }
        .padding(22)
        .frame(width: 420)
    }
}
