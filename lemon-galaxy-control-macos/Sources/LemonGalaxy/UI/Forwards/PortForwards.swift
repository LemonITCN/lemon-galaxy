import SwiftUI
import AppKit

// 端口映射：规则存在加密库（VaultData.portForward），运行中的映射由 AppModel 持有。
// 本机 127.0.0.1:<本机端口> → 经由设备或 SSH 主机 → 目标主机:目标端口。

/// 一条运行中的端口映射
struct RunningForward {
    let entryId: String
    let handle: PortForward
    /// 实际监听的本机端口（规则里填 0 时由内核自动选）
    let port: UInt16
    let startedMs: Int64
}

/// 从加密库条目里取出的端口映射规则
struct ForwardRule {
    let id: String
    let label: String
    let viaDeviceId: String?
    let viaSshHostId: String?
    let localPort: UInt16
    let remoteHost: String
    let remotePort: UInt16
    let autoStart: Bool
    let note: String

    init?(_ e: VaultEntry) {
        guard case let .portForward(label, viaDevice, viaSsh, localPort, remoteHost, remotePort, autoStart, note) = e.data else { return nil }
        id = e.id
        self.label = label
        viaDeviceId = viaDevice.flatMap { $0.isEmpty ? nil : $0 }
        viaSshHostId = viaSsh.flatMap { $0.isEmpty ? nil : $0 }
        self.localPort = localPort
        self.remoteHost = remoteHost
        self.remotePort = remotePort
        self.autoStart = autoStart
        self.note = note
    }

    var target: String {
        let h = remoteHost.contains(":") && !remoteHost.hasPrefix("[") ? "[\(remoteHost)]" : remoteHost
        return "\(h):\(remotePort)"
    }
}

extension AppModel {
    var forwardRules: [ForwardRule] {
        vault.filter { if case .portForward = $0.data { return true }; return false }
            .sorted { a, b in a.sort != b.sort ? a.sort < b.sort : a.id < b.id }
            .compactMap(ForwardRule.init)
    }

    func forwardRule(_ id: String) -> ForwardRule? { entry(id).flatMap(ForwardRule.init) }

    /// 本机地址：运行中用实际端口，没运行且规则是自动端口时显示「自动」
    func forwardLocalAddress(_ r: ForwardRule) -> String {
        if let f = forwards[r.id] { return "127.0.0.1:\(f.port)" }
        return r.localPort == 0 ? "127.0.0.1:" + L("pf.localPort.auto") : "127.0.0.1:\(r.localPort)"
    }

    /// 列表里的短写法：:15432（运行中用实际端口）
    func forwardLocalPortText(_ r: ForwardRule) -> String {
        if let f = forwards[r.id] { return ":\(f.port)" }
        return r.localPort == 0 ? ":" + L("pf.localPort.auto") : ":\(r.localPort)"
    }

    /// 经由的设备或 SSH 主机的名字
    func forwardViaName(_ r: ForwardRule) -> String {
        if let d = r.viaDeviceId { return deviceName(d) }
        if let h = r.viaSshHostId { return sshLabel(h) }
        return "—"
    }

    /// 开启一条映射。经由 SSH 主机时的主机密钥确认、密码询问走与 SSH 连接相同的卡片。
    /// auto：登录后自动开启（成功不提示，失败提示「自动开启失败」）；announce：成功后是否提示
    func startForward(_ id: String, auto: Bool = false, announce: Bool = true) {
        guard forwards[id] == nil, !forwardBusy.contains(id), let r = forwardRule(id) else { return }
        forwardBusy.insert(id)
        forwardErrors[id] = nil
        let prompt = SecretPromptBridge(model: self, title: r.label)
        Task {
            defer { forwardBusy.remove(id) }
            do {
                let f = try await client.startPortForward(entryId: id, prompt: prompt)
                // 等待期间退出登录了或规则被删了：立刻停
                guard phase == .main, entry(id) != nil else { f.stop(); return }
                let running = RunningForward(entryId: id, handle: f, port: f.localPort(), startedMs: Clock.nowMs)
                forwards[id] = running
                if !auto && announce { showToast(L("pf.started", r.label, "127.0.0.1:\(running.port)")) }
            } catch GalaxyError.Cancelled {
                // 用户在询问卡片里取消：不算失败
            } catch {
                let msg = Self.describe(error)
                forwardErrors[id] = msg
                showToast(L(auto ? "pf.autoFailed" : "pf.failed", r.label, msg), error: true)
            }
        }
    }

    func stopForward(_ id: String) {
        forwards.removeValue(forKey: id)?.handle.stop()
    }

    func toggleForward(_ id: String) {
        if forwards[id] != nil { stopForward(id) } else { startForward(id) }
    }

    func stopAllForwards() {
        for f in forwards.values { f.handle.stop() }
        forwards.removeAll()
        forwardErrors.removeAll()
        forwardsAutoStarted = false
    }

    /// 登录并上线后开启「登录后自动开启」的规则（每次登录只做一次）
    func autoStartForwards() {
        guard !forwardsAutoStarted, phase == .main, clientState == .online else { return }
        forwardsAutoStarted = true
        for r in forwardRules where r.autoStart { startForward(r.id, auto: true) }
    }

    /// 加密库变化后：规则被删掉（包括在别的设备上删）的映射停掉
    func pruneForwards() {
        let ids = Set(vault.map(\.id))
        for id in forwards.keys where !ids.contains(id) { stopForward(id) }
        if let s = selectedForward, !ids.contains(s) { selectedForward = nil }
    }

    func copyForwardAddress(_ id: String) {
        guard let f = forwards[id] else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString("127.0.0.1:\(f.port)", forType: .string)
        showToast(L("common.copied"))
    }

    /// 打开编辑卡片；id 为空时新建（排在最后）
    func editForward(_ id: String?) {
        let d = PortForwardDraft.from(id.flatMap(entry))
        if id == nil {
            let sorts = vault.compactMap { e -> Int64? in if case .portForward = e.data { return e.sort }; return nil }
            d.sort = (sorts.max() ?? -1) + 1
        }
        modal = .portForwardEditor(d)
    }

    func askDeleteForward(_ id: String) {
        guard let r = forwardRule(id) else { return }
        confirm(L("pf.delete.title"), L("pf.delete.message", r.label), button: L("common.delete")) { [weak self] in
            guard let self else { return }
            self.stopForward(id)
            Task {
                await self.delete(id)
                if self.selectedForward == id { self.selectedForward = nil }
            }
        }
    }
}

// MARK: - 编辑草稿与校验

@MainActor
final class PortForwardDraft: ObservableObject {
    let uid = UUID().uuidString
    var id = ""
    var folderId: String?
    var sort: Int64 = 0
    @Published var label = ""
    /// 经由：d:<设备 id> 或 s:<SSH 主机条目 id>，二选一；空为未选
    @Published var via = ""
    @Published var localPort = ""
    @Published var remoteHost = ""
    @Published var remotePort = ""
    @Published var autoStart = false
    @Published var note = ""
    /// 点过保存后才把问题标红，免得一打开就满屏红字
    @Published var showErrors = false
    @Published var saving = false

    /// 从加密库条目建草稿（保留文件夹与排序值）；nil 为新建
    static func from(_ e: VaultEntry?) -> PortForwardDraft {
        let d = PortForwardDraft()
        guard let e, let r = ForwardRule(e) else { return d }
        d.id = r.id
        d.folderId = e.folderId
        d.sort = e.sort
        d.label = r.label
        if let v = r.viaDeviceId { d.via = "d:" + v } else if let v = r.viaSshHostId { d.via = "s:" + v }
        d.localPort = r.localPort == 0 ? "" : "\(r.localPort)"
        d.remoteHost = r.remoteHost
        d.remotePort = "\(r.remotePort)"
        d.autoStart = r.autoStart
        d.note = r.note
        return d
    }

    var isNew: Bool { id.isEmpty }
    var viaDevice: String? { via.hasPrefix("d:") ? String(via.dropFirst(2)) : nil }
    var viaSsh: String? { via.hasPrefix("s:") ? String(via.dropFirst(2)) : nil }
    var host: String { remoteHost.trimmingCharacters(in: .whitespaces) }

    /// 留空或 0 为自动；超出范围、不是数字为 nil
    var localPortValue: UInt16? {
        let t = localPort.trimmingCharacters(in: .whitespaces)
        if t.isEmpty { return 0 }
        guard let v = Int(t), (0...65535).contains(v) else { return nil }
        return UInt16(v)
    }

    var remotePortValue: UInt16? {
        guard let v = Int(remotePort.trimmingCharacters(in: .whitespaces)), (1...65535).contains(v) else { return nil }
        return UInt16(v)
    }

    var viaInvalid: Bool { (viaDevice == nil) == (viaSsh == nil) || via.count <= 2 }
    var hostInvalid: Bool { host.isEmpty || host.contains(where: \.isWhitespace) }
    var localPortInvalid: Bool { localPortValue == nil }
    var remotePortInvalid: Bool { remotePortValue == nil }

    /// 当前所有问题（界面文字）
    var problems: [String] {
        var out: [String] = []
        if viaInvalid { out.append(L("pf.err.via")) }
        if localPortInvalid { out.append(L("pf.err.localPort")) }
        if hostInvalid { out.append(L("pf.err.host")) }
        if remotePortInvalid { out.append(L("pf.err.remotePort")) }
        return out
    }

    func build() -> VaultEntry? {
        guard problems.isEmpty, let lp = localPortValue, let rp = remotePortValue else { return nil }
        let name = label.trimmingCharacters(in: .whitespaces)
        let data = VaultData.portForward(label: name.isEmpty ? "\(host):\(rp)" : name, viaDeviceId: viaDevice, viaSshHostId: viaSsh,
                                         localPort: lp, remoteHost: host, remotePort: rp, autoStart: autoStart, note: note)
        return VaultEntry(id: id, folderId: folderId, sort: sort, data: data, updatedAt: 0)
    }
}

// MARK: - 中间列：规则列表

struct PortForwardList: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var query = ""

    var body: some View {
        VStack(spacing: 0) {
            ColumnHeader(title: loc("nav.forwards")) {
                IconButton(icon: "plus", help: loc("pf.new")) { app.editForward(nil) }
            }
            SearchField(placeholder: loc("pf.search"), text: $query).padding(.horizontal, 12).padding(.bottom, 8)
            Rectangle().fill(p.separator).frame(height: 1)
            ScrollView {
                LazyVStack(spacing: 1) {
                    let list = filtered
                    ForEach(list, id: \.id) { r in PortForwardRow(rule: r) }
                    if list.isEmpty {
                        Text(query.isEmpty ? loc("pf.empty") : loc("hosts.noMatch"))
                            .font(.system(size: 12)).foregroundStyle(p.tertiaryText).padding(16)
                    }
                }
                .padding(6)
            }
        }
    }

    private var filtered: [ForwardRule] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        let all = app.forwardRules
        guard !q.isEmpty else { return all }
        return all.filter { r in
            [r.label, r.target, app.forwardViaName(r), app.forwardLocalAddress(r)].joined(separator: " ").lowercased().contains(q)
        }
    }
}

/// 状态图标：运行中为绿色，开启中转圈，上次失败带警告角标
struct ForwardGlyph: View {
    let id: String
    var size: CGFloat = 26
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    var body: some View {
        let running = app.forwards[id] != nil
        ZStack(alignment: .bottomTrailing) {
            RoundedRectangle(cornerRadius: size * 0.24).fill(running ? p.success.opacity(0.15) : p.elevated).frame(width: size, height: size)
                .overlay(RoundedRectangle(cornerRadius: size * 0.24).stroke(running ? Color.clear : p.border, lineWidth: 0.5))
                .overlay(Image(systemName: "arrow.left.arrow.right").font(.system(size: size * 0.42, weight: .semibold))
                    .foregroundStyle(running ? p.success : p.secondaryText))
            if app.forwardBusy.contains(id) {
                ProgressView().controlSize(.mini).scaleEffect(0.7).offset(x: 4, y: 4)
            } else if !running, app.forwardErrors[id] != nil {
                Image(systemName: "exclamationmark.circle.fill").font(.system(size: size * 0.36)).foregroundStyle(p.warning)
                    .background(Circle().fill(p.list)).offset(x: 3, y: 3)
            }
        }
    }
}

struct PortForwardRow: View {
    let rule: ForwardRule
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var hover = false

    var body: some View {
        let selected = app.selectedForward == rule.id && app.activeTabId == nil
        let running = app.forwards[rule.id] != nil
        HStack(spacing: 9) {
            ForwardGlyph(id: rule.id)
            VStack(alignment: .leading, spacing: 1) {
                Text(rule.label).font(.system(size: 12.5, weight: .medium)).lineLimit(1)
                Text(verbatim: app.forwardLocalPortText(rule) + " → " + rule.target)
                    .font(.system(size: 11, design: .monospaced)).foregroundStyle(p.tertiaryText).lineLimit(1).truncationMode(.middle)
            }
            Spacer(minLength: 4)
            if running { StatusDot(state: .online, size: 7) }
            else if rule.autoStart { Image(systemName: "bolt.fill").font(.system(size: 9)).foregroundStyle(p.tertiaryText).help(loc("pf.autoStart")) }
        }
        .padding(.horizontal, 8).frame(height: 40)
        .background(RoundedRectangle(cornerRadius: 6).fill(selected ? p.selection : (hover ? p.hover : .clear)))
        .contentShape(Rectangle())
        .onHover { hover = $0 }
        .gesture(TapGesture(count: 2).onEnded { app.toggleForward(rule.id) })
        .simultaneousGesture(TapGesture().onEnded { app.selectedForward = rule.id; app.activeTabId = nil })
        .contextMenu { PortForwardMenu(rule: rule) }
    }
}

struct PortForwardMenu: View {
    let rule: ForwardRule
    @EnvironmentObject var app: AppModel
    @Environment(\.loc) var loc
    var body: some View {
        if app.forwards[rule.id] != nil {
            Button(loc("pf.stop")) { app.stopForward(rule.id) }
            Button(loc("pf.copy")) { app.copyForwardAddress(rule.id) }
        } else {
            Button(loc("pf.start")) { app.startForward(rule.id) }.disabled(app.forwardBusy.contains(rule.id))
        }
        Divider()
        Button(loc("hosts.edit")) { app.selectedForward = rule.id; app.editForward(rule.id) }
        Button(loc("common.delete"), role: .destructive) { app.askDeleteForward(rule.id) }
    }
}

// MARK: - 主区

struct PortForwardHome: View {
    @EnvironmentObject var app: AppModel
    var body: some View {
        if let id = app.selectedForward, let r = app.forwardRule(id) {
            PortForwardDetail(rule: r)
        } else {
            PortForwardOverview()
        }
    }
}

struct PortForwardOverview: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        let rules = app.forwardRules
        if rules.isEmpty {
            EmptyState(icon: "arrow.left.arrow.right", title: loc("pf.empty"), action: (loc("pf.new"), { app.editForward(nil) }))
        } else {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    HStack(alignment: .firstTextBaseline, spacing: 10) {
                        Text(loc("nav.forwards")).font(.system(size: 20, weight: .semibold))
                        Text(loc("pf.runningCount", app.forwards.count)).font(.system(size: 12.5)).foregroundStyle(p.secondaryText)
                        InfoButton(text: loc("pf.info"))
                        Spacer()
                        Button { app.editForward(nil) } label: { Label(loc("pf.new"), systemImage: "plus") }
                            .buttonStyle(SecondaryButtonStyle())
                    }
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 260, maximum: 380), spacing: 12)], spacing: 12) {
                        ForEach(rules, id: \.id) { r in PortForwardCard(rule: r) }
                    }
                }
                .padding(24)
            }
        }
    }
}

struct PortForwardCard: View {
    let rule: ForwardRule
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var hover = false

    var body: some View {
        let running = app.forwards[rule.id] != nil
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                ForwardGlyph(id: rule.id, size: 34)
                VStack(alignment: .leading, spacing: 3) {
                    Text(rule.label).font(.system(size: 13, weight: .semibold)).lineLimit(1)
                    Text(loc("hosts.via", app.forwardViaName(rule))).font(.system(size: 11.5)).foregroundStyle(p.secondaryText).lineLimit(1)
                }
                Spacer(minLength: 4)
                ForwardToggleButton(id: rule.id, small: true)
            }
            HStack(spacing: 6) {
                Text(verbatim: app.forwardLocalAddress(rule)).foregroundStyle(running ? p.text : p.secondaryText)
                Image(systemName: "arrow.right").font(.system(size: 9, weight: .semibold)).foregroundStyle(p.tertiaryText)
                Text(verbatim: rule.target).foregroundStyle(p.secondaryText)
            }
            .font(.system(size: 11.5, design: .monospaced))
            .lineLimit(1)
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 10).fill(hover ? p.elevated : p.card))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(hover ? p.accent.opacity(0.5) : p.border, lineWidth: hover ? 1 : 0.6))
        .contentShape(Rectangle())
        .onHover { hover = $0 }
        .onTapGesture { app.selectedForward = rule.id }
        .contextMenu { PortForwardMenu(rule: rule) }
    }
}

/// 开启 / 停止按钮（开启中转圈）
struct ForwardToggleButton: View {
    let id: String
    var small = false
    @EnvironmentObject var app: AppModel
    @Environment(\.loc) var loc

    var body: some View {
        if app.forwardBusy.contains(id) {
            ProgressView().controlSize(.small).frame(width: small ? 28 : 60, height: 28)
        } else if app.forwards[id] != nil {
            Button { app.stopForward(id) } label: { Label(loc("pf.stop"), systemImage: "stop.fill") }
                .buttonStyle(SecondaryButtonStyle(destructive: true))
        } else {
            Button { app.startForward(id) } label: { Label(loc("pf.start"), systemImage: "play.fill") }
                .buttonStyle(PrimaryButtonStyle())
        }
    }
}

struct PortForwardDetail: View {
    let rule: ForwardRule
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        let running = app.forwards[rule.id]
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                DetailHeader(title: rule.label, subtitle: app.forwardLocalAddress(rule) + "  →  " + rule.target) {
                    ForwardGlyph(id: rule.id, size: 52)
                } actions: {
                    Button { app.editForward(rule.id) } label: { Label(loc("hosts.edit"), systemImage: "pencil") }
                        .buttonStyle(SecondaryButtonStyle())
                }
                HStack(spacing: 8) {
                    ForwardToggleButton(id: rule.id)
                    if running != nil {
                        Button { app.copyForwardAddress(rule.id) } label: { Label(loc("pf.copy"), systemImage: "doc.on.doc") }
                            .buttonStyle(SecondaryButtonStyle())
                    }
                    InfoButton(text: loc("pf.info"))
                }
                if running == nil, let err = app.forwardErrors[rule.id] {
                    HStack(alignment: .top, spacing: 8) {
                        Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(p.warning)
                        Text(loc("pf.lastError", err)).font(.system(size: 12.5)).fixedSize(horizontal: false, vertical: true)
                    }
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(RoundedRectangle(cornerRadius: 9).fill(p.warning.opacity(0.1)))
                    .overlay(RoundedRectangle(cornerRadius: 9).stroke(p.warning.opacity(0.35), lineWidth: 0.8))
                }
                InfoGrid(rows: [
                    (loc("pf.status"), running.map { loc("pf.runningAt", Fmt.time($0.startedMs)) } ?? loc("pf.stopped")),
                    (loc("pf.via"), viaText),
                    (loc("pf.local"), app.forwardLocalAddress(rule)),
                    (loc("pf.target"), rule.target),
                    (loc("pf.autoStart"), rule.autoStart ? loc("common.on") : loc("common.off")),
                ])
                if !rule.note.isEmpty {
                    VStack(alignment: .leading, spacing: 6) {
                        SectionLabel(text: loc("hosts.note"))
                        Text(rule.note).font(.system(size: 12.5)).textSelection(.enabled)
                    }
                }
                HStack {
                    Spacer()
                    Button(loc("common.delete")) { app.askDeleteForward(rule.id) }
                        .buttonStyle(SecondaryButtonStyle(destructive: true))
                }
            }
            .padding(24)
        }
    }

    private var viaText: String {
        if let d = rule.viaDeviceId {
            let name = app.deviceName(d)
            if let dev = app.device(d), !dev.online, !dev.lanSeen { return name + " " + loc("pf.via.offline") }
            return name
        }
        if let h = rule.viaSshHostId { return app.sshLabel(h) + " · SSH" }
        return "—"
    }
}

// MARK: - 编辑卡片

struct PortForwardEditorCard: View {
    @ObservedObject var draft: PortForwardDraft
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 6) {
                Text(loc(draft.isNew ? "pf.new" : "pf.edit.title")).font(.system(size: 14, weight: .semibold))
                InfoButton(text: loc("pf.info"))
                Spacer()
                IconButton(icon: "xmark", help: loc("common.close"), size: 11) { app.modal = nil }
            }
            field(loc("editor.label")) {
                TextField(draft.host.isEmpty ? "Postgres" : "\(draft.host):\(draft.remotePort)", text: $draft.label).fieldStyle()
            }
            field(loc("pf.via"), info: loc("pf.via.info")) {
                Picker("", selection: $draft.via) {
                    Text(loc("pf.via.choose")).tag("")
                    Section(loc("hosts.devices")) {
                        ForEach(viaDevices, id: \.id) { d in Text(app.deviceName(d.id)).tag("d:" + d.id) }
                    }
                    Section(loc("hosts.sshHosts")) {
                        ForEach(viaHosts, id: \.id) { e in Text(app.sshLabel(e.id)).tag("s:" + e.id) }
                    }
                }
                .labelsHidden()
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(p.danger, lineWidth: 1).opacity(draft.showErrors && draft.viaInvalid ? 1 : 0))
            }
            HStack(alignment: .bottom, spacing: 8) {
                field(loc("pf.localPort"), info: loc("pf.localPort.info")) {
                    HStack(spacing: 4) {
                        Text(verbatim: "127.0.0.1:").font(.system(size: 12.5, design: .monospaced)).foregroundStyle(p.secondaryText)
                            .lineLimit(1).fixedSize()
                        TextField(loc("pf.localPort.auto"), text: $draft.localPort)
                            .fieldStyle(invalid: (draft.showErrors || !draft.localPort.isEmpty) && draft.localPortInvalid)
                            .frame(width: 84)
                    }
                }
                Image(systemName: "arrow.right").font(.system(size: 11, weight: .semibold)).foregroundStyle(p.tertiaryText).frame(height: 28)
                field(loc("pf.remoteHost")) {
                    TextField("127.0.0.1", text: $draft.remoteHost)
                        .fieldStyle(invalid: draft.showErrors && draft.hostInvalid)
                }
                field(loc("pf.remotePort")) {
                    TextField("5432", text: $draft.remotePort)
                        .fieldStyle(invalid: (draft.showErrors || !draft.remotePort.isEmpty) && draft.remotePortInvalid)
                        .frame(width: 72)
                }
            }
            ToggleRow(label: loc("pf.autoStart"), isOn: $draft.autoStart, info: loc("pf.autoStart.info"))
            field(loc("hosts.note")) {
                TextEditor(text: $draft.note).font(.system(size: 12.5)).frame(height: 48)
                    .scrollContentBackground(.hidden).padding(4)
                    .background(RoundedRectangle(cornerRadius: 7).fill(p.elevated))
                    .overlay(RoundedRectangle(cornerRadius: 7).stroke(p.border, lineWidth: 0.7))
            }
            if draft.showErrors, !draft.problems.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(draft.problems, id: \.self) { msg in
                        Label(msg, systemImage: "exclamationmark.circle.fill").font(.system(size: 12)).foregroundStyle(p.danger)
                    }
                }
            }
            HStack(spacing: 8) {
                if !draft.isNew {
                    Button(loc("common.delete")) { let id = draft.id; app.modal = nil; app.askDeleteForward(id) }
                        .buttonStyle(SecondaryButtonStyle(destructive: true))
                }
                Spacer()
                Button(loc("common.cancel")) { app.modal = nil }.buttonStyle(SecondaryButtonStyle()).keyboardShortcut(.cancelAction)
                Button(loc("common.save")) { save() }
                    .buttonStyle(PrimaryButtonStyle())
                    .disabled(draft.saving)
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(18)
    }

    private var viaDevices: [DeviceInfo] { app.devices.filter { !$0.isSelf && $0.kind != .mobile } }
    private var viaHosts: [VaultEntry] { app.vault.filter { if case .sshHost = $0.data { return true }; return false } }

    private func field<C: View>(_ label: String, info: String? = nil, @ViewBuilder _ c: () -> C) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 4) {
                Text(label).font(.system(size: 11.5, weight: .medium)).foregroundStyle(p.secondaryText)
                if let info { InfoButton(text: info) }
            }
            c()
        }
    }

    private func save() {
        guard let e = draft.build() else {
            draft.showErrors = true
            return
        }
        draft.saving = true
        let wasRunning = !draft.isNew && app.forwards[draft.id] != nil
        Task {
            if let saved = await app.save(e) {
                app.modal = nil
                app.selectedForward = saved.id
                app.activeTabId = nil
                if wasRunning {
                    // 运行中的规则改了：按新设置重开
                    app.stopForward(saved.id)
                    app.startForward(saved.id, announce: false)
                    app.showToast(L("pf.restarted", ForwardRule(saved)?.label ?? ""))
                } else {
                    app.showToast(L("common.saved"))
                }
            }
            draft.saving = false
        }
    }
}
