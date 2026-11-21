import SwiftUI
import GalaxyKit

/// 对话页：流式消息、工具调用卡片（需要确认时带按钮）、提问、要密码；底部输入框与运行状态。
struct ConversationView: View {
    @ObservedObject var controller: ConversationController
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var ai: AgentHub
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var input = ""
    @FocusState private var inputFocused: Bool
    @State private var modeSheet = false

    var body: some View {
        let s = controller.state
        VStack(spacing: 0) {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 12) {
                        header(s)
                        ForEach(s.items) { item in
                            ChatItemView(item: item, controller: controller)
                                .id(item.id)
                        }
                        Color.clear.frame(height: 1).id("bottom")
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 12)
                }
                .scrollDismissesKeyboard(.interactively)
                .onChange(of: s.items.count) { _ in withAnimation { proxy.scrollTo("bottom", anchor: .bottom) } }
                .onChange(of: lastText(s)) { _ in proxy.scrollTo("bottom", anchor: .bottom) }
                .onAppear { proxy.scrollTo("bottom", anchor: .bottom) }
            }
            statusBar(s)
            composer(s)
        }
        .background(p.background)
        .navigationTitle(s.title.isEmpty ? loc("ai.untitled") : s.title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    Picker(loc("ai.changeMode"), selection: Binding(get: { s.mode }, set: { m in
                        controller.setMode(m, trustMinutes: m == .fullTrust ? 60 : nil)
                    })) {
                        ForEach(AiMode.selectable(maxMode: ai.models?.maxMode ?? .fullTrust), id: \.self) { m in
                            Text(loc("mode." + m.key)).tag(m)
                        }
                    }
                } label: {
                    Label(loc("mode." + s.mode.key), systemImage: modeIcon(s.mode))
                        .labelStyle(.titleAndIcon)
                        .font(.caption.weight(.semibold))
                }
            }
        }
    }

    private func lastText(_ s: ConversationState) -> Int {
        guard let last = s.items.last else { return 0 }
        switch last {
        case .assistant(_, let t, let th, _): return t.count + th.count
        case .tool(let t): return t.output.count + (t.result == nil ? 0 : 1)
        default: return 0
        }
    }

    private func modeIcon(_ m: AiMode) -> String {
        switch m {
        case .off: return "nosign"
        case .readOnly: return "eye"
        case .confirmAll: return "hand.raised"
        case .confirmDangerous: return "exclamationmark.shield"
        case .fullTrust: return "bolt.shield"
        }
    }

    private func header(_ s: ConversationState) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "cpu").foregroundStyle(p.tertiaryText)
            Text(([s.modelName] + s.targetNames).joined(separator: " · "))
                .font(.caption)
                .foregroundStyle(p.secondaryText)
                .lineLimit(2)
            if let until = s.trustUntilMs {
                Text(loc("ai.trustUntil", Fmt.date(until))).font(.caption2).foregroundStyle(p.warning)
            }
        }
        .frame(maxWidth: .infinity, alignment: .center)
        .padding(.bottom, 4)
    }

    private func statusBar(_ s: ConversationState) -> some View {
        HStack(spacing: 8) {
            if s.running && !s.waitingForUser {
                ProgressView().controlSize(.mini)
                Text(loc("ai.running") + (s.step > 0 ? " · " + loc("ai.step", s.step) : ""))
            } else if s.waitingForUser {
                Image(systemName: "hand.raised.fill").foregroundStyle(p.warning)
                Text(loc("ai.waiting"))
            } else {
                Image(systemName: "circle").font(.system(size: 6))
                Text(loc("ai.idle"))
            }
            Spacer()
            if s.promptTokens + s.completionTokens > 0 {
                Text(loc("ai.tokens", Fmt.tokens(s.promptTokens), Fmt.tokens(s.completionTokens))).monospacedDigit()
            }
        }
        .font(.caption)
        .foregroundStyle(p.secondaryText)
        .padding(.horizontal, 16)
        .padding(.vertical, 6)
        .background(p.card)
        .overlay(Divider(), alignment: .top)
    }

    private func composer(_ s: ConversationState) -> some View {
        HStack(alignment: .bottom, spacing: 8) {
            TextField(loc("ai.inputHint"), text: $input, axis: .vertical)
                .lineLimit(1...6)
                .focused($inputFocused)
                .padding(.horizontal, 12)
                .padding(.vertical, 9)
                .background(RoundedRectangle(cornerRadius: 18, style: .continuous).fill(p.elevated))
                .overlay(RoundedRectangle(cornerRadius: 18, style: .continuous).stroke(p.border, lineWidth: 0.5))
            if s.running {
                Button { controller.stop() } label: {
                    Image(systemName: "stop.circle.fill").font(.system(size: 32)).foregroundStyle(p.danger)
                }
                .accessibilityLabel(loc("ai.stop"))
            } else {
                Button {
                    controller.send(input)
                    input = ""
                } label: {
                    Image(systemName: "arrow.up.circle.fill").font(.system(size: 32))
                }
                .disabled(input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .accessibilityLabel(loc("ai.send"))
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(p.card)
    }
}

struct ChatItemView: View {
    let item: ChatItem
    @ObservedObject var controller: ConversationController
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var showThinking = false

    var body: some View {
        switch item {
        case .user(_, let text):
            HStack {
                Spacer(minLength: 40)
                Text(text)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                    .background(RoundedRectangle(cornerRadius: 18, style: .continuous).fill(p.accent))
                    .foregroundStyle(p.accentText)
                    .textSelection(.enabled)
            }
        case .assistant(_, let text, let thinking, let streaming):
            VStack(alignment: .leading, spacing: 6) {
                if !thinking.isEmpty {
                    DisclosureGroup(isExpanded: $showThinking) {
                        Text(thinking).font(.caption).foregroundStyle(p.secondaryText).textSelection(.enabled)
                    } label: {
                        Label(loc("ai.thinking"), systemImage: "brain").font(.caption).foregroundStyle(p.tertiaryText)
                    }
                }
                if !text.isEmpty {
                    MarkdownText(text: text)
                } else if streaming {
                    ProgressView().controlSize(.small)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        case .tool(let t):
            ToolCard(entry: t, controller: controller)
        case .ask(let key, let q, let options, let answer):
            AskCard(key: key, question: q, options: options, answer: answer, controller: controller)
        case .secret(let key, let prompt, _, let provided):
            SecretCard(key: key, prompt: prompt, provided: provided, controller: controller)
        case .error(_, let message):
            Label(message, systemImage: "exclamationmark.triangle.fill")
                .font(.callout)
                .foregroundStyle(p.danger)
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 12).fill(p.danger.opacity(0.1)))
        }
    }
}

/// 轻量 Markdown：``` 代码块单独排版，其余按行内 Markdown 显示（保留换行）。
struct MarkdownText: View {
    let text: String
    @Environment(\.palette) private var p

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(segments.enumerated()), id: \.offset) { _, seg in
                if seg.code {
                    ScrollView(.horizontal, showsIndicators: false) {
                        Text(seg.text).font(.system(.footnote, design: .monospaced)).foregroundStyle(p.text).padding(10)
                    }
                    .background(RoundedRectangle(cornerRadius: 10).fill(p.elevated))
                    .textSelection(.enabled)
                } else {
                    Text(attributed(seg.text)).foregroundStyle(p.text).textSelection(.enabled)
                }
            }
        }
    }

    private var segments: [(text: String, code: Bool)] {
        var out: [(String, Bool)] = []
        let parts = text.components(separatedBy: "```")
        for (i, part) in parts.enumerated() {
            if i % 2 == 1 {
                // 去掉语言标记那一行
                var lines = part.components(separatedBy: "\n")
                if let first = lines.first, !first.contains(" "), lines.count > 1 { lines.removeFirst() }
                out.append((lines.joined(separator: "\n").trimmingCharacters(in: .newlines), true))
            } else {
                let t = part.trimmingCharacters(in: .newlines)
                if !t.isEmpty { out.append((t, false)) }
            }
        }
        return out
    }

    private func attributed(_ s: String) -> AttributedString {
        (try? AttributedString(markdown: s, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(s)
    }
}

/// 工具调用卡片：目标、工具、完整命令、风险与原因；需要确认时给「允许一次 / 同类都允许 / 拒绝 / 修改后执行」。
struct ToolCard: View {
    let entry: ToolEntry
    @ObservedObject var controller: ConversationController
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var expanded = false
    @State private var denyPrompt = false
    @State private var denyReason = ""
    @State private var editPrompt = false
    @State private var editedCommand = ""
    @State private var zoomImage: ImageData?

    var body: some View {
        let c = entry.call
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: Icons.tool(c.tool)).foregroundStyle(p.accent).frame(width: 20)
                Text(toolName(c.tool)).font(.subheadline.weight(.semibold)).foregroundStyle(p.text)
                Text(c.targetName).font(.caption).foregroundStyle(p.secondaryText).lineLimit(1)
                Spacer()
                RiskBadge(risk: c.risk)
                phaseBadge
            }
            Text(c.summary)
                .font(.system(.footnote, design: .monospaced))
                .foregroundStyle(p.text)
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 8).fill(p.elevated))
                .textSelection(.enabled)
            if entry.phase == .awaiting || expanded {
                if !c.modelReason.isEmpty {
                    labeled(loc("tool.modelReason"), c.modelReason)
                }
                if !c.riskReasons.isEmpty {
                    labeled(loc("tool.reasons"), c.riskReasons.joined(separator: "；"))
                }
            }
            if let img = c.image ?? entry.result?.image, let ui = UIImage(data: img.data) {
                Image(uiImage: ui).resizable().scaledToFit().frame(maxHeight: 220).clipShape(RoundedRectangle(cornerRadius: 8))
                    .onTapGesture { zoomImage = img }
            }
            if entry.phase == .awaiting {
                approvalButtons(c)
            } else {
                if !entry.output.isEmpty || !c.detail.isEmpty {
                    Button { withAnimation { expanded.toggle() } } label: {
                        Label(loc(expanded ? "tool.hideOutput" : "tool.showOutput"), systemImage: expanded ? "chevron.up" : "chevron.down")
                            .font(.caption)
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(p.accent)
                }
                if expanded {
                    if !c.detail.isEmpty { labeled(loc("tool.detail"), c.detail, mono: true) }
                    if !entry.output.isEmpty {
                        ScrollView {
                            Text(entry.output).font(.system(.caption2, design: .monospaced)).foregroundStyle(p.secondaryText)
                                .frame(maxWidth: .infinity, alignment: .leading).textSelection(.enabled)
                        }
                        .frame(maxHeight: 260)
                        .padding(8)
                        .background(RoundedRectangle(cornerRadius: 8).fill(p.elevated))
                    }
                }
                if let r = entry.result, !r.summary.isEmpty {
                    Text(r.summary).font(.caption).foregroundStyle(r.ok ? p.secondaryText : p.danger)
                }
            }
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).stroke(entry.phase == .awaiting ? p.warning : p.border, lineWidth: entry.phase == .awaiting ? 1.2 : 0.5))
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

    private func toolName(_ t: String) -> String {
        let key = "tool." + t
        return loc.has(key) ? loc(key) : t
    }

    private func labeled(_ title: String, _ text: String, mono: Bool = false) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title).font(.caption2.weight(.semibold)).foregroundStyle(p.tertiaryText)
            Text(text).font(mono ? .system(.caption, design: .monospaced) : .caption).foregroundStyle(p.secondaryText).textSelection(.enabled)
        }
    }

    @ViewBuilder private var phaseBadge: some View {
        switch entry.phase {
        case .awaiting: Image(systemName: "hand.raised.fill").foregroundStyle(p.warning)
        case .running: ProgressView().controlSize(.mini)
        case .done: Image(systemName: "checkmark.circle.fill").foregroundStyle(p.success)
        case .failed: Image(systemName: "xmark.circle.fill").foregroundStyle(p.danger)
        case .denied: Image(systemName: "nosign").foregroundStyle(p.offline)
        }
    }

    private func approvalButtons(_ c: ToolCallView) -> some View {
        VStack(spacing: 8) {
            HStack(spacing: 8) {
                Button { controller.decide(c.callId, .allowOnce) } label: { Text(loc("approve.once")).frame(maxWidth: .infinity) }
                    .buttonStyle(.borderedProminent)
                Button { controller.decide(c.callId, .allowSimilar) } label: { Text(loc("approve.similar")).frame(maxWidth: .infinity) }
                    .buttonStyle(.bordered)
            }
            HStack(spacing: 8) {
                if c.tool == "run_command" {
                    Button {
                        editedCommand = c.summary
                        editPrompt = true
                    } label: { Text(loc("approve.edit")).frame(maxWidth: .infinity) }
                        .buttonStyle(.bordered)
                }
                Button(role: .destructive) { denyPrompt = true } label: { Text(loc("approve.deny")).frame(maxWidth: .infinity) }
                    .buttonStyle(.bordered)
                InfoButton(text: loc("approve.similarInfo"), size: 14)
            }
        }
        .controlSize(.regular)
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
                    FlowButtons(options: options) { controller.answer(key, $0) }
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

private struct FlowButtons: View {
    let options: [String]
    let pick: (String) -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(options, id: \.self) { o in
                Button(o) { pick(o) }.buttonStyle(.bordered)
            }
        }
    }
}

/// 要密码：输入内容直接送到被控端，模型看不到，也不存进对话
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
                    Button(loc("secret.provide")) {
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
