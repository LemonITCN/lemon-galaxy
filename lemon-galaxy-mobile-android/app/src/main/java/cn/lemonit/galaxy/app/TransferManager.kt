package cn.lemonit.galaxy.app

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import cn.lemonit.galaxy.core.bridge.TransferInterface
import cn.lemonit.galaxy.core.bridge.TransferListener
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.util.concurrent.atomic.AtomicLong

enum class TransferDirection { DOWNLOAD, UPLOAD }

sealed class TransferState {
    data object Queued : TransferState()
    data object Running : TransferState()
    data object Paused : TransferState()
    data object Done : TransferState()
    data object Cancelled : TransferState()
    data class Failed(val message: String) : TransferState()

    val finished get() = this is Done || this is Cancelled || this is Failed
}

data class TransferItem(
    val id: Long,
    val direction: TransferDirection,
    val name: String,
    val remotePath: String,
    val targetName: String,
    val state: TransferState,
    val done: Long,
    val total: Long,
    val bytesPerSecond: Long = 0,
) {
    val progress: Float get() = if (total <= 0) 0f else (done.toFloat() / total).coerceIn(0f, 1f)
}

/**
 * 传输队列。内核的 download/upload 只认本机路径，安卓的存储访问框架给的是 content:// Uri，所以：
 *  - 下载：先下到应用缓存目录，完成后复制到用户选的 Uri；
 *  - 上传：先把 Uri 内容复制到缓存，再交给内核上传。
 * （建议 ffi.md 支持文件描述符或流回调，省掉这次复制，见报告。）
 */
class TransferManager(
    private val context: Context?,
    private val scope: CoroutineScope,
    private val conn: ConnectionManager,
) {
    private val ids = AtomicLong(1)
    private val _items = MutableStateFlow<List<TransferItem>>(emptyList())
    val items: StateFlow<List<TransferItem>> = _items.asStateFlow()
    private val handles = mutableMapOf<Long, TransferInterface>()
    private val retries = mutableMapOf<Long, () -> Unit>()

    private fun cacheDir() = File(context?.cacheDir ?: File(System.getProperty("java.io.tmpdir") ?: "/tmp"), "transfers").apply { mkdirs() }

    private fun patch(id: Long, block: (TransferItem) -> TransferItem) {
        _items.update { list -> list.map { if (it.id == id) block(it) else it } }
    }

    private fun listener(id: Long, onSuccess: suspend () -> Unit): TransferListener = object : TransferListener {
        private var lastAt = System.currentTimeMillis()
        private var lastDone = 0L
        override fun onProgress(done: ULong, total: ULong) {
            val now = System.currentTimeMillis()
            val d = done.toLong()
            val dt = now - lastAt
            if (dt >= 500) {
                val speed = (d - lastDone).coerceAtLeast(0) * 1000 / dt
                lastAt = now
                lastDone = d
                patch(id) { it.copy(done = d, total = total.toLong(), bytesPerSecond = speed, state = if (it.state == TransferState.Paused) it.state else TransferState.Running) }
            } else {
                patch(id) { it.copy(done = d, total = total.toLong(), state = if (it.state == TransferState.Paused) it.state else TransferState.Running) }
            }
        }

        override fun onFinished(error: String?) {
            handles.remove(id)
            if (error == null) {
                scope.launch {
                    try {
                        onSuccess()
                        patch(id) { it.copy(state = TransferState.Done, done = it.total, bytesPerSecond = 0) }
                    } catch (e: Exception) {
                        patch(id) { it.copy(state = TransferState.Failed(e.userMessage()), bytesPerSecond = 0) }
                    }
                }
            } else if (error == "cancelled") {
                patch(id) { it.copy(state = TransferState.Cancelled, bytesPerSecond = 0) }
            } else {
                patch(id) { it.copy(state = TransferState.Failed(error), bytesPerSecond = 0) }
            }
        }
    }

    /** 下载远端文件到用户选择的 Uri。 */
    fun download(target: TargetRef, targetName: String, remotePath: String, size: Long, dest: Uri) {
        val id = ids.getAndIncrement()
        val name = remotePath.substringAfterLast('/')
        _items.update { it + TransferItem(id, TransferDirection.DOWNLOAD, name, remotePath, targetName, TransferState.Queued, 0, size) }
        val start = {
            patch(id) { it.copy(state = TransferState.Queued, done = 0) }
            scope.launch {
                try {
                    val files = conn.files(target)
                    val tmp = File(cacheDir(), "dl-$id-$name")
                    handles[id] = files.download(remotePath, tmp.path, listener(id) {
                        withContext(Dispatchers.IO) {
                            val cr = context?.contentResolver ?: error("no context")
                            cr.openOutputStream(dest, "w")?.use { out -> tmp.inputStream().use { it.copyTo(out) } }
                                ?: error("cannot open destination")
                            tmp.delete()
                        }
                    })
                } catch (e: Exception) {
                    patch(id) { it.copy(state = TransferState.Failed(e.userMessage())) }
                }
            }
            Unit
        }
        retries[id] = start
        start()
    }

    /** 上传本机 Uri 到远端目录。 */
    fun upload(target: TargetRef, targetName: String, source: Uri, remoteDir: String, onDone: () -> Unit) {
        val id = ids.getAndIncrement()
        val name = displayName(source) ?: "upload-$id"
        val remote = remoteDir.trimEnd('/') + "/" + name
        _items.update { it + TransferItem(id, TransferDirection.UPLOAD, name, remote, targetName, TransferState.Queued, 0, 0) }
        val start = {
            patch(id) { it.copy(state = TransferState.Queued, done = 0) }
            scope.launch {
                try {
                    val tmp = File(cacheDir(), "ul-$id-$name")
                    withContext(Dispatchers.IO) {
                        val cr = context?.contentResolver ?: error("no context")
                        cr.openInputStream(source)?.use { input -> tmp.outputStream().use { input.copyTo(it) } } ?: error("cannot read source")
                    }
                    patch(id) { it.copy(total = tmp.length()) }
                    val files = conn.files(target)
                    handles[id] = files.upload(tmp.path, remote, listener(id) {
                        tmp.delete()
                        onDone()
                    })
                } catch (e: Exception) {
                    patch(id) { it.copy(state = TransferState.Failed(e.userMessage())) }
                }
            }
            Unit
        }
        retries[id] = start
        start()
    }

    private fun displayName(uri: Uri): String? = runCatching {
        context?.contentResolver?.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst()) c.getString(0) else null
        }
    }.getOrNull() ?: uri.lastPathSegment?.substringAfterLast('/')

    fun pause(id: Long) {
        handles[id]?.pause()
        patch(id) { it.copy(state = TransferState.Paused, bytesPerSecond = 0) }
    }

    fun resume(id: Long) {
        handles[id]?.resume()
        patch(id) { it.copy(state = TransferState.Running) }
    }

    fun cancel(id: Long) {
        val h = handles[id]
        if (h != null) h.cancel() else patch(id) { it.copy(state = TransferState.Cancelled) }
    }

    fun retry(id: Long) {
        retries[id]?.invoke()
    }

    fun clearFinished() {
        _items.update { list -> list.filterNot { it.state.finished } }
    }

    /** 截图测试与预览用。 */
    fun seed(items: List<TransferItem>) {
        _items.value = items
    }
}
