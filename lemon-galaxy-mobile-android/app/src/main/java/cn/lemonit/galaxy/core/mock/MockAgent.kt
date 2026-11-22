package cn.lemonit.galaxy.core.mock

import cn.lemonit.galaxy.core.bridge.AgentConversationInterface
import cn.lemonit.galaxy.core.bridge.AgentEvent
import cn.lemonit.galaxy.core.bridge.AgentListener
import cn.lemonit.galaxy.core.bridge.AgentMessage
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.ApprovalDecision
import cn.lemonit.galaxy.core.bridge.ContextRef
import cn.lemonit.galaxy.core.bridge.ContextRefKind
import cn.lemonit.galaxy.core.bridge.ConversationConfig
import cn.lemonit.galaxy.core.bridge.ConversationModel
import cn.lemonit.galaxy.core.bridge.ConversationSummary
import cn.lemonit.galaxy.core.bridge.GalaxyException
import cn.lemonit.galaxy.core.bridge.ImageData
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.RiskLevel
import cn.lemonit.galaxy.core.bridge.ToolCallView
import cn.lemonit.galaxy.core.bridge.ToolResultView
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

/** 模拟对话的目标信息。 */
internal data class MockTarget(val id: String, val name: String, val platform: Platform, val online: Boolean = true)

/**
 * 脚本化的模拟智能体（聊天式）：
 * - 引用了设备 / 主机（或文件、目录所在的设备）时直接在那台上干活，按命令行还是图形界面走两套剧本；
 * - 没有引用设备时先调账号查询 list_devices，再弹「AI 想连接 某设备」的确认（完全信任档除外）；
 * - 只引用了片段、端口映射、连接记录时直接回答。
 * 覆盖流式文字、思考、只读工具自动执行、危险操作确认、ask_user、request_secret、用量与停止。
 */
internal class MockConversation(
    private val id: String,
    var config: ConversationConfig,
    private val targets: List<MockTarget>,
    @Volatile var listener: AgentListener,
    private val scope: CoroutineScope,
    private val zh: Boolean,
    private val screenshot: () -> ImageData?,
    /** 目标 ID → 目标（引用里的设备、主机）。 */
    private val resolve: (String) -> MockTarget? = { null },
    /** 账号里的全部设备与主机（list_devices 用）。 */
    private val catalog: () -> List<MockTarget> = { emptyList() },
) : AgentConversationInterface {
    /** 本对话里用户引用过或同意连接过的目标。 */
    private val usable = java.util.concurrent.CopyOnWriteArrayList(targets)
    private val history = mutableListOf<AgentMessage>()
    private val decisions = ConcurrentHashMap<String, CompletableDeferred<ApprovalDecision>>()
    private val answers = ConcurrentHashMap<String, CompletableDeferred<String>>()
    private val secrets = ConcurrentHashMap<String, CompletableDeferred<String>>()
    private val similarAllowed = mutableSetOf<String>()
    private var job: Job? = null
    private var promptTokens = 0uL
    private var completionTokens = 0uL
    @Volatile var running = false
    @Volatile var waiting = false
    var title: String = config.title.orEmpty()
    var updatedAt: Long = System.currentTimeMillis()
    private var mode: AiMode = config.mode
    private var step = 0u

    private fun tr(zhText: String, en: String) = if (zh) zhText else en

    fun summary() = ConversationSummary(
        id = id,
        title = title.ifEmpty { tr("新对话", "New conversation") },
        modelId = config.modelId,
        targetNames = usable.map { it.name },
        updatedAt = updatedAt,
        config = config,
    )

    fun seed(messages: List<AgentMessage>, title: String, at: Long) {
        history.addAll(messages)
        this.title = title
        updatedAt = at
    }

    override fun id(): String = id

    override fun title(): String = title

    override fun send(text: String) = sendWithRefs(text, emptyList())

    override fun sendWithRefs(text: String, refs: List<ContextRef>) {
        if (title.isEmpty()) {
            val t = text.split(Regex("\\s+")).filter { it.isNotEmpty() }.joinToString(" ")
            title = if (t.isEmpty()) refs.firstOrNull()?.label.orEmpty() else t.take(40)
        }
        record(AgentMessage.User(text, refs))
        refs.forEach { r ->
            val tid = when (r.kind) {
                ContextRefKind.DEVICE, ContextRefKind.SSH_HOST -> r.id
                ContextRefKind.FILE, ContextRefKind.FOLDER -> r.targetId
                else -> null
            }
            tid?.let(resolve)?.let { t -> if (usable.none { it.id == t.id }) usable.add(t) }
        }
        job?.cancel()
        job = scope.launch { runScript(text, refs) }
    }

    override fun setModel(providerId: ULong, modelId: String) {
        val m = MockData.models().firstOrNull { it.providerId == providerId && it.id == modelId }
            ?: throw GalaxyException.Permission(tr("未开通该模型", "This model is not enabled for you"))
        config = config.copy(providerId = m.providerId, modelId = m.id)
    }

    override fun model(): ConversationModel = ConversationModel(config.providerId, config.modelId)

    /** 与真内核一致：切到后台停止当前任务。 */
    override fun setForeground(foreground: Boolean) {
        if (!foreground && running) stop()
    }

    override fun decide(callId: String, decision: ApprovalDecision) {
        decisions[callId]?.complete(decision)
    }

    override fun answer(questionId: String, answer: String) {
        answers[questionId]?.complete(answer)
    }

    override fun provideSecret(requestId: String, secret: String) {
        // 模拟：只确认收到，内容不记录、不进入任何历史
        secrets[requestId]?.complete("*")
    }

    override fun setMode(mode: AiMode, trustMinutes: UInt?) {
        this.mode = mode
        config = config.copy(mode = mode, trustMinutes = trustMinutes)
        val until = if (mode == AiMode.FULL_TRUST && trustMinutes != null) System.currentTimeMillis() + trustMinutes.toLong() * 60_000 else null
        emit(AgentEvent.ModeChanged(mode, mode, until))
    }

    override fun stop() {
        job?.cancel()
        job = null
        running = false
        waiting = false
        emit(AgentEvent.Status(false, step))
    }

    override fun messages(): List<AgentMessage> = synchronized(history) { history.toList() }

    private fun record(m: AgentMessage) {
        synchronized(history) { history.add(m) }
        updatedAt = System.currentTimeMillis()
    }

    private fun emit(e: AgentEvent) {
        listener.onEvent(e)
    }

    private fun pauseIfBackground() = Unit

    private suspend fun say(text: String, thinking: String? = null) {
        val mid = newId()
        if (thinking != null) {
            pauseIfBackground()
            emit(AgentEvent.Thinking(mid, thinking))
            delay(500)
        }
        val tokens = text.chunked(if (zh) 3 else 6)
        for (t in tokens) {
            pauseIfBackground()
            emit(AgentEvent.AssistantDelta(mid, t))
            completionTokens += 2u
            delay(35)
        }
        emit(AgentEvent.AssistantDone(mid))
        record(AgentMessage.Assistant(mid, text, thinking.orEmpty()))
    }

    private fun nextStep() {
        step++
        emit(AgentEvent.Status(true, step))
    }

    private fun usage(prompt: Int, completion: Int) {
        promptTokens += prompt.toUInt()
        completionTokens += completion.toUInt()
        emit(AgentEvent.Usage(promptTokens, completionTokens))
    }

    /** 走一个工具调用：按档位判定是否需要确认，返回 null 表示被拒。 */
    private suspend fun tool(
        tool: String,
        target: MockTarget,
        summary: String,
        detail: String,
        risk: RiskLevel,
        reasons: List<String>,
        modelReason: String,
        output: List<String>,
        image: ImageData? = null,
        needsSecret: String? = null,
    ): ToolResultView? {
        pauseIfBackground()
        val callId = newId()
        // 与真内核一致：run_command 的 detail 就是完整命令
        val call = ToolCallView(callId, tool, target.id, target.name, summary, if (tool == "run_command") summary else detail, risk, reasons, modelReason, if (tool == "mouse") image else null)
        val changes = risk != RiskLevel.READ
        val key = "$tool:${risk.name}"
        val connect = tool == "use_target"
        val denyReadOnly = mode == AiMode.READ_ONLY && changes && !connect
        val needsApproval = if (connect) {
            // 连接没引用过的设备：除完全信任外都要用户同意
            mode != AiMode.FULL_TRUST
        } else {
            !denyReadOnly && risk != RiskLevel.FORBIDDEN && !similarAllowed.contains(key) && when (mode) {
                AiMode.CONFIRM_ALL -> changes
                AiMode.CONFIRM_DANGEROUS -> risk == RiskLevel.DANGEROUS || risk == RiskLevel.UNKNOWN
                else -> false
            }
        }
        // 先登记等待，再发事件：界面立刻回应也不会丢
        val pending = if (needsApproval) CompletableDeferred<ApprovalDecision>().also { decisions[callId] = it } else null
        emit(AgentEvent.ToolProposed(call, needsApproval))
        var decision: ApprovalDecision? = null
        if (denyReadOnly || risk == RiskLevel.FORBIDDEN) {
            delay(200)
            val r = ToolResultView(
                false,
                if (risk == RiskLevel.FORBIDDEN) tr("被拒绝：绝对禁止的操作", "Denied: forbidden operation")
                else tr("被拒绝：当前为只读模式", "Denied: read-only mode"),
                "", null, true,
            )
            emit(AgentEvent.ToolFinished(callId, r))
            record(AgentMessage.Tool(call, r))
            return null
        }
        if (pending != null) {
            waiting = true
            decision = pending.await()
            waiting = false
            decisions.remove(callId)
            if (decision is ApprovalDecision.Deny) {
                val r = ToolResultView(
                    false,
                    tr("用户拒绝", "Denied by user") + (decision.reason?.let { tr("：$it", ": $it") } ?: ""),
                    "", null, true,
                )
                emit(AgentEvent.ToolFinished(callId, r))
                record(AgentMessage.Tool(call, r))
                return null
            }
            if (decision is ApprovalDecision.AllowSimilar) similarAllowed.add(key)
        }
        emit(AgentEvent.ToolStarted(callId))
        if (needsSecret != null) {
            val rid = newId()
            val d = CompletableDeferred<String>()
            secrets[rid] = d
            waiting = true
            emit(AgentEvent.SecretRequested(rid, needsSecret, target.id))
            d.await()
            waiting = false
            secrets.remove(rid)
        }
        val lines = if (decision is ApprovalDecision.EditAndRun) listOf("$ ${decision.command}") + output else output
        for (l in lines) {
            pauseIfBackground()
            emit(AgentEvent.ToolOutput(callId, l + "\n"))
            delay(90)
        }
        val r = ToolResultView(
            true,
            if (connect) tr("已连接", "Connected") else tr("完成，退出码 0", "Done, exit code 0"),
            lines.joinToString("\n"), if (tool == "screenshot") image else null, false,
        )
        emit(AgentEvent.ToolFinished(callId, r))
        record(AgentMessage.Tool(call, r))
        return r
    }

    private suspend fun ask(question: String, options: List<String>): String {
        pauseIfBackground()
        val qid = newId()
        val d = CompletableDeferred<String>()
        answers[qid] = d
        waiting = true
        emit(AgentEvent.AskUser(qid, question, options))
        val a = d.await()
        waiting = false
        answers.remove(qid)
        return a
    }

    private suspend fun runScript(userText: String, refs: List<ContextRef>) {
        running = true
        try {
            emit(AgentEvent.Status(true, step))
            val referenced = refs.mapNotNull { r ->
                when (r.kind) {
                    ContextRefKind.DEVICE, ContextRefKind.SSH_HOST -> r.id
                    ContextRefKind.FILE, ContextRefKind.FOLDER -> r.targetId
                    else -> null
                }
            }.firstNotNullOfOrNull { id -> usable.firstOrNull { it.id == id } }
            if (referenced == null && refs.isNotEmpty()) {
                // 只引用了片段、端口映射、连接记录：内容已随消息给模型，直接回答
                nextStep()
                say(refsAnswer(refs), thinking = tr("引用的内容已经附在消息里，直接据此回答。", "The referenced content is attached to the message; answer from it."))
                usage(2400, 90)
                return
            }
            val target = referenced ?: usable.lastOrNull() ?: pickTarget() ?: return
            if (config.planFirst) {
                nextStep()
                say(
                    tr(
                        "计划：\n1. 查看 ${target.name} 的当前状态（只读）\n2. 找出问题所在\n3. 需要改动时先请你确认\n\n同意后开始执行。",
                        "Plan:\n1. Inspect the current state of ${target.name} (read-only)\n2. Locate the problem\n3. Ask you before changing anything\n\nI will start once you approve.",
                    ),
                )
                usage(2100, 120)
                val go = ask(tr("按这个计划执行吗？", "Proceed with this plan?"), listOf(tr("执行", "Proceed"), tr("取消", "Cancel")))
                if (go != tr("执行", "Proceed")) {
                    say(tr("好的，已取消。", "OK, cancelled."))
                    return
                }
            }
            if (target.platform == Platform.MACOS || target.platform == Platform.WINDOWS) guiScript(target) else shellScript(target)
        } finally {
            running = false
            waiting = false
            emit(AgentEvent.Status(false, step))
        }
    }

    private fun refsAnswer(refs: List<ContextRef>): String {
        val kinds = refs.map { it.kind }.toSet()
        return when {
            ContextRefKind.ACTIVITY in kinds -> tr(
                "最近的记录里：\n\n| 时间 | 事件 | 对象 |\n|---|---|---|\n| 今天 09:12 | 登录 | 本机 |\n| 今天 09:13 | 连接 | web-01 (nginx) |\n| 昨天 22:40 | 连接 | Build Mac |\n\n没有看到陌生 IP 的登录。",
                "From the recent records:\n\n| Time | Event | Target |\n|---|---|---|\n| Today 09:12 | Sign-in | This phone |\n| Today 09:13 | Connect | web-01 (nginx) |\n| Yesterday 22:40 | Connect | Build Mac |\n\nNo sign-ins from unfamiliar IP addresses.",
            )
            ContextRefKind.SNIPPET in kinds -> tr(
                "这个片段的作用：\n\n```bash\ndf -h\n```\n\n列出各分区的总量、已用、可用和挂载点，只读，不会改动任何东西。",
                "What this snippet does:\n\n```bash\ndf -h\n```\n\nIt lists size, used, available and mount point for each filesystem. It is read-only.",
            )
            else -> tr(
                "这条端口映射把本机 `15432` 经 **web-01** 转发到 `10.0.3.21:5432`，适合连内网的 PostgreSQL。",
                "This rule forwards local port `15432` through **web-01** to `10.0.3.21:5432`, handy for reaching the internal PostgreSQL.",
            )
        }
    }

    /** 没引用设备：先查账号里的设备，再请用户同意连接其中一台。返回 null 表示被拒绝。 */
    private suspend fun pickTarget(): MockTarget? {
        nextStep()
        say(
            tr("我先看看你的账号里有哪些设备。", "Let me see which devices are in your account."),
            thinking = tr("用户没有指定设备，先列出账号里的设备与主机。", "No device was specified; list the account's devices and hosts first."),
        )
        usage(1800, 40)
        val all = catalog()
        accountTool(
            "list_devices", tr("列出设备与 SSH 主机", "List devices and SSH hosts"),
            all.joinToString("\n") { "${it.name}\t${it.platform.name.lowercase()}\t${if (it.online) "online" else "offline"}" },
        )
        val t = all.firstOrNull { it.online && it.platform == Platform.LINUX } ?: all.firstOrNull { it.online } ?: run {
            say(tr("账号里没有在线的设备。", "There are no online devices in your account."))
            return null
        }
        nextStep()
        val ok = tool(
            "use_target", t, tr("连接 ${t.name}", "Connect to ${t.name}"), "${t.platform.name.lowercase()}  ·  ${if (t.online) "online" else "offline"}", RiskLevel.READ,
            listOf(tr("对话里没有引用这台设备", "This device was not referenced in the conversation")), tr("需要在 ${t.name} 上查看磁盘使用情况", "Need to check disk usage on ${t.name}"), emptyList(),
        )
        if (ok == null) {
            say(tr("好的，不连接 ${t.name}。", "OK, I will not connect to ${t.name}."))
            return null
        }
        usable.add(t)
        return t
    }

    /** 账号级只读查询：不需要确认、没有目标设备。 */
    private suspend fun accountTool(tool: String, summary: String, output: String) {
        val callId = newId()
        val call = ToolCallView(callId, tool, "", "", summary, "{}", RiskLevel.READ, emptyList(), "", null)
        emit(AgentEvent.ToolProposed(call, false))
        emit(AgentEvent.ToolStarted(callId))
        delay(300)
        val r = ToolResultView(true, tr("完成", "Done"), output, null, false)
        emit(AgentEvent.ToolFinished(callId, r))
        record(AgentMessage.Tool(call, r))
    }

    private suspend fun shellScript(t: MockTarget) {
        nextStep()
        say(
            tr("好的，我先看一下 ${t.name} 的磁盘使用情况。", "Sure. Let me check disk usage on ${t.name} first."),
            thinking = tr("用户想释放磁盘空间。先用只读命令看分区使用率，再找大目录。", "The user wants to free disk space. Start with read-only commands to see usage, then find large directories."),
        )
        usage(3200, 60)
        tool(
            "run_command", t, "df -h", "{\"command\":\"df -h\"}", RiskLevel.READ,
            listOf(tr("只读命令：df", "Read-only command: df")), tr("查看各分区使用率", "Check usage of each filesystem"),
            listOf(
                "Filesystem      Size  Used Avail Use% Mounted on",
                "/dev/sda1        80G   69G   11G  87% /",
                "tmpfs           3.9G     0  3.9G   0% /dev/shm",
                "/dev/sdb1       500G  212G  288G  43% /data",
            ),
        )
        nextStep()
        say(tr("根分区已用 87%。看看 /var/log 下哪些目录最大。", "The root filesystem is 87% full. Let me see what is largest under /var/log."))
        usage(3900, 40)
        tool(
            "run_command", t, "du -sh /var/log/* | sort -rh | head -5", "{\"command\":\"du -sh /var/log/* | sort -rh | head -5\"}", RiskLevel.READ,
            listOf(tr("只读命令：du、sort、head", "Read-only commands: du, sort, head")), tr("找出占用最大的日志目录", "Find the largest log directories"),
            listOf("3.2G\t/var/log/journal", "412M\t/var/log/nginx", "96M\t/var/log/syslog.1", "18M\t/var/log/apt", "4.0M\t/var/log/auth.log"),
        )
        nextStep()
        say(
            tr(
                "systemd 日志占了 **3.2G**。建议只保留最近 7 天，这一步会删除旧日志文件，需要你确认。",
                "The systemd journal uses **3.2G**. I suggest keeping only the last 7 days. This deletes old log files, so it needs your approval.",
            ),
        )
        usage(4600, 80)
        val r = tool(
            "run_command", t, "sudo journalctl --vacuum-time=7d", "{\"command\":\"sudo journalctl --vacuum-time=7d\",\"timeout_s\":120}",
            RiskLevel.DANGEROUS,
            listOf(tr("sudo 提权执行", "Runs with sudo"), tr("journalctl --vacuum-time 会删除日志文件", "journalctl --vacuum-time deletes log files")),
            tr("清理 7 天前的系统日志，预计释放约 2.8G", "Remove journal entries older than 7 days, freeing about 2.8G"),
            listOf(
                "Vacuuming done, freed 2.8G of archived journals from /var/log/journal/7f3a.",
                "Deleted archived journal /var/log/journal/7f3a/system@0006-1.journal (128.0M).",
            ),
            needsSecret = tr("${t.name} 上 sudo 需要密码", "sudo on ${t.name} needs a password"),
        )
        if (r == null) {
            nextStep()
            say(tr("好的，这一步不执行。日志保留原样，根分区仍是 87%。", "OK, skipped. The logs are unchanged and / is still 87% full."))
            usage(5000, 40)
            return
        }
        nextStep()
        val more = ask(tr("要不要顺便清理 Docker 不再使用的镜像？", "Also remove unused Docker images?"), listOf(tr("清理", "Remove them"), tr("不用了", "No thanks")))
        if (more == tr("清理", "Remove them")) {
            tool(
                "run_command", t, "docker image prune -a -f", "{\"command\":\"docker image prune -a -f\"}", RiskLevel.DANGEROUS,
                listOf(tr("docker prune 会删除镜像", "docker prune deletes images")),
                tr("删除没有容器使用的镜像", "Delete images not used by any container"),
                listOf("Deleted Images:", "untagged: node:20-alpine", "deleted: sha256:4b1e…", "Total reclaimed space: 1.31GB"),
            )
        }
        nextStep()
        say(
            tr(
                "完成。根分区从 87% 降到约 **51%**。\n\n- 清理了 7 天前的 systemd 日志（2.8G）\n- 以后可以在 `/etc/systemd/journald.conf` 设置 `SystemMaxUse=1G` 防止再涨满",
                "Done. The root filesystem went from 87% to about **51%**.\n\n- Removed journal entries older than 7 days (2.8G)\n- Consider setting `SystemMaxUse=1G` in `/etc/systemd/journald.conf`",
            ),
        )
        usage(6400, 180)
    }

    private suspend fun guiScript(t: MockTarget) {
        val shot = screenshot()
        nextStep()
        say(
            tr("我先截一张屏看看 ${t.name} 当前的桌面。", "Let me take a screenshot of ${t.name} first."),
            thinking = tr("图形界面任务，先截图识屏。", "GUI task; start with a screenshot."),
        )
        usage(3000, 50)
        tool(
            "screenshot", t, tr("截取屏幕 1", "Capture display 1"), "{\"display\":1}", RiskLevel.READ,
            listOf(tr("只读：截图", "Read-only: screenshot")), tr("了解当前屏幕内容", "See what is on screen"),
            emptyList(), image = shot,
        )
        nextStep()
        say(tr("看到 Documents 窗口里有 Quarterly report.pdf，我双击打开它。", "I can see Quarterly report.pdf in the Documents window. Double-clicking it."))
        usage(5200, 60)
        tool(
            "mouse", t, "double_click (1452, 274)", "{\"action\":\"double_click\",\"x\":1452,\"y\":274}", RiskLevel.NORMAL,
            listOf(tr("图形界面点击", "GUI click")), tr("打开报告文件", "Open the report"),
            emptyList(), image = shot,
        )
        nextStep()
        val ans = ask(tr("报告已打开。要我把它导出成 PDF 发到桌面吗？", "The report is open. Export a copy to the desktop?"), listOf(tr("导出", "Export"), tr("不用", "No")))
        if (ans == tr("导出", "Export")) {
            tool(
                "keyboard", t, "cmd+shift+s", "{\"key\":\"cmd+shift+s\"}", RiskLevel.NORMAL,
                listOf(tr("快捷键", "Shortcut")), tr("打开「存储为」对话框", "Open the Save As dialog"), emptyList(),
            )
            tool(
                "mouse", t, tr("click「存储」(1210, 640)", "click \"Save\" (1210, 640)"), "{\"action\":\"click\",\"x\":1210,\"y\":640}",
                RiskLevel.DANGEROUS,
                listOf(tr("按钮文字含「存储」，会覆盖同名文件", "Button \"Save\" may overwrite a file with the same name")),
                tr("确认保存到桌面", "Confirm saving to the desktop"), emptyList(), image = shot,
            )
        }
        nextStep()
        say(tr("完成。", "Done."))
        usage(7300, 90)
    }

    companion object {
        fun newId(): String = UUID.randomUUID().toString().replace("-", "").take(16)
    }
}
