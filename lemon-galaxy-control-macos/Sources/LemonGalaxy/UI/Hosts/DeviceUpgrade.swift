import SwiftUI

/// 主控端远程升级一台被控端（设备详情里的「升级被控端」，不用先开远程桌面；docs/ffi.md 5.0）。
/// 点了直接开始，不再弹确认；对方自己走完整个升级（下载、核对、替换、以新版本重新启动），全程不用人操作。
/// 进度存在 AppModel.upgrades 里，离开详情再回来还能看到；结束后刷新设备列表，不在看这台设备的详情时另弹一条提示。
enum DeviceUpgrade: Equatable {
    /// 已发出请求，等对方开始（参数是服务器报的新版本）
    case requesting(String)
    /// 对方开始升级到这个版本
    case installing(String)
    /// 对方已断开，正在替换、重启
    case restarting(String)
    /// 已以新版本重新上线
    case done(String)
    /// 对方本来就是最新（列表里的「可升级」已过时）；参数是对方现在的版本，可能为空
    case upToDate(String)
    /// 失败原因（对方版本太旧不支持远程升级、没有控制权限、超时没以新版本上线等）
    case failed(String)

    var inProgress: Bool {
        switch self {
        case .requesting, .installing, .restarting: return true
        default: return false
        }
    }

    /// 对方已经开始升级（之后的「结束」才算升级成功，否则是本来就最新）
    var started: Bool {
        switch self {
        case .installing, .restarting: return true
        default: return false
        }
    }

    var targetVersion: String? {
        switch self {
        case .requesting(let v), .installing(let v), .restarting(let v): return v.isEmpty ? nil : v
        default: return nil
        }
    }
}

extension AppModel {
    /// 能远程升级的设备：被控端，或带「本机被控」的 Mac 客户端；不含本机自己与手机
    nonisolated static func supportsRemoteUpgrade(_ d: DeviceInfo) -> Bool {
        guard !d.isSelf else { return false }
        return d.kind == .worker || (d.kind == .control && d.platform == .macos)
    }

    /// 设备列表上标「可升级」：服务器上有比它新的版本
    func hasUpgrade(_ d: DeviceInfo) -> Bool { Self.supportsRemoteUpgrade(d) && !d.updateVersion.isEmpty }

    /// 详情里能不能点「升级被控端」
    func canStartUpgrade(_ d: DeviceInfo) -> Bool {
        Self.supportsRemoteUpgrade(d) && d.online && upgrades[d.id]?.inProgress != true
    }

    /// 让对方升级到服务器上的最新版本：立即返回，进度经 upgrades 显示（内核连上对方发请求，再等它以新版本重新上线，最多 10 分钟）
    func upgradeDevice(_ id: String) {
        guard let d = device(id), canStartUpgrade(d) else { return }
        upgrades[id] = .requesting(d.updateVersion)
        client.upgradeDevice(deviceId: id, listener: DeviceUpgradeListener(deviceId: id, model: self))
    }

    func upgradeStarted(_ id: String, version: String) {
        upgrades[id] = .installing(version)
    }

    func upgradeOffline(_ id: String) {
        upgrades[id] = .restarting(upgrades[id]?.targetVersion ?? device(id)?.updateVersion ?? "")
    }

    /// 正在看这台设备的详情（结果就地显示，不另弹提示）
    func isViewingDevice(_ id: String) -> Bool {
        section == .hosts && activeTabId == nil && selection == "dev:" + id
    }

    func upgradeFinished(_ id: String, version: String?, error: String?) {
        let started = upgrades[id]?.started ?? false
        let name = deviceName(id)
        let viewing = isViewingDevice(id)
        if let error, !error.isEmpty {
            upgrades[id] = .failed(error)
            if !viewing { showToast(L("upgrade.toast.failed", name, error), error: true) }
        } else if started {
            let v = version ?? upgrades[id]?.targetVersion ?? ""
            upgrades[id] = .done(v)
            if !viewing { showToast(L("upgrade.toast.done", name, v)) }
        } else {
            upgrades[id] = .upToDate(version ?? "")
        }
        reloadDevices()
        Task { try? await client.refreshDevices() }
    }
}

/// 内核线程 → 主线程
final class DeviceUpgradeListener: UpgradeListener, @unchecked Sendable {
    private let deviceId: String
    private weak var model: AppModel?

    init(deviceId: String, model: AppModel) {
        self.deviceId = deviceId
        self.model = model
    }

    func onStarted(version: String) {
        let id = deviceId
        DispatchQueue.main.async { self.model?.upgradeStarted(id, version: version) }
    }

    func onOffline() {
        let id = deviceId
        DispatchQueue.main.async { self.model?.upgradeOffline(id) }
    }

    func onFinished(version: String?, error: String?) {
        let id = deviceId
        DispatchQueue.main.async { self.model?.upgradeFinished(id, version: version, error: error) }
    }
}

/// 设备列表里的小标记：有新版本时「可升级」，升级进行中「升级中」
struct UpgradeBadge: View {
    let device: DeviceInfo
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        if app.upgrades[device.id]?.inProgress == true {
            chip(loc("upgrade.badge.busy")).help(loc("upgrade.badge.help", device.updateVersion))
        } else if app.hasUpgrade(device) {
            chip(loc("upgrade.badge")).help(loc("upgrade.badge.help", device.updateVersion))
        }
    }

    private func chip(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 10.5, weight: .medium))
            .foregroundStyle(p.accent)
            .padding(.horizontal, 6).padding(.vertical, 1.5)
            .background(RoundedRectangle(cornerRadius: 4).fill(p.accent.opacity(0.12)))
            .fixedSize()
    }
}

/// 设备详情里的版本与「升级被控端」：显示当前版本、可升级到的版本，进度就地显示
struct UpgradeCard: View {
    let device: DeviceInfo
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    /// 详情里要不要显示这张卡：能被控、有新版本或者有进行中 / 刚结束的升级
    static func shown(_ d: DeviceInfo, app: AppModel) -> Bool {
        AppModel.supportsRemoteUpgrade(d) && (!d.updateVersion.isEmpty || app.upgrades[d.id] != nil)
    }

    var body: some View {
        let state = app.upgrades[device.id]
        let hasUpdate = !device.updateVersion.isEmpty
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 12) {
                RoundedRectangle(cornerRadius: 8).fill(iconColor(state).opacity(0.12)).frame(width: 34, height: 34)
                    .overlay(Image(systemName: iconName(state)).font(.system(size: 15)).foregroundStyle(iconColor(state)))
                VStack(alignment: .leading, spacing: 3) {
                    Text(hasUpdate ? loc("upgrade.available", device.updateVersion) : loc("upgrade.currentTitle", device.appVersion))
                        .font(.system(size: 13, weight: .medium))
                    if hasUpdate {
                        Text(loc("upgrade.current", device.appVersion)).font(.system(size: 11.5)).foregroundStyle(p.secondaryText)
                    }
                }
                Spacer(minLength: 8)
                InfoButton(text: loc("upgrade.info"))
                if hasUpdate || state?.inProgress == true {
                    Button { app.upgradeDevice(device.id) } label: {
                        if case .failed = state {
                            Label(loc("upgrade.retry"), systemImage: "arrow.clockwise")
                        } else {
                            Label(loc("upgrade.action"), systemImage: "arrow.up.circle")
                        }
                    }
                    .buttonStyle(PrimaryButtonStyle())
                    .disabled(!app.canStartUpgrade(device))
                }
            }
            .padding(12)
            if let line = statusLine(state, hasUpdate: hasUpdate) {
                Rectangle().fill(p.separator).frame(height: 1)
                HStack(spacing: 7) {
                    if state?.inProgress == true {
                        ProgressView().controlSize(.mini)
                    } else {
                        Image(systemName: line.icon).font(.system(size: 12)).foregroundStyle(line.color)
                    }
                    Text(line.text).font(.system(size: 12)).foregroundStyle(line.color)
                        .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 14).padding(.vertical, 9)
            }
        }
        .background(RoundedRectangle(cornerRadius: 10).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(p.border, lineWidth: 0.6))
    }

    private struct Line {
        let icon: String
        let text: String
        let color: Color
    }

    private func statusLine(_ s: DeviceUpgrade?, hasUpdate: Bool) -> Line? {
        switch s {
        case .requesting(let v), .installing(let v):
            let text = v.isEmpty ? loc("upgrade.installingPlain") : loc("upgrade.installing", v)
            return Line(icon: "", text: text, color: p.secondaryText)
        case .restarting:
            return Line(icon: "", text: loc("upgrade.restarting"), color: p.secondaryText)
        case .done(let v):
            return Line(icon: "checkmark.circle.fill", text: loc("upgrade.done", v), color: p.success)
        case .upToDate(let v):
            return Line(icon: "checkmark.circle", text: v.isEmpty ? loc("upgrade.upToDatePlain") : loc("upgrade.upToDate", v), color: p.secondaryText)
        case .failed(let e):
            return Line(icon: "exclamationmark.triangle.fill", text: loc("upgrade.failed", e), color: p.danger)
        case nil:
            // 离线时按钮不可用：说明要在线
            if hasUpdate && !device.online { return Line(icon: "powersleep", text: loc("upgrade.needOnline"), color: p.secondaryText) }
            return nil
        }
    }

    /// 失败时图标不变（还能重试），原因只在下面那行标红
    private func iconName(_ s: DeviceUpgrade?) -> String {
        switch s {
        case .done, .upToDate: return "checkmark.circle"
        default: return "arrow.up.circle"
        }
    }

    private func iconColor(_ s: DeviceUpgrade?) -> Color {
        switch s {
        case .done: return p.success
        case .upToDate: return p.secondaryText
        default: return p.accent
        }
    }
}
