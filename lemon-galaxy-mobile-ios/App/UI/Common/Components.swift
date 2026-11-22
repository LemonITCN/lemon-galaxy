import SwiftUI
import GalaxyKit

private struct StringsKey: EnvironmentKey {
    static let defaultValue: Strings = Localization.shared.strings
}

extension EnvironmentValues {
    /// 当前语言的文字：`loc("key")`
    var loc: Strings {
        get { self[StringsKey.self] }
        set { self[StringsKey.self] = newValue }
    }
}

/// 说明文字一律收进 i 图标：点一下弹出，不在界面上堆小字。
struct InfoButton: View {
    let text: String
    var size: CGFloat = 15
    @State private var shown = false
    @Environment(\.palette) private var p

    var body: some View {
        Button { shown = true } label: {
            Image(systemName: "info.circle")
                .font(.system(size: size, weight: .regular))
                .foregroundStyle(p.tertiaryText)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .popover(isPresented: $shown) {
            ScrollView {
                Text(text)
                    .font(.callout)
                    .foregroundStyle(p.text)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(16)
            }
            .frame(idealWidth: 320, maxWidth: 360, maxHeight: 420)
            .modifier(CompactPopover())
        }
    }
}

/// iPhone 上弹出框默认变成整页：iOS 16.4 起保持气泡
struct CompactPopover: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 16.4, *) {
            content.presentationCompactAdaptation(.popover)
        } else {
            content
        }
    }
}

struct StatusDot: View {
    let color: Color
    var size: CGFloat = 8
    var body: some View {
        Circle().fill(color).frame(width: size, height: size)
    }
}

enum Icons {
    static func platform(_ p: Platform) -> String {
        switch p {
        case .macos: return "laptopcomputer"
        case .windows: return "pc"
        case .linux: return "server.rack"
        case .android: return "candybarphone"
        case .ios: return "iphone"
        case .harmonyos: return "smartphone"
        }
    }

    static func tool(_ tool: String) -> String {
        switch tool {
        case "run_command": return "terminal"
        case let t where t.hasPrefix("terminal"): return "apple.terminal"
        case "read_file": return "doc.text"
        case "write_file": return "square.and.pencil"
        case "edit_file": return "pencil.line"
        case "list_dir": return "folder"
        case "search_files": return "magnifyingglass"
        case "upload_file", "download_file": return "arrow.up.arrow.down"
        case "screenshot": return "camera.viewfinder"
        case "ui_tree": return "list.bullet.indent"
        case "mouse": return "cursorarrow.click"
        case "keyboard": return "keyboard"
        case "wait": return "hourglass"
        case "use_target": return "link"
        case "list_devices": return "desktopcomputer"
        case "list_snippets": return "chevron.left.forwardslash.chevron.right"
        case "list_port_forwards": return "arrow.left.arrow.right"
        case "recent_activity": return "clock.arrow.circlepath"
        default: return "wrench.and.screwdriver"
        }
    }

    static func file(_ e: FileEntry) -> String {
        switch e.kind {
        case .dir: return "folder.fill"
        case .symlink: return "arrow.turn.up.right"
        case .other: return "doc"
        case .file:
            let ext = (e.name as NSString).pathExtension.lowercased()
            switch ext {
            case "png", "jpg", "jpeg", "gif", "heic", "webp", "bmp", "svg": return "photo"
            case "mp4", "mov", "mkv", "avi", "webm": return "film"
            case "mp3", "wav", "flac", "aac", "m4a", "ogg": return "waveform"
            case "zip", "gz", "tgz", "xz", "bz2", "7z", "rar", "tar", "zst": return "doc.zipper"
            case "sh", "bash", "zsh", "py", "rb", "js", "ts", "go", "rs", "swift", "kt", "java", "c", "h", "cpp", "json", "yaml", "yml", "toml", "xml", "html", "css": return "chevron.left.forwardslash.chevron.right"
            case "pdf": return "doc.richtext"
            case "log", "txt", "md", "conf", "ini", "cfg", "env": return "doc.plaintext"
            default: return "doc"
            }
        }
    }
}

/// 平台图标放在圆角方块里（主机列表、AI 目标）
struct PlatformBadge: View {
    let symbol: String
    var tint: Color
    var size: CGFloat = 34
    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.26, style: .continuous)
            .fill(tint.opacity(0.14))
            .frame(width: size, height: size)
            .overlay(Image(systemName: symbol).font(.system(size: size * 0.46, weight: .medium)).foregroundStyle(tint))
    }
}

struct TagChip: View {
    let text: String
    var selected = false
    @Environment(\.palette) private var p
    var body: some View {
        Text(text)
            .font(.caption.weight(.medium))
            .padding(.horizontal, 9)
            .padding(.vertical, 4)
            .background(Capsule().fill(selected ? p.accent : p.elevated))
            .overlay(Capsule().stroke(selected ? Color.clear : p.border, lineWidth: 0.5))
            .foregroundStyle(selected ? p.accentText : p.secondaryText)
    }
}

/// 连接路径的简称与颜色（局域网 / 直连为绿，中转为黄）
enum PathStyle {
    static func label(_ kind: PathKind, _ loc: Strings) -> String {
        switch kind {
        case .lan: return loc("path.lan")
        case .direct: return loc("path.direct")
        case .relayUdp: return loc("path.relay")
        case .relayTcp: return loc("path.relayTcp")
        case .connecting: return loc("path.connecting")
        case .closed: return loc("path.closed")
        }
    }

    static func color(_ kind: PathKind, _ p: Palette) -> Color {
        switch kind {
        case .lan, .direct: return p.success
        case .relayUdp, .relayTcp: return p.warning
        case .connecting, .closed: return p.offline
        }
    }
}

/// 连接路径：局域网 / 直连 / 中转 + 往返时延
struct PathBadge: View {
    let path: PathInfo
    var light = false
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        HStack(spacing: 5) {
            StatusDot(color: color, size: 6)
            Text(label + (path.kind == .connecting || path.kind == .closed ? "" : " · \(path.rttMs) ms"))
                .font(.caption2.monospacedDigit())
                .foregroundStyle(light ? Color.white.opacity(0.75) : p.secondaryText)
        }
    }

    var label: String { PathStyle.label(path.kind, loc) }

    var color: Color { PathStyle.color(path.kind, p) }
}

struct EmptyState: View {
    let icon: String
    let title: String
    var detail: String?
    @Environment(\.palette) private var p
    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: icon).font(.system(size: 36, weight: .light)).foregroundStyle(p.tertiaryText)
            Text(title).font(.headline).foregroundStyle(p.secondaryText)
            if let detail { Text(detail).font(.subheadline).foregroundStyle(p.tertiaryText).multilineTextAlignment(.center) }
        }
        .padding(32)
        .frame(maxWidth: .infinity)
    }
}

struct Toast: Identifiable, Equatable {
    let id = UUID()
    let text: String
    let isError: Bool
}

struct ToastView: View {
    let toast: Toast
    @Environment(\.palette) private var p
    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: toast.isError ? "exclamationmark.triangle.fill" : "checkmark.circle.fill")
                .foregroundStyle(toast.isError ? p.danger : p.success)
            Text(toast.text).font(.subheadline).foregroundStyle(p.text).lineLimit(3)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .background(.regularMaterial, in: Capsule())
        .overlay(Capsule().stroke(p.border.opacity(0.6), lineWidth: 0.5))
        .shadow(color: .black.opacity(0.15), radius: 12, y: 4)
        .padding(.horizontal, 24)
    }
}

/// 列表统一外观：自定义底色与行色
struct GalaxyList: ViewModifier {
    @Environment(\.palette) private var p
    func body(content: Content) -> some View {
        content
            .scrollContentBackground(.hidden)
            .background(p.background.ignoresSafeArea())
    }
}

struct GalaxyRow: ViewModifier {
    @Environment(\.palette) private var p
    func body(content: Content) -> some View { content.listRowBackground(p.card) }
}

extension View {
    func galaxyList() -> some View { modifier(GalaxyList()) }
    func galaxyRow() -> some View { modifier(GalaxyRow()) }
}

/// 主按钮
struct PrimaryButtonStyle: ButtonStyle {
    @Environment(\.palette) private var p
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.body.weight(.semibold))
            .frame(maxWidth: .infinity, minHeight: 48)
            .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(p.accent.opacity(enabled ? (configuration.isPressed ? 0.8 : 1) : 0.4)))
            .foregroundStyle(p.accentText)
    }
}

/// 风险级别标签
struct RiskBadge: View {
    let risk: RiskLevel
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    var body: some View {
        Text(loc("risk." + key))
            .font(.caption2.weight(.semibold))
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .background(Capsule().fill(color.opacity(0.15)))
            .foregroundStyle(color)
    }
    var key: String {
        switch risk {
        case .read: return "read"
        case .normal: return "normal"
        case .dangerous: return "dangerous"
        case .unknown: return "unknown"
        case .forbidden: return "forbidden"
        }
    }
    var color: Color {
        switch risk {
        case .read: return p.success
        case .normal: return p.info
        case .dangerous: return p.danger
        case .unknown: return p.warning
        case .forbidden: return p.danger
        }
    }
}

/// 密码输入框：可以切换明文
struct SecretField: View {
    let title: String
    @Binding var text: String
    var contentType: UITextContentType? = .password
    @State private var reveal = false
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    var body: some View {
        HStack {
            Group {
                if reveal {
                    TextField(title, text: $text)
                } else {
                    SecureField(title, text: $text)
                }
            }
            .textContentType(contentType)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            Button { reveal.toggle() } label: {
                Image(systemName: reveal ? "eye.slash" : "eye").foregroundStyle(p.tertiaryText)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(reveal ? loc("login.hidePassword") : loc("login.showPassword"))
        }
    }
}

extension View {
    /// 条件修饰
    @ViewBuilder func `if`<T: View>(_ cond: Bool, _ transform: (Self) -> T) -> some View {
        if cond { transform(self) } else { self }
    }
}
