package cn.lemonit.galaxy.ui.files

import cn.lemonit.galaxy.app.ConnectionManager
import cn.lemonit.galaxy.app.TargetRef
import cn.lemonit.galaxy.app.userMessage
import cn.lemonit.galaxy.core.bridge.FileEntry
import cn.lemonit.galaxy.core.bridge.FileKind
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

enum class SortKey { NAME, SIZE, MODIFIED }

data class FileBrowserState(
    val path: String = "",
    val home: String = "",
    val entries: List<FileEntry> = emptyList(),
    val loading: Boolean = true,
    val error: String? = null,
    val sort: SortKey = SortKey.NAME,
    val descending: Boolean = false,
    val showHidden: Boolean = false,
) {
    /** 排序后的可见条目：文件夹在前。 */
    val visible: List<FileEntry>
        get() {
            val list = entries.filter { showHidden || !it.name.startsWith(".") }
            val cmp: Comparator<FileEntry> = when (sort) {
                SortKey.NAME -> compareBy { it.name.lowercase() }
                SortKey.SIZE -> compareBy { it.size }
                SortKey.MODIFIED -> compareBy { it.modifiedAt }
            }
            val ordered = if (descending) cmp.reversed() else cmp
            return list.sortedWith(compareBy<FileEntry> { if (it.kind == FileKind.DIR) 0 else 1 }.then(ordered))
        }
}

/** 一个远端文件浏览器（设备走 PeerSession.files()，SSH 主机走 SFTP）。 */
class FileBrowserController(
    private val target: TargetRef,
    private val conn: ConnectionManager,
    private val scope: CoroutineScope,
    private val onError: (String) -> Unit,
) {
    private val _state = MutableStateFlow(FileBrowserState())
    val state: StateFlow<FileBrowserState> = _state.asStateFlow()

    fun start() {
        scope.launch {
            try {
                val files = conn.files(target)
                val home = files.home()
                _state.update { it.copy(home = home) }
                open(home)
            } catch (e: Exception) {
                _state.update { it.copy(loading = false, error = e.userMessage()) }
            }
        }
    }

    fun open(path: String) {
        _state.update { it.copy(loading = true, error = null) }
        scope.launch {
            try {
                val list = conn.files(target).list(path)
                _state.update { it.copy(path = path, entries = list, loading = false) }
            } catch (e: Exception) {
                _state.update { it.copy(loading = false, error = e.userMessage()) }
            }
        }
    }

    fun refresh() = open(_state.value.path)

    fun up() {
        val p = _state.value.path
        if (p == "/" || p.isEmpty()) return
        open(p.substringBeforeLast('/').ifEmpty { "/" })
    }

    fun setSort(sort: SortKey) = _state.update { if (it.sort == sort) it.copy(descending = !it.descending) else it.copy(sort = sort, descending = sort != SortKey.NAME) }
    fun toggleHidden() = _state.update { it.copy(showHidden = !it.showHidden) }

    private fun op(block: suspend () -> Unit) {
        scope.launch {
            try {
                block()
            } catch (e: Exception) {
                e.userMessage().takeIf { it.isNotEmpty() }?.let(onError)
            }
            refresh()
        }
    }

    private fun child(name: String) = _state.value.path.trimEnd('/') + "/" + name

    fun mkdir(name: String) = op { conn.files(target).mkdir(child(name)) }
    fun rename(e: FileEntry, newName: String) = op { conn.files(target).rename(e.path, e.path.substringBeforeLast('/') + "/" + newName) }
    fun delete(e: FileEntry) = op { conn.files(target).remove(e.path, e.kind == FileKind.DIR) }
    fun chmod(e: FileEntry, mode: UInt) = op { conn.files(target).chmod(e.path, mode) }

    suspend fun readText(path: String): String = conn.files(target).readText(path, MAX_EDIT_BYTES)
    suspend fun writeText(path: String, text: String) = conn.files(target).writeText(path, text)

    companion object {
        const val MAX_EDIT_BYTES: ULong = 2_097_152uL

        fun permString(kind: FileKind, mode: UInt): String {
            val t = when (kind) {
                FileKind.DIR -> "d"
                FileKind.SYMLINK -> "l"
                FileKind.OTHER -> "s"
                FileKind.FILE -> "-"
            }
            val chars = "rwxrwxrwx"
            return t + (0 until 9).joinToString("") { i -> if (mode and (1u shl (8 - i)) != 0u) chars[i].toString() else "-" }
        }

        private val TEXT_EXT = setOf(
            "txt", "md", "log", "conf", "cfg", "ini", "json", "yaml", "yml", "toml", "xml", "html", "css", "js", "ts", "kt", "java",
            "rs", "go", "py", "rb", "sh", "bash", "zsh", "c", "h", "cpp", "hpp", "sql", "env", "properties", "gradle", "csv", "service",
        )

        /** 看扩展名猜是否文本（真正判断交给 readText 的结果）。 */
        fun looksText(e: FileEntry): Boolean {
            if (e.kind != FileKind.FILE) return false
            val ext = e.name.substringAfterLast('.', "").lowercase()
            return ext in TEXT_EXT || (!e.name.contains('.') && e.size < 512uL * 1024uL) || e.name.startsWith(".")
        }
    }
}
