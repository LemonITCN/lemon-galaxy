import SwiftUI
import GalaxyKit

/// AI 页的导航目标：新对话（可带预先放进输入框的引用）或已有对话
enum AIRoute: Hashable {
    case new(UUID, [ContextRef])
    case open(String)
}

/// AI 页（手机端）：对话列表一页，点进去是对话页（docs/ai-chat-ui.md 第 1、6 节）。
struct AIView: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var ai: AgentHub
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var path: [AIRoute] = []
    @State private var deleting: ConversationListItem?
    @State private var opening: String?
    @State private var search = ""

    private var availability: AiAvailability {
        AiAvailability.of(models: ai.models, error: ai.modelsError, loading: ai.loadingModels, userAiEnabled: account.user?.aiEnabled)
    }

    var body: some View {
        NavigationStack(path: $path) {
            List {
                if availability != .ready {
                    AiAvailabilityView(availability: availability, compact: !ai.items.isEmpty)
                        .frame(maxWidth: .infinity)
                        .listRowBackground(Color.clear)
                }
                let filtered = ai.items.filter { search.isEmpty || title($0).localizedCaseInsensitiveContains(search) }
                ForEach(ConversationGroup.group(filtered, updatedAt: \.updatedAt), id: \.0) { group, rows in
                    Section(loc(group.titleKey)) {
                        ForEach(rows) { c in
                            Button { open(c.id) } label: { ConversationRow(item: c, opening: opening == c.id) }
                                .galaxyRow()
                                .contextMenu {
                                    Button(role: .destructive) { deleting = c } label: { Label(loc("common.delete"), systemImage: "trash") }
                                }
                        }
                    }
                }
                if availability == .ready && ai.items.isEmpty {
                    EmptyState(icon: "bubble.left.and.bubble.right", title: loc("ai.empty")).listRowBackground(Color.clear)
                } else if !search.isEmpty && filtered.isEmpty {
                    EmptyState(icon: "magnifyingglass", title: loc("ai.noMatch")).listRowBackground(Color.clear)
                }
            }
            .listStyle(.insetGrouped)
            .galaxyList()
            .searchable(text: $search, placement: .navigationBarDrawer(displayMode: .automatic), prompt: loc("ai.search"))
            .refreshable { await ai.loadModels() }
            .navigationTitle(loc("nav.ai"))
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button { path = [.new(UUID(), [])] } label: { Image(systemName: "square.and.pencil") }
                        .accessibilityLabel(loc("ai.new"))
                }
                ToolbarItem(placement: .navigationBarLeading) { InfoButton(text: loc("ai.backgroundInfo")) }
            }
            .navigationDestination(for: AIRoute.self) { route in
                switch route {
                case .new(_, let refs):
                    ChatView(controller: nil, refs: refs)
                case .open(let id):
                    if let c = ai.opened(id) {
                        ChatView(controller: c)
                    } else {
                        ProgressView()
                    }
                }
            }
        }
        .alert(loc("ai.delete"), isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), presenting: deleting) { c in
            Button(loc("common.delete"), role: .destructive) { ai.delete(c.id) }
            Button(loc("common.cancel"), role: .cancel) {}
        } message: { _ in Text(loc("ai.deleteMessage")) }
        // 进入 AI 页面就取模型列表（不可用时按第 5 节提示）
        .task { await ai.loadModels() }
        .onChange(of: app.openConversationId) { id in
            if let id { open(id); app.openConversationId = nil }
        }
        .onChange(of: app.newChatRefs) { refs in
            if let refs {
                path = [.new(UUID(), refs)]
                app.newChatRefs = nil
            }
        }
        .onAppear {
            if let refs = app.newChatRefs {
                path = [.new(UUID(), refs)]
                app.newChatRefs = nil
            }
        }
    }

    private func title(_ c: ConversationListItem) -> String { c.title.isEmpty ? loc("ai.untitled") : c.title }

    private func open(_ id: String) {
        if ai.opened(id) != nil { path = [.open(id)]; return }
        opening = id
        Task {
            do {
                _ = try await ai.open(id)
                path = [.open(id)]
            } catch { app.showError(error) }
            opening = nil
        }
    }
}

/// 对话列表的一行：标题 + 时间；正在进行任务的在行尾显示进行中标记
struct ConversationRow: View {
    let item: ConversationListItem
    var opening = false
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 3) {
                Text(item.title.isEmpty ? loc("ai.untitled") : item.title)
                    .font(.body)
                    .foregroundStyle(p.text)
                    .lineLimit(1)
                Text(Fmt.relative(item.updatedAt)).font(.caption).foregroundStyle(p.tertiaryText)
            }
            Spacer(minLength: 8)
            if opening || item.running {
                ProgressView().controlSize(.small)
            } else if item.waitingForUser {
                Image(systemName: "hand.raised.fill").font(.caption).foregroundStyle(p.warning)
                    .accessibilityLabel(loc("ai.waiting"))
            }
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
    }
}
