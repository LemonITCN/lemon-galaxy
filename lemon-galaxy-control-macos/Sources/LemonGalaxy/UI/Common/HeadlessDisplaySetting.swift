import SwiftUI

/// 无头兜底屏设置（被控服务 host.json 的 headless_display；Windows / Linux 被控端的命令行 headless-display 改的是同一项）。
/// 内核接口的写法："" = 默认 1920x1080@60，"off" = 关闭，"宽x高@刷新率" = 自定义（HostService.headless_display）。
/// Mac 主控端（本机被控概览）与纯被控端（通用页）共用（纯被控端经符号链接）。
enum HeadlessDisplayMode: Equatable {
    case off
    case standard
    case custom(width: UInt32, height: UInt32, refresh: UInt32)

    static let defaultMode = (width: UInt32(1920), height: UInt32(1080), refresh: UInt32(60))

    /// 内核接口的写法 → 界面状态；写错的值内核当作关闭
    init(setting: String) {
        let t = setting.trimmingCharacters(in: .whitespaces).lowercased()
        if t.isEmpty || t == "default" {
            self = .standard
        } else if case let (w, h, hz)? = Self.parse(t) {
            self = .custom(width: w, height: h, refresh: hz)
        } else {
            self = .off
        }
    }

    var setting: String {
        switch self {
        case .off: return "off"
        case .standard: return ""
        case .custom(let w, let h, let hz): return "\(w)x\(h)@\(hz)"
        }
    }

    /// 与内核 galaxy_proto::peer::virtual_display_mode_ok 相同的范围
    static func valid(_ w: UInt32, _ h: UInt32, _ hz: UInt32) -> Bool {
        (640...7680).contains(w) && (480...4320).contains(h) && (24...240).contains(hz)
    }

    /// "宽x高[@刷新率]"，刷新率缺省 60
    static func parse(_ s: String) -> (UInt32, UInt32, UInt32)? {
        let t = s.trimmingCharacters(in: .whitespaces).lowercased()
        let parts = t.split(separator: "@", maxSplits: 1, omittingEmptySubsequences: false)
        let size = parts[0].split(whereSeparator: { $0 == "x" || $0 == "*" })
        guard size.count == 2, let w = UInt32(size[0].trimmingCharacters(in: .whitespaces)),
              let h = UInt32(size[1].trimmingCharacters(in: .whitespaces)) else { return nil }
        var hz: UInt32 = 60
        if parts.count == 2 {
            guard let r = UInt32(parts[1].trimmingCharacters(in: .whitespaces)) else { return nil }
            hz = r
        }
        return valid(w, h, hz) ? (w, h, hz) : nil
    }

    /// 规范写法（模拟内核用，与内核 HostService.set_headless_display 的校验一致）；写错为 nil
    static func normalize(_ s: String) -> String? {
        let t = s.trimmingCharacters(in: .whitespaces).lowercased()
        if t.isEmpty || t == "default" { return "" }
        if ["off", "no", "false", "0"].contains(t) { return "off" }
        return parse(t).map { "\($0.0)x\($0.1)@\($0.2)" }
    }
}

/// 无头兜底屏一行：关闭 / 默认 / 自定义（宽 × 高 @ 刷新率）。不带外框，由页面放进 Card。
/// apply 返回内核是否接受；接受后页面刷新 value。
struct HeadlessDisplaySetting: View {
    let value: String
    var apply: (String) -> Bool
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    private enum Choice: Hashable { case off, standard, custom }
    @State private var choice: Choice = .standard
    @State private var width = ""
    @State private var height = ""
    @State private var refresh = ""
    @State private var rejected = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 6) {
                Text(loc("headless.title")).font(.system(size: 13)).foregroundStyle(p.text)
                InfoButton(text: loc("headless.info"))
                Spacer()
                Picker("", selection: Binding(get: { choice }, set: { pick($0) })) {
                    Text(loc("headless.off")).tag(Choice.off)
                    Text(loc("headless.default")).tag(Choice.standard)
                    Text(loc("headless.custom")).tag(Choice.custom)
                }
                .pickerStyle(.segmented).labelsHidden().frame(width: 220)
            }
            if choice == .custom {
                HStack(spacing: 6) {
                    Spacer()
                    numberField("1920", $width, width: 64)
                    Text("×").foregroundStyle(p.secondaryText)
                    numberField("1080", $height, width: 64)
                    Text("@").foregroundStyle(p.secondaryText)
                    numberField("60", $refresh, width: 44)
                    Text("Hz").font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                    Button(loc("headless.apply")) { applyCustom() }
                        .buttonStyle(SecondaryButtonStyle())
                        .disabled(customMode == nil || customMode == HeadlessDisplayMode(setting: value))
                }
                if customMode == nil || rejected {
                    HStack {
                        Spacer()
                        Text(loc("headless.range")).font(.system(size: 11.5)).foregroundStyle(p.danger)
                    }
                }
            }
            if !VirtualDisplays.shared.available {
                HStack(spacing: 6) {
                    Image(systemName: "exclamationmark.triangle.fill").font(.system(size: 11)).foregroundStyle(p.warning)
                    Text(loc("headless.unsupported")).font(.system(size: 12)).foregroundStyle(p.secondaryText)
                }
            }
        }
        .onAppear { load(value) }
        .onChange(of: value) { load($0) }
    }

    private func numberField(_ placeholder: String, _ text: Binding<String>, width: CGFloat) -> some View {
        TextField(placeholder, text: Binding(get: { text.wrappedValue },
                                             set: { text.wrappedValue = String($0.filter(\.isNumber).prefix(4)); rejected = false }))
            .multilineTextAlignment(.center)
            .fieldStyle(invalid: customMode == nil && !text.wrappedValue.isEmpty)
            .frame(width: width)
            .onSubmit { applyCustom() }
    }

    /// 输入框里的自定义模式（不合法时为 nil）
    private var customMode: HeadlessDisplayMode? {
        guard let w = UInt32(width), let h = UInt32(height) else { return nil }
        let hz = refresh.isEmpty ? HeadlessDisplayMode.defaultMode.refresh : UInt32(refresh) ?? 0
        return HeadlessDisplayMode.valid(w, h, hz) ? .custom(width: w, height: h, refresh: hz) : nil
    }

    private func load(_ v: String) {
        let m = HeadlessDisplayMode(setting: v)
        rejected = false
        switch m {
        case .off: choice = .off
        case .standard: choice = .standard
        case .custom(let w, let h, let hz):
            choice = .custom
            width = "\(w)"; height = "\(h)"; refresh = "\(hz)"
        }
        if case .custom = m { return }
        // 切到自定义时从默认值起步
        let d = HeadlessDisplayMode.defaultMode
        if width.isEmpty { width = "\(d.width)" }
        if height.isEmpty { height = "\(d.height)" }
        if refresh.isEmpty { refresh = "\(d.refresh)" }
    }

    private func pick(_ c: Choice) {
        choice = c
        rejected = false
        switch c {
        case .off: _ = apply(HeadlessDisplayMode.off.setting)
        case .standard: _ = apply(HeadlessDisplayMode.standard.setting)
        case .custom: break    // 填好后点「应用」
        }
    }

    private func applyCustom() {
        guard let m = customMode else { return }
        rejected = !apply(m.setting)
    }
}
