import Foundation

/// 模拟本机 MCP 服务（docs/mcp.md 第 4 节）。开启后有一个已连接的客户端 claude-code、一条待批准的危险命令和几条操作记录；
/// 事件与状态经 McpListener 在自己的串行队列上回调（和真内核一样不在主线程），界面要自己切回主线程。
/// 端口 8080 模拟「被别的程序占用」，用来看开启失败的样子。
final class MockMcpService: @unchecked Sendable {
    /// 模拟被占用的端口
    static let busyPort: UInt16 = 8080

    private let zh: Bool
    private let lock = NSLock()
    private let queue = DispatchQueue(label: "mock.mcp.events")
    private var listener: McpListener?
    private var running = false
    private var port: UInt16 = 47821
    private var token = ""
    private var mode: AiMode = .confirmDangerous
    private var error: String?
    private var clients: [String: Client] = [:]
    private var seeded = false
    private var seq = 0
    /// selftest 核对：最近一次调整档位时的时限
    private(set) var lastTrustMinutes: UInt32?

    private final class Client {
        var info: McpClientInfo
        var history: [AgentMessage] = []
        /// 等批准的步骤：call_id → 调用
        var approvals: [String: ToolCallView] = [:]
        /// 等输入的密码：request_id → 所属 request_secret 调用
        var secrets: [String: ToolCallView] = [:]
        init(_ info: McpClientInfo) { self.info = info }
    }

    init(zh: Bool) {
        self.zh = zh
        token = "lgm_" + "Q2xhdWRlQ29kZS1tb2NrLXRva2VuLWxlbW9uZ2FsYXh5"
    }

    // MARK: 服务

    func start(_ config: McpConfig, _ l: McpListener) throws -> McpStatus {
        let p = config.port == 0 ? 47821 : config.port
        // 重启：先关掉旧的，已连的客户端随旧服务一起断开（与真内核一致）
        let old: [String] = lock.locked {
            listener = l
            return Array(clients.keys)
        }
        for k in old { stopClient(k, drop: true) }
        lock.lock()
        port = p
        mode = config.mode
        lastTrustMinutes = config.trustMinutes
        if p == Self.busyPort {
            running = false
            let msg = zh ? "绑定 127.0.0.1:\(p) 失败：地址已被占用（端口 \(p) 可能被别的程序占用，可以换一个端口）"
                         : "Could not bind 127.0.0.1:\(p): address already in use (port \(p) may be used by another program; try another port)"
            error = msg
            lock.unlock()
            emitStatus()
            throw GalaxyError.Other(detail: msg)
        }
        running = true
        error = nil
        lock.unlock()
        let first = lock.locked { () -> Bool in
            let f = !seeded
            seeded = true
            return f
        }
        if first { seed() }
        emitStatus()
        return status()
    }

    func stop() {
        let keys: [String] = lock.locked {
            guard running else { return [] }
            running = false
            return Array(clients.keys)
        }
        for k in keys { stopClient(k, drop: true) }
        emitStatus()
    }

    func status() -> McpStatus {
        lock.lock(); defer { lock.unlock() }
        let list = clients.values.map(\.info).sorted { $0.lastActiveMs > $1.lastActiveMs }
        return McpStatus(supported: true, running: running, url: running ? "http://127.0.0.1:\(port)/mcp" : "", port: port, token: token,
                         endpointFile: NSHomeDirectory() + "/Library/Application Support/LemonGalaxy/mcp-endpoint.json",
                         mode: mode, error: error, clients: running ? list : [])
    }

    func regenerate() -> McpStatus {
        let alphabet = Array("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_")
        lock.locked { token = "lgm_" + String((0..<43).map { _ in alphabet[Int.random(in: 0..<alphabet.count)] }) }
        emitStatus()
        return status()
    }

    func setMode(_ m: AiMode, _ trust: UInt32?) {
        lock.locked { mode = m; lastTrustMinutes = trust }
        emitStatus()
    }

    var currentMode: AiMode { lock.locked { mode } }

    func history(_ key: String) -> [AgentMessage] { lock.locked { clients[key]?.history ?? [] } }

    /// 退出登录：外部客户端的智能体全部作废，服务继续开着
    func resetClients() {
        let keys = lock.locked { Array(clients.keys) }
        for k in keys { stopClient(k, drop: true) }
        emitStatus()
    }

    // MARK: 批准与输入

    func decide(_ key: String, _ callId: String, _ d: ApprovalDecision) {
        let call: ToolCallView? = lock.locked { clients[key]?.approvals.removeValue(forKey: callId) }
        guard let call else { return }
        switch d {
        case .deny(let reason):
            finish(key, call, ToolResultView(ok: false, summary: zh ? "用户拒绝" : "Denied by user", output: reason ?? "", image: nil, denied: true))
        case .allowOnce, .allowSimilar, .editAndRun:
            emit(key, .toolStarted(callId: call.callId))
            switch call.tool {
            case "use_target": finish(key, call, ToolResultView(ok: true, summary: zh ? "已连接" : "Connected", output: "", image: nil, denied: false))
            case "use_app": finish(key, call, ToolResultView(ok: true, summary: zh ? "已允许" : "Allowed", output: "", image: nil, denied: false))
            default:
                var out = ""
                if case .editAndRun(let c) = d { out = "$ " + c + "\n" }
                out += call.tool == "run_command" ? "removed '/data/backups/2026-08-01.tar.gz'\nremoved '/data/backups/2026-08-15.tar.gz'\nremoved '/data/backups/2026-09-01.tar.gz'\n" : ""
                if !out.isEmpty { emit(key, .toolOutput(callId: call.callId, text: out)) }
                finish(key, call, ToolResultView(ok: true, summary: zh ? "退出码 0" : "Exit code 0", output: out, image: nil, denied: false))
            }
        }
        settle(key)
    }

    func provideSecret(_ key: String, _ requestId: String, _ secret: String) {
        let call: ToolCallView? = lock.locked { clients[key]?.secrets.removeValue(forKey: requestId) }
        guard let call else { return }
        if secret.isEmpty {
            finish(key, call, ToolResultView(ok: false, summary: zh ? "用户没有输入" : "Nothing was entered", output: "", image: nil, denied: false))
        } else {
            finish(key, call, ToolResultView(ok: true, summary: zh ? "已输入" : "Entered", output: "", image: nil, denied: false))
        }
        settle(key)
    }

    /// 停下某个客户端：等批准的拒绝、等输入的作废、正在跑的停止。drop 为真时连客户端一起移除（服务关闭、重启、退出登录）
    func stopClient(_ key: String, drop: Bool = false) {
        let (approvals, secrets): ([ToolCallView], [ToolCallView]) = lock.locked {
            guard let c = clients[key] else { return ([], []) }
            let a = Array(c.approvals.values), s = Array(c.secrets.values)
            c.approvals.removeAll()
            c.secrets.removeAll()
            return (a, s)
        }
        for call in approvals {
            finish(key, call, ToolResultView(ok: false, summary: zh ? "用户拒绝" : "Denied by user", output: zh ? "用户停止了任务" : "The user stopped the task", image: nil, denied: true))
        }
        for call in secrets {
            finish(key, call, ToolResultView(ok: false, summary: zh ? "用户没有输入" : "Nothing was entered", output: "", image: nil, denied: false))
        }
        settle(key)
        if drop { lock.locked { _ = clients.removeValue(forKey: key) } }
        emitStatus()
    }

    // MARK: 模拟外部调用（演示与 selftest）

    enum Simulated { case dangerousCommand, useTarget, useApp, secret, cancelled }

    func simulate(_ kind: Simulated) {
        guard lock.locked({ running }) else { return }
        switch kind {
        case .dangerousCommand:
            ask("claude-code", tool: "run_command", target: ("dev-build", buildName), summary: "systemctl restart nginx",
                detail: "sudo systemctl restart nginx", risk: .dangerous,
                reasons: [zh ? "以管理员身份执行（sudo）" : "Runs as administrator (sudo)", zh ? "会中断正在处理的请求" : "Interrupts requests in flight"],
                reason: zh ? "配置已经改好，重启后生效" : "The config is updated and needs a restart")
        case .useTarget:
            ask("codex", tool: "use_target", target: ("dev-office", officeName), summary: zh ? "连接 \(officeName)" : "Connect to \(officeName)",
                detail: zh ? "允许后，这个客户端在本次连接期间可以操作 \(officeName)" : "Once allowed, this client can use \(officeName) until it disconnects",
                risk: .normal, reasons: [], reason: zh ? "要在这台电脑上导出本月的报表" : "Needs to export this month's report on that PC")
        case .useApp:
            ask("claude-code", tool: "use_app", target: ("local", localName), summary: zh ? "操作「Safari」" : "Use “Safari”",
                detail: zh ? "允许后，这个客户端可以读取「Safari」的界面并在里面点击、输入（不会占用你的鼠标键盘）"
                           : "Once allowed, this client can read Safari's window and click and type in it (without taking over your mouse and keyboard)",
                risk: .normal, reasons: [zh ? "这个客户端第一次操作这个应用" : "First time this client uses this app"],
                reason: zh ? "要在内网页面里查值班表" : "Needs to look up the on-call rota on the intranet")
        case .secret:
            let key = "claude-code"
            let call = makeCall(tool: "request_secret", target: ("dev-build", buildName), summary: zh ? "请用户输入 sudo 密码" : "Ask the user for the sudo password",
                                detail: "", risk: .normal, reasons: [], reason: "")
            ensureClient(key)
            record(key, call)
            emit(key, .status(running: true, step: 0))
            emit(key, .toolProposed(call: call, needsApproval: false))
            emit(key, .toolStarted(callId: call.callId))
            let rid = "sec-" + call.callId
            lock.locked { clients[key]?.secrets[rid] = call }
            emit(key, .secretRequested(requestId: rid, prompt: zh ? "deploy 在 build-server-01 上的 sudo 密码" : "sudo password for deploy on build-server-01", targetId: "dev-build"))
            settle(key)
        case .cancelled:
            // 客户端断开：挂着的确认撤销，卡片标成「调用方已取消」
            let key = "claude-code"
            let call = makeCall(tool: "write_file", target: ("dev-build", buildName), summary: zh ? "写入 /etc/nginx/conf.d/app.conf" : "Write /etc/nginx/conf.d/app.conf",
                                detail: "--- /etc/nginx/conf.d/app.conf\n+++ /etc/nginx/conf.d/app.conf\n@@ -3 +3 @@\n-    proxy_read_timeout 30s;\n+    proxy_read_timeout 120s;",
                                risk: .dangerous, reasons: [zh ? "修改系统配置文件" : "Changes a system configuration file"], reason: "")
            ensureClient(key)
            record(key, call)
            emit(key, .toolProposed(call: call, needsApproval: false))
            finish(key, call, ToolResultView(ok: false, summary: "调用方已取消", output: "", image: nil, denied: true))
        }
    }

    // MARK: 内部

    private var buildName: String { zh ? "构建服务器" : "Build server" }
    private var officeName: String { zh ? "办公室电脑" : "Office PC" }
    private var localName: String { zh ? "本机（Alice-MacBook-Pro）" : "This Mac (Alice-MacBook-Pro)" }

    private func nextId() -> String {
        lock.locked { seq += 1; return String(format: "mcp%09d", seq) }
    }

    private func makeCall(tool: String, target: (String, String), summary: String, detail: String, risk: RiskLevel, reasons: [String], reason: String) -> ToolCallView {
        ToolCallView(callId: nextId(), tool: tool, targetId: target.0, targetName: target.1, summary: summary, detail: detail,
                     risk: risk, riskReasons: reasons, modelReason: reason, image: nil)
    }

    private func ensureClient(_ key: String) {
        lock.locked {
            if clients[key] == nil {
                let version = key == "codex" ? "0.63.0" : "2.1.30"
                clients[key] = Client(McpClientInfo(key: key, name: key, version: version, lastActiveMs: MockData.now, running: false, pendingApprovals: 0))
            }
            clients[key]?.info.lastActiveMs = MockData.now
        }
    }

    private func record(_ key: String, _ call: ToolCallView) {
        lock.locked { clients[key]?.history.append(.tool(call: call, result: nil)) }
    }

    /// 提出一个要批准的调用
    private func ask(_ key: String, tool: String, target: (String, String), summary: String, detail: String, risk: RiskLevel, reasons: [String], reason: String) {
        ensureClient(key)
        let call = makeCall(tool: tool, target: target, summary: summary, detail: detail, risk: risk, reasons: reasons, reason: reason)
        record(key, call)
        lock.locked { clients[key]?.approvals[call.callId] = call }
        emit(key, .status(running: true, step: 0))
        emit(key, .toolProposed(call: call, needsApproval: true))
        settle(key)
    }

    private func finish(_ key: String, _ call: ToolCallView, _ r: ToolResultView) {
        lock.locked {
            guard let c = clients[key] else { return }
            if let i = c.history.lastIndex(where: { if case .tool(let cv, _) = $0 { return cv.callId == call.callId }; return false }) {
                c.history[i] = .tool(call: call, result: r)
            }
        }
        emit(key, .toolFinished(callId: call.callId, result: r))
    }

    /// 重新算「正在执行 / 待批准」，没事可做时发 Status(running: false)
    private func settle(_ key: String) {
        let idle: Bool = lock.locked {
            guard let c = clients[key] else { return false }
            let busy = !c.approvals.isEmpty || !c.secrets.isEmpty
            let changed = c.info.running != busy
            c.info.running = busy
            c.info.pendingApprovals = UInt32(c.approvals.count)
            return changed && !busy
        }
        if idle { emit(key, .status(running: false, step: 0)) }
        emitStatus()
    }

    private func emit(_ key: String, _ e: AgentEvent) {
        let l = lock.locked { listener }
        queue.async { l?.onEvent(clientKey: key, event: e) }
    }

    private func emitStatus() {
        let l = lock.locked { listener }
        let s = status()
        queue.async { l?.onStatus(status: s) }
    }

    /// 第一次开启：claude-code 已经调用过几次，最后一条是等批准的危险命令
    private func seed() {
        let key = "claude-code"
        ensureClient(key)
        func done(_ tool: String, _ target: (String, String), _ summary: String, _ detail: String, risk: RiskLevel = .read,
                  reasons: [String] = [], reason: String = "", result: ToolResultView) {
            let call = makeCall(tool: tool, target: target, summary: summary, detail: detail, risk: risk, reasons: reasons, reason: reason)
            record(key, call)
            emit(key, .toolProposed(call: call, needsApproval: false))
            if !result.denied { emit(key, .toolStarted(callId: call.callId)) }
            finish(key, call, result)
        }
        emit(key, .status(running: true, step: 0))
        done("list_devices", ("", ""), zh ? "查看设备与主机" : "List devices and hosts", "",
             result: ToolResultView(ok: true, summary: zh ? "6 台设备、4 台 SSH 主机" : "6 devices, 4 SSH hosts", output: "", image: nil, denied: false))
        done("run_command", ("dev-build", buildName), "df -h /data", "df -h /data",
             reason: zh ? "看看 /data 还剩多少空间" : "Check how much space is left on /data",
             result: ToolResultView(ok: true, summary: zh ? "退出码 0" : "Exit code 0",
                                    output: "Filesystem      Size  Used Avail Use% Mounted on\n/dev/sdb1       1.8T  1.7T   98G  95% /data\n", image: nil, denied: false))
        done("list_dir", ("dev-build", buildName), zh ? "列出 /data/backups" : "List /data/backups", "/data/backups",
             result: ToolResultView(ok: true, summary: zh ? "38 项" : "38 entries", output: "", image: nil, denied: false))
        done("screenshot", ("dev-office", officeName), zh ? "截图" : "Screenshot", "", risk: .read,
             result: ToolResultView(ok: false, summary: zh ? "用户拒绝" : "Denied by user", output: zh ? "现在有人在用这台电脑" : "Someone is using that PC right now", image: nil, denied: true))
        done("read_file", ("s-web1", "web-01"), zh ? "读取 /var/log/nginx/error.log" : "Read /var/log/nginx/error.log", "/var/log/nginx/error.log",
             result: ToolResultView(ok: false, summary: zh ? "权限不够：/var/log/nginx/error.log" : "Permission denied: /var/log/nginx/error.log", output: "", image: nil, denied: false))
        simulate(.cancelled)
        ask(key, tool: "run_command", target: ("dev-build", buildName), summary: "rm -rf /data/backups/2026-0*",
            detail: "rm -rf /data/backups/2026-0*", risk: .dangerous,
            reasons: [zh ? "递归强制删除（rm -rf）" : "Recursive forced delete (rm -rf)", zh ? "通配符可能匹配到比预期更多的文件" : "The wildcard may match more files than expected"],
            reason: zh ? "清理 10 月以前的旧备份，给 /data 腾出空间" : "Remove backups from before October to free up /data")
    }
}
