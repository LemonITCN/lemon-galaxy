package cn.lemonit.galaxy.core.mock

import cn.lemonit.galaxy.core.bridge.FileClientInterface
import cn.lemonit.galaxy.core.bridge.FileEntry
import cn.lemonit.galaxy.core.bridge.FileKind
import cn.lemonit.galaxy.core.bridge.GalaxyException
import cn.lemonit.galaxy.core.bridge.TransferInterface
import cn.lemonit.galaxy.core.bridge.TransferListener
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.io.File
import java.io.RandomAccessFile

/** 内存里的一棵 POSIX 文件树。 */
internal class MockFs(val home: String, private val user: String, private val tr: (String, String) -> String) {
    class Node(
        var name: String,
        val kind: FileKind,
        var mode: UInt,
        var size: ULong,
        var modifiedAt: Long,
        var owner: String,
        var group: String,
        var content: ByteArray? = null,
        val children: MutableMap<String, Node> = linkedMapOf(),
        var linkTarget: String? = null,
    )

    private val root = Node("", FileKind.DIR, 0x1EDu, 4096u, MockData.NOW, "root", "root")

    init {
        seed()
    }

    private fun seed() {
        val t = MockData.NOW
        val h = home
        dir(h, owner = user)
        file("$h/.bashrc", "# ~/.bashrc\nexport PATH=\"\$HOME/.local/bin:\$PATH\"\nalias ll='ls -alF'\n", t - 40 * DAY, 0x1A4u)
        dir("$h/.ssh", 0x1C0u, user)
        file("$h/.ssh/authorized_keys", "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMockKeyForLemonGalaxy ops@laptop\n", t - 90 * DAY, 0x180u)
        file("$h/.ssh/known_hosts", "github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl\n", t - 30 * DAY, 0x1A4u)
        dir("$h/projects", owner = user)
        dir("$h/projects/lemon-galaxy", owner = user)
        file(
            "$h/projects/lemon-galaxy/README.md",
            "# LemonGalaxy\n\nSelf-hosted remote control.\n\n- Remote desktop\n- Terminal and files\n- AI agent\n",
            t - 2 * HOUR, 0x1A4u,
        )
        file("$h/projects/lemon-galaxy/Cargo.toml", "[workspace]\nmembers = [\"lemon-galaxy-common-rust/*\"]\nresolver = \"2\"\n", t - 5 * HOUR, 0x1A4u)
        dir("$h/projects/lemon-galaxy/src", owner = user)
        file("$h/projects/lemon-galaxy/src/main.rs", "fn main() {\n    println!(\"Hello, LemonGalaxy\");\n}\n", t - 5 * HOUR, 0x1A4u)
        dir("$h/logs", owner = user)
        file("$h/logs/app.log", (1..40).joinToString("\n") { "2026-11-21T21:${(it % 60).toString().padStart(2, '0')}:00Z INFO request served in ${10 + it}ms" } + "\n", t - 3 * MIN, 0x1A4u)
        file("$h/logs/error.log", "2026-11-21T20:58:12Z WARN upstream timed out (110: Connection timed out)\n", t - 32 * MIN, 0x1A4u)
        dir("$h/Downloads", owner = user)
        blob("$h/Downloads/ubuntu-24.04.3-live-server-amd64.iso", 3_213_064_192uL, t - 9 * DAY)
        blob("$h/Downloads/photo-2026-11-20.jpg", 3_482_112uL, t - DAY)
        blob("$h/Downloads/backup-2026-11.tar.gz", 734_003_200uL, t - 6 * HOUR)
        file("$h/notes.txt", tr("待办：\n- 升级 nginx\n- 检查备份\n", "TODO:\n- upgrade nginx\n- verify backups\n"), t - 50 * MIN, 0x1A4u)
        file("$h/deploy.sh", "#!/usr/bin/env bash\nset -euo pipefail\ngit pull --ff-only\nsystemctl restart app\n", t - 4 * DAY, 0x1EDu)
        link("$h/current", "projects/lemon-galaxy")
        dir("/etc", owner = "root")
        dir("/etc/nginx", owner = "root")
        file("/etc/nginx/nginx.conf", "user www-data;\nworker_processes auto;\n\nevents {\n    worker_connections 1024;\n}\n", t - 60 * DAY, 0x1A4u, "root")
        dir("/var", owner = "root")
        dir("/var/log", owner = "root")
        file("/var/log/syslog", "Nov 21 21:30:01 web-01 CRON[1201]: (root) CMD (run-parts /etc/cron.hourly)\n", t - MIN, 0x1A0u, "root")
        dir("/tmp", 0x3FFu, "root")
    }

    private fun parentAndName(path: String): Pair<Node, String> {
        val norm = normalize(path)
        val idx = norm.lastIndexOf('/')
        val parentPath = if (idx <= 0) "/" else norm.substring(0, idx)
        val parent = find(parentPath) ?: throw GalaxyException.NotFound(tr("找不到目录：$parentPath", "No such directory: $parentPath"))
        if (parent.kind != FileKind.DIR) throw GalaxyException.Other(tr("不是目录：$parentPath", "Not a directory: $parentPath"))
        return parent to norm.substring(idx + 1)
    }

    private fun dir(path: String, mode: UInt = 0x1EDu, owner: String = user) {
        var cur = root
        for (part in normalize(path).split('/').filter { it.isNotEmpty() }) {
            cur = cur.children.getOrPut(part) { Node(part, FileKind.DIR, mode, 4096u, MockData.NOW - 10 * DAY, owner, owner) }
        }
    }

    private fun file(path: String, text: String, mtime: Long, mode: UInt, owner: String = user) {
        val (p, name) = parentAndName(path)
        val bytes = text.toByteArray()
        p.children[name] = Node(name, FileKind.FILE, mode, bytes.size.toULong(), mtime, owner, owner, bytes)
    }

    private fun blob(path: String, size: ULong, mtime: Long) {
        val (p, name) = parentAndName(path)
        p.children[name] = Node(name, FileKind.FILE, 0x1A4u, size, mtime, user, user, null)
    }

    private fun link(path: String, target: String) {
        val (p, name) = parentAndName(path)
        p.children[name] = Node(name, FileKind.SYMLINK, 0x1FFu, target.length.toULong(), MockData.NOW - 4 * DAY, user, user, linkTarget = target)
    }

    fun normalize(path: String): String {
        val parts = ArrayDeque<String>()
        for (p in path.split('/')) {
            when (p) {
                "", "." -> Unit
                ".." -> parts.removeLastOrNull()
                else -> parts.addLast(p)
            }
        }
        return "/" + parts.joinToString("/")
    }

    fun resolve(cwd: String, path: String): String = if (path.startsWith("/")) normalize(path) else normalize("$cwd/$path")

    @Synchronized
    fun find(path: String): Node? {
        var cur = root
        for (part in normalize(path).split('/').filter { it.isNotEmpty() }) {
            cur = cur.children[part] ?: return null
        }
        return cur
    }

    fun entry(path: String, node: Node) = FileEntry(
        name = node.name.ifEmpty { "/" },
        path = normalize(path),
        kind = node.kind,
        size = node.size,
        mode = node.mode,
        modifiedAt = node.modifiedAt,
        owner = node.owner,
        group = node.group,
        linkTarget = node.linkTarget,
    )

    @Synchronized
    fun list(path: String): List<FileEntry> {
        val n = find(path) ?: throw GalaxyException.NotFound(tr("找不到：$path", "Not found: $path"))
        if (n.kind != FileKind.DIR) throw GalaxyException.Other(tr("不是目录：$path", "Not a directory: $path"))
        val base = normalize(path).trimEnd('/')
        return n.children.values.map { entry("$base/${it.name}", it) }
    }

    @Synchronized
    fun mkdir(path: String) {
        val (p, name) = parentAndName(path)
        if (p.children.containsKey(name)) throw GalaxyException.Other(tr("已存在：$name", "Already exists: $name"))
        p.children[name] = Node(name, FileKind.DIR, 0x1EDu, 4096u, System.currentTimeMillis(), user, user)
    }

    @Synchronized
    fun rename(from: String, to: String) {
        val (fp, fname) = parentAndName(from)
        val node = fp.children[fname] ?: throw GalaxyException.NotFound(tr("找不到：$from", "Not found: $from"))
        val (tp, tname) = parentAndName(to)
        if (tp.children.containsKey(tname)) throw GalaxyException.Other(tr("目标已存在：$tname", "Target exists: $tname"))
        fp.children.remove(fname)
        node.name = tname
        tp.children[tname] = node
    }

    @Synchronized
    fun remove(path: String, recursive: Boolean) {
        val (p, name) = parentAndName(path)
        val node = p.children[name] ?: throw GalaxyException.NotFound(tr("找不到：$path", "Not found: $path"))
        if (node.kind == FileKind.DIR && node.children.isNotEmpty() && !recursive) {
            throw GalaxyException.Other(tr("目录不为空：$name", "Directory not empty: $name"))
        }
        p.children.remove(name)
    }

    @Synchronized
    fun write(path: String, bytes: ByteArray?, size: ULong) {
        val (p, name) = parentAndName(path)
        val existing = p.children[name]
        if (existing != null && existing.kind == FileKind.FILE) {
            existing.content = bytes
            existing.size = size
            existing.modifiedAt = System.currentTimeMillis()
        } else {
            p.children[name] = Node(name, FileKind.FILE, 0x1A4u, size, System.currentTimeMillis(), user, user, bytes)
        }
    }

    companion object {
        const val MIN = 60_000L
        const val HOUR = 60 * MIN
        const val DAY = 24 * HOUR
    }
}

internal class MockFileClient(
    private val fs: MockFs,
    private val scope: CoroutineScope,
    private val tr: (String, String) -> String,
    /** 模拟链路速度（字节/秒）。 */
    private val bytesPerSecond: Long,
) : FileClientInterface {
    override suspend fun home(): String = fs.home

    override suspend fun list(path: String): List<FileEntry> {
        delay(120)
        return fs.list(path)
    }

    override suspend fun stat(path: String): FileEntry {
        val n = fs.find(path) ?: throw GalaxyException.NotFound(tr("找不到：$path", "Not found: $path"))
        return fs.entry(path, n)
    }

    override suspend fun mkdir(path: String) {
        delay(80)
        fs.mkdir(path)
    }

    override suspend fun rename(from: String, to: String) {
        delay(80)
        fs.rename(from, to)
    }

    override suspend fun remove(path: String, recursive: Boolean) {
        delay(120)
        fs.remove(path, recursive)
    }

    override suspend fun chmod(path: String, mode: UInt) {
        delay(60)
        val n = fs.find(path) ?: throw GalaxyException.NotFound(tr("找不到：$path", "Not found: $path"))
        n.mode = mode and 0xFFFu
    }

    override suspend fun readText(path: String, maxBytes: ULong): String {
        delay(150)
        val n = fs.find(path) ?: throw GalaxyException.NotFound(tr("找不到：$path", "Not found: $path"))
        val bytes = n.content ?: throw GalaxyException.Other(tr("不是文本文件，无法打开", "Not a text file"))
        if (bytes.size.toULong() > maxBytes) throw GalaxyException.Other(tr("文件太大，无法在线编辑", "File is too large to edit"))
        if (bytes.any { it == 0.toByte() }) throw GalaxyException.Other(tr("不是文本文件，无法打开", "Not a text file"))
        return bytes.decodeToString()
    }

    override suspend fun writeText(path: String, text: String) {
        delay(150)
        val bytes = text.toByteArray()
        fs.write(path, bytes, bytes.size.toULong())
    }

    override fun download(remote: String, local: String, listener: TransferListener): TransferInterface {
        val node = fs.find(remote)
        val t = MockTransfer(scope, bytesPerSecond, listener)
        if (node == null || node.kind != FileKind.FILE) {
            t.fail(tr("找不到文件：$remote", "File not found: $remote"))
            return t
        }
        val content = node.content
        t.start(node.size.toLong()) {
            // 模拟模式：大文件没有真实内容，写入不超过 256 KB 的占位数据
            File(local).parentFile?.mkdirs()
            RandomAccessFile(local, "rw").use { f ->
                if (content != null) {
                    f.setLength(0)
                    f.write(content)
                } else {
                    val n = minOf(node.size.toLong(), 256 * 1024L).toInt()
                    f.setLength(0)
                    f.write(ByteArray(n) { (it % 251).toByte() })
                }
            }
        }
        return t
    }

    override fun upload(local: String, remote: String, listener: TransferListener): TransferInterface {
        val src = File(local)
        val t = MockTransfer(scope, bytesPerSecond, listener)
        if (!src.isFile) {
            t.fail(tr("找不到本机文件：$local", "Local file not found: $local"))
            return t
        }
        val size = src.length()
        t.start(size) {
            val bytes = if (size <= 1024 * 1024) src.readBytes() else null
            fs.write(remote, bytes, size.toULong())
        }
        return t
    }
}

/** 按速度推进进度的模拟传输，支持暂停、继续、取消。 */
internal class MockTransfer(
    private val scope: CoroutineScope,
    private val bytesPerSecond: Long,
    private val listener: TransferListener,
) : TransferInterface {
    @Volatile private var paused = false
    @Volatile private var cancelled = false
    private var job: Job? = null

    fun fail(message: String) {
        scope.launch {
            delay(200)
            listener.onFinished(message)
        }
    }

    fun start(total: Long, onComplete: () -> Unit) {
        // 让所有模拟传输在 2–8 秒内完成，便于看到进度又不至于等太久
        val speed = maxOf(bytesPerSecond, total / 8).coerceAtLeast(1)
        job = scope.launch {
            var done = 0L
            listener.onProgress(0u, total.toULong())
            while (isActive && done < total) {
                delay(200)
                if (cancelled) break
                if (paused) continue
                done = minOf(total, done + speed / 5)
                listener.onProgress(done.toULong(), total.toULong())
            }
            if (cancelled) {
                listener.onFinished("cancelled")
                return@launch
            }
            try {
                onComplete()
                listener.onFinished(null)
            } catch (e: Exception) {
                listener.onFinished(e.message ?: e.toString())
            }
        }
    }

    override fun cancel() {
        cancelled = true
    }

    override fun pause() {
        paused = true
    }

    override fun resume() {
        paused = false
    }
}
