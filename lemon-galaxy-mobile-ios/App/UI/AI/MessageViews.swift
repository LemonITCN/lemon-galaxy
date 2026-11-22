import SwiftUI
import GalaxyKit

/// 消息流里的一条（docs/ai-chat-ui.md 第 4 节）。
struct ChatItemView: View {
    let item: ChatItem
    @ObservedObject var controller: ConversationController
    /// 每一轮里最后一个任务清单的 callId（只有它展开）
    var latestPlans: Set<String> = []
    /// 要显示「继续」按钮的错误条目
    var continueKey: String?
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        switch item {
        case .user(_, let text, let refs):
            UserBubble(text: text, refs: refs)
        case .assistant(_, let text, let thinking, let streaming):
            AssistantMessage(text: text, thinking: thinking, streaming: streaming)
        case .tool(let t):
            if t.phase == .awaiting {
                ApprovalCard(entry: t, controller: controller)
            } else if t.call.tool == PlanChecklist.tool {
                PlanCard(call: t.call, latest: latestPlans.contains(t.call.callId))
            } else {
                ToolRow(entry: t)
            }
        case .ask(let key, let q, let options, let answer):
            AskCard(key: key, question: q, options: options, answer: answer, controller: controller)
        case .secret(let key, let prompt, _, let provided):
            SecretCard(key: key, prompt: prompt, provided: provided, controller: controller)
        case .error(let key, let message):
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Image(systemName: "exclamationmark.circle.fill")
                Text(message).textSelection(.enabled)
                if key == continueKey {
                    Button(loc("ai.continue")) { controller.send(ChatContinue.text) }
                        .buttonStyle(.bordered)
                        .controlSize(.small)
                        .tint(p.accent)
                }
            }
            .font(.callout)
            .foregroundStyle(p.danger)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

// MARK: 用户消息

struct UserBubble: View {
    let text: String
    let refs: [ContextRef]
    @Environment(\.palette) private var p

    var body: some View {
        VStack(alignment: .trailing, spacing: 6) {
            if !refs.isEmpty {
                FlowLayout(spacing: 6, alignment: .trailing) {
                    ForEach(refs, id: \.refKey) { RefChip(ref: $0) }
                }
            }
            if !text.isEmpty {
                Text(text)
                    .foregroundStyle(p.text)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                    .background(RoundedRectangle(cornerRadius: 18, style: .continuous).fill(p.elevated))
                    .overlay(RoundedRectangle(cornerRadius: 18, style: .continuous).stroke(p.border, lineWidth: 0.5))
                    .textSelection(.enabled)
            }
        }
        .padding(.leading, 48)
        .frame(maxWidth: .infinity, alignment: .trailing)
    }
}

// MARK: 模型回复

struct AssistantMessage: View {
    let text: String
    let thinking: String
    let streaming: Bool
    @State private var showThinking = false
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !thinking.isEmpty {
                let thinkingNow = streaming && text.isEmpty
                Button { withAnimation(.easeInOut(duration: 0.15)) { showThinking.toggle() } } label: {
                    HStack(spacing: 5) {
                        Text(loc(thinkingNow ? "ai.thinkingNow" : "ai.thinking"))
                        Image(systemName: showThinking ? "chevron.down" : "chevron.right").font(.caption2.weight(.semibold))
                    }
                    .font(.footnote)
                    .foregroundStyle(p.tertiaryText)
                }
                .buttonStyle(.plain)
                if showThinking {
                    Text(thinking)
                        .font(.footnote)
                        .foregroundStyle(p.secondaryText)
                        .padding(.leading, 10)
                        .overlay(Rectangle().fill(p.border).frame(width: 2), alignment: .leading)
                        .textSelection(.enabled)
                }
            }
            if !text.isEmpty || (streaming && thinking.isEmpty) {
                MarkdownView(text: text, streaming: streaming)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// 流式输出末尾闪烁的光标
struct BlinkingCaret: View {
    @Environment(\.palette) private var p
    var body: some View {
        TimelineView(.periodic(from: .now, by: 0.5)) { ctx in
            let on = Int(ctx.date.timeIntervalSinceReferenceDate * 2) % 2 == 0
            RoundedRectangle(cornerRadius: 1).fill(p.text).frame(width: 8, height: 16).opacity(on ? 0.85 : 0)
        }
    }
}

/// Markdown：标题、列表、粗体、行内代码、代码块（语言名 + 复制）、表格、链接
struct MarkdownView: View {
    let text: String
    var streaming = false
    @Environment(\.palette) private var p

    var body: some View {
        let blocks = MarkdownParser.parse(text)
        VStack(alignment: .leading, spacing: 10) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { i, b in
                block(b, last: streaming && i == blocks.count - 1)
            }
            if streaming && (blocks.isEmpty || !caretInline(blocks.last!)) {
                BlinkingCaret()
            }
        }
        .tint(p.accent)
    }

    private func caretInline(_ b: MarkdownBlock) -> Bool {
        switch b {
        case .paragraph, .listItem, .heading: return true
        default: return false
        }
    }

    @ViewBuilder private func block(_ b: MarkdownBlock, last: Bool) -> some View {
        switch b {
        case .heading(let level, let t):
            withCaret(Text(inline(t)), last).font(level <= 1 ? .title3.weight(.bold) : level == 2 ? .headline : .subheadline.weight(.semibold))
                .foregroundStyle(p.text)
                .padding(.top, 2)
        case .paragraph(let t):
            withCaret(Text(inline(t)), last).foregroundStyle(p.text).textSelection(.enabled)
        case .listItem(let level, let marker, let t):
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(marker ?? "•").foregroundStyle(p.secondaryText).monospacedDigit()
                withCaret(Text(inline(t)), last).foregroundStyle(p.text).textSelection(.enabled)
            }
            .padding(.leading, CGFloat(level) * 16)
        case .quote(let t):
            Text(inline(t))
                .foregroundStyle(p.secondaryText)
                .padding(.leading, 10)
                .overlay(Rectangle().fill(p.border).frame(width: 3), alignment: .leading)
                .textSelection(.enabled)
        case .code(let lang, let code):
            CodeBlock(language: lang, code: code)
        case .table(let header, let rows):
            ScrollView(.horizontal, showsIndicators: false) {
                Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 6) {
                    GridRow { ForEach(Array(header.enumerated()), id: \.offset) { Text(inline($0.element)).fontWeight(.semibold) } }
                    Divider()
                    ForEach(Array(rows.enumerated()), id: \.offset) { _, r in
                        GridRow { ForEach(Array(r.enumerated()), id: \.offset) { Text(inline($0.element)) } }
                    }
                }
                .font(.subheadline)
                .foregroundStyle(p.text)
                .padding(10)
            }
            .background(RoundedRectangle(cornerRadius: 10).stroke(p.border, lineWidth: 0.5))
        case .rule:
            Divider()
        }
    }

    /// 段落末尾接一个闪烁光标（文字拼接，光标跟在最后一个字后面）
    private func withCaret(_ t: Text, _ last: Bool) -> Text {
        guard last else { return t }
        return t + Text(" \u{258D}").foregroundColor(p.accent)
    }

    private func inline(_ s: String) -> AttributedString {
        var a = (try? AttributedString(markdown: s, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(s)
        // 行内代码：等宽 + 浅底色
        for run in a.runs where run.inlinePresentationIntent?.contains(.code) == true {
            a[run.range].font = .system(.callout, design: .monospaced)
            a[run.range].backgroundColor = p.elevated
        }
        return a
    }
}

struct CodeBlock: View {
    let language: String
    let code: String
    @State private var copied = false
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(language.isEmpty ? "code" : language).font(.caption.monospaced()).foregroundStyle(p.tertiaryText)
                Spacer()
                Button {
                    UIPasteboard.general.string = code
                    copied = true
                    DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { copied = false }
                } label: {
                    Label(loc(copied ? "common.copied" : "common.copy"), systemImage: copied ? "checkmark" : "doc.on.doc")
                        .font(.caption)
                        .labelStyle(.titleAndIcon)
                }
                .buttonStyle(.plain)
                .foregroundStyle(p.secondaryText)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
            Divider()
            ScrollView(.horizontal, showsIndicators: false) {
                Text(code)
                    .font(.system(.footnote, design: .monospaced))
                    .foregroundStyle(p.text)
                    .padding(12)
                    .textSelection(.enabled)
            }
        }
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(p.elevated))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(p.border, lineWidth: 0.5))
    }
}

// MARK: 工具调用

enum ToolText {
    static func name(_ tool: String, _ loc: Strings) -> String {
        let key = "tool." + tool
        return loc.has(key) ? loc(key) : tool
    }
}

/// 一行紧凑的工具调用：图标 + 摘要 + 目标设备名 + 状态；点开看完整参数、理由、风险原因、输出与截图
struct ToolRow: View {
    let entry: ToolEntry
    @State private var expanded = false
    @State private var fullOutput = false
    @State private var zoomImage: ImageData?
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    private static let outputLines = 12

    var body: some View {
        let c = entry.call
        VStack(alignment: .leading, spacing: 8) {
            Button { withAnimation(.easeInOut(duration: 0.15)) { expanded.toggle() } } label: {
                HStack(spacing: 8) {
                    Image(systemName: Icons.tool(c.tool)).font(.footnote).foregroundStyle(p.secondaryText).frame(width: 18)
                    Text(c.summary.isEmpty ? ToolText.name(c.tool, loc) : c.summary)
                        .font(.footnote)
                        .foregroundStyle(p.text)
                        .lineLimit(1)
                    if !c.targetName.isEmpty {
                        Text(c.targetName).font(.caption).foregroundStyle(p.tertiaryText).lineLimit(1).layoutPriority(-1)
                    }
                    Spacer(minLength: 4)
                    status
                    Image(systemName: expanded ? "chevron.up" : "chevron.down").font(.caption2.weight(.semibold)).foregroundStyle(p.tertiaryText)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if expanded {
                VStack(alignment: .leading, spacing: 8) {
                    labeled(ToolText.name(c.tool, loc), c.detail.isEmpty ? c.summary : c.detail, mono: true)
                    if !c.modelReason.isEmpty { labeled(loc("tool.modelReason"), c.modelReason) }
                    if !c.riskReasons.isEmpty {
                        VStack(alignment: .leading, spacing: 3) {
                            HStack(spacing: 6) {
                                Text(loc("tool.reasons")).font(.caption2.weight(.semibold)).foregroundStyle(p.tertiaryText)
                                RiskBadge(risk: c.risk)
                            }
                            Text(c.riskReasons.joined(separator: loc("ai.listSeparator"))).font(.caption).foregroundStyle(p.secondaryText)
                        }
                    }
                    if let r = entry.result, !r.summary.isEmpty {
                        Text(r.summary).font(.caption).foregroundStyle(r.ok ? p.secondaryText : p.danger)
                    }
                    if !entry.output.isEmpty { output }
                    if let img = c.image ?? entry.result?.image, let ui = UIImage(data: img.data) {
                        Image(uiImage: ui).resizable().scaledToFit().frame(maxHeight: 180)
                            .clipShape(RoundedRectangle(cornerRadius: 8))
                            .onTapGesture { zoomImage = img }
                    }
                }
                .padding(.leading, 26)
            }
        }
        .padding(.vertical, 4)
        .sheet(item: Binding(get: { zoomImage.map { ImageBox(image: $0) } }, set: { if $0 == nil { zoomImage = nil } })) { b in
            ImageViewer(image: b.image)
        }
    }

    @ViewBuilder private var status: some View {
        switch entry.phase {
        case .awaiting: Image(systemName: "hand.raised.fill").foregroundStyle(p.warning).font(.caption)
        case .running: ProgressView().controlSize(.mini)
        case .done: Image(systemName: "checkmark").foregroundStyle(p.success).font(.caption.weight(.bold))
        case .failed, .denied: Image(systemName: "xmark").foregroundStyle(p.danger).font(.caption.weight(.bold))
        }
    }

    private var output: some View {
        let lines = entry.output.split(separator: "\n", omittingEmptySubsequences: false)
        let long = lines.count > Self.outputLines
        let shown = long && !fullOutput ? lines.prefix(Self.outputLines).joined(separator: "\n") : entry.output
        return VStack(alignment: .leading, spacing: 4) {
            ScrollView(.horizontal, showsIndicators: false) {
                Text(shown)
                    .font(.system(.caption2, design: .monospaced))
                    .foregroundStyle(p.secondaryText)
                    .textSelection(.enabled)
                    .padding(8)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 8).fill(p.elevated))
            if long {
                Button(loc(fullOutput ? "tool.lessOutput" : "tool.moreOutput", lines.count)) { fullOutput.toggle() }
                    .font(.caption)
                    .foregroundStyle(p.accent)
                    .buttonStyle(.plain)
            }
        }
    }

    private func labeled(_ title: String, _ text: String, mono: Bool = false) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title).font(.caption2.weight(.semibold)).foregroundStyle(p.tertiaryText)
            Text(text).font(mono ? .system(.caption, design: .monospaced) : .caption).foregroundStyle(p.secondaryText).textSelection(.enabled)
        }
    }
}

/// update_plan 的任务清单：每轮最后一份展开成勾选清单，前面的收成一行摘要（点开可看）。
struct PlanCard: View {
    let call: ToolCallView
    let latest: Bool
    @State private var toggled: Bool?
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        let items = PlanChecklist.parse(call.detail)
        let progress = PlanChecklist.progress(items)
        let expanded = toggled ?? latest
        VStack(alignment: .leading, spacing: 8) {
            Button { withAnimation(.easeInOut(duration: 0.15)) { toggled = !expanded } } label: {
                HStack(spacing: 8) {
                    Image(systemName: Icons.tool(call.tool)).font(.footnote).foregroundStyle(p.secondaryText).frame(width: 18)
                    Text(expanded ? loc("plan.title") : (call.summary.isEmpty ? loc("plan.title") : call.summary))
                        .font(.footnote.weight(expanded ? .semibold : .regular))
                        .foregroundStyle(p.text)
                        .lineLimit(1)
                    Spacer(minLength: 4)
                    Text("\(progress.done)/\(progress.total)").font(.caption.monospacedDigit()).foregroundStyle(p.tertiaryText)
                    Image(systemName: expanded ? "chevron.up" : "chevron.down").font(.caption2.weight(.semibold)).foregroundStyle(p.tertiaryText)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if expanded {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(Array(items.enumerated()), id: \.offset) { _, it in
                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            icon(it.status).frame(width: 18)
                            Text(it.text)
                                .font(.footnote.weight(it.status == .inProgress ? .semibold : .regular))
                                .foregroundStyle(it.status == .done ? p.tertiaryText : p.text)
                                .strikethrough(it.status == .done, color: p.tertiaryText)
                                .textSelection(.enabled)
                        }
                    }
                }
            }
        }
        .padding(.horizontal, expanded ? 12 : 0)
        .padding(.vertical, expanded ? 10 : 4)
        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(expanded ? p.card : Color.clear))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(expanded ? p.border : Color.clear, lineWidth: 0.5))
    }

    @ViewBuilder private func icon(_ s: PlanChecklist.Status) -> some View {
        switch s {
        case .done: Image(systemName: "checkmark.circle.fill").font(.footnote).foregroundStyle(p.success)
        case .inProgress: Image(systemName: "smallcircle.filled.circle").font(.footnote).foregroundStyle(p.accent)
        case .pending: Image(systemName: "circle").font(.footnote).foregroundStyle(p.tertiaryText)
        }
    }
}

/// 需要确认的工具调用：醒目的卡片。use_target（连接某设备）只有「允许」「拒绝」。
struct ApprovalCard: View {
    let entry: ToolEntry
    @ObservedObject var controller: ConversationController
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var denyPrompt = false
    @State private var denyReason = ""
    @State private var editPrompt = false
    @State private var editedCommand = ""
    @State private var zoomImage: ImageData?

    private var isSimple: Bool { SimpleApproval.applies(to: entry.call.tool) }

    private var simpleTitle: String {
        let c = entry.call
        if c.tool == "use_target" { return loc("approve.useTarget", c.targetName) }
        return c.summary.isEmpty ? ToolText.name(c.tool, loc) : c.summary
    }

    var body: some View {
        let c = entry.call
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: Icons.tool(c.tool)).foregroundStyle(p.warning)
                Text(isSimple ? simpleTitle : ToolText.name(c.tool, loc))
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(p.text)
                Spacer()
                if !isSimple { RiskBadge(risk: c.risk) }
            }
            if c.tool == "use_app" && !c.detail.isEmpty {
                Text(c.detail).font(.caption).foregroundStyle(p.secondaryText)
            }
            if !isSimple {
                if !c.targetName.isEmpty {
                    Label(c.targetName, systemImage: targetSymbol(c.targetId)).font(.caption).foregroundStyle(p.secondaryText)
                }
                Text(c.detail.isEmpty ? c.summary : c.detail)
                    .font(.system(.footnote, design: .monospaced))
                    .foregroundStyle(p.text)
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(RoundedRectangle(cornerRadius: 8).fill(p.elevated))
                    .textSelection(.enabled)
                if !c.riskReasons.isEmpty { labeled(loc("tool.reasons"), c.riskReasons.joined(separator: loc("ai.listSeparator"))) }
            }
            if !c.modelReason.isEmpty { labeled(loc("tool.modelReason"), c.modelReason) }
            if let img = c.image, let ui = UIImage(data: img.data) {
                Image(uiImage: ui).resizable().scaledToFit().frame(maxHeight: 200).clipShape(RoundedRectangle(cornerRadius: 8))
                    .onTapGesture { zoomImage = img }
            }
            buttons(c)
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).stroke(p.warning, lineWidth: 1.2))
        .alert(loc("approve.deny"), isPresented: $denyPrompt) {
            TextField(loc("approve.denyReasonHint"), text: $denyReason)
            Button(loc("approve.deny"), role: .destructive) {
                controller.decide(c.callId, .deny(reason: denyReason.isEmpty ? nil : denyReason))
            }
            Button(loc("common.cancel"), role: .cancel) {}
        }
        .alert(loc("approve.editTitle"), isPresented: $editPrompt) {
            TextField(loc("field.command"), text: $editedCommand)
            Button(loc("approve.run")) { controller.decide(c.callId, .editAndRun(command: editedCommand)) }
            Button(loc("common.cancel"), role: .cancel) {}
        }
        .sheet(item: Binding(get: { zoomImage.map { ImageBox(image: $0) } }, set: { if $0 == nil { zoomImage = nil } })) { b in
            ImageViewer(image: b.image)
        }
    }

    private func targetSymbol(_ id: String) -> String {
        if let d = account.device(id) { return Icons.platform(d.platform) }
        return "terminal"
    }

    @ViewBuilder private func buttons(_ c: ToolCallView) -> some View {
        if isSimple {
            HStack(spacing: 8) {
                Button { controller.decide(c.callId, .allowOnce) } label: { Text(loc("approve.allow")).frame(maxWidth: .infinity) }
                    .buttonStyle(.borderedProminent)
                Button(role: .destructive) { controller.decide(c.callId, .deny(reason: nil)) } label: { Text(loc("approve.deny")).frame(maxWidth: .infinity) }
                    .buttonStyle(.bordered)
            }
        } else {
            VStack(spacing: 8) {
                HStack(spacing: 8) {
                    Button { controller.decide(c.callId, .allowOnce) } label: { Text(loc("approve.once")).frame(maxWidth: .infinity) }
                        .buttonStyle(.borderedProminent)
                    Button { controller.decide(c.callId, .allowSimilar) } label: { Text(loc("approve.similar")).frame(maxWidth: .infinity) }
                        .buttonStyle(.bordered)
                }
                HStack(spacing: 8) {
                    Button(role: .destructive) { denyReason = ""; denyPrompt = true } label: { Text(loc("approve.deny")).frame(maxWidth: .infinity) }
                        .buttonStyle(.bordered)
                    if c.tool == "run_command" {
                        Button {
                            editedCommand = c.detail.isEmpty ? c.summary : c.detail
                            editPrompt = true
                        } label: { Text(loc("approve.edit")).frame(maxWidth: .infinity) }
                            .buttonStyle(.bordered)
                    }
                    InfoButton(text: loc("approve.similarInfo"), size: 14)
                }
            }
        }
    }

    private func labeled(_ title: String, _ text: String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title).font(.caption2.weight(.semibold)).foregroundStyle(p.tertiaryText)
            Text(text).font(.caption).foregroundStyle(p.secondaryText).textSelection(.enabled)
        }
    }
}

private struct ImageBox: Identifiable {
    let image: ImageData
    var id: String { "\(image.width)x\(image.height)-\(image.data.count)-\(image.data.hashValue)" }
}

struct ImageViewer: View {
    let image: ImageData
    @Environment(\.dismiss) private var dismiss
    @Environment(\.loc) private var loc
    @State private var scale: CGFloat = 1
    var body: some View {
        NavigationStack {
            GeometryReader { geo in
                ScrollView([.horizontal, .vertical]) {
                    if let ui = UIImage(data: image.data) {
                        Image(uiImage: ui).resizable().scaledToFit()
                            .frame(width: geo.size.width * scale)
                            .gesture(MagnificationGesture().onChanged { scale = min(max($0, 1), 5) })
                    }
                }
            }
            .background(Color.black)
            .navigationTitle("\(image.width)×\(image.height)")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button(loc("common.close")) { dismiss() } } }
        }
    }
}

// MARK: 提问与要密码

struct AskCard: View {
    let key: String
    let question: String
    let options: [String]
    let answer: String?
    @ObservedObject var controller: ConversationController
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var text = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label(question, systemImage: "questionmark.bubble").font(.callout.weight(.medium)).foregroundStyle(p.text)
            if let answer {
                Text(answer).font(.callout).foregroundStyle(p.secondaryText)
            } else {
                if !options.isEmpty {
                    FlowLayout(spacing: 8) {
                        ForEach(options, id: \.self) { o in
                            Button(o) { controller.answer(key, o) }.buttonStyle(.bordered)
                        }
                    }
                }
                HStack {
                    TextField(loc("ask.answerHint"), text: $text).textFieldStyle(.roundedBorder)
                    Button(loc("ai.send")) { controller.answer(key, text) }.disabled(text.isEmpty)
                }
            }
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 14).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 14).stroke(answer == nil ? p.accent : p.border, lineWidth: answer == nil ? 1.2 : 0.5))
    }
}

/// 要密码：输入内容直接送到被控端，模型看不到，也不进消息流
struct SecretCard: View {
    let key: String
    let prompt: String
    let provided: Bool
    @ObservedObject var controller: ConversationController
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var secret = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Label(loc("secret.title"), systemImage: "key.fill").font(.callout.weight(.semibold)).foregroundStyle(p.text)
                Spacer()
                InfoButton(text: loc("secret.info"), size: 13)
            }
            Text(prompt).font(.callout).foregroundStyle(p.secondaryText)
            if provided {
                Label(loc("secret.provided"), systemImage: "checkmark.seal").font(.caption).foregroundStyle(p.success)
            } else {
                HStack {
                    SecureField(loc("secret.hint"), text: $secret).textFieldStyle(.roundedBorder).textContentType(.password)
                    Button(loc("ai.send")) {
                        controller.provideSecret(key, secret)
                        secret = ""
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(secret.isEmpty)
                }
            }
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 14).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 14).stroke(provided ? p.border : p.warning, lineWidth: provided ? 0.5 : 1.2))
    }
}

// MARK: 自动换行排布（引用标签、选项按钮）

struct FlowLayout: Layout {
    var spacing: CGFloat = 6
    var alignment: HorizontalAlignment = .leading

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(width: proposal.width ?? .infinity, subviews: subviews)
        let w = rows.map(\.width).max() ?? 0
        let h = rows.reduce(0) { $0 + $1.height } + CGFloat(max(0, rows.count - 1)) * spacing
        return CGSize(width: proposal.width.map { min($0, w) } ?? w, height: h)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let rows = arrange(width: bounds.width, subviews: subviews)
        var y = bounds.minY
        for row in rows {
            var x = alignment == .trailing ? bounds.maxX - row.width : bounds.minX
            for i in row.items {
                let s = subviews[i].sizeThatFits(.unspecified)
                let fitted = CGSize(width: min(s.width, bounds.width), height: s.height)
                subviews[i].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(fitted))
                x += fitted.width + spacing
            }
            y += row.height + spacing
        }
    }

    private struct Row { var items: [Int] = []; var width: CGFloat = 0; var height: CGFloat = 0 }

    private func arrange(width: CGFloat, subviews: Subviews) -> [Row] {
        var rows: [Row] = []
        var cur = Row()
        for (i, v) in subviews.enumerated() {
            let s = v.sizeThatFits(.unspecified)
            let w = min(s.width, width)
            if !cur.items.isEmpty && cur.width + spacing + w > width {
                rows.append(cur)
                cur = Row()
            }
            cur.width += (cur.items.isEmpty ? 0 : spacing) + w
            cur.height = max(cur.height, s.height)
            cur.items.append(i)
        }
        if !cur.items.isEmpty { rows.append(cur) }
        return rows
    }
}
