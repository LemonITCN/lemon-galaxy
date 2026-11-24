import SwiftUI
import AppKit

/// 虚拟屏的模式（宽 × 高 @ 刷新率），范围与内核 galaxy_proto::peer::virtual_display_mode_ok 一致。
struct VirtualDisplayMode: Equatable, Hashable {
    var width: UInt32
    var height: UInt32
    var refresh: UInt32

    static let standard = VirtualDisplayMode(width: 1920, height: 1080, refresh: 60)
    /// 常用分辨率：16:9 四档 + 16:10 两档
    static let presets: [(UInt32, UInt32)] = [(1280, 720), (1920, 1080), (2560, 1440), (3840, 2160), (1920, 1200), (2560, 1600)]
    static let refreshPresets: [UInt32] = [30, 60, 120, 144]

    var valid: Bool { Self.sizeValid(width, height) && Self.refreshValid(refresh) }
    static func sizeValid(_ w: UInt32, _ h: UInt32) -> Bool { (640...7680).contains(w) && (480...4320).contains(h) }
    static func refreshValid(_ hz: UInt32) -> Bool { (24...240).contains(hz) }

    var sizeLabel: String { "\(width)×\(height)" }
    var label: String { "\(width)×\(height) @ \(refresh) Hz" }

    /// 本机主屏的物理像素（超出范围为 nil，界面就不给这一档）
    @MainActor static var localScreen: (UInt32, UInt32)? {
        guard let s = NSScreen.main ?? NSScreen.screens.first else { return nil }
        let w = UInt32((s.frame.width * s.backingScaleFactor).rounded())
        let h = UInt32((s.frame.height * s.backingScaleFactor).rounded())
        return sizeValid(w, h) ? (w, h) : nil
    }
}

// MARK: - 模型：加屏、删屏

extension RemoteDesktopModel {
    var virtualSupported: Bool { virtualMax > 0 }
    var virtualFull: Bool { myVirtual.count >= Int(virtualMax) }
    func isMyVirtual(_ id: UInt32) -> Bool { myVirtual[id] != nil }

    /// 在被控端加一块虚拟屏：成功后刷新屏幕列表并切过去看
    @discardableResult
    func addVirtualDisplay(_ mode: VirtualDisplayMode) async -> Bool {
        guard let peer, !virtualBusy, mode.valid else { return false }
        virtualBusy = true
        defer { virtualBusy = false }
        do {
            let d = try await peer.session.addVirtualDisplay(width: mode.width, height: mode.height, refresh: mode.refresh)
            // 等待期间会话换了或断了：被控端随旧会话已删掉，不再理会
            guard !closed, self.peer === peer, peer.closedReason == nil else { return false }
            myVirtual[d.id] = mode
            await refreshDisplays(keeping: d)
            await open(d.id)
            app?.showToast(L("remote.vd.added", mode.sizeLabel))
            return true
        } catch {
            if !closed { app?.showToast(L("remote.vd.addFailed", AppModel.describe(error)), error: true) }
            return false
        }
    }

    /// 删掉本会话加的虚拟屏；正在看它就先切回主屏，免得画面先断
    func removeVirtualDisplay(_ id: UInt32) async {
        guard let peer, !virtualBusy, isMyVirtual(id) else { return }
        virtualBusy = true
        defer { virtualBusy = false }
        if displayId == id, let next = displays.first(where: { $0.primary && $0.id != id }) ?? displays.first(where: { $0.id != id }) {
            await open(next.id)
        }
        do {
            try await peer.session.removeVirtualDisplay(displayId: id)
            myVirtual[id] = nil
            displays.removeAll { $0.id == id }
            await refreshDisplays()
            // 被控端拔屏是异步的，刚删的可能还在列表里
            displays.removeAll { $0.id == id }
            if !closed { app?.showToast(L("remote.vd.removed")) }
        } catch {
            if !closed { app?.showToast(L("remote.vd.removeFailed", AppModel.describe(error)), error: true) }
        }
    }

    /// 重新列屏幕。keeping：刚加的屏（被控端系统里可能还没出现，先放进列表）
    func refreshDisplays(keeping added: DisplayInfo? = nil) async {
        guard let peer else { return }
        if var list = try? await peer.session.listDisplays() {
            if let added, !list.contains(where: { $0.id == added.id }) { list.append(added) }
            displays = list
        } else if let added, !displays.contains(where: { $0.id == added.id }) {
            displays.append(added)
        }
        // 列表里已经没有的（被控端那边没了）就不再算我加的
        myVirtual = myVirtual.filter { id, _ in displays.contains { $0.id == id } }
    }
}

// MARK: - 工具栏按钮 + 弹出层

/// 工具栏「加虚拟屏」按钮：弹出层里是加屏表单与我加的虚拟屏列表
struct RemoteVirtualDisplayButton: View {
    @ObservedObject var tab: RemoteDesktopModel
    @Environment(\.loc) var loc
    @State private var shown = false

    var body: some View {
        IconButton(icon: "rectangle.badge.plus", help: loc("remote.vd.add"), active: shown || !tab.myVirtual.isEmpty) { shown.toggle() }
            .popover(isPresented: $shown, arrowEdge: .bottom) {
                RemoteVirtualDisplayPanel(tab: tab) { shown = false }
            }
    }
}

struct RemoteVirtualDisplayPanel: View {
    @ObservedObject var tab: RemoteDesktopModel
    var onAdded: () -> Void = {}
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    private enum SizeChoice: Hashable { case preset(UInt32, UInt32), local, custom }
    private enum RefreshChoice: Hashable { case preset(UInt32), custom }
    @State private var size = SizeChoice.preset(1920, 1080)
    @State private var refresh = RefreshChoice.preset(60)
    @State private var width = "1920"
    @State private var height = "1080"
    @State private var customHz = "60"

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Text(loc("remote.vd.title")).font(.system(size: 13, weight: .semibold)).foregroundStyle(p.text)
                InfoButton(text: loc("remote.vd.info"))
                Spacer(minLength: 8)
                Text(tab.virtualFull ? loc("remote.vd.full", tab.virtualMax) : loc("remote.vd.count", tab.myVirtual.count, tab.virtualMax))
                    .font(.system(size: 11.5).monospacedDigit())
                    .foregroundStyle(tab.virtualFull ? p.warning : p.tertiaryText)
            }
            .padding(.bottom, 12)

            label(loc("remote.vd.resolution"))
            LazyVGrid(columns: [GridItem(.flexible(), spacing: 6), GridItem(.flexible(), spacing: 6)], spacing: 6) {
                ForEach(Array(VirtualDisplayMode.presets.enumerated()), id: \.offset) { _, wh in
                    option("\(wh.0)×\(wh.1)", selected: size == .preset(wh.0, wh.1)) { size = .preset(wh.0, wh.1) }
                }
                if let local = VirtualDisplayMode.localScreen {
                    option(loc("remote.vd.local"), detail: "\(local.0)×\(local.1)", selected: size == .local) { size = .local }
                }
                option(loc("remote.vd.custom"), selected: size == .custom) { size = .custom }
            }
            if size == .custom {
                HStack(spacing: 6) {
                    numberField(loc("remote.vd.width"), $width, invalid: !customSizeValid)
                    Text("×").foregroundStyle(p.secondaryText)
                    numberField(loc("remote.vd.height"), $height, invalid: !customSizeValid)
                }
                .padding(.top, 8)
            }

            label(loc("remote.vd.refresh")).padding(.top, 14)
            HStack(spacing: 6) {
                ForEach(VirtualDisplayMode.refreshPresets, id: \.self) { hz in
                    option("\(hz) Hz", selected: refresh == .preset(hz)) { refresh = .preset(hz) }
                }
                option(loc("remote.vd.custom"), selected: refresh == .custom) { refresh = .custom }
            }
            if refresh == .custom {
                HStack(spacing: 6) {
                    numberField("60", $customHz, invalid: !customHzValid).frame(width: 72)
                    Text("Hz").font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                }
                .padding(.top, 8)
            }
            if mode == nil {
                Text(loc("remote.vd.range")).font(.system(size: 11.5)).foregroundStyle(p.danger).padding(.top, 8)
            }

            Button {
                guard let m = mode else { return }
                Task { if await tab.addVirtualDisplay(m) { onAdded() } }
            } label: {
                HStack(spacing: 6) {
                    if tab.virtualBusy { ProgressView().controlSize(.mini).frame(width: 12, height: 12) }
                    Text(loc("remote.vd.add"))
                }
                .frame(maxWidth: .infinity)
            }
            .buttonStyle(PrimaryButtonStyle())
            .disabled(mode == nil || tab.virtualFull || tab.virtualBusy)
            .padding(.top, 16)

            if !tab.myVirtual.isEmpty {
                Rectangle().fill(p.separator).frame(height: 1).padding(.vertical, 14)
                label(loc("remote.vd.mine"))
                VStack(spacing: 4) {
                    ForEach(tab.displays.filter { tab.isMyVirtual($0.id) }, id: \.id) { d in mineRow(d) }
                }
            }
        }
        .padding(16)
        .frame(width: 340)
    }

    private func label(_ text: String) -> some View {
        Text(text).font(.system(size: 11.5, weight: .medium)).foregroundStyle(p.secondaryText).padding(.bottom, 6)
    }

    private func option(_ title: String, detail: String? = nil, selected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 4) {
                Text(verbatim: title).font(.system(size: 12, weight: selected ? .semibold : .regular).monospacedDigit())
                if let detail { Text(verbatim: detail).font(.system(size: 11).monospacedDigit()).foregroundStyle(selected ? p.accent.opacity(0.8) : p.tertiaryText) }
            }
            .lineLimit(1)
            .foregroundStyle(selected ? p.accent : p.text)
            .frame(maxWidth: .infinity).frame(height: 28)
            .background(RoundedRectangle(cornerRadius: 7).fill(selected ? p.accent.opacity(0.12) : p.elevated))
            .overlay(RoundedRectangle(cornerRadius: 7).stroke(selected ? p.accent : p.border, lineWidth: selected ? 1 : 0.7))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private func numberField(_ placeholder: String, _ text: Binding<String>, invalid: Bool) -> some View {
        TextField(placeholder, text: Binding(get: { text.wrappedValue }, set: { text.wrappedValue = String($0.filter(\.isNumber).prefix(4)) }))
            .multilineTextAlignment(.center)
            .fieldStyle(invalid: invalid && !text.wrappedValue.isEmpty)
    }

    /// 我加的虚拟屏一行：屏号、模式、切过去看、删掉
    private func mineRow(_ d: DisplayInfo) -> some View {
        let index = (tab.displays.firstIndex { $0.id == d.id } ?? 0) + 1
        let viewing = tab.displayId == d.id
        return HStack(spacing: 8) {
            Image(systemName: "rectangle.dashed").font(.system(size: 12)).foregroundStyle(viewing ? p.accent : p.secondaryText)
            VStack(alignment: .leading, spacing: 1) {
                Text(loc("remote.screen", index)).font(.system(size: 12.5, weight: viewing ? .semibold : .regular)).foregroundStyle(p.text)
                Text(verbatim: tab.myVirtual[d.id]?.label ?? "\(d.width)×\(d.height)")
                    .font(.system(size: 11).monospacedDigit()).foregroundStyle(p.tertiaryText)
            }
            Spacer(minLength: 6)
            if !viewing {
                IconButton(icon: "eye", help: loc("remote.vd.view"), size: 12) { Task { await tab.open(d.id) } }
            }
            IconButton(icon: "trash", help: loc("remote.vd.remove"), size: 12) { Task { await tab.removeVirtualDisplay(d.id) } }
                .disabled(tab.virtualBusy)
        }
        .padding(.horizontal, 8).padding(.vertical, 5)
        .background(RoundedRectangle(cornerRadius: 7).fill(viewing ? p.accent.opacity(0.08) : .clear))
    }

    private var customSizeValid: Bool {
        guard let w = UInt32(width), let h = UInt32(height) else { return false }
        return VirtualDisplayMode.sizeValid(w, h)
    }
    private var customHzValid: Bool { UInt32(customHz).map(VirtualDisplayMode.refreshValid) ?? false }

    /// 表单当前选的模式；自定义值不合法时为 nil
    private var mode: VirtualDisplayMode? {
        let wh: (UInt32, UInt32)?
        switch size {
        case .preset(let w, let h): wh = (w, h)
        case .local: wh = VirtualDisplayMode.localScreen
        case .custom: wh = customSizeValid ? (UInt32(width)!, UInt32(height)!) : nil
        }
        let hz: UInt32?
        switch refresh {
        case .preset(let r): hz = r
        case .custom: hz = customHzValid ? UInt32(customHz) : nil
        }
        guard let wh, let hz else { return nil }
        return VirtualDisplayMode(width: wh.0, height: wh.1, refresh: hz)
    }
}
