import AppKit
import SwiftUI

extension SelfTest {
    /// MCP 页（docs/mcp.md 第 5 节）：默认关闭；开启后的待批准与通知、角标；use_target / use_app / 要密码；
    /// 批准、拒绝后卡片从待批准里消失；停止客户端；令牌与四种配置；重新生成先确认；档位与限时到期；
    /// 用 mcp_history 补齐记录；端口被占用；关闭服务。
    static func checkMCP(prefix: String, zh: Bool, dark: Bool, app: AppModel, window: NSWindow, host: NSHostingView<RootView>) {
        let mcp = app.mcp
        let settings = app.settings
        guard let mock = mcp.mock else { problems.append("MCP：没有模拟服务"); return }
        MCPModel.executablePathOverride = "/Applications/LemonGalaxy.app/Contents/MacOS/LemonGalaxy"
        MCPModel.pasteboardOverride = ClipboardFiles.pasteboard
        let pageSize = CGSize(width: 860, height: 1400)
        func page(_ name: String) {
            shotView(name, app, dark: dark, size: pageSize) {
                // 定宽：内容高过画布时窗口跟着长高，不裁掉
                MCPPageContent().frame(width: pageSize.width - 48).padding(24).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            }
        }

        // 默认：关闭、端口 47821、危险操作确认
        if settings.mcpEnabled || mcp.running { problems.append("MCP 服务默认应关闭") }
        if mcpDefaultPort() != 47821 || settings.mcpPort != 47821 { problems.append("MCP 默认端口应为 47821：\(settings.mcpPort)") }
        if mcp.mode != .confirmDangerous { problems.append("MCP 默认档位应为危险操作确认：\(mcp.mode)") }
        if !NavSection.allCases.elementsEqual([.hosts, .sftp, .forwards, .ai, .mcp, .snippets, .local, .settings]) {
            problems.append("侧栏「MCP」应在「AI 智能体」后面")
        }
        app.selectHome(.mcp)
        shot("\(prefix)-59a-mcp-off", app, window: window, host: host)

        // 开启：不在 MCP 页时来了待批准，应发通知；角标 = 待批准数
        app.selectHome(.hosts)
        let notified = mcp.notifyCount
        mcp.setEnabled(true)
        wait("MCP 服务开启") { mcp.running && !mcp.busy }
        wait("MCP 待批准的危险命令") { mcp.pendingCalls.count == 1 && mcp.calls.count == 7 }
        if !settings.mcpEnabled { problems.append("MCP 开关没有记进设置") }
        if mcp.notifyCount != notified + 1 { problems.append("不在 MCP 页时来了待批准应发通知：\(mcp.notifyCount - notified) 次") }
        if mcp.pendingCount != 1 { problems.append("MCP 角标应为 1：\(mcp.pendingCount)") }
        if mcp.status.clients.map(\.key) != ["claude-code"] { problems.append("MCP 已连接的客户端应只有 claude-code：\(mcp.status.clients.map(\.key))") }
        let outcomes = Dictionary(mcp.calls.map { ($0.call.tool, $0.outcome) }, uniquingKeysWith: { a, _ in a })
        if outcomes["list_devices"] != .success || outcomes["screenshot"] != .denied || outcomes["read_file"] != .failed || outcomes["write_file"] != .cancelled {
            problems.append("MCP 操作记录的结果不对：\(outcomes)")
        }
        if mcp.calls.first?.isPending != true { problems.append("MCP 操作记录应最新的在上") }
        app.selectHome(.mcp)
        shot("\(prefix)-59b-mcp-pending", app, window: window, host: host)

        // 其余三种：use_target（另一个客户端）、use_app、要密码
        mock.simulate(.useTarget)
        mock.simulate(.useApp)
        mock.simulate(.secret)
        wait("MCP 其余待批准") { mcp.pendingCalls.count == 3 && mcp.secrets.count == 1 }
        if mcp.pendingCount != 4 { problems.append("MCP 角标应为 4：\(mcp.pendingCount)") }
        if mcp.secrets.first?.callId == nil { problems.append("要密码没有对上它的 request_secret 调用") }
        if mcp.pendingCalls.first?.call.tool != "run_command" { problems.append("待批准应先来的在上：\(mcp.pendingCalls.map { "\($0.call.tool)#\($0.order)" }) 全部 \(mcp.calls.map { "\($0.call.tool)#\($0.order)" })") }
        if mcp.status.clients.count != 2 { problems.append("MCP 应有两个客户端：\(mcp.status.clients.map(\.key))") }
        page("\(prefix)-59c-mcp-page-pending")

        // 允许一次：卡片立刻从待批准里消失，执行完记录为成功
        if let danger = mcp.pendingCalls.first(where: { $0.call.tool == "run_command" }) {
            mcp.decide(danger, .allowOnce)
            if mcp.pendingCalls.contains(where: { $0 === danger }) { problems.append("点「允许一次」后卡片应从待批准里消失") }
            if mcp.pendingCount != 3 { problems.append("批准一条后角标应为 3：\(mcp.pendingCount)") }
            wait("危险命令执行完") { danger.outcome == .success }
            danger.expanded = true
        } else { problems.append("找不到待批准的危险命令") }
        // use_app 拒绝（带理由）
        if let useApp = mcp.pendingCalls.first(where: \.isUseApp) {
            useApp.denying = true
            useApp.denyReason = zh ? "现在不要动浏览器" : "Leave the browser alone for now"
            mcp.decide(useApp, .deny(reason: useApp.denyReason))
            wait("拒绝操作应用") { useApp.outcome == .denied }
            if case .denied(let r) = useApp.decision, r == nil { problems.append("拒绝理由没有带上") }
        } else { problems.append("找不到 use_app 的批准卡片") }
        // use_target 允许
        if let useTarget = mcp.pendingCalls.first(where: \.isUseTarget) {
            if useTarget.clientKey != "codex" { problems.append("use_target 应来自 codex：\(useTarget.clientKey)") }
            mcp.decide(useTarget, .allowOnce)
            wait("允许连接设备") { useTarget.outcome == .success }
        } else { problems.append("找不到 use_target 的批准卡片") }
        // 要密码：提交后卡片收起，所属调用成功
        if let s = mcp.secrets.first {
            let owner = mcp.calls.first { $0.call.callId == s.callId }
            mcp.provideSecret(s, "selftest-secret")
            wait("密码交给内核") { owner?.outcome == .success }
        }
        wait("MCP 待批准全部处理完") { mcp.pendingCount == 0 }
        if mcp.calls.contains(where: { $0.output.contains("selftest-secret") || $0.call.summary.contains("selftest-secret") }) {
            problems.append("密码不应出现在操作记录里")
        }

        // 停止客户端：它等批准的步骤被拒绝
        mock.simulate(.dangerousCommand)
        wait("再来一条待批准") { mcp.pendingCount == 1 }
        let stopped = mcp.pendingCalls.first
        mcp.stopClient("claude-code")
        wait("停止客户端后待批准被拒绝") { mcp.pendingCount == 0 && stopped?.outcome == .denied }
        app.toast = nil

        // 令牌与四种配置
        let token = mcp.token
        let url = mcp.url
        if !token.hasPrefix("lgm_") || url != "http://127.0.0.1:47821/mcp" { problems.append("MCP 令牌或地址不对：\(token) \(url)") }
        let cc = mcp.configText(.claudeCode)
        if cc != "claude mcp add --transport http --scope user lemongalaxy \(url) --header \"Authorization: Bearer \(token)\"" {
            problems.append("Claude Code 配置不对：\(cc)")
        }
        let codex = mcp.configText(.codex)
        if codex != "[mcp_servers.lemongalaxy]\nurl = \"\(url)\"\nhttp_headers = { \"Authorization\" = \"Bearer \(token)\" }" {
            problems.append("Codex 配置不对：\(codex)")
        }
        func server(_ text: String) -> [String: Any]? {
            guard let o = try? JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any],
                  let s = o["mcpServers"] as? [String: Any] else { return nil }
            return s["lemongalaxy"] as? [String: Any]
        }
        let cursor = server(mcp.configText(.cursor))
        if cursor?["url"] as? String != url || (cursor?["headers"] as? [String: String])?["Authorization"] != "Bearer " + token {
            problems.append("Cursor 配置不对：\(mcp.configText(.cursor))")
        }
        let desktop = server(mcp.configText(.claudeDesktop))
        if desktop?["command"] as? String != MCPModel.executablePathOverride || desktop?["args"] as? [String] != ["--mcp-stdio"] {
            problems.append("Claude Desktop 配置不对：\(mcp.configText(.claudeDesktop))")
        }
        mcp.copyConfig(.codex)
        if ClipboardFiles.pasteboard.string(forType: .string) != codex { problems.append("复制的 Codex 配置不对") }
        if !mcp.maskedToken.hasPrefix("lgm_") || mcp.maskedToken.contains(String(token.dropFirst(4).prefix(6))) { problems.append("令牌没有遮住") }
        app.toast = nil
        // 重新生成：先确认
        mcp.askRegenerate()
        if case .confirm(let req)? = app.modal {
            shot("\(prefix)-59d-mcp-regenerate", app, window: window, host: host)
            app.modal = nil
            req.action()
            wait("重新生成令牌") { mcp.token != token && mcp.token.hasPrefix("lgm_") }
        } else { problems.append("重新生成令牌应先确认") }
        app.toast = nil

        // 档位：完全信任 1 小时 → 设置与内核都收到
        mcp.setMode(.fullTrust, minutes: 60)
        if mock.currentMode != .fullTrust || mock.lastTrustMinutes != 60 { problems.append("完全信任 1 小时没有交给内核：\(mock.currentMode) \(String(describing: mock.lastTrustMinutes))") }
        if settings.mcpMode != "fullTrust" || settings.mcpTrustMinutes != 60 || mcp.trustUntilMs != Clock.nowMs + 3_600_000 {
            problems.append("完全信任的时限没有记进设置")
        }
        mcp.tokenShown = true
        mcp.configKind = .codex
        page("\(prefix)-59e-mcp-page")
        // 中间列点「操作记录」：页面滚到那一段
        mcp.navSelection = .records
        mcp.scrollTarget = .records
        shot("\(prefix)-59f-mcp-records", app, window: window, host: host, settle: 0.5)
        // CI 用 --only 只出部分截图，没截这张时 shot 直接返回不泵运行循环：这里自己等界面处理滚动
        wait("中间列点分区后页面滚动") { mcp.scrollTarget == nil }
        mcp.navSelection = .service
        mcp.tokenShown = false
        mcp.configKind = .claudeCode
        // 到时回到危险操作确认
        if let frozen = Clock.frozenMs {
            Clock.frozenMs = frozen + 61 * 60_000
            mcp.expireTrustIfDue()
            Clock.frozenMs = frozen
            if mcp.mode != .confirmDangerous || mock.currentMode != .confirmDangerous || settings.mcpTrustUntil != 0 {
                problems.append("限时完全信任到期后应回到危险操作确认：\(mcp.mode)")
            }
            app.toast = nil
        }

        // 界面重开：用 mcp_history 补齐
        let expected = mcp.status.clients.reduce(0) { $0 + mock.history($1.key).count }
        mcp.didLogout()
        if !mcp.calls.isEmpty { problems.append("清空后不应还有记录") }
        mcp.pageAppeared()
        if mcp.calls.count != expected || expected == 0 { problems.append("用 mcp_history 补齐的记录数不对：\(mcp.calls.count)，应为 \(expected)") }
        if !mcp.calls.contains(where: { $0.outcome == .denied }) || !mcp.calls.contains(where: { $0.outcome == .success }) {
            problems.append("补齐的记录没有带上结果")
        }

        // 端口：不合法的不让应用；被占用时显示原因
        mcp.portText = "80"
        if mcp.portValue != nil || mcp.portChanged { problems.append("端口 80 不应能应用") }
        mcp.portText = "\(MockMcpService.busyPort)"
        mcp.applyPort()
        wait("端口被占用") { !mcp.busy && mcp.startError != nil }
        if mcp.running || settings.mcpPort != Int(MockMcpService.busyPort) { problems.append("端口被占用时服务不应在运行") }
        if mcp.startError?.contains("\(MockMcpService.busyPort)") != true { problems.append("开启失败的原因应提到端口：\(mcp.startError ?? "")") }
        mcp.scrollTarget = .service
        shot("\(prefix)-59g-mcp-port-busy", app, window: window, host: host, settle: 0.5)
        mcp.portText = "47821"
        mcp.applyPort()
        wait("换回默认端口") { mcp.running && !mcp.busy }
        if mcp.startError != nil { problems.append("换回端口后错误没有清掉") }

        // 关闭：服务停止、客户端清空、待批准作废
        mock.simulate(.dangerousCommand)
        wait("关闭前的待批准") { mcp.pendingCount == 1 }
        // 记录最多 200 条：旧的先删，待批准的不删
        for _ in 0..<(MCPModel.maxRecords + 10) { mock.simulate(.cancelled) }
        wait("操作记录裁到 200 条") { mcp.calls.count == MCPModel.maxRecords && mcp.calls.first?.outcome == .cancelled }
        pump(0.2)
        if mcp.calls.count != MCPModel.maxRecords || mcp.pendingCalls.count != 1 { problems.append("操作记录应最多 200 条且保留待批准：\(mcp.calls.count)") }
        mcp.setEnabled(false)
        if mcp.running || settings.mcpEnabled { problems.append("关闭后服务仍在运行") }
        if mcp.pendingCount != 0 || !mcp.status.clients.isEmpty { problems.append("关闭后不应还有待批准或客户端：\(mcp.pendingCount)") }
        pump(0.2)
        settings.mcpPort = 47821
        settings.mcpMode = "confirmDangerous"
        settings.mcpTrustMinutes = 60
        app.toast = nil
    }
}
