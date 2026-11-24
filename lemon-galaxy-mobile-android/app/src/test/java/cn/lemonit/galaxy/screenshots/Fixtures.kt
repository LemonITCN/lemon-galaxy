package cn.lemonit.galaxy.screenshots

import android.content.Context
import cn.lemonit.galaxy.app.ChatItem
import cn.lemonit.galaxy.app.ConversationUi
import cn.lemonit.galaxy.app.HostsData
import cn.lemonit.galaxy.app.LoginStep
import cn.lemonit.galaxy.app.LoginUiState
import cn.lemonit.galaxy.app.TabStatus
import cn.lemonit.galaxy.app.ToolPhase
import cn.lemonit.galaxy.app.TransferDirection
import cn.lemonit.galaxy.app.TransferItem
import cn.lemonit.galaxy.app.TransferState
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.AiModels
import cn.lemonit.galaxy.core.bridge.ApprovalDecision
import cn.lemonit.galaxy.app.ConversationListItem
import cn.lemonit.galaxy.core.bridge.ImageData
import cn.lemonit.galaxy.core.bridge.PathInfo
import cn.lemonit.galaxy.core.bridge.PathKind
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.RiskLevel
import cn.lemonit.galaxy.core.bridge.ServerInfo
import cn.lemonit.galaxy.core.bridge.ToolCallView
import cn.lemonit.galaxy.core.bridge.ToolResultView
import cn.lemonit.galaxy.core.bridge.UserInfo
import cn.lemonit.galaxy.core.mock.MockData
import cn.lemonit.galaxy.core.mock.MockFs
import cn.lemonit.galaxy.data.VideoQuality
import cn.lemonit.galaxy.ui.files.FileBrowserState
import cn.lemonit.galaxy.ui.remote.RemoteStatus
import cn.lemonit.galaxy.ui.remote.RemoteUi
import com.termux.terminal.TerminalSession
import com.termux.terminal.TerminalSessionClient

/** 截图测试用的固定数据（与模拟内核同源）。 */
object Fixtures {
    fun tr(zh: Boolean): (String, String) -> String = { a, b -> if (zh) a else b }

    val hosts = HostsData(MockData.devices(), MockData.vault(), MockData.WAKE_RELAYS.keys)

    fun login(step: LoginStep = LoginStep.CREDENTIALS, error: String? = null) = LoginUiState(
        server = "galaxy.example.com:7443#sha256-q3Vb1oX9mH2c7uY0kN4tLwR8",
        username = "alice",
        password = "secret-pass",
        totp = if (step == LoginStep.TOTP) "1234" else "",
        newPassword = if (step == LoginStep.CHANGE_PASSWORD) "short" else "",
        confirmPassword = if (step == LoginStep.CHANGE_PASSWORD) "shor" else "",
        step = step,
        error = error,
    )

    fun files(zh: Boolean): FileBrowserState {
        val fs = MockFs("/home/demo", "demo", tr(zh))
        return FileBrowserState(path = "/home/demo", home = "/home/demo", entries = fs.list("/home/demo"), loading = false, showHidden = true)
    }

    fun transfers(zh: Boolean) = listOf(
        TransferItem(1, TransferDirection.DOWNLOAD, "backup-2026-11.tar.gz", "/home/demo/Downloads/backup-2026-11.tar.gz", "web-01 (nginx)", TransferState.Running, 312_000_000, 734_003_200, 8_400_000),
        TransferItem(2, TransferDirection.UPLOAD, "IMG_20261121_2130.jpg", "/Users/demo/IMG_20261121_2130.jpg", "Build Mac", TransferState.Paused, 1_100_000, 3_482_112),
        TransferItem(3, TransferDirection.DOWNLOAD, "nginx.conf", "/etc/nginx/nginx.conf", "web-01 (nginx)", TransferState.Done, 1_024, 1_024),
        TransferItem(4, TransferDirection.UPLOAD, "deploy.sh", "/home/demo/deploy.sh", "gitlab", TransferState.Failed(if (zh) "权限不足" else "Permission denied"), 0, 812),
    )

    fun preview(context: Context): ImageData = ImageData("image/jpeg", context.assets.open("mock/desktop_preview.jpg").use { it.readBytes() }, 960u, 540u)

    fun remote(zh: Boolean) = RemoteUi(
        title = if (zh) "Office PC" else "Office PC",
        platform = Platform.WINDOWS,
        status = RemoteStatus.STREAMING,
        displays = MockData.displaysFor(Platform.WINDOWS),
        displayId = 1u,
        quality = VideoQuality.BALANCED,
        mouseMode = true,
        cursorX = 1210,
        cursorY = 420,
        cursorVisible = true,
        videoW = 1280,
        videoH = 720,
        path = PathInfo(PathKind.RELAY_UDP, 46u, "galaxy.example.com:7443"),
        // 声音默认开
        audioSupported = true,
        audioOn = true,
        // 模拟被控端（Windows）能加 3 块虚拟屏；"Virtual 1" 是别人加的（不能删）
        virtualMax = 3,
    )

    val models = AiModels(true, AiMode.FULL_TRUST, MockData.models())

    fun summaries(zh: Boolean) = listOf(
        ConversationListItem("c1", if (zh) "web-01 磁盘快满了，帮我清理一下" else "web-01 is almost out of disk, please clean up", MockData.NOW - 2 * 60_000, running = true, waiting = true),
        ConversationListItem("c5", if (zh) "看看最近有没有陌生 IP 登录" else "Any sign-ins from unfamiliar IPs lately?", MockData.NOW - 20 * 60_000, running = true),
        ConversationListItem("c2", if (zh) "nginx 改了配置，帮我检查并重新加载" else "I changed the nginx config, please check it and reload", MockData.NOW - 3 * 3_600_000),
        ConversationListItem("c3", if (zh) "看看 Xcode 为什么卡住了" else "Why is Xcode stuck?", MockData.NOW - 26 * 3_600_000),
        ConversationListItem("c4", if (zh) "批量重启 3 台 worker 上的 docker" else "Restart docker on three workers", MockData.NOW - 4 * 86_400_000),
        ConversationListItem("c6", if (zh) "备份脚本为什么没跑" else "Why did the backup script not run?", MockData.NOW - 12 * 86_400_000),
    )

    fun conversation(zh: Boolean, image: ImageData?): ConversationUi {
        fun t(a: String, b: String) = if (zh) a else b
        val read = ToolCallView(
            "k1", "run_command", MockData.DEV_WEB_01, "web-01 (nginx)", "du -sh /var/log/* | sort -rh | head -5", "{\"command\":\"du -sh /var/log/* | sort -rh | head -5\"}",
            RiskLevel.READ, listOf(t("只读命令：du、sort、head", "Read-only commands: du, sort, head")), t("找出占用最大的日志目录", "Find the largest log directories"), null,
        )
        val danger = ToolCallView(
            "k2", "run_command", MockData.DEV_WEB_01, "web-01 (nginx)", "sudo journalctl --vacuum-time=7d", "{\"command\":\"sudo journalctl --vacuum-time=7d\",\"timeout_s\":120}",
            RiskLevel.DANGEROUS,
            listOf(t("sudo 提权执行", "Runs with sudo"), t("journalctl --vacuum-time 会删除日志文件", "journalctl --vacuum-time deletes log files")),
            t("清理 7 天前的系统日志，预计释放约 2.8G", "Remove journal entries older than 7 days, freeing about 2.8G"), null,
        )
        val shot = ToolCallView(
            "k0", "screenshot", MockData.DEV_BUILD_MAC, "Build Mac", t("截取屏幕 1", "Capture display 1"), "{\"display\":1}",
            RiskLevel.READ, emptyList(), t("了解当前屏幕内容", "See what is on screen"), null,
        )
        return ConversationUi(
            id = "c1",
            title = t("web-01 磁盘快满了，帮我清理一下", "web-01 is almost out of disk, please clean up"),
            providerId = 1u,
            modelId = "glm-5.2",
            mode = AiMode.CONFIRM_DANGEROUS,
            running = true,
            step = 3,
            promptTokens = 18_234,
            completionTokens = 1_320,
            items = listOf(
                ChatItem.User(
                    "u1", t("磁盘快满了，帮我清理一下", "It is almost out of disk, please clean up"),
                    listOf(
                        cn.lemonit.galaxy.core.bridge.ContextRef(cn.lemonit.galaxy.core.bridge.ContextRefKind.DEVICE, "", MockData.DEV_WEB_01, "web-01 (nginx)"),
                        cn.lemonit.galaxy.core.bridge.ContextRef(cn.lemonit.galaxy.core.bridge.ContextRefKind.FOLDER, MockData.DEV_WEB_01, "/var/log", "log"),
                    ),
                ),
                ChatItem.Assistant(
                    "a1", t("好的，我先看一下 **web-01** 的磁盘使用情况。根分区已用 `87%`。", "Sure. The root filesystem on **web-01** is `87%` full."),
                    t("用户想释放磁盘空间。先用只读命令看分区使用率。", "The user wants to free disk space."), false,
                ),
                ChatItem.Tool(
                    "k1", read, false, ToolPhase.DONE,
                    "3.2G\t/var/log/journal\n412M\t/var/log/nginx\n96M\t/var/log/syslog.1\n18M\t/var/log/apt\n4.0M\t/var/log/auth.log\n",
                    ToolResultView(true, t("完成，退出码 0", "Done, exit code 0"), "", null, false), null,
                ),
                ChatItem.Tool("k0", shot, false, ToolPhase.DONE, "", ToolResultView(true, t("完成", "Done"), "", image, false), null),
                ChatItem.Assistant(
                    "a2",
                    t(
                        "systemd 日志占了 **3.2G**。建议只保留最近 7 天：\n\n- 释放约 2.8G\n- 不影响正在运行的服务",
                        "The systemd journal uses **3.2G**. I suggest keeping only the last 7 days:\n\n- Frees about 2.8G\n- Running services are not affected",
                    ).replace("\n\n- ", "\n\n| ${t("目录", "Folder")} | ${t("大小", "Size")} |\n|---|---|\n| /var/log/journal | 3.2G |\n| /var/log/nginx | 412M |\n\n```bash\nsudo journalctl --vacuum-time=7d\n```\n\n- ").let { it },
                    null, false,
                ),
                ChatItem.Tool("k2", danger, true, ToolPhase.AWAITING, "", null, null),
            ),
        )
    }

    fun conversationLater(zh: Boolean): ConversationUi {
        fun t(a: String, b: String) = if (zh) a else b
        val base = conversation(zh, null)
        val danger = (base.items.last() as ChatItem.Tool)
        return base.copy(
            items = base.items.dropLast(1) + listOf(
                danger.copy(phase = ToolPhase.RUNNING, decision = ApprovalDecision.AllowOnce, needsApproval = true),
                ChatItem.Secret("s1", t("web-01 上 sudo 需要密码", "sudo on web-01 needs a password"), MockData.DEV_WEB_01, false),
                ChatItem.Ask("q1", t("要不要顺便清理 Docker 不再使用的镜像？", "Also remove unused Docker images?"), listOf(t("清理", "Remove them"), t("不用了", "No thanks")), null),
                ChatItem.Error("e1", t("模型网关超时，正在重试", "Model gateway timed out, retrying")),
            ),
        )
    }

    val user = UserInfo(3u, "alice", "Alice Chen", true, true)
    val server = ServerInfo("galaxy.example.com:7443#sha256-q3Vb1oX9mH2c7uY0kN4tLwR8zPaF5sDgE6jK1lQ2xYc", "LemonGalaxy", "0.1.0")

    val path = PathInfo(PathKind.LAN, 3u, "192.168.1.31:51820")

    /** 一段带颜色的终端输出（模拟登录后执行了几条命令）。 */
    fun terminalSample(zh: Boolean): String {
        val p = "\u001b[1;32mdemo@web-01\u001b[0m:\u001b[1;34m~\u001b[0m$ "
        return "\u001b[1mLemonGalaxy\u001b[0m ${if (zh) "模拟终端" else "mock terminal"} · Linux 6.8.0-49-generic x86_64\r\n\r\n" +
            "Last login: Sat Nov 21 21:12:04 2026 from 192.168.1.20\r\n" +
            p + "uptime\r\n 21:30:12 up 12 days,  3:41,  2 users,  load average: 0.42, 0.38, 0.35\r\n" +
            p + "ls -la\r\n" +
            "total 40\r\n" +
            "drwxr-xr-x  1 demo demo  4096 Nov 21 21:18 \u001b[1;34m.ssh\u001b[0m\r\n" +
            "-rw-r--r--  1 demo demo   102 Oct 12 09:30 .bashrc\r\n" +
            "lrwxrwxrwx  1 demo demo    21 Nov 17 18:02 \u001b[1;36mcurrent\u001b[0m -> projects/lemon-galaxy\r\n" +
            "-rwxr-xr-x  1 demo demo    86 Nov 17 18:02 \u001b[1;32mdeploy.sh\u001b[0m\r\n" +
            "drwxr-xr-x  1 demo demo  4096 Nov 20 11:02 \u001b[1;34mDownloads\u001b[0m\r\n" +
            "drwxr-xr-x  1 demo demo  4096 Nov 21 20:58 \u001b[1;34mlogs\u001b[0m\r\n" +
            "-rw-r--r--  1 demo demo    52 Nov 21 20:40 notes.txt\r\n" +
            "drwxr-xr-x  1 demo demo  4096 Nov 21 19:30 \u001b[1;34mprojects\u001b[0m\r\n" +
            p + "df -h /\r\n" +
            "Filesystem      Size  Used Avail Use% Mounted on\r\n" +
            "/dev/sda1        80G   69G   11G  \u001b[1;31m87%\u001b[0m /\r\n" +
            p + "git -C projects/lemon-galaxy status -sb\r\n" +
            "## main...origin/main\r\n" +
            " \u001b[31mM\u001b[0m docs/ffi.md\r\n" +
            "\u001b[32m??\u001b[0m lemon-galaxy-mobile-android/\r\n" +
            p
    }

    fun terminalSession(text: String): TerminalSession {
        val client = object : TerminalSessionClient {
            override fun onTextChanged(changedSession: TerminalSession) = Unit
            override fun onTitleChanged(changedSession: TerminalSession) = Unit
            override fun onSessionFinished(finishedSession: TerminalSession) = Unit
            override fun onCopyTextToClipboard(session: TerminalSession, text: String) = Unit
            override fun onPasteTextFromClipboard(session: TerminalSession?) = Unit
            override fun onBell(session: TerminalSession) = Unit
            override fun onColorsChanged(session: TerminalSession) = Unit
            override fun onTerminalCursorStateChange(state: Boolean) = Unit
            override fun getTerminalCursorStyle(): Int? = null
            override fun logError(tag: String?, message: String?) = Unit
            override fun logWarn(tag: String?, message: String?) = Unit
            override fun logInfo(tag: String?, message: String?) = Unit
            override fun logDebug(tag: String?, message: String?) = Unit
            override fun logVerbose(tag: String?, message: String?) = Unit
            override fun logStackTraceWithMessage(tag: String?, message: String?, e: Exception?) = Unit
            override fun logStackTrace(tag: String?, e: Exception?) = Unit
        }
        val transport = object : TerminalSession.Transport {
            override fun write(data: ByteArray, offset: Int, count: Int) = Unit
            override fun resize(columns: Int, rows: Int) = Unit
            override fun close() = Unit
        }
        val s = TerminalSession(transport, 500, client)
        val bytes = text.toByteArray()
        s.appendRemoteOutput(bytes, 0, bytes.size)
        return s
    }

    fun tabStatus() = TabStatus.Connected
}
