import SwiftUI
import AppKit

// AI 聊天界面（docs/ai-chat-ui.md）：左侧对话列表，右侧消息流 + 输入框。

// MARK: - 左侧：对话列表

struct AIConversationList: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var ai: AIHubModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(spacing: 0) {
            ColumnHeader(title: loc("nav.ai")) {
                IconButton(icon: "square.and.pencil", help: loc("ai.new") + "  ⌘N") { ai.newChat(); app.selectHome(.ai) }
            }
            SearchField(placeholder: loc("ai.search"), text: $ai.search)
                .padding(.horizontal, 10).padding(.bottom, 8)
            Rectangle().fill(p.separator).frame(height: 1)
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 1) {
                    let groups = Self.grouped(ai.rows)
                    ForEach(groups, id: \.key) { g in
                        SectionLabel(text: loc(g.key)).padding(.horizontal, 10).padding(.top, 10).padding(.bottom, 3)
                        ForEach(g.rows) { r in AIChatRowView(row: r) }
                    }
                    if groups.isEmpty {
                        Text(loc(ai.search.isEmpty ? "ai.noConversations" : "ai.noMatch"))
                            .font(.system(size: 12)).foregroundStyle(p.tertiaryText)
                            .frame(maxWidth: .infinity).padding(.top, 24)
                    }
                }
                .padding(6)
            }
        }
        .onAppear { ai.reloadSummaries() }
    }

    struct ChatGroup { let key: String; let rows: [AIChatRow] }

    /// 今天 / 昨天 / 最近 7 天 / 更早
    static func grouped(_ rows: [AIChatRow]) -> [ChatGroup] {
        let cal = Calendar.current
        let now = Date(timeIntervalSince1970: TimeInterval(Clock.nowMs) / 1000)
        let today = cal.startOfDay(for: now)
        let yesterday = cal.date(byAdding: .day, value: -1, to: today)!
        let week = cal.date(byAdding: .day, value: -6, to: today)!
        var buckets: [String: [AIChatRow]] = [:]
        for r in rows {
            let d = Date(timeIntervalSince1970: TimeInterval(r.updatedAt) / 1000)
            let k = d >= today ? "ai.group.today" : d >= yesterday ? "ai.group.yesterday" : d >= week ? "ai.group.week" : "ai.group.earlier"
            buckets[k, default: []].append(r)
        }
        return ["ai.group.today", "ai.group.yesterday", "ai.group.week", "ai.group.earlier"].compactMap { k in
            buckets[k].map { ChatGroup(key: k, rows: $0) }
        }
    }
}

struct AIChatRowView: View {
    let row: AIChatRow
    var body: some View {
        if let live = row.live { LiveRow(conv: live, row: row) } else { AIChatRowContent(row: row, running: false, waiting: false) }
    }

    /// 打开过的对话要跟着刷新进行中标记
    struct LiveRow: View {
        @ObservedObject var conv: AIConversationModel
        let row: AIChatRow
        var body: some View { AIChatRowContent(row: row, running: conv.running, waiting: conv.hasPendingApproval || conv.pendingSecret != nil) }
    }
}

struct AIChatRowContent: View {
    let row: AIChatRow
    let running: Bool
    let waiting: Bool
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var ai: AIHubModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var hover = false

    var body: some View {
        let selected = ai.current?.conversationId == row.id && app.activeTabId == nil && app.section == .ai
        Button {
            if let s = row.summary, row.live == nil { ai.open(s) } else if let l = row.live { ai.select(l) }
        } label: {
            HStack(spacing: 8) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(row.title.isEmpty ? loc("ai.newConversation") : row.title)
                        .font(.system(size: 12.5, weight: selected ? .medium : .regular)).lineLimit(1)
                    Text(time).font(.system(size: 11).monospacedDigit()).foregroundStyle(p.tertiaryText)
                }
                Spacer(minLength: 0)
                if waiting {
                    Circle().fill(p.warning).frame(width: 7, height: 7).help(loc("ai.waitingApproval"))
                } else if running {
                    ProgressView().controlSize(.mini).help(loc("ai.running"))
                }
            }
            .padding(.horizontal, 10).padding(.vertical, 6)
            .background(RoundedRectangle(cornerRadius: 7).fill(selected ? p.selection : (hover ? p.hover : .clear)))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
        .contextMenu {
            Button(loc("common.delete")) {
                app.confirm(loc("ai.delete.title"), loc("ai.delete.body", row.title), button: loc("common.delete")) { ai.delete(row) }
            }
        }
    }

    private var time: String {
        let d = Date(timeIntervalSince1970: TimeInterval(row.updatedAt) / 1000)
        let now = Date(timeIntervalSince1970: TimeInterval(Clock.nowMs) / 1000)
        if Calendar.current.isDate(d, inSameDayAs: now) {
            let f = DateFormatter()
            f.dateFormat = "HH:mm"
            return f.string(from: d)
        }
        return Fmt.shortDate(row.updatedAt, zh: loc.isChinese)
    }
}

// MARK: - 右侧：AI 页面

struct AIHomePage: View {
    @EnvironmentObject var ai: AIHubModel
    @EnvironmentObject var app: AppModel

    var body: some View {
        ZStack {
            if let c = ai.current { AIChatView(conv: c).id(c.id) }
            // ⌘N 新对话
            Button("") { ai.newChat(); app.selectHome(.ai) }
                .keyboardShortcut("n", modifiers: .command)
                .opacity(0).frame(width: 0, height: 0).allowsHitTesting(false)
        }
        .onAppear { ai.ensureCurrent(); ai.loadModels() }
    }
}

enum RefPickerKind: String, Identifiable {
    case targets, files, snippets, forwards
    var id: String { rawValue }
}

struct AIChatView: View {
    @ObservedObject var conv: AIConversationModel
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var ai: AIHubModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var picker: RefPickerKind?
    @State private var pinned = true
    @State private var viewHeight: CGFloat = 0
    @State private var contentHeight: CGFloat = 0

    static let contentWidth: CGFloat = 760

    var body: some View {
        Group {
            if conv.isDraft { newChat } else { chat }
        }
        .overlay {
            if let kind = picker {
                ZStack {
                    Color.black.opacity(p.isDark ? 0.4 : 0.2).onTapGesture { picker = nil }
                    RefPickerPanel(kind: kind, conv: conv) { picker = nil }
                }
            }
            if let img = conv.zoomImage, let ns = NSImage(data: img.data) {
                ZStack {
                    Color.black.opacity(0.82).onTapGesture { conv.zoomImage = nil }
                    Image(nsImage: ns).resizable().aspectRatio(contentMode: .fit).padding(30).onTapGesture { conv.zoomImage = nil }
                }
            }
        }
    }

    // 新对话：问候语居中，输入框在下面
    private var newChat: some View {
        VStack(spacing: 22) {
            Spacer(minLength: 20)
            switch ai.availability {
            case .ready:
                Text(loc("ai.greeting")).font(.system(size: 24, weight: .semibold)).foregroundStyle(p.text)
                AIComposer(conv: conv) { picker = $0 }
                    .frame(maxWidth: Self.contentWidth)
            default:
                AIUnavailableView()
            }
            Spacer(minLength: 20)
            Spacer(minLength: 20)
        }
        .padding(.horizontal, 24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .contentShape(Rectangle())
        .onTapGesture { conv.mention = nil }
    }

    private var chat: some View {
        VStack(spacing: 0) {
            header
            Rectangle().fill(p.separator).frame(height: 1)
            messages
            VStack(spacing: 8) {
                if ai.availability != .ready { AIUnavailableBar() }
                AIComposer(conv: conv) { picker = $0 }
            }
            .padding(.horizontal, 24).padding(.top, 6).padding(.bottom, 16)
            .frame(maxWidth: Self.contentWidth + 48)
            .frame(maxWidth: .infinity)
        }
    }

    private var header: some View {
        HStack(spacing: 10) {
            Text(conv.title.isEmpty ? loc("ai.newConversation") : conv.title)
                .font(.system(size: 13, weight: .semibold)).lineLimit(1)
            if let eff = conv.effectiveMode, eff != conv.mode {
                HStack(spacing: 4) {
                    Image(systemName: eff.icon).font(.system(size: 10))
                    Text(loc("ai.mode.effective", loc(eff.labelKey))).font(.system(size: 11))
                }
                .foregroundStyle(p.warning)
                .help(loc("ai.mode.effective.help"))
            }
            Spacer()
            if conv.promptTokens + conv.completionTokens > 0 {
                Text(loc("ai.tokens", Self.k(conv.promptTokens), Self.k(conv.completionTokens)))
                    .font(.system(size: 11).monospacedDigit()).foregroundStyle(p.tertiaryText)
                    .help(loc("ai.tokens.help"))
            }
        }
        .padding(.horizontal, 18).frame(height: 40)
    }

    static func k(_ n: UInt64) -> String { n >= 1000 ? String(format: "%.1fk", Double(n) / 1000) : "\(n)" }

    private var messages: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 16) {
                    ForEach(conv.items) { item in
                        ChatItemView(item: item, conv: conv).id(item.id)
                    }
                    if showWorking {
                        HStack(spacing: 8) {
                            ProgressView().controlSize(.small)
                            if conv.connecting { Text(loc("ai.connecting")).font(.system(size: 12)).foregroundStyle(p.tertiaryText) }
                        }
                        .id("working")
                    }
                    Color.clear.frame(height: 1).id("bottom")
                }
                .padding(.horizontal, 24).padding(.vertical, 20)
                .frame(maxWidth: Self.contentWidth + 48)
                .frame(maxWidth: .infinity)
                .background(GeometryReader { g in
                    Color.clear.preference(key: ScrollMetricsKey.self, value: ScrollMetrics(bottom: g.frame(in: .named("aiScroll")).maxY, height: g.size.height))
                })
            }
            .coordinateSpace(name: "aiScroll")
            .background(GeometryReader { g in Color.clear.onAppear { viewHeight = g.size.height }.onChange(of: g.size.height) { viewHeight = $0 } })
            .onPreferenceChange(ScrollMetricsKey.self) { m in
                // 内容变高（新消息、流式输出）不改「贴底」状态；只有用户滚动才改
                if abs(m.height - contentHeight) > 0.5 {
                    contentHeight = m.height
                    if pinned { proxy.scrollTo("bottom", anchor: .bottom) }
                    return
                }
                let atBottom = m.bottom <= viewHeight + 40
                if atBottom != pinned { pinned = atBottom }
            }
            .onChange(of: conv.contentVersion) { _ in
                if pinned { proxy.scrollTo("bottom", anchor: .bottom) }
            }
            .onAppear { DispatchQueue.main.async { proxy.scrollTo("bottom", anchor: .bottom) } }
            .contentShape(Rectangle())
            .onTapGesture { conv.mention = nil }
            .overlay(alignment: .bottomTrailing) {
                if !pinned {
                    Button {
                        pinned = true
                        withAnimation(.easeOut(duration: 0.2)) { proxy.scrollTo("bottom", anchor: .bottom) }
                    } label: {
                        Image(systemName: "arrow.down").font(.system(size: 12, weight: .semibold)).foregroundStyle(p.text)
                            .frame(width: 30, height: 30)
                            .background(Circle().fill(p.elevated).shadow(color: .black.opacity(0.18), radius: 6, y: 2))
                            .overlay(Circle().stroke(p.border, lineWidth: 0.6))
                    }
                    .buttonStyle(.plain)
                    .help(loc("ai.scrollBottom"))
                    .padding(.trailing, 20).padding(.bottom, 12)
                }
            }
        }
    }

    /// 任务进行中且最后一条不是正在流式输出的文字、也不是等用户回应的卡片时，底部转一个小圈
    private var showWorking: Bool {
        if conv.connecting { return true }
        guard conv.running, let last = conv.items.last else { return false }
        switch last {
        case .assistant(let id): return conv.doneMessages.contains(id)
        case .thinking: return false
        case .tool(let t): return t.decision != .pending && t.phase == .finished
        case .ask(let a): return a.answer != nil
        case .secret(let s): return s.done || s.cancelled
        default: return true
        }
    }
}

struct ScrollMetrics: Equatable { var bottom: CGFloat; var height: CGFloat }
struct ScrollMetricsKey: PreferenceKey {
    static var defaultValue = ScrollMetrics(bottom: 0, height: 0)
    static func reduce(value: inout ScrollMetrics, nextValue: () -> ScrollMetrics) { value = nextValue() }
}

// MARK: - 不可用时的提示（第 5 节）

struct AIUnavailableView: View {
    @EnvironmentObject var ai: AIHubModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(spacing: 14) {
            switch ai.availability {
            case .loading:
                ProgressView().controlSize(.small)
            case .failed(let e):
                Image(systemName: "exclamationmark.triangle").font(.system(size: 26, weight: .light)).foregroundStyle(p.warning)
                Text(loc("ai.state.failed")).font(.system(size: 14, weight: .medium))
                Text(e).font(.system(size: 12)).foregroundStyle(p.secondaryText).multilineTextAlignment(.center).textSelection(.enabled)
                Button(loc("common.retry")) { ai.retry() }.buttonStyle(PrimaryButtonStyle())
            case .disabled:
                Image(systemName: "lock").font(.system(size: 26, weight: .light)).foregroundStyle(p.tertiaryText)
                Text(loc("ai.state.disabled")).font(.system(size: 13.5)).foregroundStyle(p.secondaryText).multilineTextAlignment(.center)
            case .noModels:
                Image(systemName: "cpu").font(.system(size: 26, weight: .light)).foregroundStyle(p.tertiaryText)
                Text(loc("ai.state.noModels")).font(.system(size: 13.5)).foregroundStyle(p.secondaryText).multilineTextAlignment(.center)
                Button(loc("common.retry")) { ai.retry() }.buttonStyle(SecondaryButtonStyle())
            case .ready:
                EmptyView()
            }
        }
        .frame(maxWidth: 440)
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// 已有对话里模型列表不可用：输入框上方一行提示
struct AIUnavailableBar: View {
    @EnvironmentObject var ai: AIHubModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        HStack(spacing: 8) {
            switch ai.availability {
            case .loading:
                ProgressView().controlSize(.mini)
            case .failed(let e):
                Image(systemName: "exclamationmark.triangle").foregroundStyle(p.warning)
                Text(loc("ai.state.failed") + (loc.isChinese ? "：" : ": ") + e).lineLimit(2)
                Spacer(minLength: 4)
                Button(loc("common.retry")) { ai.retry() }.buttonStyle(.plain).foregroundStyle(p.accent).font(.system(size: 12, weight: .semibold))
            case .disabled:
                Image(systemName: "lock").foregroundStyle(p.tertiaryText)
                Text(loc("ai.state.disabled")).lineLimit(2)
            case .noModels:
                Image(systemName: "cpu").foregroundStyle(p.tertiaryText)
                Text(loc("ai.state.noModels")).lineLimit(2)
                Spacer(minLength: 4)
                Button(loc("common.retry")) { ai.retry() }.buttonStyle(.plain).foregroundStyle(p.accent).font(.system(size: 12, weight: .semibold))
            case .ready:
                EmptyView()
            }
            Spacer(minLength: 0)
        }
        .font(.system(size: 12))
        .foregroundStyle(p.secondaryText)
        .padding(.horizontal, 12).padding(.vertical, 8)
        .background(RoundedRectangle(cornerRadius: 9).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 9).stroke(p.border, lineWidth: 0.6))
    }
}

// MARK: - 输入框（第 2 节）

struct AIComposer: View {
    @ObservedObject var conv: AIConversationModel
    let openPicker: (RefPickerKind) -> Void
    @EnvironmentObject var ai: AIHubModel
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var textHeight: CGFloat = 20
    @State private var showPlus = false
    @State private var showModels = false
    @State private var showModes = false

    private var enabled: Bool { ai.availability == .ready && !conv.connecting }

    var body: some View {
        let candidates = conv.mention.map { ai.candidates($0.query) } ?? []
        VStack(alignment: .leading, spacing: 6) {
            if let e = conv.composerError {
                HStack(alignment: .top, spacing: 6) {
                    Image(systemName: "exclamationmark.circle.fill").foregroundStyle(p.danger)
                    Text(e).foregroundStyle(p.danger).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                    Button { conv.composerError = nil } label: { Image(systemName: "xmark").font(.system(size: 9, weight: .bold)).foregroundStyle(p.tertiaryText) }
                        .buttonStyle(.plain)
                }
                .font(.system(size: 12))
                .padding(.horizontal, 4)
            }
            VStack(alignment: .leading, spacing: 8) {
                if !conv.refs.isEmpty {
                    FlowLayout(spacing: 6) {
                        ForEach(conv.refs, id: \.key) { r in RefChip(ref: r) { conv.removeRef(r) } }
                    }
                }
                ZStack(alignment: .topLeading) {
                    ComposerTextView(
                        text: $conv.text, height: $textHeight, enabled: enabled,
                        focusToken: conv.focusToken, caretRequest: conv.caretRequest,
                        onCaretApplied: { conv.caretRequest = nil },
                        onSubmit: { conv.send() },
                        onMention: { m in
                            if var m {
                                if let old = conv.mention, old.query == m.query { m.selected = old.selected }
                                if conv.mention != m { conv.mention = m }
                            } else if conv.mention != nil { conv.mention = nil }
                        },
                        onMentionKey: { key in mentionKey(key, candidates) }
                    )
                    .frame(height: textHeight)
                    if conv.text.isEmpty {
                        Text(loc("ai.composer.placeholder")).font(.system(size: 13)).foregroundStyle(p.tertiaryText)
                            .padding(.leading, 5).allowsHitTesting(false)
                    }
                }
                .overlay(alignment: .topLeading) {
                    if let m = conv.mention {
                        // 候选浮在 @ 所在行的上方
                        MentionPopup(candidates: candidates, selected: m.selected) { conv.pickMention($0) }
                            .fixedSize()
                            .offset(x: min(m.caret.x, 400), y: m.caret.y - MentionPopup.height(candidates.count) - 6)
                    }
                }
                toolbar
            }
            .padding(.horizontal, 12).padding(.top, 11).padding(.bottom, 8)
            .background(RoundedRectangle(cornerRadius: 16).fill(p.elevated).shadow(color: .black.opacity(p.isDark ? 0.25 : 0.06), radius: 8, y: 2))
            .overlay(RoundedRectangle(cornerRadius: 16).stroke(p.border, lineWidth: 0.7))
            .opacity(enabled ? 1 : 0.6)
        }
    }

    private func mentionKey(_ key: ComposerKey, _ cands: [RefCandidate]) -> Bool {
        guard var m = conv.mention else { return false }
        switch key {
        case .up:
            m.selected = max(0, m.selected - 1)
            conv.mention = m
        case .down:
            m.selected = min(max(cands.count - 1, 0), m.selected + 1)
            conv.mention = m
        case .choose:
            guard !cands.isEmpty else { conv.mention = nil; return true }
            conv.pickMention(cands[min(m.selected, cands.count - 1)].ref)
        case .cancel:
            conv.mention = nil
        }
        return true
    }

    private var toolbar: some View {
        HStack(spacing: 4) {
            ToolbarButton(help: loc("ai.ref.add")) {
                Image(systemName: "plus").font(.system(size: 13, weight: .medium))
            } action: { showPlus = true }
            .popover(isPresented: $showPlus, arrowEdge: .bottom) { plusMenu }
            .disabled(!enabled)

            ToolbarButton(help: loc("ai.model")) {
                HStack(spacing: 4) {
                    Text(conv.modelName.isEmpty ? loc("ai.model.none") : conv.modelName).font(.system(size: 12, weight: .medium)).lineLimit(1)
                    if conv.model?.vision == true { Image(systemName: "eye").font(.system(size: 9.5)) }
                    Image(systemName: "chevron.down").font(.system(size: 8, weight: .bold))
                }
            } action: { showModels = true }
            .popover(isPresented: $showModels, arrowEdge: .bottom) { ModelMenu(conv: conv) { showModels = false } }
            .disabled(!enabled)

            ToolbarButton(help: loc("ai.mode")) {
                HStack(spacing: 4) {
                    Image(systemName: conv.mode.icon).font(.system(size: 11))
                    Text(modeTitle).font(.system(size: 12, weight: .medium)).lineLimit(1)
                    Image(systemName: "chevron.down").font(.system(size: 8, weight: .bold))
                }
                .foregroundStyle(conv.mode == .fullTrust ? p.danger : p.secondaryText)
            } action: { showModes = true }
            .popover(isPresented: $showModes, arrowEdge: .bottom) { ModeMenu(conv: conv) { showModes = false } }
            .disabled(!enabled)

            Spacer(minLength: 6)
            if conv.running && conv.conversation != nil {
                Button { conv.stop() } label: {
                    Image(systemName: "square.fill").font(.system(size: 10)).foregroundStyle(p.content)
                        .frame(width: 28, height: 28)
                        .background(Circle().fill(p.text))
                }
                .buttonStyle(.plain)
                .help(loc("ai.stop"))
                .keyboardShortcut(".", modifiers: .command)
            } else {
                Button { conv.send() } label: {
                    Image(systemName: "arrow.up").font(.system(size: 13, weight: .bold))
                        .foregroundStyle(conv.canSend ? p.accentText : p.tertiaryText)
                        .frame(width: 28, height: 28)
                        .background(Circle().fill(conv.canSend ? p.accent : p.hover.opacity(2.5)))
                }
                .buttonStyle(.plain)
                .disabled(!conv.canSend)
                .help(loc("ai.send"))
            }
        }
    }

    private var modeTitle: String {
        var s = loc(conv.mode.labelKey)
        if conv.mode == .fullTrust {
            s += " · " + (conv.trustMinutes == 0 ? loc("ai.trust.conversation") : conv.trustMinutes >= 60 ? loc("ai.trust.hours", conv.trustMinutes / 60) : loc("ai.trust.minutes", conv.trustMinutes))
        }
        return s
    }

    private var plusMenu: some View {
        VStack(alignment: .leading, spacing: 1) {
            MenuRow(icon: "desktopcomputer", title: loc("ai.ref.targets")) { showPlus = false; openPicker(.targets) }
            MenuRow(icon: "folder", title: loc("ai.ref.files")) { showPlus = false; openPicker(.files) }
            MenuRow(icon: ContextRefKind.snippet.icon, title: loc("ai.ref.snippets")) { showPlus = false; openPicker(.snippets) }
            MenuRow(icon: ContextRefKind.portForward.icon, title: loc("ai.ref.forwards")) { showPlus = false; openPicker(.forwards) }
            MenuRow(icon: ContextRefKind.activity.icon, title: loc("ai.ref.activity")) {
                showPlus = false
                conv.addRef(ContextRef(kind: .activity, targetId: "", id: "", label: loc("ai.ref.activity")))
                conv.focusToken += 1
            }
        }
        .padding(5)
        .frame(width: 220)
    }
}

/// 输入框底部工具条的紧凑按钮
struct ToolbarButton<Label: View>: View {
    let help: String
    @ViewBuilder let label: Label
    let action: () -> Void
    @Environment(\.palette) var p
    @Environment(\.isEnabled) var isEnabled
    @State private var hover = false

    var body: some View {
        Button(action: action) {
            label
                .foregroundStyle(p.secondaryText)
                .padding(.horizontal, 8).frame(minWidth: 28).frame(height: 28)
                .background(RoundedRectangle(cornerRadius: 8).fill(hover && isEnabled ? p.hover.opacity(2.2) : .clear))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
        .help(help)
    }
}

/// 弹出菜单里的一行
struct MenuRow<Trailing: View>: View {
    var icon: String? = nil
    let title: String
    var selected = false
    var enabled = true
    var trailing: Trailing
    let action: () -> Void
    @Environment(\.palette) var p
    @State private var hover = false

    init(icon: String? = nil, title: String, selected: Bool = false, enabled: Bool = true,
         @ViewBuilder trailing: () -> Trailing, action: @escaping () -> Void) {
        self.icon = icon; self.title = title; self.selected = selected; self.enabled = enabled
        self.trailing = trailing(); self.action = action
    }

    var body: some View {
        Button(action: action) {
            HStack(spacing: 9) {
                if let icon { Image(systemName: icon).font(.system(size: 12)).frame(width: 18).foregroundStyle(enabled ? p.secondaryText : p.tertiaryText) }
                Text(title).font(.system(size: 12.5)).foregroundStyle(enabled ? p.text : p.tertiaryText).lineLimit(1)
                Spacer(minLength: 6)
                trailing
                if selected { Image(systemName: "checkmark").font(.system(size: 10.5, weight: .semibold)).foregroundStyle(p.accent) }
            }
            .padding(.horizontal, 8).frame(height: 28)
            .background(RoundedRectangle(cornerRadius: 6).fill(hover && enabled ? p.hover.opacity(2.2) : .clear))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .onHover { hover = $0 }
    }
}

extension MenuRow where Trailing == EmptyView {
    init(icon: String? = nil, title: String, selected: Bool = false, enabled: Bool = true, action: @escaping () -> Void) {
        self.init(icon: icon, title: title, selected: selected, enabled: enabled, trailing: { EmptyView() }, action: action)
    }
}

/// 模型选择：按供应商分组，能看图的带「看图」标记
struct ModelMenu: View {
    @ObservedObject var conv: AIConversationModel
    let close: () -> Void
    @EnvironmentObject var ai: AIHubModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        let items = ai.models?.items ?? []
        let providers = items.reduce(into: [(UInt64, String)]()) { acc, m in
            if !acc.contains(where: { $0.0 == m.providerId }) { acc.append((m.providerId, m.providerName)) }
        }
        ScrollView {
            VStack(alignment: .leading, spacing: 1) {
                ForEach(providers, id: \.0) { prov in
                    Text(prov.1).font(.system(size: 11, weight: .semibold)).foregroundStyle(p.tertiaryText)
                        .padding(.horizontal, 8).padding(.top, 6).padding(.bottom, 2)
                    ForEach(items.filter { $0.providerId == prov.0 }, id: \.id) { m in
                        MenuRow(title: m.name, selected: m.providerId == conv.providerId && m.id == conv.modelId) {
                            if m.vision {
                                Text(loc("ai.vision")).font(.system(size: 10, weight: .medium)).foregroundStyle(p.accent)
                                    .padding(.horizontal, 5).padding(.vertical, 1.5)
                                    .background(Capsule().fill(p.accent.opacity(0.12)))
                            }
                        } action: {
                            conv.setModel(m)
                            close()
                        }
                    }
                }
            }
            .padding(5)
        }
        .frame(width: 270)
        .frame(maxHeight: 380)
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// 权限档位：四档，高于管理员上限的置灰；完全信任要选时限；底部「先出计划」
struct ModeMenu: View {
    @ObservedObject var conv: AIConversationModel
    let close: () -> Void
    @EnvironmentObject var ai: AIHubModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var trustOpen = false

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            HStack(spacing: 5) {
                Text(loc("ai.mode")).font(.system(size: 11, weight: .semibold)).foregroundStyle(p.tertiaryText)
                InfoButton(text: loc("ai.mode.info"))
                Spacer()
            }
            .padding(.horizontal, 8).padding(.top, 4).padding(.bottom, 2)
            ForEach(AiMode.selectable, id: \.self) { m in
                let allowed = m.rank <= ai.maxMode.rank
                MenuRow(icon: m.icon, title: loc(m.labelKey), selected: conv.mode == m, enabled: allowed) {
                    if m == .fullTrust { Image(systemName: trustOpen ? "chevron.down" : "chevron.right").font(.system(size: 9, weight: .semibold)).foregroundStyle(p.tertiaryText) }
                } action: {
                    if m == .fullTrust { trustOpen.toggle() } else { conv.setMode(m); close() }
                }
                .help(allowed ? "" : loc("ai.mode.capped"))
                if m == .fullTrust && trustOpen {
                    ForEach([15, 60, 0], id: \.self) { mins in
                        MenuRow(title: mins == 0 ? loc("ai.trust.conversation") : mins == 60 ? loc("ai.trust.hours", 1) : loc("ai.trust.minutes", mins),
                                selected: conv.mode == .fullTrust && conv.trustMinutes == mins) {
                            conv.setMode(.fullTrust, minutes: mins)
                            close()
                        }
                        .padding(.leading, 26)
                    }
                }
            }
            Rectangle().fill(p.separator).frame(height: 1).padding(.vertical, 4).padding(.horizontal, 6)
            Toggle(isOn: Binding(get: { conv.planFirst }, set: { conv.setPlanFirst($0) })) {
                HStack(spacing: 5) {
                    Text(loc("ai.planFirst")).font(.system(size: 12.5))
                    InfoButton(text: loc("ai.planFirst.info"))
                }
            }
            .toggleStyle(.checkbox)
            .disabled(conv.conversation != nil)
            .padding(.horizontal, 9).padding(.vertical, 5)
        }
        .padding(5)
        .frame(width: 250)
        .onAppear { trustOpen = conv.mode == .fullTrust }
    }
}

/// 引用标签：图标 + 名称 + 删除叉；文件与文件夹悬停显示完整路径与所在设备
struct RefChip: View {
    let ref: ContextRef
    var onRemove: (() -> Void)? = nil
    var compact = false
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var ai: AIHubModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        HStack(spacing: 5) {
            icon.foregroundStyle(p.secondaryText)
            Text(ref.label.isEmpty ? loc("ai.ref.kind." + kindKey) : ref.label)
                .font(.system(size: 11.5, weight: .medium)).lineLimit(1).truncationMode(.middle)
                .frame(maxWidth: 220, alignment: .leading).fixedSize()
            if let onRemove {
                Button(action: onRemove) {
                    Image(systemName: "xmark").font(.system(size: 8, weight: .bold)).foregroundStyle(p.tertiaryText)
                        .frame(width: 14, height: 14).contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help(loc("ai.ref.remove"))
            }
        }
        .padding(.leading, 7).padding(.trailing, onRemove == nil ? 8 : 4).frame(height: compact ? 22 : 24)
        .background(RoundedRectangle(cornerRadius: 6).fill(p.hover.opacity(1.8)))
        .overlay(RoundedRectangle(cornerRadius: 6).stroke(p.border, lineWidth: 0.5))
        .help(tooltip)
    }

    @ViewBuilder private var icon: some View {
        if ref.kind == .device, let d = app.device(ref.id) {
            PlatformGlyph(platform: d.platform, size: 10)
        } else {
            Image(systemName: ref.kind.icon).font(.system(size: 10.5))
        }
    }

    private var kindKey: String {
        switch ref.kind {
        case .device: return "device"
        case .sshHost: return "sshHost"
        case .file: return "file"
        case .folder: return "folder"
        case .snippet: return "snippet"
        case .portForward: return "portForward"
        case .activity: return "activity"
        }
    }

    private var tooltip: String {
        switch ref.kind {
        case .file, .folder: return ref.id + "\n" + ai.targetName(ref.targetId)
        default: return loc("ai.ref.kind." + kindKey) + " · " + ref.label
        }
    }
}

/// @ 候选：账号里的设备（不含本机与手机）与 SSH 主机
struct MentionPopup: View {
    let candidates: [RefCandidate]
    let selected: Int
    let pick: (ContextRef) -> Void
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    static let rowHeight: CGFloat = 28
    /// 浮层高度（定位用）：最多 8 行
    static func height(_ n: Int) -> CGFloat { n == 0 ? 8 + 36 : 8 + CGFloat(min(n, 8)) * rowHeight + CGFloat(min(n, 8) - 1) }

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            if candidates.isEmpty {
                Text(loc("ai.mention.none")).font(.system(size: 12)).foregroundStyle(p.tertiaryText).padding(.horizontal, 10).frame(height: 36)
            }
            ForEach(Array(candidates.prefix(8).enumerated()), id: \.element.id) { i, c in
                Button { pick(c.ref) } label: {
                    HStack(spacing: 8) {
                        Group {
                            if let pf = c.platform { PlatformGlyph(platform: pf, size: 11) } else { Image(systemName: "server.rack").font(.system(size: 11)) }
                        }
                        .foregroundStyle(p.secondaryText).frame(width: 16)
                        Text(c.ref.label).font(.system(size: 12.5)).lineLimit(1)
                        if c.ref.kind == .sshHost { TagChip(text: "SSH") }
                        Spacer(minLength: 8)
                        if c.ref.kind == .device { StatusDot(state: c.online ? .online : .offline, size: 7) }
                    }
                    .padding(.horizontal, 8).frame(height: Self.rowHeight)
                    .background(RoundedRectangle(cornerRadius: 6).fill(i == min(selected, candidates.count - 1) ? p.selection : .clear))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
        .padding(4)
        .frame(width: 270)
        .background(RoundedRectangle(cornerRadius: 10).fill(p.elevated).shadow(color: .black.opacity(0.2), radius: 12, y: 4))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(p.border, lineWidth: 0.6))
    }
}

/// 权限档位分段选择（设置页的默认档位用）；超过上限的档位置灰
struct ModePicker: View {
    @Binding var mode: AiMode
    var max: AiMode = .fullTrust
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    var body: some View {
        HStack(spacing: 0) {
            ForEach(AiMode.selectable, id: \.self) { m in
                let sel = mode == m
                let allowed = m.rank <= max.rank
                Button { mode = m } label: {
                    VStack(spacing: 3) {
                        Image(systemName: m.icon).font(.system(size: 13))
                        Text(loc(m.labelKey)).font(.system(size: 11.5, weight: sel ? .semibold : .regular)).lineLimit(1)
                    }
                    .foregroundStyle(sel ? p.accentText : (allowed ? p.text : p.tertiaryText))
                    .frame(maxWidth: .infinity).frame(height: 50)
                    .background(RoundedRectangle(cornerRadius: 7).fill(sel ? (m == .fullTrust ? p.danger : p.accent) : .clear))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(!allowed)
            }
        }
        .padding(3)
        .background(RoundedRectangle(cornerRadius: 9).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 9).stroke(p.border, lineWidth: 0.6))
        .frame(maxWidth: 560)
    }
}

/// 简单的流式换行布局；trailing 为真时每行靠右
struct FlowLayout: Layout {
    var spacing: CGFloat = 6
    var trailing = false

    private func rows(_ width: CGFloat, _ subviews: Subviews) -> [[(Int, CGSize)]] {
        var out: [[(Int, CGSize)]] = [[]]
        var x: CGFloat = 0
        for (i, s) in subviews.enumerated() {
            let sz = s.sizeThatFits(.unspecified)
            if x + sz.width > width, x > 0 { out.append([]); x = 0 }
            out[out.count - 1].append((i, sz))
            x += sz.width + spacing
        }
        return out
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? 600
        let rs = rows(width, subviews)
        var h: CGFloat = 0
        var w: CGFloat = 0
        for r in rs {
            h += rowHeight(r)
            w = max(w, rowWidth(r))
        }
        if rs.count > 1 { h += spacing * CGFloat(rs.count - 1) }
        let outW: CGFloat
        if proposal.width == nil { outW = w } else if trailing { outW = min(width, w) } else { outW = width }
        return CGSize(width: outW, height: h)
    }

    private func rowWidth(_ r: [(Int, CGSize)]) -> CGFloat {
        var x: CGFloat = 0
        for item in r { x += item.1.width }
        if r.count > 1 { x += spacing * CGFloat(r.count - 1) }
        return x
    }

    private func rowHeight(_ r: [(Int, CGSize)]) -> CGFloat {
        var y: CGFloat = 0
        for item in r { y = max(y, item.1.height) }
        return y
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for r in rows(bounds.width, subviews) {
            let rowW = rowWidth(r)
            var x = trailing ? bounds.maxX - rowW : bounds.minX
            let rowH = rowHeight(r)
            for (i, sz) in r {
                subviews[i].place(at: CGPoint(x: x, y: y + (rowH - sz.height) / 2), proposal: ProposedViewSize(sz))
                x += sz.width + spacing
            }
            y += rowH + spacing
        }
    }
}

struct AnyButtonStyle: ButtonStyle {
    private let make: (Configuration) -> AnyView
    init<S: ButtonStyle>(_ s: S) { make = { AnyView(s.makeBody(configuration: $0)) } }
    func makeBody(configuration: Configuration) -> some View { make(configuration) }
}
