import SwiftUI
import GalaxyKit

/// 对话页（手机端）：消息流 + 底部输入框。新对话还没有消息时，问候语与输入框居中；发出第一条后输入框回到底部。
struct ChatView: View {
    @StateObject private var draft: ChatDraft

    init(controller: ConversationController?, refs: [ContextRef] = []) {
        _draft = StateObject(wrappedValue: ChatDraft(controller: controller, refs: refs))
    }

    var body: some View {
        if let c = draft.controller {
            LiveChat(draft: draft, controller: c)
        } else {
            ChatLayout(draft: draft, state: nil, controller: nil)
        }
    }
}

private struct LiveChat: View {
    @ObservedObject var draft: ChatDraft
    @ObservedObject var controller: ConversationController
    var body: some View {
        ChatLayout(draft: draft, state: controller.state, controller: controller)
    }
}

private struct BottomOffsetKey: PreferenceKey {
    static var defaultValue: CGFloat = .infinity
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = min(value, nextValue()) }
}

private struct ChatLayout: View {
    @ObservedObject var draft: ChatDraft
    let state: ConversationState?
    let controller: ConversationController?
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var ai: AgentHub
    @EnvironmentObject var account: GalaxyAccount
    @EnvironmentObject var settings: AppSettings
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var sheet: ComposerSheet?
    @State private var atBottom = true
    @State private var viewportHeight: CGFloat = 0

    private var availability: AiAvailability {
        AiAvailability.of(models: ai.models, error: ai.modelsError, loading: ai.loadingModels, userAiEnabled: account.user?.aiEnabled)
    }

    private var items: [ChatItem] { state?.items ?? [] }

    var body: some View {
        let ready = availability == .ready
        VStack(spacing: 0) {
            if items.isEmpty {
                // 新对话（或不可用时）：问候 / 状态居中，输入框在其下方
                ScrollView {
                    VStack(spacing: 20) {
                        if ready {
                            Text(loc("ai.greeting"))
                                .font(.title2.weight(.semibold))
                                .foregroundStyle(p.text)
                                .multilineTextAlignment(.center)
                            composerBlock(enabled: true)
                        } else {
                            AiAvailabilityView(availability: availability)
                        }
                    }
                    .frame(maxWidth: 760)
                    .padding(.horizontal, 14)
                    .frame(maxWidth: .infinity, minHeight: max(viewportHeight - 24, 0))
                    .contentShape(Rectangle())
                    .onTapGesture { draft.mentionDismissedText = draft.text }
                }
                .scrollDismissesKeyboard(.interactively)
            } else {
                messages
                if !ready {
                    AiAvailabilityView(availability: availability, compact: true)
                        .padding(.horizontal, 14)
                        .padding(.bottom, 4)
                }
                composerBlock(enabled: ready)
                    .frame(maxWidth: 760)
                    .padding(.horizontal, 10)
                    .padding(.bottom, 6)
                    .frame(maxWidth: .infinity)
            }
        }
        .background(GeometryReader { g in Color.clear.onAppear { viewportHeight = g.size.height }.onChange(of: g.size.height) { viewportHeight = $0 } })
        .background(p.background)
        .navigationTitle(navTitle)
        .navigationBarTitleDisplayMode(.inline)
        .onAppear { draft.applyDefaults(models: ai.models, settings: settings) }
        .onReceive(ai.$models) { draft.applyDefaults(models: $0, settings: settings) }
        .task { if ai.models == nil && !ai.loadingModels { await ai.loadModels() } }
        .sheet(item: $sheet) { s in sheetView(s) }
    }

    private var navTitle: String {
        if let t = state?.title, !t.isEmpty { return t }
        return loc("ai.untitled")
    }

    // MARK: 消息流

    private var messages: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    ForEach(items) { item in
                        if let controller {
                            ChatItemView(item: item, controller: controller).id(item.id)
                        }
                    }
                    Color.clear.frame(height: 1).id("bottom")
                        .background(GeometryReader { g in
                            Color.clear.preference(key: BottomOffsetKey.self, value: g.frame(in: .named("chat")).maxY)
                        })
                }
                .frame(maxWidth: 760)
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
                .frame(maxWidth: .infinity)
            }
            .coordinateSpace(name: "chat")
            .scrollDismissesKeyboard(.interactively)
            .background(GeometryReader { g in Color.clear.preference(key: ViewportKey.self, value: g.size.height) })
            .onPreferenceChange(ViewportKey.self) { scrollHeight = $0 }
            .onPreferenceChange(BottomOffsetKey.self) { y in atBottom = y <= scrollHeight + 60 }
            .onChange(of: contentSignature) { _ in
                if atBottom { proxy.scrollTo("bottom", anchor: .bottom) }
            }
            .onChange(of: items.count) { _ in
                // 自己发的消息总是滚到底
                if case .user? = items.last { withAnimation { proxy.scrollTo("bottom", anchor: .bottom) } }
            }
            .onAppear { proxy.scrollTo("bottom", anchor: .bottom) }
            .overlay(alignment: .bottomTrailing) {
                if !atBottom {
                    Button { withAnimation { proxy.scrollTo("bottom", anchor: .bottom) } } label: {
                        Image(systemName: "arrow.down")
                            .font(.system(size: 14, weight: .semibold))
                            .foregroundStyle(p.text)
                            .frame(width: 36, height: 36)
                            .background(Circle().fill(p.elevated))
                            .overlay(Circle().stroke(p.border, lineWidth: 0.5))
                            .shadow(color: .black.opacity(0.12), radius: 6, y: 2)
                    }
                    .padding(.trailing, 16)
                    .padding(.bottom, 10)
                    .accessibilityLabel(loc("ai.scrollToBottom"))
                    .transition(.opacity)
                }
            }
            .simultaneousGesture(TapGesture().onEnded { draft.mentionDismissedText = draft.text })
        }
    }

    @State private var scrollHeight: CGFloat = 0

    private var contentSignature: Int {
        guard let last = items.last else { return 0 }
        var n = items.count * 1_000_003
        switch last {
        case .assistant(_, let t, let th, _): n += t.count + th.count
        case .tool(let t): n += t.output.count + (t.result == nil ? 0 : 1) + (t.phase == .awaiting ? 7 : 0)
        default: break
        }
        return n
    }

    // MARK: 输入框

    @ViewBuilder private func composerBlock(enabled: Bool) -> some View {
        VStack(spacing: 6) {
            if let q = draft.mentionQuery, enabled {
                MentionPanel(query: q) { draft.pickMention($0) }
            }
            if let e = draft.error {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Image(systemName: "exclamationmark.circle.fill")
                    Text(e).frame(maxWidth: .infinity, alignment: .leading)
                    Button { draft.error = nil } label: { Image(systemName: "xmark").font(.caption2.weight(.bold)) }
                        .buttonStyle(.plain)
                        .accessibilityLabel(loc("common.close"))
                }
                .font(.footnote)
                .foregroundStyle(p.danger)
                .padding(.horizontal, 6)
            }
            ChatComposer(draft: draft, running: state?.running ?? false, enabled: enabled,
                         onSend: { draft.send(ai: ai, models: ai.models) },
                         onStop: { controller?.stop() },
                         sheet: $sheet)
        }
    }

    @ViewBuilder private func sheetView(_ s: ComposerSheet) -> some View {
        Group {
            switch s {
            case .plus:
                PlusSheet(choose: { next in
                    sheet = nil
                    // 当前弹层收起后再弹下一个
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.35) { sheet = next }
                }, addActivity: {
                    draft.addRefs([.activity(label: loc("ai.ref.activityLabel"))])
                    sheet = nil
                })
            case .devices: DeviceRefPicker { draft.addRefs($0) }
            case .files: FileRefPicker { draft.addRefs([$0]) }
            case .snippets: VaultRefPicker(kind: .snippets) { draft.addRefs($0) }
            case .forwards: VaultRefPicker(kind: .forwards) { draft.addRefs($0) }
            case .model: ModelSheet(draft: draft)
            case .mode: ModeSheet(draft: draft)
            }
        }
        .environmentObject(app)
        .environmentObject(ai)
        .environmentObject(account)
        .environmentObject(settings)
        .environment(\.palette, p)
        .environment(\.loc, loc)
    }
}

private struct ViewportKey: PreferenceKey {
    static var defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

/// AI 不可用时的提示（docs/ai-chat-ui.md 第 5 节）：加载中 / 加载失败 + 重试 / 账号没开通 / 没有可用模型 + 重试
struct AiAvailabilityView: View {
    let availability: AiAvailability
    var compact = false
    @EnvironmentObject var ai: AgentHub
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        switch availability {
        case .ready:
            EmptyView()
        case .loading:
            ProgressView().controlSize(compact ? .small : .regular).frame(maxWidth: .infinity).padding(compact ? 6 : 24)
        case .failed(let e):
            message(icon: "exclamationmark.triangle", text: e, tint: p.danger, retry: true)
        case .disabled:
            message(icon: "sparkles", text: loc("ai.disabledMessage"), tint: p.secondaryText, retry: false)
        case .noModels:
            message(icon: "cpu", text: loc("ai.noModelsMessage"), tint: p.secondaryText, retry: true)
        }
    }

    private func message(icon: String, text: String, tint: Color, retry: Bool) -> some View {
        Group {
            if compact {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Image(systemName: icon)
                    Text(text).frame(maxWidth: .infinity, alignment: .leading)
                    if retry { retryButton.buttonStyle(.borderless) }
                }
                .font(.footnote)
                .foregroundStyle(tint)
            } else {
                VStack(spacing: 14) {
                    Image(systemName: icon).font(.system(size: 30, weight: .light)).foregroundStyle(p.tertiaryText)
                    Text(text)
                        .font(.callout)
                        .foregroundStyle(tint == p.danger ? p.danger : p.text)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                    if retry { retryButton.buttonStyle(.bordered) }
                }
                .padding(24)
                .frame(maxWidth: 420)
            }
        }
    }

    private var retryButton: some View {
        Button {
            Task { await ai.loadModels() }
        } label: {
            if ai.loadingModels { ProgressView().controlSize(.small) } else { Text(loc("common.retry")) }
        }
        .disabled(ai.loadingModels)
    }
}
