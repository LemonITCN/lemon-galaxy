import SwiftUI
import GalaxyKit

/// 经设备转发的 RDP：本机临时端口已经开好。切到 Windows App 后 LemonGalaxy 会被系统挂起，转发可能中断。
struct RdpLaunchSheet: View {
    let target: RdpTarget
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.dismiss) private var dismiss
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    LabeledContent(loc("rdp.localAddress")) {
                        Text("127.0.0.1:\(target.port)").font(.body.monospacedDigit()).textSelection(.enabled)
                    }
                    if !target.username.isEmpty {
                        LabeledContent(loc("field.username"), value: (target.domain.map { $0 + "\\" } ?? "") + target.username)
                    }
                } footer: {
                    HStack { Spacer(); InfoButton(text: loc("rdp.forwardInfo"), size: 13) }
                }
                .galaxyRow()
                Section {
                    Label(loc("rdp.suspendWarning"), systemImage: "exclamationmark.triangle.fill")
                        .font(.callout)
                        .foregroundStyle(p.warning)
                }
                .galaxyRow()
                Section {
                    Button {
                        dismiss()
                        app.launchRdp(target)
                    } label: {
                        Label(loc("rdp.openWindowsApp"), systemImage: "arrow.up.forward.app")
                    }
                    Button(role: .destructive) {
                        account.closeRdp(target.forwardId)
                        dismiss()
                    } label: {
                        Label(loc("rdp.stopForward"), systemImage: "stop.circle")
                    }
                }
                .galaxyRow()
            }
            .galaxyList()
            .navigationTitle(target.label)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(loc("common.close")) { dismiss() } }
            }
        }
        .presentationDetents([.medium, .large])
    }
}

/// 主机页顶部：进行中的 RDP 转发，可以重新打开或停止
struct RdpForwardRow: View {
    let forward: RdpForward
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        HStack(spacing: 12) {
            PlatformBadge(symbol: "display", tint: p.info, size: 30)
            VStack(alignment: .leading, spacing: 2) {
                Text(forward.label).foregroundStyle(p.text).lineLimit(1)
                Text("127.0.0.1:\(forward.localPort)").font(.caption.monospacedDigit()).foregroundStyle(p.secondaryText)
            }
            Spacer()
            Button {
                if let t = rdpTarget { app.launchRdp(t) }
            } label: { Image(systemName: "arrow.up.forward.app") }
                .buttonStyle(.borderless)
                .accessibilityLabel(loc("rdp.openWindowsApp"))
            Button(role: .destructive) { account.closeRdp(forward.id) } label: { Image(systemName: "stop.circle") }
                .buttonStyle(.borderless)
                .accessibilityLabel(loc("rdp.stopForward"))
        }
    }

    /// 重新打开时沿用同一个转发端口
    private var rdpTarget: RdpTarget? {
        guard let e = account.entry(forward.hostId), case let .rdpHost(label, _, _, user, _, domain, _, _, _, _) = e.data else { return nil }
        return RdpTarget(label: label, host: "127.0.0.1", port: forward.localPort, username: user, domain: domain, password: nil,
                         forwarded: true, forwardId: forward.id)
    }
}
