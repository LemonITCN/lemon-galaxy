import SwiftUI
import AppKit

// MCP 页（docs/mcp.md 第 5 节）：中间列是分区导航，主区自上而下是
// 服务、待批准（有才显示）、连接方式、权限、已连接的客户端、操作记录。

// MARK: - 中间列

struct MCPNav: View {
    @EnvironmentObject var app: AppModel
    @EnvironmentObject var mcp: MCPModel
    @EnvironmentObject var settings: AppSettings
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(spacing: 0) {
            ColumnHeader(title: loc("nav.mcp")) { MCPStateDot() }
            Rectangle().fill(p.separator).frame(height: 1)
            VStack(spacing: 2) {
                ForEach(MCPSection.allCases) { s in row(s) }
            }
            .padding(8)
            Spacer()
        }
    }

    private func row(_ s: MCPSection) -> some View {
        let sel = mcp.navSelection == s && app.activeTabId == nil
        let enabled = s != .pending || mcp.pendingCount > 0
        return Button {
            mcp.navSelection = s
            mcp.scrollTarget = s
            app.activeTabId = nil
        } label: {
            HStack(spacing: 10) {
                Image(systemName: s.icon).font(.system(size: 13)).frame(width: 20)
                    .foregroundStyle(sel ? p.accent : (enabled ? p.secondaryText : p.tertiaryText))
                Text(loc(s.titleKey)).font(.system(size: 13, weight: sel ? .semibold : .regular))
                    .foregroundStyle(enabled ? p.text : p.tertiaryText)
                Spacer(minLength: 4)
                trailing(s)
            }
            .padding(.horizontal, 10).frame(height: 32)
            .background(RoundedRectangle(cornerRadius: 7).fill(sel ? p.selection : .clear))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
    }

    @ViewBuilder private func trailing(_ s: MCPSection) -> some View {
        let small = Font.system(size: 11.5).monospacedDigit()
        switch s {
        case .service:
            let failed = settings.mcpEnabled && !mcp.running && !mcp.busy && mcp.startError != nil
            Text(loc(mcp.running ? "mcp.service.running" : failed ? "mcp.service.failed" : "mcp.service.off")).font(small)
                .foregroundStyle(mcp.running ? p.success : failed ? p.danger : p.tertiaryText)
        case .pending:
            if mcp.pendingCount > 0 { CountBadge(count: mcp.pendingCount) }
        case .permission:
            Text(loc(mcp.mode.labelKey)).font(small).foregroundStyle(mcp.mode == .fullTrust ? p.danger : p.tertiaryText).lineLimit(1)
        case .clients:
            if !mcp.status.clients.isEmpty { Text("\(mcp.status.clients.count)").font(small).foregroundStyle(p.tertiaryText) }
        case .records:
            if !mcp.calls.isEmpty { Text("\(mcp.calls.count)").font(small).foregroundStyle(p.tertiaryText) }
        case .connection:
            EmptyView()
        }
    }
}

/// 服务状态点：运行中绿、开启失败红、没开灰
struct MCPStateDot: View {
    @EnvironmentObject var mcp: MCPModel
    @EnvironmentObject var settings: AppSettings
    var body: some View {
        StatusDot(state: mcp.running ? .online : (settings.mcpEnabled && mcp.startError != nil ? .busy : .offline), size: 7)
            .padding(.trailing, 6)
    }
}

/// 待批准数（侧栏、中间列、分区标题共用）
struct CountBadge: View {
    let count: Int
    @Environment(\.palette) var p
    var body: some View {
        Text("\(count)")
            .font(.system(size: 10.5, weight: .bold).monospacedDigit())
            .foregroundStyle(.white)
            .padding(.horizontal, 5.5).frame(minWidth: 17, minHeight: 16)
            .background(Capsule().fill(p.warning))
    }
}

// MARK: - 主区

struct MCPPage: View {
    @EnvironmentObject var mcp: MCPModel

    var body: some View {
        GeometryReader { g in
            ScrollViewReader { proxy in
                ScrollView {
                    MCPPageContent()
                        .padding(24)
                        .frame(width: min(g.size.width, 860), alignment: .leading)
                        .frame(width: g.size.width)
                }
                .onChange(of: mcp.scrollTarget) { t in
                    guard let t else { return }
                    withAnimation(.easeOut(duration: 0.2)) { proxy.scrollTo(t, anchor: .top) }
                    mcp.scrollTarget = nil
                }
            }
        }
        .onAppear { mcp.pageAppeared() }
    }
}

struct MCPPageContent: View {
    @EnvironmentObject var mcp: MCPModel
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            PageTitle(text: loc("nav.mcp"), info: loc("mcp.info"))
            MCPServiceSection().id(MCPSection.service)
            if mcp.pendingCount > 0 { MCPPendingSection().id(MCPSection.pending) }
            MCPConnectionSection().id(MCPSection.connection)
            MCPPermissionSection().id(MCPSection.permission)
            MCPClientsSection().id(MCPSection.clients)
            MCPRecordsSection().id(MCPSection.records)
        }
    }
}

// MARK: 服务

struct MCPServiceSection: View {
    @EnvironmentObject var mcp: MCPModel
    @EnvironmentObject var settings: AppSettings
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    private var failed: Bool { settings.mcpEnabled && !mcp.running && !mcp.busy && mcp.startError != nil }

    var body: some View {
        SettingsSection(title: loc("mcp.section.service")) {
            HStack(spacing: 12) {
                let color = mcp.running ? p.success : (failed ? p.danger : p.tertiaryText)
                RoundedRectangle(cornerRadius: 8).fill(color.opacity(0.14)).frame(width: 34, height: 34)
                    .overlay(Image(systemName: "point.3.connected.trianglepath.dotted").font(.system(size: 15, weight: .medium)).foregroundStyle(color))
                VStack(alignment: .leading, spacing: 3) {
                    Text(loc("mcp.service.name")).font(.system(size: 13, weight: .semibold))
                    statusLine
                }
                Spacer(minLength: 8)
                if mcp.busy { ProgressView().controlSize(.small) }
                Toggle("", isOn: Binding(get: { settings.mcpEnabled }, set: { mcp.setEnabled($0) }))
                    .toggleStyle(PaletteSwitchStyle()).labelsHidden()
            }
            Rectangle().fill(p.separator).frame(height: 1)
            HStack(spacing: 8) {
                Text(loc("mcp.port")).font(.system(size: 13))
                InfoButton(text: loc("mcp.port.info"))
                Spacer()
                if !mcp.portText.isEmpty && mcp.portValue == nil {
                    Text(loc("mcp.port.invalid")).font(.system(size: 11.5)).foregroundStyle(p.danger)
                }
                TextField(String(mcpDefaultPort()), text: $mcp.portText)
                    .fieldStyle(invalid: !mcp.portText.isEmpty && mcp.portValue == nil)
                    .font(.system(size: 13, design: .monospaced))
                    .frame(width: 92)
                    .onSubmit { if mcp.portChanged { mcp.applyPort() } }
                Button(loc("common.apply")) { mcp.applyPort() }
                    .buttonStyle(SecondaryButtonStyle())
                    .disabled(!mcp.portChanged || mcp.busy)
            }
            if failed, let err = mcp.startError {
                HStack(alignment: .top, spacing: 8) {
                    Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(p.danger)
                    Text(err).font(.system(size: 12.5)).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                }
                .padding(10)
                .background(RoundedRectangle(cornerRadius: 8).fill(p.danger.opacity(0.08)))
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(p.danger.opacity(0.3), lineWidth: 0.8))
            }
        }
    }

    @ViewBuilder private var statusLine: some View {
        HStack(spacing: 6) {
            if mcp.busy {
                Text(loc("mcp.service.starting")).foregroundStyle(p.secondaryText)
            } else if mcp.running {
                Text(loc("mcp.service.running")).foregroundStyle(p.success)
                Text(verbatim: mcp.status.url).font(.system(size: 12, design: .monospaced)).foregroundStyle(p.secondaryText)
                    .textSelection(.enabled).lineLimit(1)
            } else if failed {
                Text(loc("mcp.service.failed")).foregroundStyle(p.danger)
            } else {
                Text(loc("mcp.service.off")).foregroundStyle(p.tertiaryText)
            }
        }
        .font(.system(size: 12))
    }
}

// MARK: 待批准

struct MCPPendingSection: View {
    @EnvironmentObject var mcp: MCPModel
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 6) {
                SectionLabel(text: loc("mcp.section.pending"))
                CountBadge(count: mcp.pendingCount)
            }
            ForEach(mcp.pendingCalls) { c in MCPApprovalCard(call: c) }
            ForEach(mcp.secrets) { r in MCPSecretCard(req: r) }
        }
    }
}

/// 客户端名标签
struct MCPClientChip: View {
    let key: String
    @EnvironmentObject var mcp: MCPModel
    @Environment(\.palette) var p
    var body: some View {
        Text(mcp.clientName(key))
            .font(.system(size: 11.5, weight: .semibold))
            .foregroundStyle(p.accent)
            .lineLimit(1)
            .padding(.horizontal, 7).frame(height: 20)
            .background(RoundedRectangle(cornerRadius: 5).fill(p.accent.opacity(p.isDark ? 0.2 : 0.1)))
    }
}

/// 外部智能体要做一步需要批准的操作：客户端、目标、操作摘要、详情、风险与原因、它给的理由
struct MCPApprovalCard: View {
    @ObservedObject var call: MCPCall
    @EnvironmentObject var mcp: MCPModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var risk: RiskLevel { call.call.risk }
    var accent: Color { call.isSimpleApproval ? p.accent : risk.color(p) }

    private var heading: String {
        if call.isUseTarget { return loc("mcp.useTarget.ask", call.call.targetName) }
        if call.isUseApp { return call.call.summary.isEmpty ? loc("ai.tool.use_app") : call.call.summary }
        return loc("ai.tool." + call.call.tool)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 11) {
            HStack(spacing: 8) {
                MCPClientChip(key: call.clientKey)
                Image(systemName: ToolRow.icon(call.call.tool)).font(.system(size: 13)).foregroundStyle(accent)
                Text(heading).font(.system(size: 13, weight: .semibold)).lineLimit(1)
                if !call.isUseTarget && !call.call.targetName.isEmpty {
                    HStack(spacing: 4) {
                        Image(systemName: "arrow.right").font(.system(size: 9.5))
                        Text(call.call.targetName).font(.system(size: 12)).lineLimit(1)
                    }
                    .foregroundStyle(p.secondaryText)
                }
                Spacer(minLength: 6)
                if !call.isSimpleApproval { RiskChip(risk: risk) }
                Text(MCPTime.text(call.at)).font(.system(size: 11.5).monospacedDigit()).foregroundStyle(p.tertiaryText)
            }
            if call.isSimpleApproval {
                if !call.call.detail.isEmpty {
                    Text(call.call.detail).font(.system(size: 12)).foregroundStyle(p.secondaryText).textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                }
            } else {
                let command = call.call.detail.isEmpty ? call.call.summary : call.call.detail
                CommandBlock(text: command, diff: ToolDetails.looksLikeDiff(call.call.tool, command))
            }
            if let img = call.call.image { MCPImage(img: img) }
            if !call.call.riskReasons.isEmpty { RiskReasons(risk: call.isSimpleApproval ? .unknown : risk, reasons: call.call.riskReasons) }
            if !call.call.modelReason.isEmpty { ReasonLine(text: call.call.modelReason) }
            buttons
        }
        .padding(14)
        .background(RoundedRectangle(cornerRadius: 11).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 11).stroke(accent.opacity(0.75), lineWidth: 1.2))
    }

    @ViewBuilder private var buttons: some View {
        if call.denying {
            VStack(alignment: .leading, spacing: 8) {
                TextField(loc("mcp.deny.reason"), text: $call.denyReason).fieldStyle().onSubmit { deny() }
                HStack {
                    Spacer()
                    Button(loc("common.cancel")) { call.denying = false }.buttonStyle(SecondaryButtonStyle())
                    Button(loc("ai.approve.deny")) { deny() }.buttonStyle(PrimaryButtonStyle(destructive: true))
                }
            }
        } else if call.isSimpleApproval {
            HStack(spacing: 8) {
                Button(loc("ai.approve.allow")) { mcp.decide(call, .allowOnce) }.buttonStyle(PrimaryButtonStyle())
                Spacer()
                Button(loc("ai.approve.deny")) { call.denying = true }.buttonStyle(SecondaryButtonStyle(destructive: true))
            }
        } else {
            HStack(spacing: 8) {
                Button(loc("mcp.approve.once")) { mcp.decide(call, .allowOnce) }.buttonStyle(PrimaryButtonStyle())
                Button(loc("mcp.approve.similar")) { mcp.decide(call, .allowSimilar) }.buttonStyle(SecondaryButtonStyle())
                Spacer()
                Button(loc("ai.approve.deny")) { call.denying = true }.buttonStyle(SecondaryButtonStyle(destructive: true))
            }
        }
    }

    private func deny() {
        let r = call.denyReason.trimmingCharacters(in: .whitespacesAndNewlines)
        mcp.decide(call, .deny(reason: r.isEmpty ? nil : r))
    }
}

/// request_secret：安全输入框，内容只交给 mcp_provide_secret
struct MCPSecretCard: View {
    let req: MCPSecretRequest
    @EnvironmentObject var mcp: MCPModel
    @EnvironmentObject var ai: AIHubModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var secret = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                MCPClientChip(key: req.clientKey)
                Image(systemName: "key.horizontal").font(.system(size: 13)).foregroundStyle(p.warning)
                Text(req.prompt).font(.system(size: 13, weight: .semibold)).lineLimit(2)
                InfoButton(text: loc("mcp.secret.info"))
                Spacer(minLength: 6)
                Text(MCPTime.text(req.at)).font(.system(size: 11.5).monospacedDigit()).foregroundStyle(p.tertiaryText)
            }
            HStack(spacing: 8) {
                SecureField(loc("ai.secret.placeholder"), text: $secret).fieldStyle().onSubmit(submit)
                Button(loc("ai.secret.send"), action: submit).buttonStyle(PrimaryButtonStyle()).disabled(secret.isEmpty)
                Button(loc("common.cancel")) { secret = ""; mcp.cancelSecret(req) }.buttonStyle(SecondaryButtonStyle())
            }
            .frame(maxWidth: 480)
            if !req.targetId.isEmpty {
                Text(loc("ai.secret.target", ai.targetName(req.targetId))).font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
            }
        }
        .padding(14)
        .background(RoundedRectangle(cornerRadius: 11).fill(p.card))
        .overlay(RoundedRectangle(cornerRadius: 11).stroke(p.warning.opacity(0.7), lineWidth: 1.2))
    }

    private func submit() {
        guard !secret.isEmpty else { return }
        let s = secret
        secret = ""
        mcp.provideSecret(req, s)
    }
}

struct MCPImage: View {
    let img: ImageData
    @Environment(\.palette) var p
    var body: some View {
        if let ns = NSImage(data: img.data) {
            Image(nsImage: ns).resizable().aspectRatio(contentMode: .fit)
                .frame(maxWidth: 320, maxHeight: 180, alignment: .leading)
                .clipShape(RoundedRectangle(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(p.border, lineWidth: 0.5))
        }
    }
}

// MARK: 连接方式

struct MCPConnectionSection: View {
    @EnvironmentObject var mcp: MCPModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        SettingsSection(title: loc("mcp.section.connection"), info: loc("mcp.token.info")) {
            HStack(spacing: 8) {
                label(loc("mcp.address"))
                Text(verbatim: mcp.url).font(.system(size: 12.5, design: .monospaced)).textSelection(.enabled).lineLimit(1)
                Spacer(minLength: 8)
                IconButton(icon: "doc.on.doc", help: loc("common.copy"), size: 11.5) { mcp.copyURL() }
            }
            Rectangle().fill(p.separator).frame(height: 1)
            HStack(spacing: 8) {
                label(loc("mcp.token"))
                if mcp.token.isEmpty {
                    Text(loc("mcp.token.none")).font(.system(size: 12.5)).foregroundStyle(p.tertiaryText)
                } else {
                    Text(verbatim: mcp.tokenShown ? mcp.token : mcp.maskedToken)
                        .font(.system(size: 12.5, design: .monospaced)).textSelection(.enabled).lineLimit(1).truncationMode(.middle)
                }
                Spacer(minLength: 8)
                if !mcp.token.isEmpty {
                    IconButton(icon: mcp.tokenShown ? "eye.slash" : "eye", help: loc(mcp.tokenShown ? "mcp.token.hide" : "mcp.token.show"), size: 11.5) {
                        mcp.tokenShown.toggle()
                    }
                    IconButton(icon: "doc.on.doc", help: loc("common.copy"), size: 11.5) { mcp.copyToken() }
                }
                Button(loc("mcp.token.regenerate")) { mcp.askRegenerate() }.buttonStyle(SecondaryButtonStyle())
            }
            Rectangle().fill(p.separator).frame(height: 1)
            HStack(spacing: 6) {
                Text(loc("mcp.config")).font(.system(size: 13))
                InfoButton(text: loc(mcp.configKind.infoKey))
                Spacer(minLength: 8)
                Picker("", selection: $mcp.configKind) {
                    ForEach(MCPConfigKind.allCases) { k in Text(verbatim: k.title).tag(k) }
                }
                .pickerStyle(.segmented).labelsHidden().frame(maxWidth: 440)
                Button { mcp.copyConfig(mcp.configKind) } label: { Label(loc("mcp.config.copy"), systemImage: "doc.on.doc") }
                    .buttonStyle(SecondaryButtonStyle())
                    .disabled(mcp.token.isEmpty)
            }
            // 令牌没显示时配置里也遮住；复制的是完整配置
            let shown = mcp.configText(mcp.configKind, token: mcp.token.isEmpty ? "<token>" : (mcp.tokenShown ? nil : mcp.maskedToken))
            MCPConfigBlock(text: shown)
        }
    }

    private func label(_ t: String) -> some View {
        Text(t).font(.system(size: 13)).foregroundStyle(p.secondaryText).frame(width: 56, alignment: .leading)
    }
}

/// 配置文字：等宽、自动换行（命令一行很长，不用横向滚动）
struct MCPConfigBlock: View {
    let text: String
    @Environment(\.palette) var p
    var body: some View {
        Text(verbatim: text)
            .font(.system(size: 12, design: .monospaced))
            .foregroundStyle(p.text)
            .textSelection(.enabled)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, 10).padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 7).fill(p.isDark ? Color.black.opacity(0.25) : Color.black.opacity(0.04)))
    }
}

// MARK: 权限

struct MCPPermissionSection: View {
    @EnvironmentObject var mcp: MCPModel
    @EnvironmentObject var settings: AppSettings
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        SettingsSection(title: loc("mcp.section.permission"), info: loc("mcp.mode.info")) {
            ModePicker(mode: Binding(get: { mcp.mode }, set: { mcp.setMode($0) }))
            if mcp.mode == .fullTrust {
                HStack(spacing: 10) {
                    Text(loc("mcp.trust")).font(.system(size: 13))
                    Spacer()
                    if let until = mcp.trustUntilMs {
                        Text(loc("mcp.trust.until", MCPTime.hm(until))).font(.system(size: 12)).foregroundStyle(p.secondaryText)
                    }
                    Picker("", selection: Binding(get: { settings.mcpTrustMinutes }, set: { mcp.setMode(.fullTrust, minutes: $0) })) {
                        Text(loc("ai.trust.minutes", 15)).tag(15)
                        Text(loc("ai.trust.hours", 1)).tag(60)
                        Text(loc("ai.trust.hours", 4)).tag(240)
                        Text(loc("mcp.trust.unlimited")).tag(0)
                    }
                    .pickerStyle(.segmented).labelsHidden().frame(width: 340)
                }
            }
        }
    }
}

// MARK: 已连接的客户端

struct MCPClientsSection: View {
    @EnvironmentObject var mcp: MCPModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        let clients = mcp.status.clients
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 6) {
                SectionLabel(text: loc("mcp.section.clients"))
                if !clients.isEmpty { Text("\(clients.count)").font(.system(size: 11).monospacedDigit()).foregroundStyle(p.tertiaryText) }
            }
            Card(padding: 0) {
                if clients.isEmpty {
                    Text(loc("mcp.clients.empty")).font(.system(size: 12.5)).foregroundStyle(p.tertiaryText)
                        .padding(.horizontal, 14).padding(.vertical, 12)
                } else {
                    VStack(spacing: 0) {
                        ForEach(Array(clients.enumerated()), id: \.element.key) { i, c in
                            MCPClientRow(client: c)
                            if i < clients.count - 1 { Rectangle().fill(p.separator).frame(height: 1).padding(.leading, 56) }
                        }
                    }
                }
            }
        }
    }
}

struct MCPClientRow: View {
    let client: McpClientInfo
    @EnvironmentObject var mcp: MCPModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        let pending = mcp.pendingCount(for: client.key)
        HStack(spacing: 12) {
            RoundedRectangle(cornerRadius: 8).fill(p.accent.opacity(p.isDark ? 0.2 : 0.11)).frame(width: 30, height: 30)
                .overlay(Text(String(client.name.prefix(1)).uppercased()).font(.system(size: 13, weight: .bold)).foregroundStyle(p.accent))
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(client.name).font(.system(size: 13, weight: .semibold)).lineLimit(1)
                    if !client.version.isEmpty {
                        Text(verbatim: client.version).font(.system(size: 11.5, design: .monospaced)).foregroundStyle(p.tertiaryText)
                    }
                }
                Text(loc("mcp.client.lastActive", Fmt.relative(client.lastActiveMs, now: Clock.nowMs, loc: loc)))
                    .font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
            }
            Spacer(minLength: 8)
            Group {
                if pending > 0 {
                    Text(loc("mcp.client.pending", pending)).foregroundStyle(p.warning)
                } else if client.running {
                    HStack(spacing: 5) {
                        ProgressView().controlSize(.mini)
                        Text(loc("mcp.client.running")).foregroundStyle(p.info)
                    }
                } else {
                    Text(loc("mcp.client.idle")).foregroundStyle(p.tertiaryText)
                }
            }
            .font(.system(size: 12))
            Button(loc("mcp.client.stop")) { mcp.stopClient(client.key) }
                .buttonStyle(SecondaryButtonStyle(destructive: true))
                .disabled(!client.running && pending == 0)
                .help(loc("mcp.client.stop.help"))
        }
        .padding(.horizontal, 14).padding(.vertical, 10)
    }
}

// MARK: 操作记录

struct MCPRecordsSection: View {
    @EnvironmentObject var mcp: MCPModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 6) {
                SectionLabel(text: loc("mcp.section.records"))
                if !mcp.calls.isEmpty { Text("\(mcp.calls.count)").font(.system(size: 11).monospacedDigit()).foregroundStyle(p.tertiaryText) }
            }
            Card(padding: 0) {
                if mcp.calls.isEmpty {
                    Text(loc("mcp.records.empty")).font(.system(size: 12.5)).foregroundStyle(p.tertiaryText)
                        .padding(.horizontal, 14).padding(.vertical, 12)
                } else {
                    VStack(spacing: 0) {
                        ForEach(Array(mcp.calls.enumerated()), id: \.element.id) { i, c in
                            MCPRecordRow(call: c)
                            if i < mcp.calls.count - 1 { Rectangle().fill(p.separator).frame(height: 1) }
                        }
                    }
                }
            }
        }
    }
}

/// 一条操作记录：时间、客户端、目标、摘要、结果；点开看命令、理由、风险、输出
struct MCPRecordRow: View {
    @ObservedObject var call: MCPCall
    @EnvironmentObject var mcp: MCPModel
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var hover = false

    var body: some View {
        VStack(spacing: 0) {
            Button { call.expanded.toggle() } label: {
                HStack(spacing: 10) {
                    Text(MCPTime.text(call.at)).font(.system(size: 11.5).monospacedDigit()).foregroundStyle(p.tertiaryText)
                        .frame(width: 66, alignment: .leading)
                    Text(mcp.clientName(call.clientKey)).font(.system(size: 12, weight: .medium)).foregroundStyle(p.secondaryText)
                        .lineLimit(1).frame(width: 92, alignment: .leading)
                    Text(call.call.targetName.isEmpty ? "—" : call.call.targetName).font(.system(size: 12)).foregroundStyle(p.secondaryText)
                        .lineLimit(1).truncationMode(.middle).frame(width: 118, alignment: .leading)
                    Image(systemName: ToolRow.icon(call.call.tool)).font(.system(size: 11.5)).foregroundStyle(p.secondaryText).frame(width: 16)
                    Text(title).font(.system(size: 12.5)).foregroundStyle(p.text).lineLimit(1).truncationMode(.middle)
                    Spacer(minLength: 6)
                    MCPOutcomeChip(outcome: call.outcome)
                    Image(systemName: "chevron.right").font(.system(size: 8.5, weight: .bold)).foregroundStyle(p.tertiaryText)
                        .rotationEffect(.degrees(call.expanded ? 90 : 0))
                }
                .padding(.horizontal, 14).frame(height: 34)
                .background(hover ? p.hover : .clear)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .onHover { hover = $0 }
            .help(loc("ai.tool." + call.call.tool))
            if call.expanded {
                Rectangle().fill(p.separator).frame(height: 1)
                MCPCallDetails(call: call).padding(.horizontal, 14).padding(.vertical, 12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(p.isDark ? Color.black.opacity(0.1) : Color.black.opacity(0.015))
            }
        }
    }

    private var title: String {
        if call.isUseTarget { return loc("ai.useTarget.row", call.call.targetName) }
        let s = call.call.summary.replacingOccurrences(of: "\n", with: " ")
        return s.isEmpty ? loc("ai.tool." + call.call.tool) : s
    }
}

struct MCPOutcomeChip: View {
    let outcome: MCPCall.Outcome
    @Environment(\.palette) var p
    @Environment(\.loc) var loc

    var body: some View {
        let (key, color): (String, Color) = {
            switch outcome {
            case .pending: return ("mcp.result.pending", p.accent)
            case .running: return ("mcp.result.running", p.info)
            case .success: return ("mcp.result.success", p.success)
            case .failed: return ("mcp.result.failed", p.danger)
            case .denied: return ("mcp.result.denied", p.warning)
            case .cancelled: return ("mcp.result.cancelled", p.tertiaryText)
            }
        }()
        Text(loc(key))
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(color)
            .padding(.horizontal, 7).frame(height: 19)
            .background(Capsule().fill(color.opacity(0.13)))
            .fixedSize()
    }
}

/// 展开后的调用详情
struct MCPCallDetails: View {
    @ObservedObject var call: MCPCall
    @Environment(\.palette) var p
    @Environment(\.loc) var loc
    @State private var allOutput = false
    static let maxLines = 12

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            let command = call.decision.editedCommand ?? (call.call.detail.isEmpty ? call.call.summary : call.call.detail)
            if !command.isEmpty && !call.isSimpleApproval { CommandBlock(text: command, diff: ToolDetails.looksLikeDiff(call.call.tool, command)) }
            if call.isSimpleApproval && !call.call.detail.isEmpty {
                Text(call.call.detail).font(.system(size: 12)).foregroundStyle(p.secondaryText).textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if !call.call.modelReason.isEmpty { ReasonLine(text: call.call.modelReason) }
            if !call.call.riskReasons.isEmpty { RiskReasons(risk: call.call.risk, reasons: call.call.riskReasons) }
            if let img = call.call.image { MCPImage(img: img) }
            decisionNote
            if let r = call.result, !r.summary.isEmpty {
                let bad = r.denied || !r.ok
                HStack(spacing: 6) {
                    Image(systemName: bad ? "xmark.circle" : "checkmark.circle").foregroundStyle(bad ? p.danger : p.success)
                    Text(r.summary).foregroundStyle(bad ? p.danger : p.secondaryText).textSelection(.enabled)
                }
                .font(.system(size: 12))
            }
            if !call.output.isEmpty { output }
            if let img = call.result?.image { MCPImage(img: img) }
        }
    }

    @ViewBuilder private var decisionNote: some View {
        switch call.decision {
        case .approvedSimilar:
            Label(loc("mcp.decision.similar"), systemImage: "checkmark.circle").font(.system(size: 11.5)).foregroundStyle(p.tertiaryText)
        case .denied(let r?) where !r.isEmpty:
            Label(loc("ai.decision.denied") + (loc.isChinese ? "：" : ": ") + r, systemImage: "xmark.circle").font(.system(size: 11.5)).foregroundStyle(p.danger)
        default:
            EmptyView()
        }
    }

    private var output: some View {
        let trimmed = call.output.trimmingCharacters(in: .newlines)
        let lines = trimmed.split(separator: "\n", omittingEmptySubsequences: false)
        let over = lines.count > Self.maxLines
        let shown = allOutput || !over ? trimmed : lines.prefix(Self.maxLines).joined(separator: "\n")
        return VStack(alignment: .leading, spacing: 4) {
            ScrollView(.horizontal, showsIndicators: false) {
                Text(shown).font(.system(size: 11.5, design: .monospaced)).foregroundStyle(p.secondaryText)
                    .textSelection(.enabled).fixedSize().padding(9)
            }
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 7).fill(p.isDark ? Color.black.opacity(0.25) : Color.black.opacity(0.035)))
            if over {
                Button { allOutput.toggle() } label: {
                    Text(allOutput ? loc("ai.output.collapse") : loc("ai.output.all", lines.count)).font(.system(size: 11.5)).foregroundStyle(p.accent)
                }
                .buttonStyle(.plain)
            }
        }
    }
}

/// 记录里的时间：今天只显示时:分:秒，否则月-日 时:分
enum MCPTime {
    static func text(_ ms: Int64) -> String {
        let d = Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
        let now = Date(timeIntervalSince1970: TimeInterval(Clock.nowMs) / 1000)
        let f = DateFormatter()
        f.dateFormat = Calendar.current.isDate(d, inSameDayAs: now) ? "HH:mm:ss" : "MM-dd HH:mm"
        return f.string(from: d)
    }

    static func hm(_ ms: Int64) -> String {
        let f = DateFormatter()
        f.dateFormat = "HH:mm"
        return f.string(from: Date(timeIntervalSince1970: TimeInterval(ms) / 1000))
    }
}
