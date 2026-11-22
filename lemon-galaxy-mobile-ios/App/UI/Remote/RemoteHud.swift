import SwiftUI
import GalaxyKit

/// 画面上方居中的短暂提示（缩放倍数、切换显示器、操作方式）。单独成一个对象：捏合时每帧更新提示文字，只刷新提示本身。
@MainActor
final class RemoteHud: ObservableObject {
    struct Notice: Equatable {
        let title: String
        let subtitle: String?
    }

    @Published private(set) var notice: Notice?
    private var hideTask: Task<Void, Never>?

    func show(_ title: String, _ subtitle: String? = nil) {
        let n = Notice(title: title, subtitle: subtitle)
        if n != notice { notice = n }
        hideTask?.cancel()
        hideTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 1_600_000_000)
            guard !Task.isCancelled else { return }
            self?.notice = nil
        }
    }
}

/// 提示层：不挡触控
struct RemoteNoticeLayer: View {
    @ObservedObject var hud: RemoteHud

    var body: some View {
        ZStack {
            if let n = hud.notice {
                RemoteNoticeView(title: n.title, subtitle: n.subtitle)
                    .transition(.opacity)
            }
        }
        .animation(.easeOut(duration: 0.2), value: hud.notice == nil)
        .allowsHitTesting(false)
    }
}

struct RemoteNoticeView: View {
    let title: String
    let subtitle: String?

    var body: some View {
        VStack(spacing: 2) {
            Text(title)
                .font(.subheadline.weight(.semibold).monospacedDigit())
                .foregroundStyle(.white)
            if let subtitle, !subtitle.isEmpty {
                Text(subtitle)
                    .font(.caption)
                    .foregroundStyle(DockColors.dim)
                    .lineLimit(1)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(DockColors.notice))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).stroke(DockColors.line, lineWidth: 1))
        .accessibilityElement(children: .combine)
    }
}

/// 菜单收起时左上角的录像标记（红点 + 计时，不挡触控）
struct RecordingBadge: View {
    let start: Date

    var body: some View {
        TimelineView(.periodic(from: start, by: 1)) { ctx in
            badge(RecordingClock.text(since: start, now: ctx.date))
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    private func badge(_ text: String) -> some View {
        HStack(spacing: 6) {
            Circle().fill(DockColors.record).frame(width: 8, height: 8)
            Text(text).font(.caption.monospacedDigit().weight(.semibold)).foregroundStyle(.white)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .background(Capsule().fill(DockColors.handle))
    }
}

enum RecordingClock {
    static func text(since start: Date, now: Date) -> String {
        Fmt.elapsed(Int(now.timeIntervalSince(start)))
    }
}

/// 远程桌面深色浮层的配色（与安卓端 DockColors 相同）
enum DockColors {
    static let bg = Color(hex: "#1B1E2B").opacity(0.95)
    static let handle = Color(hex: "#1B1E2B").opacity(0.9)
    static let notice = Color(hex: "#1B1E2B").opacity(0.9)
    static let line = Color.white.opacity(0.14)
    static let chip = Color.white.opacity(0.08)
    static let accent = Color(hex: "#C6E35A")
    static let onAccent = Color(hex: "#1B1E2B")
    static let danger = Color(hex: "#FF7A85")
    static let record = Color(hex: "#FF4D4F")
    static let dim = Color.white.opacity(0.62)
    static let disabled = Color.white.opacity(0.35)
}
