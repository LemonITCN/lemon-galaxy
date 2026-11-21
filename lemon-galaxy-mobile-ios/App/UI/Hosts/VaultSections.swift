import SwiftUI
import GalaxyKit

struct CredentialsSection: View {
    let query: String
    @Binding var editor: EditorTarget?
    @Binding var confirm: HostsView.ConfirmAction?
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        let list = account.hosts.credentials.filter { e in
            guard !query.isEmpty, case .credential(let label, let user, _, _, _) = e.data else { return true }
            return (label + " " + user).lowercased().contains(query.lowercased())
        }
        if list.isEmpty {
            EmptyState(icon: "key", title: loc("credentials.empty")).listRowBackground(Color.clear)
        } else {
            Section {
                ForEach(list, id: \.id) { e in
                    if case .credential(let label, let user, let pw, let key, _) = e.data {
                        HStack(spacing: 12) {
                            PlatformBadge(symbol: key != nil ? "key.horizontal" : "lock", tint: p.warning)
                            VStack(alignment: .leading, spacing: 3) {
                                Text(label).font(.body.weight(.medium)).foregroundStyle(p.text)
                                Text([user.isEmpty ? nil : user, key != nil ? loc("credential.hasKey") : nil, pw != nil ? loc("credential.hasPassword") : nil]
                                    .compactMap { $0 }.joined(separator: " · "))
                                    .font(.caption).foregroundStyle(p.secondaryText)
                            }
                        }
                        .contentShape(Rectangle())
                        .onTapGesture { editor = .credential(e) }
                        .swipeActions {
                            Button(role: .destructive) {
                                confirm = .init(title: loc("common.delete"), message: loc("vault.deleteMessage", label), button: loc("common.delete")) {
                                    Task { do { try await account.delete(e.id) } catch { app.showError(error) } }
                                }
                            } label: { Label(loc("common.delete"), systemImage: "trash") }
                        }
                        .galaxyRow()
                    }
                }
            } footer: {
                HStack { Spacer(); InfoButton(text: loc("credentials.info"), size: 13) }
            }
        }
    }
}

struct SnippetsSection: View {
    let query: String
    @Binding var editor: EditorTarget?
    @Binding var confirm: HostsView.ConfirmAction?
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        let list = account.hosts.snippets.filter { e in
            guard !query.isEmpty, case .snippet(let label, let cmd, let tags) = e.data else { return true }
            return ([label, cmd] + tags).joined(separator: " ").lowercased().contains(query.lowercased())
        }
        if list.isEmpty {
            EmptyState(icon: "chevron.left.forwardslash.chevron.right", title: loc("snippets.empty")).listRowBackground(Color.clear)
        } else {
            Section {
                ForEach(list, id: \.id) { e in
                    if case .snippet(let label, let cmd, let tags) = e.data {
                        VStack(alignment: .leading, spacing: 4) {
                            HStack {
                                Text(label).font(.body.weight(.medium)).foregroundStyle(p.text)
                                Spacer()
                                if !tags.isEmpty { Text(tags.joined(separator: " · ")).font(.caption2).foregroundStyle(p.tertiaryText) }
                            }
                            Text(cmd).font(.caption.monospaced()).foregroundStyle(p.secondaryText).lineLimit(3)
                        }
                        .padding(.vertical, 2)
                        .contentShape(Rectangle())
                        .onTapGesture { editor = .snippet(e) }
                        .contextMenu {
                            Button { UIPasteboard.general.string = cmd; app.showToast(loc("common.copied")) } label: { Label(loc("common.copy"), systemImage: "doc.on.doc") }
                            Button { editor = .snippet(e) } label: { Label(loc("common.edit"), systemImage: "pencil") }
                        }
                        .swipeActions {
                            Button(role: .destructive) {
                                confirm = .init(title: loc("common.delete"), message: loc("vault.deleteMessage", label), button: loc("common.delete")) {
                                    Task { do { try await account.delete(e.id) } catch { app.showError(error) } }
                                }
                            } label: { Label(loc("common.delete"), systemImage: "trash") }
                        }
                        .galaxyRow()
                    }
                }
            } footer: {
                HStack { Spacer(); InfoButton(text: loc("snippets.info"), size: 13) }
            }
        }
    }
}

struct KnownHostsSection: View {
    let query: String
    @Binding var confirm: HostsView.ConfirmAction?
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var account: GalaxyAccount
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        let list = account.hosts.knownHosts.filter { e in
            guard !query.isEmpty, case .knownHost(let h, _, _, let fp) = e.data else { return true }
            return (h + " " + fp).lowercased().contains(query.lowercased())
        }
        if list.isEmpty {
            EmptyState(icon: "checkmark.shield", title: loc("knownHosts.empty")).listRowBackground(Color.clear)
        } else {
            Section {
                ForEach(list, id: \.id) { e in
                    if case .knownHost(let h, let port, let type, let fp) = e.data {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(port == 22 ? h : "\(h):\(port)").font(.body.weight(.medium)).foregroundStyle(p.text)
                            Text(type).font(.caption).foregroundStyle(p.secondaryText)
                            Text(fp).font(.caption2.monospaced()).foregroundStyle(p.tertiaryText).lineLimit(2).textSelection(.enabled)
                        }
                        .padding(.vertical, 2)
                        .swipeActions {
                            Button(role: .destructive) {
                                confirm = .init(title: loc("knownHosts.forget"), message: loc("knownHosts.forgetMessage", h), button: loc("knownHosts.forget")) {
                                    Task { do { try await account.delete(e.id) } catch { app.showError(error) } }
                                }
                            } label: { Label(loc("knownHosts.forget"), systemImage: "trash") }
                        }
                        .galaxyRow()
                    }
                }
            } footer: {
                HStack { Spacer(); InfoButton(text: loc("knownHosts.info"), size: 13) }
            }
        }
    }
}
