import SwiftUI
import GalaxyKit

/// AI 页：对话列表 + 新建对话 + 打开对话。
struct AIView: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var ai: AgentHub
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var path: [String] = []
    @State private var deleting: ConversationListItem?
    @State private var opening: String?

    var body: some View {
        NavigationStack(path: $path) {
            List {
                if account.user?.aiEnabled == false || ai.models?.enabled == false {
                    EmptyState(icon: "sparkles", title: loc("ai.disabled")).listRowBackground(Color.clear)
                } else {
                    if let e = ai.modelsError {
                        Section {
                            Label(e, systemImage: "exclamationmark.triangle").foregroundStyle(p.danger)
                            Button(loc("common.retry")) { Task { await ai.loadModels() } }
                        }
                        .galaxyRow()
                    }
                    if ai.items.isEmpty {
                        VStack(spacing: 16) {
                            EmptyState(icon: "sparkles", title: loc("ai.empty"))
                            Button { app.newConversationTargets = [] } label: { Label(loc("ai.new"), systemImage: "plus") }
                                .buttonStyle(.borderedProminent)
                        }
                        .frame(maxWidth: .infinity)
                        .listRowBackground(Color.clear)
                    }
                    ForEach(ai.items) { c in
                        Button { open(c.id) } label: { ConversationRow(item: c, opening: opening == c.id) }
                            .galaxyRow()
                            .swipeActions {
                                Button(role: .destructive) { deleting = c } label: { Label(loc("common.delete"), systemImage: "trash") }
                            }
                    }
                }
            }
            .listStyle(.insetGrouped)
            .galaxyList()
            .refreshable { await ai.loadModels() }
            .navigationTitle(loc("nav.ai"))
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button { app.newConversationTargets = [] } label: { Image(systemName: "square.and.pencil") }
                        .disabled(ai.models == nil || ai.models?.enabled == false || ai.models?.items.isEmpty == true)
                        .accessibilityLabel(loc("ai.new"))
                }
                ToolbarItem(placement: .navigationBarLeading) { InfoButton(text: loc("ai.backgroundInfo")) }
            }
            .navigationDestination(for: String.self) { id in
                if let c = ai.opened(id) {
                    ConversationView(controller: c)
                } else {
                    ProgressView()
                }
            }
        }
        .sheet(item: Binding(get: { app.newConversationTargets.map { TargetsBox(targets: $0) } }, set: { if $0 == nil { app.newConversationTargets = nil } })) { box in
            NewConversationSheet(initialTargets: box.targets) { c in
                app.newConversationTargets = nil
                path = [c.id]
            }
            .environmentObject(app)
            .environmentObject(ai)
            .environmentObject(account)
        }
        .alert(loc("ai.delete"), isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), presenting: deleting) { c in
            Button(loc("common.delete"), role: .destructive) { ai.delete(c.id) }
            Button(loc("common.cancel"), role: .cancel) {}
        } message: { _ in Text(loc("ai.deleteMessage")) }
        .task { if ai.models == nil { await ai.loadModels() } else { ai.refreshList() } }
        .onChange(of: app.openConversationId) { id in
            if let id { open(id); app.openConversationId = nil }
        }
    }

    private func open(_ id: String) {
        if ai.opened(id) != nil { path = [id]; return }
        opening = id
        Task {
            do {
                _ = try await ai.open(id)
                path = [id]
            } catch { app.showError(error) }
            opening = nil
        }
    }
}

private struct TargetsBox: Identifiable {
    let targets: [AgentTarget]
    var id: String { "new:" + targets.map { "\($0.kind)-\($0.id)" }.joined(separator: ",") }
}

struct ConversationRow: View {
    let item: ConversationListItem
    var opening = false
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    var body: some View {
        HStack(spacing: 12) {
            ZStack {
                Circle().fill(p.accent.opacity(0.14)).frame(width: 36, height: 36)
                if opening || item.running {
                    ProgressView().controlSize(.small)
                } else {
                    Image(systemName: "sparkles").foregroundStyle(p.accent)
                }
            }
            VStack(alignment: .leading, spacing: 3) {
                HStack {
                    Text(item.title.isEmpty ? loc("ai.untitled") : item.title).font(.body.weight(.medium)).foregroundStyle(p.text).lineLimit(1)
                    if item.waitingForUser {
                        Text(loc("ai.waiting")).font(.caption2.weight(.semibold)).padding(.horizontal, 6).padding(.vertical, 2)
                            .background(Capsule().fill(p.warning.opacity(0.18))).foregroundStyle(p.warning)
                    }
                }
                Text(([item.modelName] + item.targetNames).joined(separator: " · ")).font(.caption).foregroundStyle(p.secondaryText).lineLimit(1)
            }
            Spacer()
            Text(Fmt.relative(item.updatedAt)).font(.caption2).foregroundStyle(p.tertiaryText)
        }
        .padding(.vertical, 2)
    }
}

/// 新建对话：模型、目标设备、权限档位、先出计划。
struct NewConversationSheet: View {
    let initialTargets: [AgentTarget]
    let created: (ConversationController) -> Void
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var ai: AgentHub
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.dismiss) private var dismiss
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var modelKey = ""
    @State private var targets: Set<String> = []
    @State private var mode: AiMode = .confirmDangerous
    @State private var trust: UInt32? = 60
    @State private var planFirst = false
    @State private var title = ""
    @State private var creating = false
    @State private var error: String?

    private func key(_ m: AiModel) -> String { "\(m.providerId)/\(m.id)" }
    private var model: AiModel? { ai.models?.items.first { key($0) == modelKey } }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Picker(loc("ai.model"), selection: $modelKey) {
                        ForEach(groupedModels, id: \.0) { provider, models in
                            Section(provider) {
                                ForEach(models, id: \.id) { m in
                                    Text(m.name).tag(key(m))
                                }
                            }
                        }
                    }
                    if let m = model {
                        HStack(spacing: 8) {
                            Label(m.vision ? loc("ai.vision") : loc("ai.textOnly"), systemImage: m.vision ? "eye" : "text.alignleft")
                            Text("·")
                            Text(loc("ai.context", m.contextLength / 1000))
                            Spacer()
                            if !m.vision { InfoButton(text: loc("ai.visionInfo"), size: 13) }
                        }
                        .font(.caption)
                        .foregroundStyle(p.secondaryText)
                    }
                }
                .galaxyRow()
                Section {
                    let devices = account.devices.filter { $0.kind != .mobile && !$0.isSelf }
                    ForEach(devices, id: \.id) { d in
                        targetRow(id: "device:" + d.id, title: account.deviceName(d.id), subtitle: d.osVersion,
                                  symbol: Icons.platform(d.platform), enabled: d.online || d.lanSeen)
                    }
                    ForEach(account.hosts.sshHosts, id: \.id) { e in
                        targetRow(id: "ssh:" + e.id, title: account.hosts.sshLabel(e.id), subtitle: HostItem.ssh(e).subtitle, symbol: "terminal", enabled: true)
                    }
                } header: {
                    Text(loc("ai.targets"))
                } footer: {
                    HStack { Spacer(); InfoButton(text: loc("ai.targetsInfo"), size: 13) }
                }
                .galaxyRow()
                Section {
                    Picker(loc("ai.mode"), selection: $mode) {
                        ForEach(AiMode.selectable(maxMode: ai.models?.maxMode ?? .fullTrust), id: \.self) { m in
                            Text(loc("mode." + m.key)).tag(m)
                        }
                    }
                    if mode == .fullTrust {
                        Picker(loc("ai.trustFor"), selection: $trust) {
                            Text(loc("trust.15")).tag(UInt32?.some(15))
                            Text(loc("trust.60")).tag(UInt32?.some(60))
                            Text(loc("trust.unlimited")).tag(UInt32?.none)
                        }
                    }
                    Toggle(loc("ai.planFirst"), isOn: $planFirst)
                } footer: {
                    HStack(spacing: 10) {
                        Spacer()
                        InfoButton(text: loc("ai.modeInfo"), size: 13)
                        InfoButton(text: loc("ai.planFirstInfo"), size: 13)
                    }
                }
                .galaxyRow()
                Section {
                    TextField(loc("ai.titleOptional"), text: $title)
                }
                .galaxyRow()
                if let error {
                    Section { Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(p.danger) }.galaxyRow()
                }
            }
            .galaxyList()
            .navigationTitle(loc("ai.new"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(loc("common.cancel")) { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if creating { ProgressView() } else {
                        Button(loc("ai.start"), action: start).disabled(model == nil || targets.isEmpty).fontWeight(.semibold)
                    }
                }
            }
            .onAppear {
                if modelKey.isEmpty, let m = ai.models?.items.first(where: \.tools) ?? ai.models?.items.first { modelKey = key(m) }
                mode = AiMode.clamp(app.settings.defaultAiMode, to: ai.models?.maxMode ?? .fullTrust)
                for t in initialTargets { targets.insert((t.kind == .device ? "device:" : "ssh:") + t.id) }
            }
        }
    }

    private var groupedModels: [(String, [AiModel])] {
        var order: [String] = []
        var map: [String: [AiModel]] = [:]
        for m in ai.models?.items ?? [] {
            if map[m.providerName] == nil { order.append(m.providerName) }
            map[m.providerName, default: []].append(m)
        }
        return order.map { ($0, map[$0]!) }
    }

    private func targetRow(id: String, title: String, subtitle: String, symbol: String, enabled: Bool) -> some View {
        Button {
            if targets.contains(id) { targets.remove(id) } else { targets.insert(id) }
        } label: {
            HStack(spacing: 12) {
                PlatformBadge(symbol: symbol, tint: enabled ? p.accent : p.offline, size: 28)
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).foregroundStyle(p.text)
                    Text(enabled ? subtitle : loc("status.offline")).font(.caption).foregroundStyle(p.secondaryText).lineLimit(1)
                }
                Spacer()
                Image(systemName: targets.contains(id) ? "checkmark.circle.fill" : "circle")
                    .foregroundStyle(targets.contains(id) ? p.accent : p.tertiaryText)
                    .font(.title3)
            }
        }
        .disabled(!enabled)
    }

    private func start() {
        guard let m = model else { return }
        let ts: [AgentTarget] = targets.sorted().map { s in
            s.hasPrefix("device:") ? AgentTarget(kind: .device, id: String(s.dropFirst(7))) : AgentTarget(kind: .sshHost, id: String(s.dropFirst(4)))
        }
        let config = ConversationConfig(providerId: m.providerId, modelId: m.id, targets: ts, mode: mode, planFirst: planFirst,
                                        trustMinutes: mode == .fullTrust ? trust : nil,
                                        title: title.trimmingCharacters(in: .whitespaces).isEmpty ? nil : title)
        creating = true
        error = nil
        Task {
            do {
                let c = try await ai.create(config)
                dismiss()
                created(c)
            } catch {
                self.error = describeError(error)
            }
            creating = false
        }
    }
}
