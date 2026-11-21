import SwiftUI
import AppKit

// 通用小组件。图标一律 SF Symbols 或这里自绘的矢量形状。

/// 平台图标：macOS 用系统苹果标志；Windows、Linux、安卓自绘矢量。
struct PlatformGlyph: View {
    let platform: Platform
    var size: CGFloat = 14

    var body: some View {
        Group {
            switch platform {
            case .macos, .ios:
                Image(systemName: platform == .ios ? "iphone" : "applelogo").font(.system(size: size * 0.95, weight: .medium))
            case .windows:
                WindowsLogo().frame(width: size * 0.9, height: size * 0.9)
            case .linux:
                LinuxLogo().frame(width: size * 0.9, height: size)
            case .android:
                AndroidLogo().frame(width: size, height: size * 0.8)
            case .harmonyos:
                Image(systemName: "circle.hexagongrid").font(.system(size: size * 0.9))
            }
        }
        .frame(width: size + 2, height: size + 2)
    }
}

struct WindowsLogo: View {
    var body: some View {
        GeometryReader { g in
            let s = min(g.size.width, g.size.height)
            let gap = s * 0.08
            let h = (s - gap) / 2
            ZStack(alignment: .topLeading) {
                ForEach(0..<4) { i in
                    Rectangle()
                        .frame(width: h, height: h)
                        .offset(x: CGFloat(i % 2) * (h + gap), y: CGFloat(i / 2) * (h + gap))
                }
            }
            .frame(width: s, height: s, alignment: .topLeading)
        }
    }
}

/// 简化的企鹅轮廓：身体、肚皮镂空、两只脚
struct LinuxLogo: View {
    var body: some View {
        GeometryReader { g in
            let w = g.size.width, h = g.size.height
            ZStack {
                // 头 + 身体（一整块轮廓）
                Path { p in
                    p.addRoundedRect(in: CGRect(x: w * 0.12, y: 0, width: w * 0.76, height: h * 0.9),
                                     cornerSize: CGSize(width: w * 0.38, height: h * 0.4))
                    // 肚皮：奇偶填充挖空
                    p.addEllipse(in: CGRect(x: w * 0.3, y: h * 0.4, width: w * 0.4, height: h * 0.42))
                    // 眼睛
                    p.addEllipse(in: CGRect(x: w * 0.32, y: h * 0.16, width: w * 0.12, height: h * 0.1))
                    p.addEllipse(in: CGRect(x: w * 0.56, y: h * 0.16, width: w * 0.12, height: h * 0.1))
                }
                .fill(style: FillStyle(eoFill: true))
                // 两只脚
                Path { p in
                    p.addEllipse(in: CGRect(x: 0, y: h * 0.82, width: w * 0.44, height: h * 0.18))
                    p.addEllipse(in: CGRect(x: w * 0.56, y: h * 0.82, width: w * 0.44, height: h * 0.18))
                }
            }
        }
    }
}

struct AndroidLogo: View {
    var body: some View {
        GeometryReader { g in
            let w = g.size.width, h = g.size.height
            Path { p in
                p.move(to: CGPoint(x: 0, y: h))
                p.addArc(center: CGPoint(x: w / 2, y: h), radius: w / 2, startAngle: .degrees(180), endAngle: .degrees(0), clockwise: false)
                p.closeSubpath()
                p.move(to: CGPoint(x: w * 0.25, y: h * 0.18)); p.addLine(to: CGPoint(x: w * 0.14, y: 0))
                p.move(to: CGPoint(x: w * 0.75, y: h * 0.18)); p.addLine(to: CGPoint(x: w * 0.86, y: 0))
            }
            .fill(style: FillStyle())
            .overlay(Path { p in
                p.move(to: CGPoint(x: w * 0.27, y: h * 0.22)); p.addLine(to: CGPoint(x: w * 0.14, y: 0))
                p.move(to: CGPoint(x: w * 0.73, y: h * 0.22)); p.addLine(to: CGPoint(x: w * 0.86, y: 0))
            }.stroke(lineWidth: max(1, w * 0.08)))
        }
    }
}

/// 设备状态点：在线（绿）、仅局域网可见（蓝）、离线（灰）
struct StatusDot: View {
    enum State { case online, lan, offline, busy }
    let state: State
    var size: CGFloat = 8
    @Environment(\.palette) var p

    var body: some View {
        Circle().fill(color).frame(width: size, height: size)
            .overlay(Circle().stroke(p.list, lineWidth: 1.5).frame(width: size + 2, height: size + 2))
    }

    var color: Color {
        switch state {
        case .online: return p.success
        case .lan: return p.info
        case .offline: return p.tertiaryText.opacity(0.7)
        case .busy: return p.warning
        }
    }

    static func of(_ d: DeviceInfo) -> State { d.online ? .online : (d.lanSeen ? .lan : .offline) }
}

/// 小 i 图标：说明文字只放在弹出层里，界面上不铺描述小字。
struct InfoButton: View {
    let text: String
    @State private var shown = false
    @Environment(\.palette) var p

    var body: some View {
        Button { shown.toggle() } label: {
            Image(systemName: "info.circle").font(.system(size: 12)).foregroundStyle(p.tertiaryText)
        }
        .buttonStyle(.plain)
        .popover(isPresented: $shown, arrowEdge: .bottom) {
            Text(text)
                .font(.system(size: 12))
                .foregroundStyle(p.text)
                .fixedSize(horizontal: false, vertical: true)
                .frame(width: 280, alignment: .leading)
                .padding(14)
        }
    }
}

struct TagChip: View {
    let text: String
    var selected = false
    @Environment(\.palette) var p
    var body: some View {
        Text(text)
            .font(.system(size: 10.5, weight: .medium))
            .foregroundStyle(selected ? p.accentText : p.secondaryText)
            .padding(.horizontal, 6).padding(.vertical, 1.5)
            .background(RoundedRectangle(cornerRadius: 4).fill(selected ? p.accent : p.hover.opacity(2.2)))
            .overlay(RoundedRectangle(cornerRadius: 4).stroke(selected ? Color.clear : p.border.opacity(0.8), lineWidth: 0.5))
    }
}

struct SectionLabel: View {
    let text: String
    @Environment(\.palette) var p
    var body: some View {
        Text(text.uppercased())
            .font(.system(size: 10.5, weight: .semibold))
            .tracking(0.4)
            .foregroundStyle(p.tertiaryText)
    }
}

/// 无边框图标按钮（工具栏用）
struct IconButton: View {
    let icon: String
    var help: String = ""
    var active = false
    var size: CGFloat = 13
    let action: () -> Void
    @Environment(\.palette) var p
    @State private var hover = false

    var body: some View {
        Button(action: action) {
            Image(systemName: icon)
                .font(.system(size: size, weight: .medium))
                .foregroundStyle(active ? p.accent : p.secondaryText)
                .frame(width: 26, height: 24)
                .background(RoundedRectangle(cornerRadius: 6).fill(active ? p.accent.opacity(0.14) : (hover ? p.hover.opacity(2) : .clear)))
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
        .help(help)
    }
}

/// 搜索框
struct SearchField: View {
    let placeholder: String
    @Binding var text: String
    @Environment(\.palette) var p

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass").font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
            TextField(placeholder, text: $text).textFieldStyle(.plain).font(.system(size: 12.5))
            if !text.isEmpty {
                Button { text = "" } label: { Image(systemName: "xmark.circle.fill").foregroundStyle(p.tertiaryText) }.buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 8).frame(height: 26)
        .background(RoundedRectangle(cornerRadius: 7).fill(p.elevated))
        .overlay(RoundedRectangle(cornerRadius: 7).stroke(p.border, lineWidth: 0.5))
    }
}

/// 页面滚动容器：内容按固定宽度排版（不超过 maxWidth，窄窗口时贴满），居中显示
struct PageScroll<Content: View>: View {
    var maxWidth: CGFloat = 860
    @ViewBuilder let content: Content
    var body: some View {
        GeometryReader { g in
            ScrollView {
                content
                    .padding(24)
                    .frame(width: min(g.size.width, maxWidth), alignment: .leading)
                    .frame(width: g.size.width)
            }
        }
    }
}

/// 卡片容器
struct Card<Content: View>: View {
    var padding: CGFloat = 16
    @ViewBuilder let content: Content
    @Environment(\.palette) var p
    var body: some View {
        content
            .padding(padding)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 10).fill(p.card))
            .overlay(RoundedRectangle(cornerRadius: 10).stroke(p.border, lineWidth: 0.5))
    }
}

/// 设置 / 编辑表单的一行：左标签右控件
struct FormRow<Content: View>: View {
    let label: String
    var info: String? = nil
    var labelWidth: CGFloat = 120
    @ViewBuilder let content: Content
    @Environment(\.palette) var p

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            HStack(spacing: 4) {
                Text(label).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                if let info { InfoButton(text: info) }
            }
            .frame(width: labelWidth, alignment: .trailing)
            content
            Spacer(minLength: 0)
        }
    }
}

/// 统一输入框外观
struct FieldStyle: ViewModifier {
    @Environment(\.palette) var p
    var invalid = false
    func body(content: Content) -> some View {
        content
            .textFieldStyle(.plain)
            .font(.system(size: 13))
            .padding(.horizontal, 9).frame(height: 28)
            .background(RoundedRectangle(cornerRadius: 7).fill(p.elevated))
            .overlay(RoundedRectangle(cornerRadius: 7).stroke(invalid ? p.danger : p.border, lineWidth: invalid ? 1 : 0.7))
    }
}

extension View {
    func fieldStyle(invalid: Bool = false) -> some View { modifier(FieldStyle(invalid: invalid)) }
}

/// 主按钮 / 次按钮
struct PrimaryButtonStyle: ButtonStyle {
    @Environment(\.palette) var p
    @Environment(\.isEnabled) var enabled
    var destructive = false
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 12.5, weight: .semibold))
            .foregroundStyle(destructive ? Color.white : p.accentText)
            .padding(.horizontal, 14).frame(height: 28)
            .background(RoundedRectangle(cornerRadius: 7).fill(destructive ? p.danger : p.accent)
                .opacity(enabled ? (configuration.isPressed ? 0.82 : 1) : 0.45))
    }
}

struct SecondaryButtonStyle: ButtonStyle {
    @Environment(\.palette) var p
    @Environment(\.isEnabled) var enabled
    var destructive = false
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 12.5, weight: .medium))
            .foregroundStyle(destructive ? p.danger : p.text)
            .padding(.horizontal, 12).frame(height: 28)
            .background(RoundedRectangle(cornerRadius: 7).fill(configuration.isPressed ? p.hover.opacity(3) : p.elevated))
            .overlay(RoundedRectangle(cornerRadius: 7).stroke(p.border, lineWidth: 0.7))
            .opacity(enabled ? 1 : 0.5)
    }
}

/// 空状态：图标 + 一行标题，不铺说明文字
struct EmptyState: View {
    let icon: String
    let title: String
    var action: (String, () -> Void)? = nil
    @Environment(\.palette) var p
    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: icon).font(.system(size: 30, weight: .light)).foregroundStyle(p.tertiaryText)
            Text(title).font(.system(size: 13)).foregroundStyle(p.secondaryText)
            if let action {
                Button(action.0, action: action.1).buttonStyle(PrimaryButtonStyle())
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// 开关样式：用界面配色的强调色画（系统开关只认系统强调色，换配色后不一致）
struct PaletteSwitchStyle: ToggleStyle {
    var small = false
    func makeBody(configuration: Configuration) -> some View { PaletteSwitch(configuration: configuration, small: small) }
}

private struct PaletteSwitch: View {
    let configuration: ToggleStyleConfiguration
    let small: Bool
    @Environment(\.palette) var p
    @Environment(\.isEnabled) var enabled
    var body: some View {
        let w: CGFloat = small ? 30 : 36, h: CGFloat = small ? 18 : 21
        HStack(spacing: 8) {
            configuration.label
            ZStack(alignment: configuration.isOn ? .trailing : .leading) {
                Capsule().fill(configuration.isOn ? p.accent : (p.isDark ? Color.white.opacity(0.16) : Color.black.opacity(0.12)))
                Circle().fill(.white).shadow(color: .black.opacity(0.2), radius: 1, y: 0.5).padding(2)
            }
            .frame(width: w, height: h)
            .opacity(enabled ? 1 : 0.45)
            .onTapGesture { withAnimation(.easeOut(duration: 0.12)) { configuration.isOn.toggle() } }
        }
    }
}

/// 开关（统一用强调色）
struct ToggleRow: View {
    let label: String
    @Binding var isOn: Bool
    var info: String? = nil
    @Environment(\.palette) var p
    var body: some View {
        HStack(spacing: 6) {
            Text(label).font(.system(size: 13)).foregroundStyle(p.text)
            if let info { InfoButton(text: info) }
            Spacer()
            Toggle("", isOn: $isOn).toggleStyle(PaletteSwitchStyle(small: true)).labelsHidden()
        }
    }
}

enum Fmt {
    static func bytes(_ n: UInt64) -> String {
        let f = ByteCountFormatter()
        f.countStyle = .binary
        f.allowedUnits = [.useBytes, .useKB, .useMB, .useGB, .useTB]
        return f.string(fromByteCount: Int64(min(n, UInt64(Int64.max))))
    }

    static func date(_ ms: Int64, zh: Bool) -> String {
        let d = Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
        let f = DateFormatter()
        f.locale = Locale(identifier: zh ? "zh_CN" : "en_US")
        f.dateFormat = "yyyy-MM-dd HH:mm"
        return f.string(from: d)
    }

    /// 列表里的短日期：今年只显示 月-日 时:分
    static func shortDate(_ ms: Int64, zh: Bool) -> String {
        let d = Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
        let f = DateFormatter()
        f.locale = Locale(identifier: zh ? "zh_CN" : "en_US")
        let sameYear = Calendar.current.component(.year, from: d) == Calendar.current.component(.year, from: Date(timeIntervalSince1970: TimeInterval(Clock.nowMs) / 1000))
        f.dateFormat = sameYear ? "MM-dd HH:mm" : "yyyy-MM-dd"
        return f.string(from: d)
    }

    static func time(_ ms: Int64) -> String {
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss"
        return f.string(from: Date(timeIntervalSince1970: TimeInterval(ms) / 1000))
    }

    static func relative(_ ms: Int64, now: Int64, loc: Strings) -> String {
        let diff = max(0, (now - ms) / 1000)
        if diff < 60 { return loc("time.justNow") }
        if diff < 3600 { return loc("time.minutesAgo", diff / 60) }
        if diff < 86_400 { return loc("time.hoursAgo", diff / 3600) }
        return loc("time.daysAgo", diff / 86_400)
    }

    static func mode(_ m: UInt32) -> String {
        let chars: [Character] = ["r", "w", "x"]
        var s = ""
        for shift in stride(from: 6, through: 0, by: -3) {
            for (i, c) in chars.enumerated() {
                s.append((m >> UInt32(shift + 2 - i)) & 1 == 1 ? c : "-")
            }
        }
        return s
    }
}
