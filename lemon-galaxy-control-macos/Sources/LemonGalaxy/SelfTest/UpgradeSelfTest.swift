import AppKit
import SwiftUI

extension SelfTest {
    /// 远程升级被控端（UI/Hosts/DeviceUpgrade.swift）：列表标记、详情按钮、离线不可用、进行中、成功、失败、本来就最新。
    /// 模拟内核里构建服务器能走完整个升级，前台电脑离线且版本太旧（开机后再升级会失败），Design-iMac 没有新版本
    static func checkUpgrade(prefix: String, zh: Bool, app: AppModel, window: NSWindow, host: NSHostingView<RootView>) {
        guard let mock = app.client as? MockGalaxyClient else { problems.append("远程升级：不是模拟内核"); return }
        func dev(_ id: String) -> DeviceInfo? { app.device(id) }
        func state(_ id: String) -> DeviceUpgrade? { app.upgrades[id] }

        // 哪些设备能升级、哪些标「可升级」
        let badged = Set(app.devices.filter { app.hasUpgrade($0) }.map(\.id))
        if badged != ["dev-build", "dev-reception"] { problems.append("可升级标记应只在构建服务器与前台电脑上：\(badged.sorted())") }
        for (id, want) in [("dev-build", true), ("dev-imac", true), ("dev-reception", true), ("dev-self", false), ("dev-phone", false), ("dev-office", false)] {
            if let d = dev(id), AppModel.supportsRemoteUpgrade(d) != want { problems.append("远程升级支持判断不对：\(id) 应为 \(want)") }
        }
        let otherMac = DeviceInfo(id: "mac-control", kind: .control, platform: .macos, hostname: "Studio", osVersion: "macOS 15", arch: "arm64",
                                  appVersion: "0.1.0", online: true, lanSeen: false, lastSeenAt: 0, isSelf: false, updateVersion: "0.2.0")
        if !AppModel.supportsRemoteUpgrade(otherMac) { problems.append("别的 Mac 客户端（带本机被控）应能远程升级") }
        if let r = dev("dev-reception"), app.canStartUpgrade(r) { problems.append("离线的前台电脑不应能点升级") }
        if let b = dev("dev-build"), !app.canStartUpgrade(b) { problems.append("在线的构建服务器应能点升级") }

        app.selection = nil
        shot("\(prefix)-19b-hosts-upgrade-list", app, window: window, host: host)
        app.selection = "dev:dev-build"
        shot("\(prefix)-19c-hosts-upgrade-detail", app, window: window, host: host)
        app.selection = "dev:dev-reception"
        shot("\(prefix)-19d-hosts-upgrade-offline", app, window: window, host: host)

        // 进行中的两步：直接设状态截图（模拟内核在自测里一口气走完，后面再真正走一遍）
        app.selection = "dev:dev-build"
        app.upgrades["dev-build"] = .installing("0.2.0")
        if let b = dev("dev-build"), app.canStartUpgrade(b) { problems.append("升级进行中按钮应不可用") }
        shot("\(prefix)-19e-hosts-upgrade-installing", app, window: window, host: host)
        // 离开详情再回来，进度还在
        app.selection = "dev:dev-office"
        pump(0.1)
        app.selection = "dev:dev-build"
        app.upgrades["dev-build"] = .restarting("0.2.0")
        shot("\(prefix)-19f-hosts-upgrade-restarting", app, window: window, host: host)
        app.upgrades["dev-build"] = nil

        // 真正走一遍：请求 → 对方开始 → 断开 → 以新版本上线；进行中再点不会重复发。
        // 点完回到概览（不在看这台设备的详情），结束时应另弹提示
        var seen: [DeviceUpgrade] = []
        let sub = app.$upgrades.sink { all in
            if let s = all["dev-build"], seen.last != s { seen.append(s) }
        }
        app.upgradeDevice("dev-build")
        app.upgradeDevice("dev-build")
        app.selection = nil
        wait("远程升级完成") { if case .done = state("dev-build") { return true }; return false }
        sub.cancel()
        let expected: [DeviceUpgrade] = [.requesting("0.2.0"), .installing("0.2.0"), .restarting("0.2.0"), .done("0.2.0")]
        if seen != expected { problems.append("远程升级的进度顺序不对：\(seen)") }
        wait("升级后设备列表刷新") { dev("dev-build")?.appVersion == "0.2.0" }
        if let b = dev("dev-build") {
            if !b.online || !b.updateVersion.isEmpty { problems.append("升级完成后应在线且没有新版本：\(b)") }
            if app.hasUpgrade(b) { problems.append("升级完成后不应再标可升级") }
        }
        if app.toast?.isError != false || app.toast?.text.contains("0.2.0") != true {
            problems.append("不在详情里时升级成功应提示新版本：\(app.toast?.text ?? "无提示")")
        }
        app.toast = nil
        app.selection = "dev:dev-build"
        shot("\(prefix)-19g-hosts-upgrade-done", app, window: window, host: host)
        app.toast = nil

        // 失败：前台电脑开机后再升级，版本太旧不支持远程升级
        mock.mockUpdateDevice("dev-reception") { $0.online = true; $0.lastSeenAt = MockData.now }
        wait("前台电脑上线") { dev("dev-reception")?.online == true }
        app.selection = "dev:dev-reception"
        app.upgradeDevice("dev-reception")
        wait("远程升级失败") { if case .failed = state("dev-reception") { return true }; return false }
        if case .failed(let e) = state("dev-reception"), !e.contains(zh ? "太旧" : "too old") { problems.append("升级失败应带原因：\(e)") }
        if dev("dev-reception")?.appVersion != "0.1.0" { problems.append("升级失败不应改设备版本") }
        if let r = dev("dev-reception"), !app.canStartUpgrade(r) { problems.append("升级失败后应能重试") }
        if app.toast != nil { problems.append("正在看详情时结果就地显示，不应另弹提示：\(app.toast?.text ?? "")") }
        shot("\(prefix)-19h-hosts-upgrade-failed", app, window: window, host: host)
        app.toast = nil

        // 本来就是最新：内核直接报对方现在的版本，不算「已升级」，也不弹提示
        app.upgradeDevice("dev-imac")
        wait("已是最新") { if case .upToDate = state("dev-imac") { return true }; return false }
        if state("dev-imac") != .upToDate("0.1.0") { problems.append("本来就最新的状态不对：\(String(describing: state("dev-imac")))") }
        if app.toast != nil { problems.append("本来就最新不应弹提示：\(app.toast?.text ?? "")") }
        app.selection = "dev:dev-imac"
        shot("\(prefix)-19i-hosts-upgrade-uptodate", app, window: window, host: host)

        // 还原，后面的截图不受影响
        mock.mockUpdateDevice("dev-reception") { $0.online = false }
        mock.mockUpdateDevice("dev-build") { $0.appVersion = "0.1.0"; $0.updateVersion = "0.2.0" }
        wait("设备还原") { dev("dev-reception")?.online == false && dev("dev-build")?.updateVersion == "0.2.0" }
        app.upgrades = [:]
        app.selection = nil
        app.toast = nil
    }
}
