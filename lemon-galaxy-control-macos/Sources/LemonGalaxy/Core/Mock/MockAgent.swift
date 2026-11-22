import Foundation

/// 模拟 AI 智能体：按脚本流式吐文字、提出工具调用、按权限档位决定是否要确认、等用户回应确认卡片 / 提问 / 密码。
/// 聊天式：消息里引用的设备直接用；没引用设备时先 list_devices 再用 use_target 请用户同意连接。
/// 事件全部走 AgentListener.on_event，和真内核一致。
final class MockAgentConversation: AgentConversation, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("模拟对象不能由内核指针构造") }
    private let client: MockGalaxyClient
    private var config: ConversationConfig
    /// 本对话正在用的设备（引用过或用户同意连接过）
    private var target: AgentTarget?
    private var authorized: Set<String> = []
    var listener: AgentListener?
    private let convId: String
    private let lock = NSLock()
    private var history: [AgentMessage] = []
    private var mode: AiMode
    private var trustUntil: Date?
    private var task: Task<Void, Never>?
    private var approvals: [String: CheckedContinuation<ApprovalDecision, Never>] = [:]
    private var answers: [String: CheckedContinuation<String, Never>] = [:]
    private var secrets: [String: CheckedContinuation<String?, Never>] = [:]
    private var allowSimilar: Set<String> = []
    private var promptTokens: UInt64 = 0
    private var completionTokens: UInt64 = 0
    private var titleText = ""
    private var updatedAt = MockData.now
    private var step: UInt32 = 0
    private let events = DispatchQueue(label: "mock.agent.events")

    init(client: MockGalaxyClient, config: ConversationConfig, listener: AgentListener) {
        self.client = client
        self.config = config
        self.listener = listener
        self.mode = config.mode
        self.trustUntil = config.trustMinutes.map { Date().addingTimeInterval(TimeInterval($0) * 60) }
        self.titleText = config.title ?? ""
        self.convId = "conv-" + UUID().uuidString.prefix(8).lowercased()
        self.target = config.targets.first
        self.authorized = Set(config.targets.map(\.id))
        super.init(noHandle: NoHandle())
    }

    private var zh: Bool { client.zh }

    override func id() -> String { convId }
    override func title() -> String { lock.locked { titleText } }
    override func setForeground(foreground: Bool) { if !foreground { stop() } }
    override func messages() -> [AgentMessage] { lock.lock(); defer { lock.unlock() }; return history }

    func summary() -> ConversationSummary {
        lock.lock(); defer { lock.unlock() }
        return ConversationSummary(id: convId, title: titleText.isEmpty ? (zh ? "新对话" : "New conversation") : titleText,
                                   modelId: config.modelId, targetNames: target.map { [client.targetName($0)] } ?? [], updatedAt: updatedAt, config: config)
    }

    private func emit(_ e: AgentEvent) {
        let l = listener
        events.async { l?.onEvent(event: e) }
    }

    private func record(_ m: AgentMessage) {
        lock.locked { history.append(m); updatedAt = MockData.now }
    }

    private func pause(_ ms: Int) async { await client.delay(ms) }

    override func send(text: String) { sendWithRefs(text: text, refs: []) }

    override func sendWithRefs(text: String, refs: [ContextRef]) {
        lock.locked { if titleText.isEmpty { titleText = String((text.isEmpty ? refs.map(\.label).joined(separator: ", ") : text).prefix(40)) } }
        record(.user(text: text, refs: refs))
        task?.cancel()
        task = Task { [weak self] in
            guard let self else { return }
            await self.runScript(userText: text, refs: refs)
        }
    }

    override func model() -> ConversationModel {
        lock.locked { ConversationModel(providerId: config.providerId, modelId: config.modelId) }
    }

    override func setModel(providerId: UInt64, modelId: String) throws {
        guard !modelId.isEmpty else { throw GalaxyError.NotFound(detail: zh ? "模型不存在" : "Model not found") }
        lock.locked {
            config.providerId = providerId
            config.modelId = modelId
        }
    }

    override func decide(callId: String, decision: ApprovalDecision) {
        let c = lock.locked { approvals.removeValue(forKey: callId) }
        c?.resume(returning: decision)
    }

    override func answer(questionId: String, answer: String) {
        let c = lock.locked { answers.removeValue(forKey: questionId) }
        c?.resume(returning: answer)
    }

    override func provideSecret(requestId: String, secret: String) {
        let c = lock.locked { secrets.removeValue(forKey: requestId) }
        // 密码只交给被控端，不进模型上下文、不进历史
        c?.resume(returning: secret.isEmpty ? nil : secret)
    }

    override func setMode(mode: AiMode, trustMinutes: UInt32?) {
        let until = trustMinutes.map { Date().addingTimeInterval(TimeInterval($0) * 60) }
        lock.locked {
            self.mode = mode
            trustUntil = until
        }
        // 实际档位 = min(界面选择, 管理员上限, 被控端上限)；模拟内核的上限都是完全信任
        emit(.modeChanged(mode: mode, effective: mode, untilMs: until.map { Int64($0.timeIntervalSince1970 * 1000) }))
    }

    override func stop() {
        task?.cancel()
        lock.lock()
        let a = approvals; approvals.removeAll()
        let q = answers; answers.removeAll()
        let s = secrets; secrets.removeAll()
        lock.unlock()
        a.values.forEach { $0.resume(returning: .deny(reason: nil)) }
        q.values.forEach { $0.resume(returning: "") }
        s.values.forEach { $0.resume(returning: nil) }
        emit(.status(running: false, step: step))
    }

    // MARK: 脚本

    private var current: AgentTarget { target ?? AgentTarget(kind: .device, id: "dev-build") }
    private var targetName: String { client.targetName(current) }
    private var isGui: Bool { [.windows, .macos].contains(client.targetPlatform(current)) }

    private func say(_ text: String) async {
        let id = "a-" + UUID().uuidString.prefix(6)
        var acc = ""
        // 按 4 个字符一段流式输出
        var chunk = ""
        for ch in text {
            chunk.append(ch)
            if chunk.count >= 4 {
                emit(.assistantDelta(messageId: id, text: chunk)); acc += chunk; chunk = ""
                await pause(30)
                if Task.isCancelled { return }
            }
        }
        if !chunk.isEmpty { emit(.assistantDelta(messageId: id, text: chunk)); acc += chunk }
        emit(.assistantDone(messageId: id))
        record(.assistant(id: id, text: acc, thinking: ""))
        completionTokens += UInt64(acc.count * 2)
        promptTokens += 1_800
        emit(.usage(promptTokens: promptTokens, completionTokens: completionTokens))
    }

    private func needsApproval(_ risk: RiskLevel, tool: String) -> Bool? {
        let (m, until, similar) = lock.locked { (mode, trustUntil, allowSimilar.contains(tool + String(describing: risk))) }
        if risk == .forbidden { return nil }
        if similar { return false }
        var effective = m
        if effective == .fullTrust, let until, until < Date() { effective = .confirmDangerous }
        switch effective {
        case .off: return nil
        case .readOnly: return risk == .read ? false : nil
        case .confirmAll: return risk != .read
        case .confirmDangerous: return risk == .dangerous || risk == .unknown
        case .fullTrust: return false
        }
    }

    /// 返回 nil 表示被拒绝
    private func tool(_ tool: String, summary: String, detail: String = "", risk: RiskLevel, reasons: [String] = [], why: String,
                      output: String, resultSummary: String, image: ImageData? = nil, resultImage: ImageData? = nil, accountLevel: Bool = false) async -> String? {
        step += 1
        emit(.status(running: true, step: step))
        let callId = "call-" + UUID().uuidString.prefix(6)
        let call = ToolCallView(callId: callId, tool: tool, targetId: accountLevel ? "" : current.id, targetName: accountLevel ? "" : targetName, summary: summary,
                                detail: detail.isEmpty ? summary : detail, risk: risk, riskReasons: reasons, modelReason: why, image: image)
        let approval = needsApproval(risk, tool: tool)
        emit(.toolProposed(call: call, needsApproval: approval == true))
        var finalCommand = summary
        if approval == nil {
            let r = ToolResultView(ok: false, summary: risk == .forbidden ? (zh ? "绝对禁止的操作，已拒绝" : "Forbidden operation, refused")
                                   : (zh ? "被拒绝：当前为只读模式" : "Denied: read-only mode"), output: "", image: nil, denied: true)
            emit(.toolFinished(callId: callId, result: r))
            record(.tool(call: call, result: r))
            return nil
        }
        if approval == true {
            let decision = await withCheckedContinuation { (c: CheckedContinuation<ApprovalDecision, Never>) in
                lock.locked { approvals[callId] = c }
            }
            switch decision {
            case .deny(let reason):
                let r = ToolResultView(ok: false, summary: (zh ? "用户拒绝" : "Denied by user") + (reason.map { (zh ? "：" : ": ") + $0 } ?? ""),
                                       output: "", image: nil, denied: true)
                emit(.toolFinished(callId: callId, result: r))
                record(.tool(call: call, result: r))
                return nil
            case .allowSimilar:
                lock.locked { allowSimilar.insert(tool + String(describing: risk)) }
            case .editAndRun(let cmd):
                finalCommand = cmd
            case .allowOnce:
                break
            }
        }
        if Task.isCancelled { return nil }
        emit(.toolStarted(callId: callId))
        await pause(250)
        for line in output.split(separator: "\n", omittingEmptySubsequences: false) {
            emit(.toolOutput(callId: callId, text: String(line) + "\n"))
            await pause(40)
            if Task.isCancelled { return nil }
        }
        let r = ToolResultView(ok: true, summary: finalCommand == summary ? resultSummary : (zh ? "已执行修改后的命令" : "Ran the edited command"),
                               output: output, image: resultImage, denied: false)
        emit(.toolFinished(callId: callId, result: r))
        record(.tool(call: call, result: r))
        return output
    }

    private func ask(_ question: String, options: [String]) async -> String {
        let qid = "q-" + UUID().uuidString.prefix(6)
        emit(.askUser(questionId: qid, question: question, options: options))
        emit(.status(running: true, step: step))
        return await withCheckedContinuation { c in lock.locked { answers[qid] = c } }
    }

    private func secret(_ prompt: String) async -> String? {
        let rid = "s-" + UUID().uuidString.prefix(6)
        emit(.secretRequested(requestId: rid, prompt: prompt, targetId: current.id))
        return await withCheckedContinuation { c in lock.locked { secrets[rid] = c } }
    }

    /// 没引用过的设备：先请用户同意连接（完全信任档除外），然后连接
    private func useTarget(_ t: AgentTarget, why: String) async -> Bool {
        step += 1
        let callId = "call-" + UUID().uuidString.prefix(6) + "-use"
        let name = client.targetName(t)
        let (m, known) = lock.locked { (mode, authorized.contains(t.id)) }
        let needs = !known && m != .fullTrust
        let d = client.device(t.id)
        let detail = d.map { "\($0.platform) · \($0.osVersion)" } ?? "SSH"
        let call = ToolCallView(callId: callId, tool: "use_target", targetId: t.id, targetName: name, summary: (zh ? "连接 " : "Connect to ") + name,
                                detail: detail, risk: .read, riskReasons: needs ? [zh ? "对话里没有引用这台设备" : "This device was not referenced in the conversation"] : [],
                                modelReason: why, image: nil)
        emit(.toolProposed(call: call, needsApproval: needs))
        if needs {
            let decision = await withCheckedContinuation { (c: CheckedContinuation<ApprovalDecision, Never>) in
                lock.locked { approvals[callId] = c }
            }
            if case .deny(let reason) = decision {
                let r = ToolResultView(ok: false, summary: zh ? "用户拒绝" : "Denied by user", output: reason ?? "", image: nil, denied: true)
                emit(.toolFinished(callId: callId, result: r))
                record(.tool(call: call, result: r))
                return false
            }
        }
        if Task.isCancelled { return false }
        emit(.toolStarted(callId: callId))
        await pause(300)
        let r = ToolResultView(ok: true, summary: zh ? "已连接" : "Connected", output: "", image: nil, denied: false)
        emit(.toolFinished(callId: callId, result: r))
        record(.tool(call: call, result: r))
        lock.locked {
            authorized.insert(t.id)
            target = t
        }
        return true
    }

    private func listDevices() async {
        let list = client.devices().filter { !$0.isSelf && $0.kind != .mobile }
        let out = list.map { "\($0.id)\t\(client.targetName(AgentTarget(kind: .device, id: $0.id)))\t\($0.platform)\t\($0.online ? "online" : "offline")" }
            .joined(separator: "\n")
        _ = await tool("list_devices", summary: zh ? "查看账号里的设备与主机" : "List devices and hosts", risk: .read, why: zh ? "找到要操作的机器" : "Find the machine to work on",
                       output: out, resultSummary: zh ? "\(list.count) 台设备" : "\(list.count) devices", accountLevel: true)
    }

    private func recentActivity() async {
        let out = zh ? "11-21 09:30\t登录\t北京 · 203.0.113.8\n11-21 09:12\t连接 build-server-01\t终端\n11-20 22:47\t连接 办公室电脑\t远程桌面"
                     : "11-21 09:30\tSign-in\tBeijing · 203.0.113.8\n11-21 09:12\tConnect build-server-01\tTerminal\n11-20 22:47\tConnect Office PC\tRemote desktop"
        _ = await tool("recent_activity", summary: zh ? "最近的登录与连接记录" : "Recent sign-ins and connections", risk: .read, why: zh ? "查看账号最近的活动" : "Check recent account activity",
                       output: out, resultSummary: zh ? "3 条记录" : "3 entries", accountLevel: true)
    }

    private func runScript(userText: String, refs: [ContextRef]) async {
        step = 0
        emit(.status(running: true, step: 0))
        let thinkingId = "th-" + UUID().uuidString.prefix(6)
        emit(.thinking(messageId: thinkingId, text: zh ? "先确认目标机器的状态，再决定下一步。" : "Check the target first, then decide what to do."))
        await pause(300)
        // 引用过的设备、主机，以及文件、目录所在的设备，本对话内可直接使用
        var picked: AgentTarget?
        for r in refs {
            switch r.kind {
            case .device: picked = picked ?? AgentTarget(kind: .device, id: r.id)
            case .sshHost: picked = picked ?? AgentTarget(kind: .sshHost, id: r.id)
            case .file, .folder:
                let t = AgentTarget(kind: client.device(r.targetId) != nil ? .device : .sshHost, id: r.targetId)
                picked = picked ?? t
            default: break
            }
            if let p = picked { lock.locked { _ = authorized.insert(p.id) } }
        }
        if let picked { lock.locked { target = picked } }
        if refs.contains(where: { $0.kind == .activity }) {
            await recentActivity()
            guard !Task.isCancelled else { return }
            await say(zh ? "最近 24 小时只有你自己的登录与连接，没有陌生地址。" : "In the last 24 hours there were only your own sign-ins and connections, from known addresses.")
            if picked == nil { emit(.status(running: false, step: step)); return }
        }
        if lock.locked({ target }) == nil {
            await listDevices()
            guard !Task.isCancelled else { return }
            let t = AgentTarget(kind: .device, id: userText.contains(zh ? "记事本" : "Notepad") ? "dev-office" : "dev-build")
            guard await useTarget(t, why: zh ? "这台机器最符合你的描述" : "This machine matches your description") else {
                await say(zh ? "好的，不连接这台设备。你可以用 @ 指定要用的设备。" : "OK, I will not connect to it. Use @ to tell me which device to use.")
                emit(.status(running: false, step: step))
                return
            }
        } else if let t = lock.locked({ target }), !lock.locked({ authorized.contains(t.id) }) {
            guard await useTarget(t, why: "") else { emit(.status(running: false, step: step)); return }
        }
        guard !Task.isCancelled else { return }
        if config.planFirst {
            await say(zh ? "计划：\n1. 查看磁盘与目录占用\n2. 找出可清理的旧文件\n3. 经你确认后清理\n4. 复查结果\n\n同意后我开始执行。"
                         : "Plan:\n1. Check disk and directory usage\n2. Find old files that can be removed\n3. Clean up after your approval\n4. Verify the result\n\nI will start once you agree.")
            let a = await ask(zh ? "按这个计划执行吗？" : "Proceed with this plan?", options: zh ? ["执行", "取消"] : ["Proceed", "Cancel"])
            if a != (zh ? "执行" : "Proceed") { emit(.status(running: false, step: step)); return }
        }
        if isGui { await guiScript() } else { await shellScript() }
        emit(.status(running: false, step: step))
    }

    private func shellScript() async {
        await say(zh ? "好的，我先看一下 \(targetName) 的磁盘使用情况。" : "OK. Let me check the disk usage on \(targetName) first.")
        guard !Task.isCancelled else { return }
        let df = MockShell.run(command: "df -h", device: client.device("dev-build")!, cwd: "~", zh: zh).text
        _ = await tool("run_command", summary: "df -h", risk: .read, reasons: [zh ? "只读命令" : "Read-only command"],
                       why: zh ? "查看各分区使用率" : "See usage of each partition", output: df, resultSummary: zh ? "退出码 0" : "Exit code 0")
        guard !Task.isCancelled else { return }
        await say(zh ? "/data 分区已用 92%。再看看是哪些目录占得多。" : "/data is 92% full. Let me see which directories use the most space.")
        let du = MockShell.run(command: "du", device: client.device("dev-build")!, cwd: "~", zh: zh).text
        _ = await tool("run_command", summary: "du -sh /data/* | sort -rh | head", risk: .read, reasons: [zh ? "只读命令" : "Read-only command"],
                       why: zh ? "找出占用最大的目录" : "Find the largest directories", output: du, resultSummary: zh ? "退出码 0" : "Exit code 0")
        guard !Task.isCancelled else { return }
        await say(zh ? "/data/backups 占了 1.1T，里面有不少 30 天前的旧备份。建议删除 30 天前的备份文件："
                     : "/data/backups uses 1.1T and holds many backups older than 30 days. I suggest deleting them:")
        let cmd = "find /data/backups -name '*.tar.gz' -mtime +30 -delete"
        let res = await tool("run_command", summary: cmd, risk: .dangerous,
                             reasons: zh ? ["find -delete 会删除文件", "作用于 /data 下的大量文件"] : ["find -delete removes files", "Affects many files under /data"],
                             why: zh ? "清理 30 天前的旧备份以释放空间" : "Remove backups older than 30 days to free space",
                             output: zh ? "已删除 23 个文件\n" : "23 files removed\n", resultSummary: zh ? "释放 412 GB" : "Freed 412 GB")
        guard !Task.isCancelled else { return }
        guard res != nil else {
            await say(zh ? "好的，不删除。你也可以先把旧备份挪到别的盘，或者告诉我只保留最近几份。"
                         : "OK, nothing was deleted. You could move old backups elsewhere, or tell me how many recent ones to keep.")
            return
        }
        await say(zh ? "清理完成，/data 现在使用 70%。" : "Done. /data is now 70% full.")
        let a = await ask(zh ? "要不要顺便设置每天自动清理 30 天前的备份？" : "Should I also schedule a daily cleanup of backups older than 30 days?",
                          options: zh ? ["设置", "不用了"] : ["Schedule it", "No thanks"])
        guard !Task.isCancelled else { return }
        if a == (zh ? "设置" : "Schedule it") {
            guard await secret(zh ? "写入 root 的定时任务需要 deploy 的 sudo 密码" : "Writing root's crontab needs the sudo password for deploy") != nil else {
                await say(zh ? "没有拿到密码，先不设置了。" : "No password provided, so I did not schedule it.")
                return
            }
            _ = await tool("run_command", summary: "sudo crontab -l | { cat; echo '0 3 * * * find /data/backups -mtime +30 -delete'; } | sudo crontab -",
                           risk: .dangerous, reasons: zh ? ["修改 crontab", "使用 sudo"] : ["Modifies crontab", "Uses sudo"],
                           why: zh ? "每天 3 点自动清理" : "Clean up every day at 03:00",
                           output: "", resultSummary: zh ? "退出码 0" : "Exit code 0")
            await say(zh ? "已设置，每天凌晨 3 点执行。" : "Scheduled for 03:00 every day.")
        } else {
            await say(zh ? "好的，不设置。" : "OK, not scheduling it.")
        }
    }

    private func guiScript() async {
        guard let d = client.device(current.id) else { return }
        let display = MockData.displays(for: d).first!
        let renderer = MockDesktopRenderer(device: d, display: display, zh: zh, size: CGSize(width: 1280, height: Int(1280 * Double(display.height) / Double(display.width))))
        await say(zh ? "我先截个图看看 \(targetName) 现在的屏幕。" : "Let me take a screenshot of \(targetName) first.")
        let shot = renderer.pngSnapshot(maxWidth: 1280)
        _ = await tool("screenshot", summary: zh ? "截取屏幕 1" : "Capture display 1", risk: .read, reasons: [zh ? "只读操作" : "Read-only"],
                       why: zh ? "了解当前屏幕内容" : "See what is on screen", output: "", resultSummary: "1280×\(shot.height)", resultImage: shot)
        guard !Task.isCancelled else { return }
        await say(zh ? "记事本里有未保存的内容。我点一下保存。" : "Notepad has unsaved text. I will click Save.")
        let p = renderer.saveButtonPoint
        let marked = renderer.pngSnapshot(maxWidth: 1280, mark: p)
        _ = await tool("mouse", summary: zh ? "单击 (\(Int(p.x)), \(Int(p.y)))「保存」" : "Click (\(Int(p.x)), \(Int(p.y))) \"Save\"", risk: .normal,
                       reasons: [zh ? "图形界面点击" : "GUI click"], why: zh ? "保存记事本内容" : "Save the Notepad content",
                       output: "", resultSummary: zh ? "已点击" : "Clicked", image: marked)
        guard !Task.isCancelled else { return }
        await say(zh ? "已保存。" : "Saved.")
    }
}
