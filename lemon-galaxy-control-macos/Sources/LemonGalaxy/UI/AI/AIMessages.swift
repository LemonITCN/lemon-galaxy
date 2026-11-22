import SwiftUI
import AppKit

// 消息流里的各种条目（docs/ai-chat-ui.md 第 4 节）

struct ChatItemView: View {
    let item: ChatItem
    @ObservedObject var conv: AIConversationModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        switch item {
        case .user(_, let text, let refs):
            HStack {
                Spacer(minLength: 80)
                VStack(alignment: .trailing, spacing: 6) {
                    if !refs.isEmpty {
                        FlowLayout(spacing: 5, trailing: true) {
                            ForEach(refs, id: \.key) { r in RefChip(ref: r, compact: true) }
                        }
                    }
                    if !text.isEmpty {
                        Text(text).font(.system(size: 13.5)).foregroundStyle(p.text).textSelection(.enabled)
                            .lineSpacing(2)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.horizontal, 13).padding(.vertical, 9)
                            .background(RoundedRectangle(cornerRadius: 14).fill(p.accent.opacity(p.isDark ? 0.22 : 0.12)))
                    }
                }
                .frame(maxWidth: 560, alignment: .trailing)
            }
        case .assistant(let id):
            MarkdownView(text: conv.texts[id] ?? "", streaming: conv.running && !conv.doneMessages.contains(id))
        case .thinking(let id):
            ThinkingRow(id: id, conv: conv)
        case .tool(let card):
            if card.isPlan {
                PlanCard(card: card, superseded: conv.isPlanSuperseded(card))
            } else {
                ToolRow(card: card, conv: conv)
            }
        case .ask(let ask):
            AskCard(ask: ask, conv: conv)
        case .secret(let req):
            SecretCard(req: req, conv: conv)
        case .error(let id, let text):
            HStack(alignment: .top, spacing: 6) {
                Image(systemName: "exclamationmark.circle.fill").font(.system(size: 12)).padding(.top, 1)
                Text(text).font(.system(size: 12.5)).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                // 连续执行太多步暂停：提示里让回复「继续」，旁边直接给个按钮（只在最后一条、没在运行时）
                if text.contains("「继续」"), conv.items.last?.id == id, !conv.running, conv.conversation != nil {
                    Button { conv.sendQuick("继续") } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "play.fill").font(.system(size: 8.5))
                            Text(loc("ai.continue")).font(.system(size: 12, weight: .medium))
                        }
                        .foregroundStyle(p.accent)
                        .padding(.horizontal, 9).frame(height: 22)
                        .background(Capsule().fill(p.accent.opacity(0.12)))
                        .contentShape(Capsule())
                    }
                    .buttonStyle(.plain)
                    .padding(.top, -2)
                }
            }
            .foregroundStyle(p.danger)
        }
    }
}

// MARK: - 思考过程

struct ThinkingRow: View {
    let id: String
    @ObservedObject var conv: AIConversationModel
    @State private var expanded = false
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        let inProgress = conv.running && conv.items.last?.id == id
        VStack(alignment: .leading, spacing: 6) {
            Button { expanded.toggle() } label: {
                HStack(spacing: 6) {
                    if inProgress { ProgressView().controlSize(.mini) } else { Image(systemName: "sparkle").font(.system(size: 10.5)) }
                    Text(loc(inProgress ? "ai.thinking.active" : "ai.thinking")).font(.system(size: 12))
                    Image(systemName: "chevron.right").font(.system(size: 8, weight: .bold)).rotationEffect(.degrees(expanded ? 90 : 0))
                }
                .foregroundStyle(p.tertiaryText)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if expanded {
                Text(conv.texts[id] ?? "").font(.system(size: 12)).foregroundStyle(p.tertiaryText).textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.leading, 10)
                    .overlay(alignment: .leading) { Rectangle().fill(p.border).frame(width: 2) }
            }
        }
    }
}

// MARK: - 工具调用

struct ToolRow: View {
    @ObservedObject var card: ToolCardModel
    @ObservedObject var conv: AIConversationModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        if card.decision == .pending {
            ApprovalCard(card: card, conv: conv)
        } else {
            VStack(alignment: .leading, spacing: 0) {
                Button { card.expanded.toggle() } label: {
                    HStack(spacing: 8) {
                        Image(systemName: Self.icon(card.call.tool)).font(.system(size: 11.5)).foregroundStyle(p.secondaryText).frame(width: 16)
                        Text(Self.title(card, loc)).font(.system(size: 12.5)).foregroundStyle(p.text).lineLimit(1).truncationMode(.middle)
                        if !card.call.targetName.isEmpty && !card.isUseTarget {
                            Text(card.call.targetName).font(.system(size: 11.5)).foregroundStyle(p.tertiaryText).lineLimit(1).layoutPriority(-1)
                        }
                        Spacer(minLength: 6)
                        status
                        Image(systemName: "chevron.right").font(.system(size: 8.5, weight: .bold)).foregroundStyle(p.tertiaryText)
                            .rotationEffect(.degrees(card.expanded ? 90 : 0))
                    }
                    .padding(.horizontal, 10).frame(height: 32)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help(loc("ai.tool." + card.call.tool))
                if card.expanded {
                    Rectangle().fill(p.separator).frame(height: 1)
                    ToolDetails(card: card, conv: conv).padding(12)
                }
            }
            .background(RoundedRectangle(cornerRadius: 9).fill(p.card))
            .overlay(RoundedRectangle(cornerRadius: 9).stroke(p.border, lineWidth: 0.6))
        }
    }

    @ViewBuilder private var status: some View {
        switch card.phase {
        case .running:
            ProgressView().controlSize(.mini)
        case .finished:
            if card.failed {
                Image(systemName: "xmark.circle.fill").font(.system(size: 12)).foregroundStyle(p.danger)
            } else {
                Image(systemName: "checkmark.circle.fill").font(.system(size: 12)).foregroundStyle(p.success)
            }
        case .proposed:
            if case .denied = card.decision {
                Image(systemName: "xmark.circle.fill").font(.system(size: 12)).foregroundStyle(p.danger)
            } else if conv.running {
                ProgressView().controlSize(.mini)
            } else {
                Image(systemName: "minus.circle").font(.system(size: 12)).foregroundStyle(p.tertiaryText)
            }
        }
    }

    static func title(_ card: ToolCardModel, _ loc: Strings) -> String {
        if card.isUseTarget { return loc("ai.useTarget.row", card.call.targetName) }
        let s = card.call.summary.replacingOccurrences(of: "\n", with: " ")
        return s.isEmpty ? loc("ai.tool." + card.call.tool) : s
    }

    static func icon(_ tool: String) -> String {
        switch tool {
        case "run_command", "terminal_open", "terminal_send", "terminal_read", "terminal_close": return "terminal"
        case "screenshot": return "camera.viewfinder"
        case "mouse": return "cursorarrow.click"
        case "keyboard": return "keyboard"
        case "read_file", "list_dir", "search_files": return "doc.text.magnifyingglass"
        case "write_file", "edit_file": return "square.and.pencil"
        case "upload_file", "download_file": return "arrow.up.arrow.down"
        case "ui_tree": return "list.bullet.indent"
        case "use_target": return "link"
        case "use_app", "app_list", "app_state", "app_click", "app_set_value", "app_type", "app_key", "app_scroll", "app_action", "app_open":
            return "macwindow"
        case "web_fetch": return "globe"
        case "system_info": return "cpu"
        case "update_plan": return "checklist"
        case "list_devices": return "server.rack"
        case "list_snippets": return ContextRefKind.snippet.icon
        case "list_port_forwards": return ContextRefKind.portForward.icon
        case "recent_activity": return ContextRefKind.activity.icon
        case "wait": return "hourglass"
        default: return "wrench.and.screwdriver"
        }
    }
}

/// 展开后的工具调用：完整命令或参数、模型理由、风险原因、输出、截图
struct ToolDetails: View {
    @ObservedObject var card: ToolCardModel
    @ObservedObject var conv: AIConversationModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    static let maxLines = 12

    var risk: RiskLevel { card.call.risk }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            let command = card.decision.editedCommand ?? (card.call.detail.isEmpty ? card.call.summary : card.call.detail)
            if !command.isEmpty && !card.isSimpleApproval { CommandBlock(text: command, diff: Self.looksLikeDiff(card.call.tool, command)) }
            if card.isSimpleApproval && !card.call.detail.isEmpty {
                Text(card.call.detail).font(.system(size: 12)).foregroundStyle(p.secondaryText).textSelection(.enabled)
            }
            if !card.call.modelReason.isEmpty { ReasonLine(text: card.call.modelReason) }
            if !card.call.riskReasons.isEmpty { RiskReasons(risk: risk, reasons: card.call.riskReasons) }
            if let img = card.call.image { Thumbnail(img: img, conv: conv) }
            decisionNote
            if let r = card.result, !r.summary.isEmpty {
                HStack(spacing: 6) {
                    Image(systemName: card.failed ? "xmark.circle" : "checkmark.circle").foregroundStyle(card.failed ? p.danger : p.success)
                    Text(r.summary).foregroundStyle(card.failed ? p.danger : p.secondaryText).textSelection(.enabled)
                }
                .font(.system(size: 12))
            }
            if !card.output.isEmpty { output }
            if let img = card.result?.image { Thumbnail(img: img, conv: conv) }
        }
    }

    @ViewBuilder private var decisionNote: some View {
        switch card.decision {
        case .approvedSimilar: Label(loc("ai.decision.similar"), systemImage: "checkmark.circle").font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
        case .edited: Label(loc("ai.decision.edited"), systemImage: "pencil.circle").font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
        case .denied(let r?) where !r.isEmpty:
            Label(loc("ai.decision.denied") + (loc.isChinese ? "：" : ": ") + r, systemImage: "xmark.circle").font(.system(size: 11.5)).foregroundStyle(p.danger)
        default: EmptyView()
        }
    }

    private var output: some View {
        let trimmed = card.output.trimmingCharacters(in: .newlines)
        let lines = trimmed.split(separator: "\n", omittingEmptySubsequences: false)
        let over = lines.count > Self.maxLines
        let shown: String
        if card.outputExpanded || !over { shown = trimmed }
        else if card.phase == .running { shown = lines.suffix(Self.maxLines).joined(separator: "\n") }
        else { shown = lines.prefix(Self.maxLines).joined(separator: "\n") }
        return VStack(alignment: .leading, spacing: 4) {
            ScrollView(.horizontal, showsIndicators: false) {
                Text(shown).font(.system(size: 11.5, design: .monospaced)).foregroundStyle(p.secondaryText)
                    .textSelection(.enabled).fixedSize().padding(9)
            }
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 7).fill(p.isDark ? Color.black.opacity(0.25) : Color.black.opacity(0.035)))
            if over {
                Button { card.outputExpanded.toggle() } label: {
                    Text(card.outputExpanded ? loc("ai.output.collapse") : loc("ai.output.all", lines.count))
                        .font(.system(size: 11.5)).foregroundStyle(p.accent)
                }
                .buttonStyle(.plain)
            }
        }
    }

    static func looksLikeDiff(_ tool: String, _ s: String) -> Bool {
        guard tool == "edit_file" || tool == "write_file" || s.hasPrefix("--- ") || s.contains("\n@@ ") else { return false }
        return s.split(separator: "\n").contains { $0.hasPrefix("+") || $0.hasPrefix("-") }
    }
}

/// 命令或文件改动（diff 按行着色）
struct CommandBlock: View {
    let text: String
    var diff = false
    @Environment(\.palette) var p

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            Group {
                if diff { diffText } else { Text(text) }
            }
            .font(.system(size: 12, design: .monospaced))
            .foregroundStyle(p.text)
            .textSelection(.enabled)
            .fixedSize()
            .padding(.horizontal, 10).padding(.vertical, 8)
        }
        .fixedSize(horizontal: false, vertical: true)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 7).fill(p.isDark ? Color.black.opacity(0.25) : Color.black.opacity(0.04)))
    }

    private var diffText: Text {
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
        var out = Text("")
        for (i, l) in lines.enumerated() {
            let s = String(l) + (i < lines.count - 1 ? "\n" : "")
            let color: Color = l.hasPrefix("+") && !l.hasPrefix("+++") ? p.success
                : l.hasPrefix("-") && !l.hasPrefix("---") ? p.danger
                : l.hasPrefix("@@") ? p.info : p.text
            out = out + Text(s).foregroundColor(color)
        }
        return out
    }
}

struct ReasonLine: View {
    let text: String
    @Environment(\.palette) var p
    var body: some View {
        HStack(alignment: .top, spacing: 6) {
            Image(systemName: "text.bubble").font(.system(size: 11)).foregroundStyle(p.tertiaryText).padding(.top, 1)
            Text(text).font(.system(size: 12)).foregroundStyle(p.secondaryText).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
        }
    }
}

struct RiskReasons: View {
    let risk: RiskLevel
    let reasons: [String]
    @Environment(\.palette) var p
    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            ForEach(reasons, id: \.self) { r in
                HStack(alignment: .top, spacing: 6) {
                    Image(systemName: risk.icon).font(.system(size: 10)).foregroundStyle(risk.color(p)).padding(.top, 2)
                    Text(r).font(.system(size: 12)).foregroundStyle(p.text).fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }
}

struct RiskChip: View {
    let risk: RiskLevel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: risk.icon).font(.system(size: 9.5))
            Text(loc("ai.risk." + risk.key)).font(.system(size: 11, weight: .semibold))
        }
        .foregroundStyle(risk.color(p))
        .padding(.horizontal, 7).padding(.vertical, 2.5)
        .background(Capsule().fill(risk.color(p).opacity(0.13)))
    }
}

struct Thumbnail: View {
    let img: ImageData
    @ObservedObject var conv: AIConversationModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    var body: some View {
        if let ns = NSImage(data: img.data) {
            Image(nsImage: ns).resizable().aspectRatio(contentMode: .fit)
                .frame(maxWidth: 320, maxHeight: 180, alignment: .leading)
                .clipShape(RoundedRectangle(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(p.border, lineWidth: 0.5))
                .onTapGesture { conv.zoomImage = img }
                .help(loc("ai.enlarge"))
        }
    }
}

extension ToolCardModel.Decision {
    var editedCommand: String? { if case .edited(let c) = self { return c }; return nil }
}

/// 需要确认的工具调用：醒目的卡片
struct ApprovalCard: View {
    @ObservedObject var card: ToolCardModel
    @ObservedObject var conv: AIConversationModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var risk: RiskLevel { card.call.risk }
    var accent: Color { card.isSimpleApproval ? p.accent : risk.color(p) }

    private var heading: String {
        if card.isUseTarget { return loc("ai.useTarget.ask", card.call.targetName) }
        if card.isUseApp { return card.call.summary.isEmpty ? loc("ai.tool.use_app") : card.call.summary }
        return loc("ai.tool." + card.call.tool)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 11) {
            HStack(spacing: 8) {
                Image(systemName: ToolRow.icon(card.call.tool)).font(.system(size: 13)).foregroundStyle(accent)
                Text(heading)
                    .font(.system(size: 13, weight: .semibold))
                if !card.isUseTarget && !card.call.targetName.isEmpty {
                    HStack(spacing: 4) {
                        Image(systemName: "arrow.right").font(.system(size: 9.5))
                        Text(card.call.targetName).font(.system(size: 12))
                    }
                    .foregroundStyle(p.secondaryText)
                }
                Spacer()
                if !card.isSimpleApproval { RiskChip(risk: risk) }
            }
            if card.isSimpleApproval {
                if !card.call.detail.isEmpty {
                    Text(card.call.detail).font(.system(size: 12)).foregroundStyle(p.secondaryText).textSelection(.enabled)
                }
            } else if card.editing {
                TextField("", text: $card.editText, axis: .vertical).fieldStyle().font(.system(size: 12, design: .monospaced))
            } else {
                let command = card.call.detail.isEmpty ? card.call.summary : card.call.detail
                CommandBlock(text: command, diff: ToolDetails.looksLikeDiff(card.call.tool, command))
            }
            if let img = card.call.image { Thumbnail(img: img, conv: conv) }
            if !card.call.riskReasons.isEmpty { RiskReasons(risk: card.isSimpleApproval ? .unknown : risk, reasons: card.call.riskReasons) }
            if !card.call.modelReason.isEmpty { ReasonLine(text: card.call.modelReason) }
            buttons
        }
        .padding(14)
        .background(RoundedRectangle(cornerRadius: 11).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 11).stroke(accent.opacity(0.75), lineWidth: 1.2))
    }

    @ViewBuilder private var buttons: some View {
        if card.denying {
            VStack(alignment: .leading, spacing: 8) {
                TextField(loc("ai.deny.reason"), text: $card.denyReason).fieldStyle()
                    .onSubmit { deny() }
                HStack {
                    Spacer()
                    Button(loc("common.cancel")) { card.denying = false }.buttonStyle(SecondaryButtonStyle())
                    Button(loc("ai.approve.deny")) { deny() }.buttonStyle(PrimaryButtonStyle(destructive: true))
                }
            }
        } else if card.editing {
            HStack {
                Spacer()
                Button(loc("common.cancel")) { card.editing = false }.buttonStyle(SecondaryButtonStyle())
                Button(loc("ai.approve.runEdited")) { conv.decide(card, .editAndRun(command: card.editText)) }.buttonStyle(PrimaryButtonStyle())
            }
        } else if card.isSimpleApproval {
            HStack(spacing: 8) {
                Button(loc("ai.approve.allow")) { conv.decide(card, .allowOnce) }.buttonStyle(PrimaryButtonStyle())
                Button(loc("ai.approve.deny")) { conv.decide(card, .deny(reason: nil)) }.buttonStyle(SecondaryButtonStyle(destructive: true))
                Spacer()
            }
        } else {
            HStack(spacing: 8) {
                Button(loc("ai.approve.once")) { conv.decide(card, .allowOnce) }.buttonStyle(PrimaryButtonStyle())
                Button(loc("ai.approve.similar")) { conv.decide(card, .allowSimilar) }.buttonStyle(SecondaryButtonStyle())
                if card.call.tool == "run_command" {
                    Button(loc("ai.approve.edit")) { card.editText = card.call.summary; card.editing = true }.buttonStyle(SecondaryButtonStyle())
                }
                Spacer()
                Button(loc("ai.approve.deny")) { card.denying = true }.buttonStyle(SecondaryButtonStyle(destructive: true))
            }
        }
    }

    private func deny() {
        let r = card.denyReason.trimmingCharacters(in: .whitespacesAndNewlines)
        conv.decide(card, .deny(reason: r.isEmpty ? nil : r))
    }
}

// MARK: - 任务清单（update_plan）

/// update_plan 的清单：detail 每行一项，前缀 "[x] " 已完成、"[>] " 进行中、"[ ] " 待做。
/// 一轮里只有最新的一份展开，之前的收成一行摘要（点开可看）。
struct PlanCard: View {
    @ObservedObject var card: ToolCardModel
    let superseded: Bool
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    enum Status { case done, active, pending }
    struct Item: Hashable { let status: Status; let text: String }

    static func parse(_ detail: String) -> [Item] {
        detail.split(whereSeparator: \.isNewline).compactMap { raw in
            let line = raw.trimmingCharacters(in: .whitespaces)
            guard !line.isEmpty else { return nil }
            for (prefix, st) in [("[x]", Status.done), ("[X]", .done), ("[>]", .active), ("[ ]", .pending)] where line.hasPrefix(prefix) {
                return Item(status: st, text: String(line.dropFirst(prefix.count)).trimmingCharacters(in: .whitespaces))
            }
            return Item(status: .pending, text: line)
        }
    }

    /// 最新的一份默认展开，被后面的清单取代的默认收起；点标题切换
    var open: Bool { superseded ? card.expanded : !card.expanded }

    var body: some View {
        let items = Self.parse(card.call.detail)
        VStack(alignment: .leading, spacing: 0) {
            Button { card.expanded.toggle() } label: {
                HStack(spacing: 8) {
                    Image(systemName: "checklist").font(.system(size: 11.5)).foregroundStyle(p.secondaryText).frame(width: 16)
                    Text(card.call.summary.isEmpty ? loc("ai.tool.update_plan") : card.call.summary)
                        .font(.system(size: 12.5, weight: open ? .medium : .regular)).foregroundStyle(open ? p.text : p.secondaryText)
                        .lineLimit(1).truncationMode(.tail)
                    Spacer(minLength: 6)
                    Image(systemName: "chevron.right").font(.system(size: 8.5, weight: .bold)).foregroundStyle(p.tertiaryText)
                        .rotationEffect(.degrees(open ? 90 : 0))
                }
                .padding(.horizontal, 10).frame(height: 32)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(loc("ai.tool.update_plan"))
            if open && !items.isEmpty {
                Rectangle().fill(p.separator).frame(height: 1)
                VStack(alignment: .leading, spacing: 7) {
                    ForEach(Array(items.enumerated()), id: \.offset) { _, it in
                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            icon(it.status).frame(width: 16)
                            Text(it.text)
                                .font(.system(size: 12.5, weight: it.status == .active ? .medium : .regular))
                                .foregroundStyle(it.status == .pending ? p.secondaryText : it.status == .done ? p.tertiaryText : p.text)
                                .strikethrough(it.status == .done, color: p.tertiaryText)
                                .textSelection(.enabled)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                }
                .padding(.horizontal, 10).padding(.vertical, 10)
            }
        }
        .background(RoundedRectangle(cornerRadius: 9).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 9).stroke(p.border, lineWidth: 0.6))
    }

    @ViewBuilder private func icon(_ s: Status) -> some View {
        switch s {
        case .done: Image(systemName: "checkmark.circle.fill").font(.system(size: 12)).foregroundStyle(p.success)
        case .active: Image(systemName: "circle.inset.filled").font(.system(size: 12)).foregroundStyle(p.accent)
        case .pending: Image(systemName: "circle").font(.system(size: 12)).foregroundStyle(p.tertiaryText)
        }
    }
}

// MARK: - 提问、要密码

struct AskCard: View {
    @ObservedObject var ask: AskModel
    @ObservedObject var conv: AIConversationModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top, spacing: 7) {
                Image(systemName: "questionmark.bubble").foregroundStyle(p.accent).padding(.top, 1)
                Text(ask.question).font(.system(size: 13, weight: .medium)).fixedSize(horizontal: false, vertical: true)
            }
            if let a = ask.answer {
                Label(a.isEmpty ? loc("ai.ask.skipped") : a, systemImage: "arrowshape.turn.up.left").font(.system(size: 12)).foregroundStyle(p.secondaryText)
            } else {
                if !ask.options.isEmpty {
                    FlowLayout(spacing: 8) {
                        ForEach(ask.options, id: \.self) { o in
                            Button(o) { conv.answer(ask, o) }
                                .buttonStyle(o == ask.options.first ? AnyButtonStyle(PrimaryButtonStyle()) : AnyButtonStyle(SecondaryButtonStyle()))
                        }
                    }
                }
                HStack(spacing: 8) {
                    TextField(loc("ai.ask.other"), text: $ask.freeText).fieldStyle()
                        .onSubmit(submit)
                    Button(loc("ai.ask.reply"), action: submit).buttonStyle(SecondaryButtonStyle())
                        .disabled(ask.freeText.trimmingCharacters(in: .whitespaces).isEmpty)
                }
                .frame(maxWidth: 420)
            }
        }
        .padding(14)
        .background(RoundedRectangle(cornerRadius: 11).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 11).stroke(ask.answer == nil ? p.accent.opacity(0.6) : p.border, lineWidth: ask.answer == nil ? 1.2 : 0.6))
    }

    private func submit() {
        let t = ask.freeText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return }
        conv.answer(ask, t)
    }
}

/// request_secret：卡片里一个不回显的输入框，内容只交给 provide_secret，不进消息流
struct SecretCard: View {
    @ObservedObject var req: SecretRequestModel
    @ObservedObject var conv: AIConversationModel
    @EnvironmentObject var ai: AIHubModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var secret = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top, spacing: 7) {
                Image(systemName: "key.horizontal").foregroundStyle(p.warning).padding(.top, 1)
                Text(req.prompt).font(.system(size: 13, weight: .medium)).fixedSize(horizontal: false, vertical: true)
                InfoButton(text: loc("ai.secret.info"))
                Spacer(minLength: 0)
            }
            if req.done {
                Label(loc("ai.secret.sent"), systemImage: "checkmark").font(.system(size: 12)).foregroundStyle(p.success)
            } else if req.cancelled {
                Text(loc("ai.secret.cancelled")).font(.system(size: 12)).foregroundStyle(p.tertiaryText)
            } else {
                HStack(spacing: 8) {
                    SecureField(loc("ai.secret.placeholder"), text: $secret).fieldStyle().onSubmit(submit)
                    Button(loc("ai.secret.send"), action: submit).buttonStyle(PrimaryButtonStyle()).disabled(secret.isEmpty)
                    Button(loc("common.cancel")) { conv.cancelSecret(req); secret = "" }.buttonStyle(SecondaryButtonStyle())
                }
                .frame(maxWidth: 460)
                if !req.targetId.isEmpty {
                    Text(loc("ai.secret.target", ai.targetName(req.targetId))).font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
                }
            }
        }
        .padding(14)
        .background(RoundedRectangle(cornerRadius: 11).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 11).stroke(req.done || req.cancelled ? p.border : p.warning.opacity(0.6), lineWidth: req.done || req.cancelled ? 0.6 : 1.2))
    }

    private func submit() {
        guard !secret.isEmpty else { return }
        conv.provideSecret(req, secret)
        secret = ""
    }
}

// MARK: - Markdown

/// 模型回复按 Markdown 渲染：标题、列表、粗体、行内代码、代码块（语言名 + 复制）、表格、链接、引用、分隔线。
/// 流式输出时末尾一个闪烁的光标。
struct MarkdownView: View {
    let text: String
    var streaming = false
    @Environment(\.palette) var p

    var body: some View {
        let blocks = MD.parse(text)
        VStack(alignment: .leading, spacing: 10) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { i, b in
                MDBlockView(block: b, cursor: streaming && i == blocks.count - 1)
            }
            if streaming && blocks.isEmpty { BlinkingCursor { Text("") + $0 } }
        }
        .textSelection(.enabled)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// 末尾的闪烁光标：把光标拼在最后一段文字后面
struct BlinkingCursor: View {
    let content: (Text) -> Text
    @Environment(\.palette) var p
    var body: some View {
        TimelineView(.periodic(from: .now, by: 0.5)) { ctx in
            let on = Int(ctx.date.timeIntervalSinceReferenceDate * 2) % 2 == 0
            content(Text(" \u{258D}").foregroundColor(p.accent.opacity(on ? 0.9 : 0)))
        }
    }
}

enum MDBlock {
    case heading(Int, String)
    case paragraph(String)
    case list([MDListItem])
    case code(String, String)
    case table([String], [[String]])
    case quote(String)
    case rule
}

struct MDListItem {
    var indent: Int
    var marker: String
    var text: String
}

enum MD {
    static func parse(_ s: String) -> [MDBlock] {
        let lines = s.replacingOccurrences(of: "\r\n", with: "\n").components(separatedBy: "\n")
        var blocks: [MDBlock] = []
        var para: [String] = []
        func flush() {
            if !para.isEmpty { blocks.append(.paragraph(para.joined(separator: "\n"))); para = [] }
        }
        var i = 0
        while i < lines.count {
            let line = lines[i]
            let t = line.trimmingCharacters(in: .whitespaces)
            if t.hasPrefix("```") || t.hasPrefix("~~~") {
                flush()
                let fence = String(t.prefix(3))
                let lang = t.dropFirst(3).trimmingCharacters(in: .whitespaces)
                var code: [String] = []
                i += 1
                while i < lines.count, !lines[i].trimmingCharacters(in: .whitespaces).hasPrefix(fence) {
                    code.append(lines[i])
                    i += 1
                }
                i += 1
                blocks.append(.code(lang, code.joined(separator: "\n")))
                continue
            }
            if t.isEmpty { flush(); i += 1; continue }
            if let h = heading(t) { flush(); blocks.append(h); i += 1; continue }
            if isRule(t) { flush(); blocks.append(.rule); i += 1; continue }
            if t.contains("|"), i + 1 < lines.count, isTableSeparator(lines[i + 1]) {
                flush()
                let header = cells(t)
                var rows: [[String]] = []
                i += 2
                while i < lines.count {
                    let r = lines[i].trimmingCharacters(in: .whitespaces)
                    guard !r.isEmpty, r.contains("|") else { break }
                    rows.append(cells(r))
                    i += 1
                }
                blocks.append(.table(header, rows))
                continue
            }
            if let first = listItem(line) {
                flush()
                var items = [first]
                i += 1
                while i < lines.count {
                    if let it = listItem(lines[i]) {
                        items.append(it)
                        i += 1
                    } else if lines[i].hasPrefix("  "), !lines[i].trimmingCharacters(in: .whitespaces).isEmpty {
                        items[items.count - 1].text += "\n" + lines[i].trimmingCharacters(in: .whitespaces)
                        i += 1
                    } else { break }
                }
                blocks.append(.list(items))
                continue
            }
            if t.hasPrefix(">") {
                flush()
                var q: [String] = []
                while i < lines.count {
                    let x = lines[i].trimmingCharacters(in: .whitespaces)
                    guard x.hasPrefix(">") else { break }
                    q.append(String(x.dropFirst()).trimmingCharacters(in: .whitespaces))
                    i += 1
                }
                blocks.append(.quote(q.joined(separator: "\n")))
                continue
            }
            para.append(line)
            i += 1
        }
        flush()
        return blocks
    }

    static func heading(_ t: String) -> MDBlock? {
        var level = 0
        for ch in t { if ch == "#" { level += 1 } else { break } }
        guard (1...6).contains(level), t.count > level, t[t.index(t.startIndex, offsetBy: level)] == " " else { return nil }
        return .heading(level, String(t.dropFirst(level)).trimmingCharacters(in: .whitespaces))
    }

    static func isRule(_ t: String) -> Bool {
        let s = t.replacingOccurrences(of: " ", with: "")
        guard s.count >= 3, let c = s.first, "-*_".contains(c) else { return false }
        return s.allSatisfy { $0 == c }
    }

    static func isTableSeparator(_ line: String) -> Bool {
        let t = line.trimmingCharacters(in: .whitespaces)
        guard t.contains("-"), t.contains("|") || t.hasPrefix(":") || t.hasPrefix("-") else { return false }
        return t.allSatisfy { "|-: ".contains($0) }
    }

    static func cells(_ line: String) -> [String] {
        var t = line.trimmingCharacters(in: .whitespaces)
        if t.hasPrefix("|") { t.removeFirst() }
        if t.hasSuffix("|") { t.removeLast() }
        return t.components(separatedBy: "|").map { $0.trimmingCharacters(in: .whitespaces) }
    }

    static func listItem(_ line: String) -> MDListItem? {
        var indent = 0
        var idx = line.startIndex
        while idx < line.endIndex, line[idx] == " " || line[idx] == "\t" {
            indent += line[idx] == "\t" ? 4 : 1
            idx = line.index(after: idx)
        }
        let rest = line[idx...]
        for b in ["- ", "* ", "+ "] where rest.hasPrefix(b) {
            return MDListItem(indent: indent / 2, marker: "\u{2022}", text: String(rest.dropFirst(2)))
        }
        let digits = rest.prefix { $0.isASCII && $0.isNumber }
        if !digits.isEmpty, digits.count <= 3 {
            let after = rest.dropFirst(digits.count)
            if after.hasPrefix(". ") || after.hasPrefix(") ") {
                return MDListItem(indent: indent / 2, marker: digits + ".", text: String(after.dropFirst(2)))
            }
        }
        return nil
    }

    /// 行内 Markdown：粗体、斜体、行内代码（等宽 + 底色）、链接（强调色）、删除线
    static func inline(_ s: String, p: Palette, size: CGFloat) -> AttributedString {
        let opts = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace, failurePolicy: .returnPartiallyParsedIfPossible)
        var a = (try? AttributedString(markdown: s, options: opts)) ?? AttributedString(s)
        var codeRanges: [Range<AttributedString.Index>] = []
        var linkRanges: [Range<AttributedString.Index>] = []
        for run in a.runs {
            if let intent = run.inlinePresentationIntent, intent.contains(.code) { codeRanges.append(run.range) }
            if run.link != nil { linkRanges.append(run.range) }
        }
        for r in codeRanges {
            a[r].font = .system(size: size - 1, design: .monospaced)
            a[r].backgroundColor = p.isDark ? Color.white.opacity(0.1) : Color.black.opacity(0.06)
        }
        for r in linkRanges { a[r].foregroundColor = p.accent }
        return a
    }
}

struct MDBlockView: View {
    let block: MDBlock
    var cursor = false
    @Environment(\.palette) var p
    static let size: CGFloat = 13.5

    var body: some View {
        switch block {
        case .heading(let level, let t):
            let size: CGFloat = level == 1 ? 19 : level == 2 ? 16.5 : level == 3 ? 14.5 : 13.5
            textWithCursor(Text(MD.inline(t, p: p, size: size)))
                .font(.system(size: size, weight: .semibold))
                .padding(.top, level <= 2 ? 6 : 2)
                .fixedSize(horizontal: false, vertical: true)
        case .paragraph(let t):
            textWithCursor(Text(MD.inline(t, p: p, size: Self.size)))
                .font(.system(size: Self.size)).lineSpacing(3.5)
                .fixedSize(horizontal: false, vertical: true)
        case .list(let items):
            VStack(alignment: .leading, spacing: 5) {
                ForEach(Array(items.enumerated()), id: \.offset) { i, it in
                    HStack(alignment: .firstTextBaseline, spacing: 7) {
                        Text(it.marker).foregroundStyle(p.secondaryText).frame(minWidth: 14, alignment: .trailing)
                        let body = Text(MD.inline(it.text, p: p, size: Self.size))
                        Group {
                            if i == items.count - 1 { textWithCursor(body) } else { body }
                        }
                        .lineSpacing(3)
                        .fixedSize(horizontal: false, vertical: true)
                    }
                    .font(.system(size: Self.size))
                    .padding(.leading, CGFloat(it.indent) * 18)
                }
            }
        case .code(let lang, let code):
            VStack(alignment: .leading, spacing: 4) {
                CodeBlockView(lang: lang, code: code)
                if cursor { BlinkingCursor { $0 } }
            }
        case .table(let header, let rows):
            TableBlockView(header: header, rows: rows)
        case .quote(let t):
            textWithCursor(Text(MD.inline(t, p: p, size: Self.size)))
                .font(.system(size: Self.size)).foregroundStyle(p.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.leading, 12)
                .overlay(alignment: .leading) { Rectangle().fill(p.border).frame(width: 3) }
        case .rule:
            Rectangle().fill(p.separator).frame(height: 1).padding(.vertical, 4)
        }
    }

    @ViewBuilder private func textWithCursor(_ t: Text) -> some View {
        if cursor { BlinkingCursor { t + $0 } } else { t }
    }
}

struct CodeBlockView: View {
    let lang: String
    let code: String
    @State private var copied = false
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Text(lang.isEmpty ? "text" : lang).font(.system(size: 11, design: .monospaced)).foregroundStyle(p.tertiaryText)
                Spacer()
                Button {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(code, forType: .string)
                    copied = true
                    DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { copied = false }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: copied ? "checkmark" : "doc.on.doc").font(.system(size: 10.5))
                        Text(loc(copied ? "common.copied" : "common.copy")).font(.system(size: 11))
                    }
                    .foregroundStyle(p.secondaryText)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            .padding(.horizontal, 12).frame(height: 30)
            .background(p.isDark ? Color.white.opacity(0.04) : Color.black.opacity(0.035))
            ScrollView(.horizontal, showsIndicators: false) {
                Text(code).font(.system(size: 12, design: .monospaced)).foregroundStyle(p.text)
                    .textSelection(.enabled).fixedSize().padding(12)
            }
            .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 9).fill(p.isDark ? Color.black.opacity(0.3) : Color.black.opacity(0.025)))
        .clipShape(RoundedRectangle(cornerRadius: 9))
        .overlay(RoundedRectangle(cornerRadius: 9).stroke(p.border, lineWidth: 0.6))
    }
}

struct TableBlockView: View {
    let header: [String]
    let rows: [[String]]
    @Environment(\.palette) var p

    var body: some View {
        let n = max(header.count, rows.map(\.count).max() ?? 0)
        ScrollView(.horizontal, showsIndicators: false) {
            Grid(alignment: .leading, horizontalSpacing: 0, verticalSpacing: 0) {
                GridRow {
                    ForEach(0..<n, id: \.self) { c in
                        cell(c < header.count ? header[c] : "", bold: true)
                            .background(p.isDark ? Color.white.opacity(0.05) : Color.black.opacity(0.035))
                    }
                }
                ForEach(Array(rows.enumerated()), id: \.offset) { _, r in
                    Rectangle().fill(p.separator).frame(height: 1).gridCellUnsizedAxes(.horizontal)
                    GridRow {
                        ForEach(0..<n, id: \.self) { c in cell(c < r.count ? r[c] : "", bold: false) }
                    }
                }
            }
            .overlay(RoundedRectangle(cornerRadius: 7).stroke(p.border, lineWidth: 0.7))
            .clipShape(RoundedRectangle(cornerRadius: 7))
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    private func cell(_ s: String, bold: Bool) -> some View {
        Text(MD.inline(s, p: p, size: 12.5))
            .font(.system(size: 12.5, weight: bold ? .semibold : .regular))
            .fixedSize(horizontal: false, vertical: true)
            .frame(minWidth: 60, maxWidth: 320, alignment: .leading)
            .padding(.horizontal, 10).padding(.vertical, 6)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
    }
}
