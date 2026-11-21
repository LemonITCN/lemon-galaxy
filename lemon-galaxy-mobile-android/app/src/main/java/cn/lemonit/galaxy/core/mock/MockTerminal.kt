package cn.lemonit.galaxy.core.mock

import cn.lemonit.galaxy.core.bridge.FileKind
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.TerminalChannelInterface
import cn.lemonit.galaxy.core.bridge.TerminalSink
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.random.Random

/**
 * 模拟被控端的一个终端会话：行编辑 + 少量命令 + 全屏 top。
 * 会话在「被控端」常驻：detach 后保留，重新 attach 时回放最近的输出。
 */
internal class MockShell(
    val id: String,
    private val hostname: String,
    private val user: String,
    private val platform: Platform,
    private val fs: MockFs,
    private val scope: CoroutineScope,
    private val tr: (String, String) -> String,
    private val onEnded: (MockShell) -> Unit,
) {
    val createdAt = System.currentTimeMillis()
    val title: String get() = if (platform == Platform.WINDOWS) "PowerShell" else if (platform == Platform.MACOS) "zsh" else "bash"

    @Volatile var sink: TerminalSink? = null
    val attached: Boolean get() = sink != null

    private val scrollback = java.io.ByteArrayOutputStream()
    private val input = Channel<ByteArray>(Channel.UNLIMITED)
    private var cwd = fs.home
    private val line = StringBuilder()
    private val history = mutableListOf<String>()
    private var historyIndex = 0
    private var escape: StringBuilder? = null
    private var topJob: Job? = null
    private var cols = 80
    private var rows = 24
    private var ended = false

    private val isWindows get() = platform == Platform.WINDOWS

    fun start() {
        scope.launch {
            for (bytes in input) handle(bytes)
        }
        val banner = if (isWindows) {
            "Windows PowerShell\r\nCopyright (C) Microsoft Corporation. All rights reserved.\r\n\r\n"
        } else {
            val os = if (platform == Platform.MACOS) "Darwin 25.2.0 arm64" else "Linux 6.8.0-49-generic x86_64"
            "\u001b[1mLemonGalaxy\u001b[0m ${tr("模拟终端", "mock terminal")} · $os\r\n" +
                "${tr("输入", "Type")} \u001b[1;32mhelp\u001b[0m ${tr("查看可用命令", "to list commands")}.\r\n\r\n" +
                "Last login: Sat Nov 21 21:12:04 2026 from 192.168.1.20\r\n"
        }
        out(banner)
        prompt()
    }

    fun attach(newSink: TerminalSink) {
        sink = newSink
        val replay = synchronized(scrollback) { scrollback.toByteArray() }
        if (replay.isNotEmpty()) newSink.onOutput(replay)
    }

    fun write(data: ByteArray) {
        input.trySend(data)
    }

    fun resize(c: Int, r: Int) {
        cols = c
        rows = r
    }

    fun kill() {
        end(null)
    }

    private fun end(code: Int?) {
        if (ended) return
        ended = true
        topJob?.cancel()
        input.close()
        sink?.onExit(code)
        onEnded(this)
    }

    private fun out(s: String) {
        val bytes = s.toByteArray()
        synchronized(scrollback) {
            scrollback.write(bytes)
            if (scrollback.size() > 64 * 1024) {
                val all = scrollback.toByteArray()
                scrollback.reset()
                scrollback.write(all, all.size - 32 * 1024, 32 * 1024)
            }
        }
        sink?.onOutput(bytes)
    }

    private fun prompt() {
        if (isWindows) {
            out("PS ${winPath(cwd)}> ")
        } else {
            val dir = if (cwd == fs.home) "~" else if (cwd.startsWith(fs.home + "/")) "~" + cwd.removePrefix(fs.home) else cwd
            out("\u001b[1;32m$user@$hostname\u001b[0m:\u001b[1;34m$dir\u001b[0m$ ")
        }
    }

    private fun winPath(p: String) = "C:" + p.replace('/', '\\')

    private fun handle(bytes: ByteArray) {
        if (topJob != null) {
            if (bytes.any { it == 'q'.code.toByte() || it == 3.toByte() }) stopTop()
            return
        }
        val text = bytes.decodeToString()
        var i = 0
        while (i < text.length) {
            val ch = text[i]
            val esc = escape
            if (esc != null && esc.isEmpty() && ch != '[' && ch != 'O') {
                // 单独的 Esc（或 Alt+键）：丢掉 Esc，字符照常处理
                escape = null
            } else if (esc != null) {
                esc.append(ch)
                if (esc.length >= 2 && (ch.isLetter() || ch == '~')) {
                    onEscape(esc.toString())
                    escape = null
                }
                i++
                continue
            }
            when (ch) {
                '\u001b' -> escape = StringBuilder()
                '\r', '\n' -> {
                    out("\r\n")
                    val cmd = line.toString()
                    line.clear()
                    if (cmd.isNotBlank()) {
                        history.add(cmd)
                    }
                    historyIndex = history.size
                    run(cmd.trim())
                    if (ended) return
                    if (topJob == null) prompt()
                    // \r\n 组合只算一次回车
                    if (ch == '\r' && i + 1 < text.length && text[i + 1] == '\n') i++
                }
                '\u007f', '\b' -> if (line.isNotEmpty()) {
                    line.deleteCharAt(line.length - 1)
                    out("\b \b")
                }
                '\u0003' -> {
                    out("^C\r\n")
                    line.clear()
                    prompt()
                }
                '\u0004' -> if (line.isEmpty()) {
                    out("logout\r\n")
                    end(0)
                    return
                }
                '\u000c' -> {
                    out("\u001b[2J\u001b[H")
                    prompt()
                    out(line.toString())
                }
                '\t' -> out("\u0007")
                else -> if (ch >= ' ') {
                    line.append(ch)
                    out(ch.toString())
                }
            }
            i++
        }
    }

    private fun onEscape(seq: String) {
        when (seq) {
            "[A", "OA" -> if (historyIndex > 0) {
                historyIndex--
                replaceLine(history[historyIndex])
            }
            "[B", "OB" -> if (historyIndex < history.size) {
                historyIndex++
                replaceLine(if (historyIndex == history.size) "" else history[historyIndex])
            }
        }
    }

    private fun replaceLine(s: String) {
        repeat(line.length) { out("\b \b") }
        line.clear()
        line.append(s)
        out(s)
    }

    private fun run(cmdLine: String) {
        if (cmdLine.isEmpty()) return
        val parts = cmdLine.split(Regex("\\s+"))
        val cmd = parts[0]
        val args = parts.drop(1)
        if (isWindows) runWindows(cmd.lowercase(), args, cmdLine) else runPosix(cmd, args, cmdLine)
    }

    private fun runPosix(cmd: String, args: List<String>, full: String) {
        when (cmd) {
            "help" -> out(
                tr(
                    "可用命令：ls [-l] cd pwd cat whoami hostname uname date uptime df free ps echo clear top exit\r\n",
                    "Commands: ls [-l] cd pwd cat whoami hostname uname date uptime df free ps echo clear top exit\r\n",
                ),
            )
            "ls", "ll" -> ls(args.filter { !it.startsWith("-") }.firstOrNull(), cmd == "ll" || args.any { it.startsWith("-l") }, args.any { it.contains('a') && it.startsWith("-") })
            "cd" -> {
                val target = fs.resolve(cwd, args.firstOrNull()?.replace("~", fs.home) ?: fs.home)
                val node = fs.find(target)
                when {
                    node == null -> out("bash: cd: ${args.first()}: No such file or directory\r\n")
                    node.kind != FileKind.DIR && node.kind != FileKind.SYMLINK -> out("bash: cd: ${args.first()}: Not a directory\r\n")
                    node.kind == FileKind.SYMLINK -> cwd = fs.resolve(target.substringBeforeLast('/'), node.linkTarget ?: "")
                    else -> cwd = target
                }
            }
            "pwd" -> out("$cwd\r\n")
            "cat" -> args.forEach { a ->
                val node = fs.find(fs.resolve(cwd, a))
                when {
                    node == null -> out("cat: $a: No such file or directory\r\n")
                    node.kind == FileKind.DIR -> out("cat: $a: Is a directory\r\n")
                    node.content == null -> out("cat: $a: binary file\r\n")
                    else -> out(node.content!!.decodeToString().replace("\n", "\r\n"))
                }
            }
            "whoami" -> out("$user\r\n")
            "hostname" -> out("$hostname\r\n")
            "uname" -> out(
                if (platform == Platform.MACOS) "Darwin $hostname 25.2.0 Darwin Kernel Version 25.2.0 arm64\r\n"
                else if (args.contains("-a")) "Linux $hostname 6.8.0-49-generic #49-Ubuntu SMP x86_64 GNU/Linux\r\n" else "Linux\r\n",
            )
            "date" -> out(SimpleDateFormat("EEE MMM d HH:mm:ss zzz yyyy", Locale.US).format(Date()) + "\r\n")
            "uptime" -> out(" 21:30:12 up 12 days,  3:41,  2 users,  load average: 0.42, 0.38, 0.35\r\n")
            "df" -> out(
                "Filesystem      Size  Used Avail Use% Mounted on\r\n" +
                    "/dev/sda1        80G   69G   11G  87% /\r\n" +
                    "tmpfs           3.9G     0  3.9G   0% /dev/shm\r\n" +
                    "/dev/sdb1       500G  212G  288G  43% /data\r\n",
            )
            "free" -> out(
                "               total        used        free      shared  buff/cache   available\r\n" +
                    "Mem:           7.8Gi       2.1Gi       1.2Gi        12Mi       4.5Gi       5.4Gi\r\n" +
                    "Swap:          2.0Gi          0B       2.0Gi\r\n",
            )
            "ps" -> out(
                "    PID TTY          TIME CMD\r\n   2817 pts/0    00:00:00 bash\r\n   3120 pts/0    00:00:00 ps\r\n",
            )
            "echo" -> out(full.removePrefix("echo").trim().trim('"', '\'') + "\r\n")
            "clear" -> out("\u001b[2J\u001b[H")
            "top", "htop" -> startTop()
            "exit", "logout" -> {
                out("logout\r\n")
                end(0)
            }
            "sudo" -> out("sudo: ${tr("模拟终端不支持提权", "privilege escalation is not available in the mock terminal")}\r\n")
            else -> out("$cmd: command not found\r\n")
        }
    }

    private fun runWindows(cmd: String, args: List<String>, full: String) {
        when (cmd) {
            "help" -> out(tr("可用命令：dir ls cd pwd type whoami hostname Get-Process cls exit\r\n", "Commands: dir ls cd pwd type whoami hostname Get-Process cls exit\r\n"))
            "dir", "ls", "get-childitem", "gci" -> {
                val entries = runCatching { fs.list(cwd) }.getOrDefault(emptyList())
                out("\r\n    Directory: ${winPath(cwd)}\r\n\r\n")
                out("Mode                 LastWriteTime         Length Name\r\n")
                out("----                 -------------         ------ ----\r\n")
                val fmt = SimpleDateFormat("yyyy/M/d     HH:mm", Locale.US)
                entries.forEach { e ->
                    val mode = if (e.kind == FileKind.DIR) "d----" else "-a---"
                    val len = if (e.kind == FileKind.DIR) "" else e.size.toString()
                    out("%-6s %20s %14s %s\r\n".format(mode, fmt.format(Date(e.modifiedAt)), len, e.name))
                }
                out("\r\n")
            }
            "cd", "set-location" -> {
                val a = args.firstOrNull()?.replace('\\', '/')?.removePrefix("C:") ?: fs.home
                val target = fs.resolve(cwd, a)
                if (fs.find(target)?.kind == FileKind.DIR) cwd = target
                else out("Set-Location: Cannot find path '${args.firstOrNull()}' because it does not exist.\r\n")
            }
            "pwd", "get-location" -> out("\r\nPath\r\n----\r\n${winPath(cwd)}\r\n\r\n")
            "type", "cat", "get-content" -> args.forEach { a ->
                val node = fs.find(fs.resolve(cwd, a.replace('\\', '/')))
                if (node?.content != null) out(node.content!!.decodeToString().replace("\n", "\r\n"))
                else out("Get-Content: Cannot find path '$a'.\r\n")
            }
            "whoami" -> out("${hostname.lowercase()}\\$user\r\n")
            "hostname" -> out("$hostname\r\n")
            "get-process", "gps" -> out(
                "\r\nHandles  NPM(K)    PM(K)      WS(K)     CPU(s)     Id  SI ProcessName\r\n" +
                    "-------  ------    -----      -----     ------     --  -- -----------\r\n" +
                    "    512      28    61240      78412      12.31   4120   1 explorer\r\n" +
                    "    238      15    40012      52100       3.02   6012   1 powershell\r\n" +
                    "    905      41   182004     210336      88.70   7780   1 chrome\r\n\r\n",
            )
            "cls", "clear" -> out("\u001b[2J\u001b[H")
            "exit" -> end(0)
            "echo", "write-output" -> out(full.substringAfter(' ').trim('"', '\'') + "\r\n")
            else -> out(
                "$cmd : The term '$cmd' is not recognized as the name of a cmdlet, function, script file, or operable program.\r\n",
            )
        }
    }

    private fun ls(arg: String?, long: Boolean, all: Boolean) {
        val path = fs.resolve(cwd, arg ?: ".")
        val node = fs.find(path)
        if (node == null) {
            out("ls: cannot access '$arg': No such file or directory\r\n")
            return
        }
        val entries = if (node.kind == FileKind.DIR) fs.list(path) else listOf(fs.entry(path, node))
        val shown = entries.filter { all || !it.name.startsWith(".") }.sortedBy { it.name.lowercase() }
        if (long) {
            out("total ${shown.size * 4}\r\n")
            val fmt = SimpleDateFormat("MMM d HH:mm", Locale.US)
            shown.forEach { e ->
                val perms = (if (e.kind == FileKind.DIR) "d" else if (e.kind == FileKind.SYMLINK) "l" else "-") + permString(e.mode)
                val name = colored(e.name, e.kind, e.mode) + (e.linkTarget?.let { " -> $it" } ?: "")
                out("%s %2d %-8s %-8s %10d %s %s\r\n".format(perms, 1, e.owner, e.group, e.size.toLong(), fmt.format(Date(e.modifiedAt)), name))
            }
        } else {
            out(shown.joinToString("  ") { colored(it.name, it.kind, it.mode) } + "\r\n")
        }
    }

    private fun colored(name: String, kind: FileKind, mode: UInt) = when {
        kind == FileKind.DIR -> "\u001b[1;34m$name\u001b[0m"
        kind == FileKind.SYMLINK -> "\u001b[1;36m$name\u001b[0m"
        mode and 0x40u != 0u -> "\u001b[1;32m$name\u001b[0m"
        else -> name
    }

    private fun startTop() {
        out("\u001b[?1049h\u001b[?25l")
        topJob = scope.launch {
            var tick = 0
            while (isActive) {
                out(renderTop(tick++))
                delay(1000)
            }
        }
    }

    private fun stopTop() {
        topJob?.cancel()
        topJob = null
        out("\u001b[?25h\u001b[?1049l")
        prompt()
    }

    private fun renderTop(tick: Int): String {
        val r = Random(tick)
        val sb = StringBuilder("\u001b[H\u001b[2J")
        val load = "%.2f, %.2f, %.2f".format(0.3 + r.nextDouble() * 0.4, 0.38, 0.35)
        sb.append("top - 21:30:${(12 + tick) % 60} up 12 days,  3:41,  2 users,  load average: $load\r\n")
        sb.append("Tasks: \u001b[1m143\u001b[0m total,   \u001b[1m1\u001b[0m running, \u001b[1m142\u001b[0m sleeping\r\n")
        val us = 3 + r.nextInt(8)
        sb.append("%%Cpu(s): \u001b[1m%4.1f\u001b[0m us,  1.2 sy,  0.0 ni, %4.1f id,  0.1 wa\r\n".format(us.toDouble(), 96.0 - us))
        sb.append("MiB Mem :   7951.2 total,   1221.4 free,   2150.8 used,   4579.0 buff/cache\r\n\r\n")
        sb.append("\u001b[7m    PID USER      PR  NI    VIRT    RES  %CPU  %MEM     TIME+ COMMAND            \u001b[0m\r\n")
        val procs = listOf("nginx", "postgres", "galaxy-worker", "node", "redis-server", "sshd", "systemd", "cron")
        procs.forEachIndexed { i, p ->
            val cpu = if (i < 3) r.nextDouble() * 6 else r.nextDouble()
            sb.append(
                "%7d %-8s  20   0 %7d %6d %5.1f %5.1f %9s %-18s\r\n".format(
                    1000 + i * 137, if (i == 2) "galaxy" else "root", 200_000 + i * 9000, 30_000 + i * 2100,
                    cpu, 0.4 + i * 0.3, "${12 + i}:0${i}.31", p,
                ),
            )
        }
        sb.append("\r\n\u001b[2m${tr("按 q 退出", "Press q to quit")}\u001b[0m")
        return sb.toString()
    }

    private fun permString(mode: UInt): String {
        val chars = "rwxrwxrwx"
        return (0 until 9).joinToString("") { i -> if (mode and (1u shl (8 - i)) != 0u) chars[i].toString() else "-" }
    }
}

internal class MockTerminalChannel(private val shell: MockShell) : TerminalChannelInterface {
    override fun sessionId(): String = shell.id
    override fun write(data: ByteArray) = shell.write(data)
    override fun resize(cols: UShort, rows: UShort) = shell.resize(cols.toInt(), rows.toInt())
    override fun detach() {
        shell.sink = null
    }

    override fun kill() = shell.kill()
}
