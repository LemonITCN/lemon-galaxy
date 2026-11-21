import SwiftUI
import AppKit

// MARK: - 中间列：对话列表

struct AIConversationList: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var ai: AIHubModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(spacing: 0) {
            ColumnHeader(title: loc("nav.ai")) {
                IconButton(icon: "square.and.pencil", help: loc("ai.new")) { app.selectHome(.ai) }
            }
            Rectangle().fill(p.separator).frame(height: 1)
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 2) {
                    if !ai.conversations.isEmpty {
                        SectionLabel(text: loc("ai.open")).padding(.horizontal, 10).padding(.top, 8).padding(.bottom, 2)
                        ForEach(ai.conversations) { c in ConversationRow(conv: c) }
                    }
                    if !ai.summaries.isEmpty {
                        SectionLabel(text: loc("ai.history")).padding(.horizontal, 10).padding(.top, 12).padding(.bottom, 2)
                        ForEach(ai.summaries, id: \.id) { s in
                            Button { ai.reopen(s) } label: {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(s.title).font(.system(size: 12.5)).lineLimit(1)
                                    Text(Fmt.date(s.updatedAt, zh: loc.isChinese)).font(.system(size: 11)).foregroundStyle(p.tertiaryText)
                                }
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(.horizontal, 10).padding(.vertical, 6)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    if ai.conversations.isEmpty && ai.summaries.isEmpty {
                        Text(loc("ai.noConversations")).font(.system(size: 12)).foregroundStyle(p.tertiaryText).padding(14)
                    }
                }
                .padding(6)
            }
        }
        .onAppear { ai.loadModels(); ai.reloadSummaries() }
    }
}

struct ConversationRow: View {
    @ObservedObject var conv: AIConversationModel
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var ai: AIHubModel
    @Environment(\.palette) var p
    var body: some View {
        let selected = app.activeTabId == conv.id
        Button { ai.focus(conv) } label: {
            HStack(alignment: .top, spacing: 9) {
                Image(systemName: "bubble.left.and.text.bubble.right").font(.system(size: 12)).foregroundStyle(p.secondaryText).padding(.top, 2)
                VStack(alignment: .leading, spacing: 3) {
                    Text(conv.title).font(.system(size: 12.5, weight: .medium)).lineLimit(1)
                    HStack(spacing: 5) {
                        Text(conv.model.name).font(.system(size: 11)).foregroundStyle(p.tertiaryText)
                        Text("·").foregroundStyle(p.tertiaryText)
                        Text(conv.targetNames.joined(separator: ", ")).font(.system(size: 11)).foregroundStyle(p.tertiaryText).lineLimit(1)
                    }
                }
                Spacer(minLength: 0)
                if conv.running { ProgressView().controlSize(.mini) }
                else if conv.items.contains(where: { if case .tool(let t) = $0 { return t.decision == .pending }; return false }) {
                    Circle().fill(p.warning).frame(width: 7, height: 7).padding(.top, 4)
                }
            }
            .padding(.horizontal, 10).padding(.vertical, 7)
            .background(RoundedRectangle(cornerRadius: 7).fill(selected ? p.selection : .clear))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

// MARK: - 新建对话

struct NewConversationPage: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var ai: AIHubModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        PageScroll(maxWidth: 860) {
            VStack(alignment: .leading, spacing: 22) {
                HStack(spacing: 10) {
                    Text(loc("ai.new")).font(.system(size: 20, weight: .semibold))
                    InfoButton(text: loc("ai.new.info"))
                    Spacer()
                }
                if app.user?.aiEnabled == false || ai.models?.enabled == false {
                    Card { Label(loc("ai.disabled"), systemImage: "nosign").foregroundStyle(p.secondaryText) }
                } else {
                    modelSection
                    targetSection
                    modeSection
                    promptSection
                }
            }
        }
        .onAppear { ai.loadModels() }
    }

    private func title(_ t: String, info: String? = nil) -> some View {
        HStack(spacing: 5) {
            Text(t).font(.system(size: 13, weight: .semibold))
            if let info { InfoButton(text: info) }
        }
    }

    private var modelSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            title(loc("ai.model"), info: loc("ai.model.info"))
            if ai.loadingModels { ProgressView().controlSize(.small) }
            if let e = ai.modelsError { Text(e).font(.system(size: 12)).foregroundStyle(p.danger) }
            if let models = ai.models {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 250), spacing: 10)], spacing: 10) {
                    ForEach(models.items, id: \.id) { m in
                        let sel = ai.draftModel == ai.key(m)
                        Button { ai.draftModel = ai.key(m) } label: {
                            HStack(spacing: 10) {
                                Image(systemName: sel ? "largecircle.fill.circle" : "circle").foregroundStyle(sel ? p.accent : p.tertiaryText)
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(m.name).font(.system(size: 12.5, weight: .semibold))
                                    Text(verbatim: "\(m.providerName) · \(m.contextLength / 1000)K")
                                        .font(.system(size: 11)).foregroundStyle(p.tertiaryText).lineLimit(1)
                                }
                                Spacer()
                                if m.vision {
                                    Label(loc("ai.vision"), systemImage: "eye").font(.system(size: 10.5, weight: .medium))
                                        .foregroundStyle(p.accent).padding(.horizontal, 6).padding(.vertical, 2)
                                        .background(Capsule().fill(p.accent.opacity(0.12)))
                                }
                            }
                            .padding(10)
                            .background(RoundedRectangle(cornerRadius: 9).fill(p.card))
                            .overlay(RoundedRectangle(cornerRadius: 9).stroke(sel ? p.accent : p.border, lineWidth: sel ? 1.2 : 0.6))
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
                if let m = ai.selectedModel, !m.vision, ai.draftTargets.contains(where: { isGui($0) }) {
                    Label(loc("ai.noVisionHint"), systemImage: "info.circle").font(.system(size: 11.5)).foregroundStyle(p.warning)
                }
            }
        }
    }

    private func isGui(_ t: AgentTarget) -> Bool {
        guard t.kind == .device, let d = app.device(t.id) else { return false }
        return d.platform == .windows || d.platform == .macos
    }

    private var targetSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            title(loc("ai.targets"), info: loc("ai.targets.info"))
            let devs = app.devices.filter { !$0.isSelf && $0.kind != .mobile }
            let hosts = app.vault.filter { if case .sshHost = $0.data { return true }; return false }
            FlowLayout(spacing: 6) {
                ForEach(devs, id: \.id) { d in
                    chip(AgentTarget(kind: .device, id: d.id), name: app.deviceName(d.id), enabled: d.online || d.lanSeen) {
                        PlatformGlyph(platform: d.platform, size: 11)
                    }
                }
                ForEach(hosts, id: \.id) { e in
                    chip(AgentTarget(kind: .sshHost, id: e.id), name: app.sshLabel(e.id), enabled: app.sshSupported) {
                        Text("SSH").font(.system(size: 7.5, weight: .bold))
                    }
                }
            }
        }
    }

    private func chip<I: View>(_ t: AgentTarget, name: String, enabled: Bool, @ViewBuilder icon: () -> I) -> some View {
        let sel = ai.draftTargets.contains(t)
        return Button { ai.toggleTarget(t) } label: {
            HStack(spacing: 6) {
                icon().foregroundStyle(sel ? p.accentText : p.secondaryText)
                Text(name).font(.system(size: 12))
                if sel { Image(systemName: "checkmark").font(.system(size: 9, weight: .bold)) }
            }
            .foregroundStyle(sel ? p.accentText : (enabled ? p.text : p.tertiaryText))
            .padding(.horizontal, 10).frame(height: 26)
            .background(Capsule().fill(sel ? p.accent : p.card))
            .overlay(Capsule().stroke(sel ? .clear : p.border, lineWidth: 0.6))
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
    }

    private var modeSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            title(loc("ai.mode"), info: loc("ai.mode.info"))
            ModePicker(mode: $ai.draftMode, max: ai.models?.maxMode ?? .fullTrust)
            if ai.draftMode == .fullTrust {
                HStack(spacing: 8) {
                    Text(loc("ai.trust.limit")).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                    Picker("", selection: $ai.draftTrustMinutes) {
                        Text(loc("ai.trust.none")).tag(0)
                        Text(loc("ai.trust.minutes", 15)).tag(15)
                        Text(loc("ai.trust.hours", 1)).tag(60)
                        Text(loc("ai.trust.hours", 4)).tag(240)
                    }
                    .labelsHidden().fixedSize()
                }
            }
            ToggleRow(label: loc("ai.planFirst"), isOn: $ai.draftPlanFirst, info: loc("ai.planFirst.info")).frame(maxWidth: 360)
        }
    }

    private var promptSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            title(loc("ai.task"))
            ZStack(alignment: .topLeading) {
                TextEditor(text: $ai.draftPrompt)
                    .font(.system(size: 13))
                    .scrollContentBackground(.hidden)
                    .padding(8)
                if ai.draftPrompt.isEmpty {
                    Text(loc("ai.task.placeholder")).font(.system(size: 13)).foregroundStyle(p.tertiaryText).padding(.horizontal, 13).padding(.vertical, 8)
                        .allowsHitTesting(false)
                }
            }
            .frame(height: 110)
            .background(RoundedRectangle(cornerRadius: 9).fill(p.elevated))
            .overlay(RoundedRectangle(cornerRadius: 9).stroke(p.border, lineWidth: 0.7))
            HStack {
                Spacer()
                Button { ai.start() } label: { Label(loc("ai.start"), systemImage: "paperplane.fill") }
                    .buttonStyle(PrimaryButtonStyle())
                    .disabled(!ai.canStart)
                    .keyboardShortcut(.return, modifiers: .command)
            }
        }
    }
}

/// 权限档位分段选择；超过管理员上限的档位置灰
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
                        Image(systemName: icon(m)).font(.system(size: 13))
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

    private func icon(_ m: AiMode) -> String {
        switch m {
        case .readOnly: return "eye"
        case .confirmAll: return "hand.raised"
        case .confirmDangerous: return "exclamationmark.shield"
        case .fullTrust: return "bolt"
        case .off: return "nosign"
        }
    }
}

/// 简单的流式换行布局
struct FlowLayout: Layout {
    var spacing: CGFloat = 6
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? 600
        var x: CGFloat = 0, y: CGFloat = 0, rowH: CGFloat = 0
        for s in subviews {
            let sz = s.sizeThatFits(.unspecified)
            if x + sz.width > width, x > 0 { x = 0; y += rowH + spacing; rowH = 0 }
            x += sz.width + spacing
            rowH = max(rowH, sz.height)
        }
        return CGSize(width: width, height: y + rowH)
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var x = bounds.minX, y = bounds.minY, rowH: CGFloat = 0
        for s in subviews {
            let sz = s.sizeThatFits(.unspecified)
            if x + sz.width > bounds.maxX, x > bounds.minX { x = bounds.minX; y += rowH + spacing; rowH = 0 }
            s.place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(sz))
            x += sz.width + spacing
            rowH = max(rowH, sz.height)
        }
    }
}

// MARK: - 对话视图

struct AIConversationView: View {
    @ObservedObject var tab: AIConversationModel
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(spacing: 0) {
            header
            Rectangle().fill(p.separator).frame(height: 1)
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 14) {
                        ForEach(tab.items) { item in
                            ChatItemView(item: item, tab: tab).id(item.id)
                        }
                        if tab.running {
                            HStack(spacing: 8) {
                                ProgressView().controlSize(.small)
                                Text(loc("ai.working", tab.step)).font(.system(size: 12)).foregroundStyle(p.tertiaryText)
                            }
                            .id("running")
                        }
                    }
                    .padding(.horizontal, 24).padding(.vertical, 18)
                    .frame(maxWidth: 860, alignment: .leading)
                    .frame(maxWidth: .infinity)
                }
                .onChange(of: tab.items.count) { _ in
                    if let last = tab.items.last { withAnimation(.easeOut(duration: 0.2)) { proxy.scrollTo(last.id, anchor: .bottom) } }
                }
            }
            composer
        }
        .overlay {
            if let req = tab.activeSecret {
                ZStack {
                    Color.black.opacity(p.isDark ? 0.45 : 0.25)
                    SecretInputCard(req: req, tab: tab)
                        .frame(width: 400)
                        .background(RoundedRectangle(cornerRadius: 12).fill(p.card).shadow(color: .black.opacity(0.25), radius: 20, y: 6))
                        .overlay(RoundedRectangle(cornerRadius: 12).stroke(p.border, lineWidth: 0.5))
                }
            }
            if let img = tab.zoomImage, let ns = NSImage(data: img.data) {
                ZStack {
                    Color.black.opacity(0.8).onTapGesture { tab.zoomImage = nil }
                    Image(nsImage: ns).resizable().aspectRatio(contentMode: .fit).padding(30).onTapGesture { tab.zoomImage = nil }
                }
            }
        }
    }

    private var header: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 3) {
                Text(tab.title).font(.system(size: 13.5, weight: .semibold)).lineLimit(1)
                HStack(spacing: 6) {
                    Text(tab.model.name).font(.system(size: 11)).foregroundStyle(p.secondaryText)
                    if tab.model.vision { Image(systemName: "eye").font(.system(size: 9.5)).foregroundStyle(p.secondaryText) }
                    ForEach(tab.targetNames, id: \.self) { n in TagChip(text: n) }
                }
            }
            Spacer()
            Menu {
                ForEach(AiMode.selectable, id: \.self) { m in
                    if m == .fullTrust {
                        Menu(loc(m.labelKey)) {
                            Button(loc("ai.trust.none")) { tab.changeMode(.fullTrust, minutes: 0) }
                            Button(loc("ai.trust.minutes", 15)) { tab.changeMode(.fullTrust, minutes: 15) }
                            Button(loc("ai.trust.hours", 1)) { tab.changeMode(.fullTrust, minutes: 60) }
                        }
                    } else {
                        Button(loc(m.labelKey)) { tab.changeMode(m, minutes: 0) }
                    }
                }
            } label: {
                HStack(spacing: 5) {
                    Image(systemName: tab.mode == .fullTrust ? "bolt.fill" : "shield.lefthalf.filled").font(.system(size: 11))
                    Text(loc(tab.mode.labelKey) + (tab.mode == .fullTrust && tab.trustMinutes > 0 ? " · \(tab.trustMinutes) min" : ""))
                        .font(.system(size: 11.5, weight: .medium))
                }
                .foregroundStyle(tab.mode == .fullTrust ? p.danger : p.text)
            }
            .menuStyle(.borderlessButton).fixedSize()
            HStack(spacing: 4) {
                Image(systemName: "number").font(.system(size: 10))
                Text(loc("ai.tokens", Self.k(tab.promptTokens), Self.k(tab.completionTokens)))
            }
            .font(.system(size: 11).monospacedDigit()).foregroundStyle(p.tertiaryText)
            .help(loc("ai.tokens.help"))
            if tab.running {
                Button { tab.stop() } label: {
                    Label(loc("ai.stop"), systemImage: "stop.fill").font(.system(size: 12, weight: .medium))
                        .foregroundStyle(p.danger)
                        .padding(.horizontal, 10).frame(height: 26)
                        .background(RoundedRectangle(cornerRadius: 6).fill(p.danger.opacity(0.12)))
                }
                .buttonStyle(.plain)
                .keyboardShortcut(".", modifiers: .command)
            }
        }
        .padding(.horizontal, 16).frame(height: 52)
    }

    static func k(_ n: UInt64) -> String { n >= 1000 ? String(format: "%.1fk", Double(n) / 1000) : "\(n)" }

    private var composer: some View {
        HStack(alignment: .bottom, spacing: 8) {
            ZStack(alignment: .topLeading) {
                TextEditor(text: $tab.composer)
                    .font(.system(size: 13))
                    .scrollContentBackground(.hidden)
                    .frame(minHeight: 36, maxHeight: 110)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 6).padding(.vertical, 4)
                if tab.composer.isEmpty {
                    Text(loc("ai.composer.placeholder")).font(.system(size: 13)).foregroundStyle(p.tertiaryText)
                        .padding(.horizontal, 11).padding(.vertical, 4).allowsHitTesting(false)
                }
            }
            .background(RoundedRectangle(cornerRadius: 9).fill(p.elevated))
            .overlay(RoundedRectangle(cornerRadius: 9).stroke(p.border, lineWidth: 0.7))
            Button {
                let t = tab.composer
                tab.composer = ""
                tab.send(t)
            } label: {
                Image(systemName: "arrow.up").font(.system(size: 13, weight: .bold)).foregroundStyle(p.accentText)
                    .frame(width: 32, height: 32)
                    .background(Circle().fill(p.accent))
            }
            .buttonStyle(.plain)
            .disabled(tab.running || tab.composer.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            .opacity(tab.running || tab.composer.isEmpty ? 0.5 : 1)
            .keyboardShortcut(.return, modifiers: .command)
        }
        .padding(.horizontal, 24).padding(.vertical, 12)
        .frame(maxWidth: 860)
        .frame(maxWidth: .infinity)
        .background(p.content)
        .overlay(Rectangle().fill(p.separator).frame(height: 1), alignment: .top)
    }
}

struct ChatItemView: View {
    let item: ChatItem
    @ObservedObject var tab: AIConversationModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        switch item {
        case .user(_, let text):
            HStack {
                Spacer(minLength: 80)
                Text(text).font(.system(size: 13)).foregroundStyle(p.text).textSelection(.enabled)
                    .padding(.horizontal, 12).padding(.vertical, 8)
                    .background(RoundedRectangle(cornerRadius: 12).fill(p.accent.opacity(0.14)))
            }
        case .assistant(let id):
            Text(tab.texts[id] ?? "").font(.system(size: 13)).foregroundStyle(p.text).textSelection(.enabled)
                .lineSpacing(3).fixedSize(horizontal: false, vertical: true)
        case .thinking(let id):
            DisclosureGroup {
                Text(tab.texts[id] ?? "").font(.system(size: 12)).foregroundStyle(p.tertiaryText).italic()
            } label: {
                Label(loc("ai.thinking"), systemImage: "brain").font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
            }
        case .tool(let card):
            ToolCard(card: card, tab: tab)
        case .ask(let ask):
            AskCard(ask: ask, tab: tab)
        case .secret(let req):
            SecretRow(req: req, tab: tab)
        case .error(_, let text):
            Label(text, systemImage: "exclamationmark.octagon").font(.system(size: 12.5)).foregroundStyle(p.danger)
        }
    }
}

/// 工具调用卡片：工具、目标、命令、风险级别与原因、模型说明、输出、截图；需要确认时带操作按钮
struct ToolCard: View {
    @ObservedObject var card: ToolCardModel
    @ObservedObject var tab: AIConversationModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var risk: RiskLevel { card.call.risk }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            headerRow
            VStack(alignment: .leading, spacing: 10) {
                commandBlock
                if !card.call.modelReason.isEmpty {
                    HStack(alignment: .top, spacing: 6) {
                        Image(systemName: "text.bubble").font(.system(size: 11)).foregroundStyle(p.tertiaryText).padding(.top, 1)
                        Text(card.call.modelReason).font(.system(size: 12)).foregroundStyle(p.secondaryText)
                    }
                }
                if !card.call.riskReasons.isEmpty && (risk == .dangerous || risk == .unknown || risk == .forbidden || card.decision == .pending) {
                    VStack(alignment: .leading, spacing: 3) {
                        ForEach(card.call.riskReasons, id: \.self) { r in
                            HStack(alignment: .top, spacing: 6) {
                                Image(systemName: risk.icon).font(.system(size: 10)).foregroundStyle(risk.color(p)).padding(.top, 2)
                                Text(r).font(.system(size: 12)).foregroundStyle(p.text)
                            }
                        }
                    }
                }
                if let img = card.call.image { thumbnail(img) }
                if card.decision == .pending { approvalArea }
                statusRow
                if !card.output.isEmpty || card.result?.image != nil { outputArea }
            }
            .padding(12)
        }
        .background(RoundedRectangle(cornerRadius: 10).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(card.decision == .pending ? risk.color(p).opacity(0.7) : p.border, lineWidth: card.decision == .pending ? 1.2 : 0.6))
        .overlay(alignment: .leading) {
            RoundedRectangle(cornerRadius: 2).fill(risk.color(p)).frame(width: 3).padding(.vertical, 10)
        }
    }

    private var headerRow: some View {
        HStack(spacing: 8) {
            Image(systemName: toolIcon).font(.system(size: 12)).foregroundStyle(p.secondaryText)
            Text(loc("ai.tool." + card.call.tool)).font(.system(size: 12, weight: .semibold))
            Text(card.call.tool).font(.system(size: 11, design: .monospaced)).foregroundStyle(p.tertiaryText)
            Text("→").foregroundStyle(p.tertiaryText).font(.system(size: 11))
            Text(card.call.targetName).font(.system(size: 12)).foregroundStyle(p.secondaryText).lineLimit(1)
            Spacer()
            HStack(spacing: 4) {
                Image(systemName: risk.icon).font(.system(size: 9.5))
                Text(loc("ai.risk." + risk.key)).font(.system(size: 11, weight: .semibold))
            }
            .foregroundStyle(risk.color(p))
            .padding(.horizontal, 7).padding(.vertical, 2.5)
            .background(Capsule().fill(risk.color(p).opacity(0.13)))
        }
        .padding(.horizontal, 12).padding(.top, 10)
    }

    private var toolIcon: String {
        switch card.call.tool {
        case "run_command": return "terminal"
        case "screenshot": return "camera.viewfinder"
        case "mouse": return "cursorarrow.click"
        case "keyboard": return "keyboard"
        case "read_file", "list_dir", "search_files": return "doc.text.magnifyingglass"
        case "write_file", "edit_file": return "square.and.pencil"
        case "ui_tree": return "list.bullet.indent"
        default: return "wrench.and.screwdriver"
        }
    }

    private var commandBlock: some View {
        Text(card.decision.editedCommand ?? card.call.summary)
            .font(.system(size: 12, design: .monospaced))
            .foregroundStyle(p.text)
            .textSelection(.enabled)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 10).padding(.vertical, 8)
            .background(RoundedRectangle(cornerRadius: 7).fill(p.isDark ? Color.black.opacity(0.25) : Color.black.opacity(0.04)))
    }

    private func thumbnail(_ img: ImageData) -> some View {
        Group {
            if let ns = NSImage(data: img.data) {
                Image(nsImage: ns).resizable().aspectRatio(contentMode: .fit)
                    .frame(maxWidth: 320, maxHeight: 180, alignment: .leading)
                    .clipShape(RoundedRectangle(cornerRadius: 6))
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(p.border, lineWidth: 0.5))
                    .onTapGesture { tab.zoomImage = img }
                    .help(loc("ai.enlarge"))
            }
        }
    }

    @ViewBuilder
    private var approvalArea: some View {
        if card.editing {
            VStack(alignment: .leading, spacing: 8) {
                TextField("", text: $card.editText, axis: .vertical).fieldStyle().font(.system(size: 12, design: .monospaced))
                HStack {
                    Spacer()
                    Button(loc("common.cancel")) { card.editing = false }.buttonStyle(SecondaryButtonStyle())
                    Button(loc("ai.approve.runEdited")) { tab.decide(card, .editAndRun(command: card.editText)) }.buttonStyle(PrimaryButtonStyle())
                }
            }
        } else if card.denying {
            VStack(alignment: .leading, spacing: 8) {
                TextField(loc("ai.deny.reason"), text: $card.denyReason).fieldStyle()
                HStack {
                    Spacer()
                    Button(loc("common.cancel")) { card.denying = false }.buttonStyle(SecondaryButtonStyle())
                    Button(loc("ai.approve.deny")) {
                        tab.decide(card, .deny(reason: card.denyReason.isEmpty ? nil : card.denyReason))
                    }
                    .buttonStyle(PrimaryButtonStyle(destructive: true))
                }
            }
        } else {
            HStack(spacing: 8) {
                Button(loc("ai.approve.once")) { tab.decide(card, .allowOnce) }.buttonStyle(PrimaryButtonStyle())
                Button(loc("ai.approve.similar")) { tab.decide(card, .allowSimilar) }.buttonStyle(SecondaryButtonStyle())
                if card.call.tool == "run_command" {
                    Button(loc("ai.approve.edit")) { card.editing = true }.buttonStyle(SecondaryButtonStyle())
                }
                Spacer()
                Button(loc("ai.approve.deny")) { card.denying = true }.buttonStyle(SecondaryButtonStyle(destructive: true))
            }
        }
    }

    @ViewBuilder
    private var statusRow: some View {
        HStack(spacing: 6) {
            switch card.decision {
            case .approvedSimilar: Label(loc("ai.decision.similar"), systemImage: "checkmark.circle").foregroundStyle(p.tertiaryText)
            case .edited: Label(loc("ai.decision.edited"), systemImage: "pencil.circle").foregroundStyle(p.tertiaryText)
            case .denied(let r): Label(loc("ai.decision.denied") + (r.map { (loc.isChinese ? "：" : ": ") + $0 } ?? ""), systemImage: "xmark.circle").foregroundStyle(p.danger)
            default: EmptyView()
            }
            Spacer()
            switch card.phase {
            case .running:
                ProgressView().controlSize(.mini)
                Text(loc("ai.tool.running")).foregroundStyle(p.tertiaryText)
            case .finished:
                if let r = card.result, !r.denied {
                    Image(systemName: r.ok ? "checkmark.circle.fill" : "xmark.octagon.fill").foregroundStyle(r.ok ? p.success : p.danger)
                    Text(r.summary).foregroundStyle(p.secondaryText)
                } else if let r = card.result, r.denied, case .auto = card.decision {
                    Image(systemName: "nosign").foregroundStyle(p.danger)
                    Text(r.summary).foregroundStyle(p.danger)
                } else if let r = card.result, r.denied, case .denied = card.decision {
                    EmptyView()
                } else if let r = card.result {
                    Text(r.summary).foregroundStyle(p.secondaryText)
                }
            case .proposed:
                if card.decision == .pending {
                    Image(systemName: "hourglass").foregroundStyle(risk.color(p))
                    Text(loc("ai.waitingApproval")).foregroundStyle(risk.color(p))
                }
            }
        }
        .font(.system(size: 11.5))
    }

    private var outputArea: some View {
        VStack(alignment: .leading, spacing: 6) {
            if !card.output.isEmpty {
                Button { card.expanded.toggle() } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "chevron.right").font(.system(size: 8.5, weight: .bold)).rotationEffect(.degrees(card.expanded ? 90 : 0))
                        Text(loc("ai.output", card.output.trimmingCharacters(in: .newlines).split(separator: "\n").count)).font(.system(size: 11.5))
                    }
                    .foregroundStyle(p.secondaryText)
                }
                .buttonStyle(.plain)
                let trimmed = card.output.trimmingCharacters(in: .newlines)
                let lines = trimmed.split(separator: "\n", omittingEmptySubsequences: false)
                let shown = card.expanded ? trimmed : lines.suffix(card.phase == .running ? 4 : 3).joined(separator: "\n")
                Text(shown)
                    .font(.system(size: 11.5, design: .monospaced))
                    .foregroundStyle(p.secondaryText)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(8)
                    .background(RoundedRectangle(cornerRadius: 6).fill(p.isDark ? Color.black.opacity(0.25) : Color.black.opacity(0.035)))
            }
            if let img = card.result?.image { thumbnail(img) }
        }
    }
}

extension ToolCardModel.Decision {
    var editedCommand: String? { if case .edited(let c) = self { return c }; return nil }
}

struct AskCard: View {
    @ObservedObject var ask: AskModel
    @ObservedObject var tab: AIConversationModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 6) {
                Image(systemName: "questionmark.bubble").foregroundStyle(p.accent)
                Text(ask.question).font(.system(size: 13, weight: .medium))
            }
            if let a = ask.answer {
                Label(a, systemImage: "arrowshape.turn.up.left").font(.system(size: 12)).foregroundStyle(p.secondaryText)
            } else {
                HStack(spacing: 8) {
                    ForEach(ask.options, id: \.self) { o in
                        Button(o) { tab.answer(ask, o) }.buttonStyle(o == ask.options.first ? AnyButtonStyle(PrimaryButtonStyle()) : AnyButtonStyle(SecondaryButtonStyle()))
                    }
                    TextField(loc("ai.ask.other"), text: $ask.freeText).fieldStyle().frame(maxWidth: 220)
                        .onSubmit { if !ask.freeText.isEmpty { tab.answer(ask, ask.freeText) } }
                }
            }
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 10).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(ask.answer == nil ? p.accent.opacity(0.6) : p.border, lineWidth: ask.answer == nil ? 1.2 : 0.6))
    }
}

struct AnyButtonStyle: ButtonStyle {
    private let make: (Configuration) -> AnyView
    init<S: ButtonStyle>(_ s: S) { make = { AnyView(s.makeBody(configuration: $0)) } }
    func makeBody(configuration: Configuration) -> some View { make(configuration) }
}

struct SecretRow: View {
    @ObservedObject var req: SecretRequestModel
    @ObservedObject var tab: AIConversationModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "key.horizontal").foregroundStyle(p.warning)
            Text(req.prompt).font(.system(size: 12.5))
            Spacer()
            if req.done {
                Label(loc("ai.secret.sent"), systemImage: "checkmark").font(.system(size: 11.5)).foregroundStyle(p.success)
            } else if req.cancelled {
                Text(loc("ai.secret.cancelled")).font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
            } else {
                Button(loc("ai.secret.enter")) { tab.activeSecret = req }.buttonStyle(PrimaryButtonStyle())
            }
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 10).fill(p.warning.opacity(0.08)))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(p.warning.opacity(0.4), lineWidth: 0.8))
    }
}

/// request_secret 的安全输入框：内容只交给 provide_secret，大模型看不到
struct SecretInputCard: View {
    @ObservedObject var req: SecretRequestModel
    @ObservedObject var tab: AIConversationModel
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var secret = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 8) {
                Image(systemName: "lock.shield").font(.system(size: 18)).foregroundStyle(p.accent)
                Text(loc("ai.secret.title")).font(.system(size: 14, weight: .semibold))
                InfoButton(text: loc("ai.secret.info"))
            }
            Text(req.prompt).font(.system(size: 12.5)).foregroundStyle(p.secondaryText).fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 6) {
                Image(systemName: "arrow.right.circle").font(.system(size: 11)).foregroundStyle(p.tertiaryText)
                Text(loc("ai.secret.target", app.ai.targetName(AgentTarget(kind: app.device(req.targetId) != nil ? .device : .sshHost, id: req.targetId))))
                    .font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
            }
            SecureField("", text: $secret).fieldStyle().onSubmit(submit)
            HStack {
                Spacer()
                Button(loc("common.cancel")) { tab.cancelSecret(req); secret = "" }.buttonStyle(SecondaryButtonStyle())
                Button(loc("ai.secret.send"), action: submit).buttonStyle(PrimaryButtonStyle()).disabled(secret.isEmpty).keyboardShortcut(.defaultAction)
            }
        }
        .padding(18)
    }

    private func submit() {
        guard !secret.isEmpty else { return }
        tab.provideSecret(req, secret)
        secret = ""
    }
}
