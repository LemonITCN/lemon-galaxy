package cn.lemonit.galaxy.core.mock

import cn.lemonit.galaxy.core.bridge.AgentConversationInterface
import cn.lemonit.galaxy.core.bridge.AgentEvent
import cn.lemonit.galaxy.core.bridge.AgentListener
import cn.lemonit.galaxy.core.bridge.AgentMessage
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.ApprovalDecision
import cn.lemonit.galaxy.core.bridge.ConversationConfig
import cn.lemonit.galaxy.core.bridge.ConversationSummary
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
internal data class MockTarget(val id: String, val name: String, val platform: Platform)

/**
 * 脚本化的模拟智能体：按目标是命令行还是图形界面走两套剧本，
 * 覆盖流式文字、思考、只读工具自动执行、危险操作确认、ask_user、request_secret、用量与停止。
 */
internal class MockConversation(
    private val id: String,
    var config: ConversationConfig,
    private val modelName: String,
    private val targets: List<MockTarget>,
    @Volatile var listener: AgentListener,
    private val scope: CoroutineScope,
    private val zh: Boolean,
    private val screenshot: () -> ImageData?,
) : AgentConversationInterface {
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
        targetNames = targets.map { it.name },
        updatedAt = updatedAt,
    )

    fun seed(messages: List<AgentMessage>, title: String, at: Long) {
        history.addAll(messages)
        this.title = title
        updatedAt = at
    }

    override fun id(): String = id

    override fun title(): String = title

    override fun send(text: String) {
        if (title.isEmpty()) title = text.take(40)
        record(AgentMessage.User(text))
        job?.cancel()
        job = scope.launch { runScript(text) }
    }

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
        val call = ToolCallView(callId, tool, target.id, target.name, summary, detail, risk, reasons, modelReason, if (tool == "mouse") image else null)
        val changes = risk != RiskLevel.READ
        val key = "$tool:${risk.name}"
        val denyReadOnly = mode == AiMode.READ_ONLY && changes
        val needsApproval = !denyReadOnly && risk != RiskLevel.FORBIDDEN && !similarAllowed.contains(key) && when (mode) {
            AiMode.CONFIRM_ALL -> changes
            AiMode.CONFIRM_DANGEROUS -> risk == RiskLevel.DANGEROUS || risk == RiskLevel.UNKNOWN
            else -> false
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
        val r = ToolResultView(true, tr("完成，退出码 0", "Done, exit code 0"), lines.joinToString("\n"), if (tool == "screenshot") image else null, false)
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

    private suspend fun runScript(userText: String) {
        running = true
        try {
            val target = targets.firstOrNull() ?: MockTarget("", "?", Platform.LINUX)
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
