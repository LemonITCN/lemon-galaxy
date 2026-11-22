import SwiftUI
import GalaxyKit

/// 一个对话页的输入框状态：文字、引用、模型、档位、先出计划；新对话发第一条消息时才建对话（docs/ai-chat-ui.md 第 3 节）。
@MainActor
final class ChatDraft: ObservableObject {
    @Published var text = ""
    @Published var refs: [ContextRef] = []
    @Published var modelKey = ""
    @Published var mode: AiMode = .confirmDangerous
    /// 完全信任的时限（分钟），nil 表示本对话
    @Published var trustMinutes: UInt32? = 60
    @Published var planFirst = false
    /// 建对话 / 换模型失败：显示在输入框上方
    @Published var error: String?
    @Published var creating = false
    @Published private(set) var controller: ConversationController?
    /// 用户点空白处关掉 @ 候选时的文字（文字变了再出现）
    @Published var mentionDismissedText: String?
    private var defaultsApplied = false

    init(controller: ConversationController?, refs: [ContextRef] = []) {
        self.controller = controller
        self.refs = refs
    }

    /// 输入框里的模型与档位：历史对话取对话保存的配置，新对话取本机记住的
    func applyDefaults(models: AiModels?, settings: AppSettings) {
        guard let models, !models.items.isEmpty else { return }
        if let c = controller {
            let s = c.state
            if models.items.contains(where: { $0.providerId == s.providerId && $0.id == s.modelId }) {
                modelKey = ModelChoice.key(providerId: s.providerId, modelId: s.modelId)
            } else if modelKey.isEmpty {
                modelKey = ModelChoice.key(providerId: s.providerId, modelId: s.modelId)
            }
            mode = s.mode
            planFirst = s.planFirst
            defaultsApplied = true
            return
        }
        if !models.items.contains(where: { ModelChoice.key($0) == modelKey }) {
            modelKey = ModelChoice.pick(models.items, remembered: settings.aiLastModel).map(ModelChoice.key) ?? ""
        }
        if !defaultsApplied {
            mode = AiMode.initial(saved: settings.defaultAiMode, maxMode: models.maxMode)
            trustMinutes = settings.aiTrustMinutes > 0 ? UInt32(settings.aiTrustMinutes) : nil
            planFirst = settings.aiPlanFirst
            defaultsApplied = true
        } else {
            mode = AiMode.clamp(mode, to: models.maxMode)
        }
    }

    func model(in models: AiModels?) -> AiModel? { models?.items.first { ModelChoice.key($0) == modelKey } }

    var mentionQuery: String? {
        guard mentionDismissedText != text else { return nil }
        return MentionParser.query(in: text)
    }

    var isEmpty: Bool { text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && refs.isEmpty }

    func addRefs(_ rs: [ContextRef]) { for r in rs { refs.addRef(r) } }

    func pickMention(_ r: ContextRef) {
        text = MentionParser.removeQuery(from: text)
        refs.addRef(r)
    }

    /// 发送：新对话先建对话再发；建失败时消息留在输入框里
    func send(ai: AgentHub, models: AiModels?) {
        let text = self.text.trimmingCharacters(in: .whitespacesAndNewlines)
        let refs = self.refs
        guard !text.isEmpty || !refs.isEmpty, !creating else { return }
        error = nil
        if let c = controller {
            c.send(text, refs: refs)
            self.text = ""
            self.refs = []
            return
        }
        guard let m = model(in: models) else { return }
        let config = ConversationConfig(providerId: m.providerId, modelId: m.id, targets: [], mode: mode, planFirst: planFirst,
                                        trustMinutes: mode == .fullTrust ? trustMinutes : nil, title: nil)
        creating = true
        Task {
            do {
                let c = try await ai.create(config)
                c.send(text, refs: refs)
                self.text = ""
                self.refs = []
                self.controller = c
                ai.refreshList()
            } catch {
                self.error = describeError(error)
            }
            creating = false
        }
    }

    func selectModel(_ m: AiModel, settings: AppSettings) {
        error = nil
        if let c = controller {
            do { try c.setModel(m) } catch {
                self.error = describeError(error)
                return
            }
        }
        modelKey = ModelChoice.key(m)
        settings.aiLastModel = modelKey
    }

    func selectMode(_ m: AiMode, trust: UInt32?, settings: AppSettings) {
        mode = m
        trustMinutes = trust
        settings.defaultAiMode = m
        if m == .fullTrust { settings.aiTrustMinutes = Int(trust ?? 0) }
        controller?.setMode(m, trustMinutes: m == .fullTrust ? trust : nil)
    }

    func setPlanFirst(_ on: Bool, settings: AppSettings) {
        planFirst = on
        settings.aiPlanFirst = on
    }
}

enum ModeStyle {
    static func icon(_ m: AiMode) -> String {
        switch m {
        case .off: return "nosign"
        case .readOnly: return "eye"
        case .confirmAll: return "hand.raised"
        case .confirmDangerous: return "exclamationmark.shield"
        case .fullTrust: return "bolt.shield"
        }
    }
}

@MainActor
enum RefStyle {
    static func icon(_ r: ContextRef, account: GalaxyAccount) -> String {
        switch r.kind {
        case .device: return account.device(r.id).map { Icons.platform($0.platform) } ?? "desktopcomputer"
        case .sshHost: return "terminal"
        case .file: return "doc"
        case .folder: return "folder"
        case .snippet: return "chevron.left.forwardslash.chevron.right"
        case .portForward: return "arrow.left.arrow.right"
        case .activity: return "clock.arrow.circlepath"
        }
    }
}

/// 引用标签：图标 + 名称（+ 删除叉）。文件与文件夹长按显示完整路径与所在设备。
struct RefChip: View {
    let ref: ContextRef
    var onRemove: (() -> Void)?
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var detail = false

    var body: some View {
        HStack(spacing: 5) {
            Image(systemName: RefStyle.icon(ref, account: account)).font(.caption2.weight(.medium)).foregroundStyle(p.accent)
            Text(ref.label).font(.caption.weight(.medium)).foregroundStyle(p.text).lineLimit(1).truncationMode(.middle)
            if let onRemove {
                Button(action: onRemove) {
                    Image(systemName: "xmark").font(.system(size: 9, weight: .bold)).foregroundStyle(p.tertiaryText)
                        .frame(width: 16, height: 16)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(loc("common.remove"))
            }
        }
        .padding(.leading, 8)
        .padding(.trailing, onRemove == nil ? 8 : 3)
        .padding(.vertical, 4)
        .background(Capsule().fill(p.card))
        .overlay(Capsule().stroke(p.border, lineWidth: 0.5))
        .onLongPressGesture { if ref.kind == .file || ref.kind == .folder { detail = true } }
        .popover(isPresented: $detail) {
            VStack(alignment: .leading, spacing: 6) {
                Text(ref.id).font(.system(.footnote, design: .monospaced)).foregroundStyle(p.text).textSelection(.enabled)
                Label(account.targetName(account.device(ref.targetId) != nil ? .device(ref.targetId) : .ssh(ref.targetId)),
                      systemImage: account.device(ref.targetId).map { Icons.platform($0.platform) } ?? "terminal")
                    .font(.caption)
                    .foregroundStyle(p.secondaryText)
            }
            .padding(14)
            .frame(maxWidth: 320, alignment: .leading)
            .modifier(CompactPopover())
        }
    }
}

/// 输入框：引用标签行、多行文本框、底部工具条（「+」、模型、档位、发送 / 停止）
struct ChatComposer: View {
    @ObservedObject var draft: ChatDraft
    /// 打开中的对话状态（新对话为 nil）
    let running: Bool
    let enabled: Bool
    let onSend: () -> Void
    let onStop: () -> Void
    @Binding var sheet: ComposerSheet?
    @EnvironmentObject var ai: AgentHub
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !draft.refs.isEmpty {
                FlowLayout(spacing: 6) {
                    ForEach(draft.refs, id: \.refKey) { r in
                        RefChip(ref: r) { draft.refs.removeAll { $0.refKey == r.refKey } }
                    }
                }
            }
            TextField(loc("ai.inputHint"), text: $draft.text, axis: .vertical)
                .lineLimit(1...8)
                .focused($focused)
                .disabled(!enabled)
                .padding(.horizontal, 4)
            HStack(spacing: 6) {
                Button { sheet = .plus } label: {
                    Image(systemName: "plus").font(.system(size: 15, weight: .semibold))
                        .frame(width: 30, height: 30)
                        .background(Circle().stroke(p.border, lineWidth: 1))
                }
                .buttonStyle(.plain)
                .foregroundStyle(p.secondaryText)
                .accessibilityLabel(loc("ai.addRef"))
                Button { sheet = .model } label: {
                    HStack(spacing: 3) {
                        Text(draft.model(in: ai.models)?.name ?? loc("ai.model")).lineLimit(1)
                        Image(systemName: "chevron.down").font(.system(size: 9, weight: .bold))
                    }
                    .font(.footnote.weight(.medium))
                    .padding(.horizontal, 8)
                    .frame(height: 30)
                }
                .buttonStyle(.plain)
                .foregroundStyle(p.secondaryText)
                Button { sheet = .mode } label: {
                    HStack(spacing: 4) {
                        Image(systemName: ModeStyle.icon(draft.mode)).font(.caption)
                        Text(loc("mode." + draft.mode.key)).lineLimit(1)
                    }
                    .font(.footnote.weight(.medium))
                    .padding(.horizontal, 8)
                    .frame(height: 30)
                }
                .buttonStyle(.plain)
                .foregroundStyle(draft.mode == .fullTrust ? p.warning : p.secondaryText)
                Spacer(minLength: 0)
                if running {
                    Button(action: onStop) {
                        Image(systemName: "stop.fill").font(.system(size: 12, weight: .bold))
                            .foregroundStyle(p.accentText)
                            .frame(width: 32, height: 32)
                            .background(Circle().fill(p.text))
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(loc("ai.stop"))
                } else {
                    let can = enabled && !draft.isEmpty && !draft.creating
                    Button(action: onSend) {
                        ZStack {
                            Circle().fill(can ? p.accent : p.border)
                            if draft.creating {
                                ProgressView().controlSize(.small).tint(p.accentText)
                            } else {
                                Image(systemName: "arrow.up").font(.system(size: 15, weight: .bold)).foregroundStyle(can ? p.accentText : p.tertiaryText)
                            }
                        }
                        .frame(width: 32, height: 32)
                    }
                    .buttonStyle(.plain)
                    .disabled(!can)
                    .accessibilityLabel(loc("ai.send"))
                }
            }
            .disabled(!enabled)
            .opacity(enabled ? 1 : 0.5)
        }
        .padding(.horizontal, 10)
        .padding(.top, 10)
        .padding(.bottom, 8)
        .background(RoundedRectangle(cornerRadius: 22, style: .continuous).fill(p.elevated))
        .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).stroke(p.border, lineWidth: 0.5))
    }
}

/// @ 候选：显示在输入框上方
struct MentionPanel: View {
    let query: String
    let pick: (ContextRef) -> Void
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        let list = TargetCandidate.all(account: account, query: query)
        VStack(spacing: 0) {
            if list.isEmpty {
                Text(loc("ai.noMatch")).font(.footnote).foregroundStyle(p.tertiaryText).padding(14).frame(maxWidth: .infinity)
            } else {
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(list) { c in
                            Button { pick(c.ref) } label: { TargetCandidateRow(c: c, selected: nil) }
                                .buttonStyle(.plain)
                        }
                    }
                }
                .frame(maxHeight: 240)
            }
        }
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).stroke(p.border, lineWidth: 0.5))
        .shadow(color: .black.opacity(0.08), radius: 10, y: 2)
    }
}

/// 设备与 SSH 主机候选（不含本机与手机）
struct TargetCandidate: Identifiable {
    let ref: ContextRef
    let symbol: String
    let detail: String
    let online: Bool
    var id: String { ref.refKey }

    @MainActor static func all(account: GalaxyAccount, query: String) -> [TargetCandidate] {
        let devices = account.devices.filter { $0.kind != .mobile && !$0.isSelf }.map { d in
            TargetCandidate(ref: .device(d.id, label: account.deviceName(d.id)), symbol: Icons.platform(d.platform),
                            detail: d.osVersion, online: d.online || d.lanSeen)
        }
        let ssh = account.hosts.sshHosts.map { e in
            TargetCandidate(ref: .sshHost(e.id, label: account.hosts.sshLabel(e.id)), symbol: "terminal", detail: HostItem.ssh(e).subtitle, online: true)
        }
        return MentionParser.filter(devices + ssh, query: query, name: \.ref.label, detail: \.detail, online: \.online)
    }
}

struct TargetCandidateRow: View {
    let c: TargetCandidate
    /// 多选时的勾选状态；nil 不显示勾选框
    let selected: Bool?
    var dimOffline = true
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        HStack(spacing: 12) {
            PlatformBadge(symbol: c.symbol, tint: c.online ? p.accent : p.offline, size: 30)
            VStack(alignment: .leading, spacing: 2) {
                Text(c.ref.label).foregroundStyle(p.text).lineLimit(1)
                HStack(spacing: 5) {
                    if c.ref.kind == .device {
                        StatusDot(color: c.online ? p.success : p.offline, size: 6)
                        Text(loc(c.online ? "status.online" : "status.offline"))
                    }
                    if !c.detail.isEmpty { Text(c.detail).lineLimit(1) }
                }
                .font(.caption)
                .foregroundStyle(p.secondaryText)
            }
            Spacer()
            if let selected {
                Image(systemName: selected ? "checkmark.circle.fill" : "circle")
                    .font(.title3)
                    .foregroundStyle(selected ? p.accent : p.tertiaryText)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .contentShape(Rectangle())
    }
}

enum ComposerSheet: String, Identifiable {
    case plus, devices, files, snippets, forwards, model, mode
    var id: String { rawValue }
}

// MARK: 模型与档位弹层

struct ModelSheet: View {
    @ObservedObject var draft: ChatDraft
    @EnvironmentObject var ai: AgentHub
    @EnvironmentObject var settings: AppSettings
    @Environment(\.dismiss) private var dismiss
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        NavigationStack {
            List {
                ForEach(ModelChoice.grouped(ai.models?.items ?? []), id: \.provider) { g in
                    Section(g.provider) {
                        ForEach(g.models, id: \.id) { m in
                            Button {
                                draft.selectModel(m, settings: settings)
                                dismiss()
                            } label: {
                                HStack(spacing: 8) {
                                    Text(m.name).foregroundStyle(p.text)
                                    if m.vision {
                                        Text(loc("ai.vision")).font(.caption2.weight(.semibold))
                                            .padding(.horizontal, 6).padding(.vertical, 2)
                                            .background(Capsule().fill(p.accent.opacity(0.14)))
                                            .foregroundStyle(p.accent)
                                    }
                                    Spacer()
                                    if ModelChoice.key(m) == draft.modelKey {
                                        Image(systemName: "checkmark").foregroundStyle(p.accent).font(.body.weight(.semibold))
                                    }
                                }
                            }
                        }
                    }
                    .galaxyRow()
                }
            }
            .galaxyList()
            .navigationTitle(loc("ai.model"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(loc("common.close")) { dismiss() } }
                ToolbarItem(placement: .primaryAction) { InfoButton(text: loc("ai.visionInfo")) }
            }
        }
        .presentationDetents([.medium, .large])
    }
}

struct ModeSheet: View {
    @ObservedObject var draft: ChatDraft
    @EnvironmentObject var ai: AgentHub
    @EnvironmentObject var settings: AppSettings
    @Environment(\.dismiss) private var dismiss
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    /// 选了完全信任，等选时限
    @State private var choosingTrust = false

    var body: some View {
        let maxMode = ai.models?.maxMode ?? .fullTrust
        NavigationStack {
            List {
                Section {
                    ForEach(AiMode.chatModes, id: \.self) { m in
                        let allowed = m.allowed(maxMode: maxMode)
                        Button {
                            if m == .fullTrust {
                                choosingTrust = true
                            } else {
                                draft.selectMode(m, trust: nil, settings: settings)
                                dismiss()
                            }
                        } label: {
                            HStack(spacing: 12) {
                                Image(systemName: ModeStyle.icon(m)).frame(width: 22).foregroundStyle(allowed ? p.accent : p.tertiaryText)
                                Text(loc("mode." + m.key)).foregroundStyle(allowed ? p.text : p.tertiaryText)
                                Spacer()
                                if draft.mode == m && !choosingTrust {
                                    Image(systemName: "checkmark").foregroundStyle(p.accent).font(.body.weight(.semibold))
                                }
                            }
                        }
                        .disabled(!allowed)
                    }
                } footer: {
                    HStack { Spacer(); InfoButton(text: loc("ai.modeInfo"), size: 13) }
                }
                .galaxyRow()
                if choosingTrust {
                    Section(loc("ai.trustFor")) {
                        ForEach([UInt32?.some(15), .some(60), .none], id: \.self) { t in
                            Button {
                                draft.selectMode(.fullTrust, trust: t, settings: settings)
                                dismiss()
                            } label: {
                                HStack {
                                    Text(loc(t == 15 ? "trust.15" : t == 60 ? "trust.60" : "trust.conversation")).foregroundStyle(p.text)
                                    Spacer()
                                    if draft.mode == .fullTrust && draft.trustMinutes == t {
                                        Image(systemName: "checkmark").foregroundStyle(p.accent).font(.body.weight(.semibold))
                                    }
                                }
                            }
                        }
                    }
                    .galaxyRow()
                }
                Section {
                    Toggle(isOn: Binding(get: { draft.planFirst }, set: { draft.setPlanFirst($0, settings: settings) })) {
                        HStack(spacing: 6) {
                            Text(loc("ai.planFirst"))
                            InfoButton(text: loc(draft.controller == nil ? "ai.planFirstInfo" : "ai.planFirstLocked"), size: 13)
                        }
                    }
                    .disabled(draft.controller != nil)
                }
                .galaxyRow()
            }
            .galaxyList()
            .navigationTitle(loc("ai.mode"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button(loc("common.close")) { dismiss() } } }
        }
        .presentationDetents([.medium, .large])
    }
}

/// 「+」菜单：设备或主机 / 文件或文件夹 / 命令片段 / 端口映射 / 连接记录
struct PlusSheet: View {
    let choose: (ComposerSheet?) -> Void
    let addActivity: () -> Void
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        NavigationStack {
            List {
                row("desktopcomputer", "ai.ref.devices") { choose(.devices) }
                row("folder", "ai.ref.files") { choose(.files) }
                row("chevron.left.forwardslash.chevron.right", "ai.ref.snippets") { choose(.snippets) }
                row("arrow.left.arrow.right", "ai.ref.forwards") { choose(.forwards) }
                row("clock.arrow.circlepath", "ai.ref.activity") { addActivity() }
            }
            .galaxyList()
            .navigationTitle(loc("ai.addRef"))
            .navigationBarTitleDisplayMode(.inline)
        }
        .presentationDetents([.medium])
    }

    private func row(_ icon: String, _ key: String, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 14) {
                Image(systemName: icon).frame(width: 24).foregroundStyle(p.accent)
                Text(loc(key)).foregroundStyle(p.text)
                Spacer()
                Image(systemName: "chevron.right").font(.caption.weight(.semibold)).foregroundStyle(p.tertiaryText)
            }
        }
        .galaxyRow()
    }
}
