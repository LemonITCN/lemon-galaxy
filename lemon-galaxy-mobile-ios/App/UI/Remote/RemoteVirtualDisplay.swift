import SwiftUI
import UIKit
import GalaxyKit

// 显示器面板里的「加虚拟屏」（与安卓端一致）：屏幕卡片横排最后的虚线框卡片，点开在同一面板内展开加屏表单。
// 表单状态在 DockPanelView 里（面板内容在 ViewThatFits 里有两份，状态要只有一份），这里只放视图。

/// 屏幕卡片横排最后的虚线框卡片：加号 + 「加虚拟屏」+「n / N」，已加满时置灰
struct AddVirtualDisplayCard: View {
    let quota: VirtualDisplayQuota
    let enabled: Bool
    let action: () -> Void
    @Environment(\.loc) private var loc

    private var fg: Color { enabled ? .white : DockColors.disabled }

    var body: some View {
        Button(action: action) { face }
            .buttonStyle(.plain)
            .disabled(!enabled)
            .accessibilityLabel(loc("remote.vdAdd"))
            .accessibilityValue(loc("remote.vdCount", quota.added, quota.max))
    }

    private var face: some View {
        HStack(spacing: 8) {
            Image(systemName: "plus")
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(fg)
                .frame(width: 24, height: 24)
                .background(RoundedRectangle(cornerRadius: 6, style: .continuous).fill(Color.white.opacity(enabled ? 0.14 : 0.06)))
            VStack(alignment: .leading, spacing: 1) {
                Text(loc("remote.vdAdd")).font(.caption).foregroundStyle(fg).lineLimit(1)
                Text("\(quota.added) / \(quota.max)")
                    .font(.system(size: 11).monospacedDigit())
                    .foregroundStyle(enabled ? DockColors.dim : DockColors.disabled)
                    .lineLimit(1)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .frame(minWidth: 128, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .strokeBorder(enabled ? Color.white.opacity(0.35) : DockColors.line, style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
        )
        .contentShape(Rectangle())
    }
}

/// 加屏表单的标题行（替换面板标题）：返回、「加虚拟屏」、说明、「已加 n / N」
struct VirtualDisplayFormHeader: View {
    @ObservedObject var session: RemoteSession
    let onBack: () -> Void
    @Environment(\.loc) private var loc

    var body: some View {
        let quota = session.virtualQuota
        HStack(spacing: 4) {
            Button(action: onBack) {
                Image(systemName: "chevron.left")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(.white)
                    .frame(width: 40, height: 40)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(loc("common.back"))
            Text(loc("remote.vdTitle"))
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.white)
                .lineLimit(1)
                .minimumScaleFactor(0.8)
            InfoButton(text: loc("remote.vdInfo"))
            Spacer(minLength: 4)
            Text(loc("remote.vdCount", quota.added, quota.max))
                .font(.caption.monospacedDigit())
                .foregroundStyle(DockColors.dim)
                .lineLimit(1)
        }
        .padding(.leading, 4)
        .padding(.trailing, 16)
        .padding(.top, 4)
    }
}

/// 加屏表单：分辨率（常用档位 / 本机屏幕 / 自定义宽高）、刷新率（四档 / 自定义）、主按钮「加虚拟屏」
struct VirtualDisplayFormView: View {
    @ObservedObject var session: RemoteSession
    @Binding var form: VirtualDisplayForm
    /// 加好了：表单收起回到屏幕卡片视图（画面已切到新屏）
    let onAdded: () -> Void
    @Environment(\.loc) private var loc
    @FocusState private var focus: Field?

    enum Field: Hashable { case width, height, refresh }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            PanelSectionTitle(text: loc("remote.vdResolution"))
            DockGrid(items: form.resolutionChoices, columns: 3) { r in
                DockChip(label: label(r), selected: form.resolution == r) { form.selectResolution(r) }
            }
            if form.resolution == .local, let s = form.local {
                Text(s.label)
                    .font(.system(size: 11).monospacedDigit())
                    .foregroundStyle(DockColors.dim)
                    .padding(.top, 6)
            }
            if form.resolution == .custom {
                sizeInputs.padding(.top, 8)
                if form.rangeHintAfterSize { rangeHint }
            }
            PanelSectionTitle(text: loc("remote.vdRefresh"))
            HStack(spacing: 6) {
                ForEach(form.refreshChoices, id: \.self) { r in
                    DockChip(label: label(r), selected: form.refresh == r) { form.selectRefresh(r) }
                }
            }
            if form.refresh == .custom {
                refreshInput.padding(.top, 8)
                if form.rangeHintAfterRefresh { rangeHint }
            }
            submit.padding(.top, 14)
        }
    }

    private func label(_ r: VirtualDisplayForm.Resolution) -> String {
        switch r {
        case .preset(let s): return s.label
        case .local: return loc("remote.vdLocal")
        case .custom: return loc("remote.vdCustom")
        }
    }

    private func label(_ r: VirtualDisplayForm.Refresh) -> String {
        switch r {
        case .preset(let hz): return "\(hz) Hz"
        case .custom: return loc("remote.vdCustom")
        }
    }

    private var sizeInputs: some View {
        HStack(spacing: 6) {
            numberField(loc("remote.vdWidth"), text: Binding(get: { form.customWidth }, set: { form.setCustomWidth($0) }), field: .width)
            Text("×").font(.system(size: 13)).foregroundStyle(DockColors.dim)
            numberField(loc("remote.vdHeight"), text: Binding(get: { form.customHeight }, set: { form.setCustomHeight($0) }), field: .height)
        }
    }

    private var refreshInput: some View {
        HStack(spacing: 6) {
            numberField(loc("remote.vdRefresh"), text: Binding(get: { form.customRefresh }, set: { form.setCustomRefresh($0) }), field: .refresh)
                .frame(maxWidth: 110)
            Text("Hz").font(.system(size: 13)).foregroundStyle(DockColors.dim)
            Spacer(minLength: 0)
        }
    }

    /// 数字输入框（数字键盘）：深色底、居中、聚焦时描边
    private func numberField(_ name: String, text: Binding<String>, field: Field) -> some View {
        TextField("", text: text, prompt: Text(name).foregroundColor(DockColors.disabled))
            .keyboardType(.numberPad)
            .focused($focus, equals: field)
            .font(.system(size: 14).monospacedDigit())
            .foregroundStyle(.white)
            .tint(DockColors.accent)
            .multilineTextAlignment(.center)
            .padding(.horizontal, 8)
            .frame(height: 36)
            .frame(maxWidth: .infinity)
            .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(DockColors.chip))
            .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).stroke(focus == field ? DockColors.accent : .clear, lineWidth: 1))
            .environment(\.colorScheme, .dark)
            .accessibilityLabel(name)
    }

    /// 自定义值不合法时的范围说明（校验提示）：已经输入了内容用醒目颜色，还没填时只是提示
    private var rangeHint: some View {
        Text(loc("remote.vdRange"))
            .font(.system(size: 11))
            .foregroundStyle(form.rangeHintIsError ? DockColors.danger : DockColors.dim)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.top, 6)
    }

    private var submit: some View {
        let quota = session.virtualQuota
        let busy = session.virtualAdding
        let enabled = form.canSubmit(quota: quota, busy: busy)
        let lit = enabled || busy
        return Button(action: add) {
            HStack(spacing: 8) {
                if busy {
                    ProgressView().tint(DockColors.onAccent).controlSize(.small)
                }
                Text(quota.full ? loc("remote.vdFull", quota.max) : loc("remote.vdAdd"))
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
            .foregroundStyle(lit ? DockColors.onAccent : DockColors.disabled)
            .frame(maxWidth: .infinity)
            .frame(height: 44)
            .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(lit ? DockColors.accent : DockColors.chip))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
    }

    private func add() {
        guard let r = form.request else { return }
        focus = nil
        Task {
            if await session.addVirtualDisplay(r) { onAdded() }
        }
    }
}

/// 竖向滚动区：内容不高时贴合内容高度，超出可用高度才滚动。
/// 不像 ViewThatFits 那样在可用高度变化（键盘弹出）时换成另一份视图，输入框不会因此丢焦点、键盘不会来回弹。
struct HuggingScrollView<Content: View>: View {
    @ViewBuilder let content: () -> Content
    @State private var height: CGFloat = 0

    var body: some View {
        ScrollView(.vertical, showsIndicators: false) {
            content()
                .background(GeometryReader { g in Color.clear.preference(key: HuggingHeightKey.self, value: g.size.height) })
        }
        .frame(maxHeight: height > 0 ? height : nil)
        .scrollDismissesKeyboard(.interactively)
        .onPreferenceChange(HuggingHeightKey.self) { height = $0 }
    }
}

private struct HuggingHeightKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

/// 「本机屏幕」档位：本机主屏物理像素（取当前窗口所在的屏幕），横屏方向；超出合法范围为 nil
@MainActor
enum LocalScreen {
    static var virtualDisplaySize: VirtualDisplaySize? {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        guard let scene = scenes.first(where: { $0.activationState == .foregroundActive }) ?? scenes.first else { return nil }
        let px = scene.screen.nativeBounds.size
        return VirtualDisplayForm.localSize(pixelWidth: Double(px.width), pixelHeight: Double(px.height))
    }
}
