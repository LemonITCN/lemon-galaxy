package cn.lemonit.galaxy.app

import android.content.Context
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.core.bridge.GalaxyClientInterface
import cn.lemonit.galaxy.core.bridge.TransferListener
import cn.lemonit.galaxy.core.bridge.UpdateInfo
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import java.io.File

sealed class UpdateState {
    data object Idle : UpdateState()
    data object Checking : UpdateState()
    data class UpToDate(val current: String) : UpdateState()
    data class Available(val info: UpdateInfo) : UpdateState()
    data class Downloading(val info: UpdateInfo, val done: Long, val total: Long) : UpdateState()
    data class Downloaded(val info: UpdateInfo, val file: File) : UpdateState()
    data class Failed(val message: String, val info: UpdateInfo?) : UpdateState()
}

/**
 * 检查更新（ffi.md 5.0）：向自己的服务器查询 mobile/android 的新版本（内核已验发布签名），
 * 下载到缓存目录（内核核对 SHA-256），由界面经 FileProvider 交给系统安装器。
 */
class UpdateController(
    private val client: GalaxyClientInterface,
    private val scope: CoroutineScope,
    private val appContext: Context?,
    private val currentVersion: () -> String,
    private val toast: (String) -> Unit,
) {
    private val _state = MutableStateFlow<UpdateState>(UpdateState.Idle)
    val state: StateFlow<UpdateState> = _state.asStateFlow()

    private val _dialog = MutableStateFlow(false)
    /** 更新弹框是否打开（有新版本、下载中、下载完成、下载失败）。 */
    val dialog: StateFlow<Boolean> = _dialog.asStateFlow()

    private fun str(id: Int, vararg args: Any): String = appContext?.getString(id, *args) ?: ""

    val busy: Boolean get() = _state.value is UpdateState.Checking || _state.value is UpdateState.Downloading

    fun check() {
        if (busy) return
        _state.value = UpdateState.Checking
        scope.launch {
            try {
                val info = client.checkUpdate(COMPONENT, PLATFORM, "", currentVersion())
                if (info == null) {
                    _state.value = UpdateState.UpToDate(currentVersion())
                    toast(str(R.string.update_up_to_date, currentVersion()))
                } else {
                    _state.value = UpdateState.Available(info)
                    _dialog.value = true
                }
            } catch (e: Exception) {
                val msg = str(R.string.update_check_failed, e.userMessage())
                _state.value = UpdateState.Failed(msg, null)
                toast(msg)
            }
        }
    }

    fun download() {
        val info = when (val s = _state.value) {
            is UpdateState.Available -> s.info
            is UpdateState.Failed -> s.info
            else -> null
        } ?: return
        val dir = appContext?.cacheDir?.resolve("updates") ?: return
        dir.mkdirs()
        // 只留这一个安装包，旧的删掉
        dir.listFiles()?.forEach { it.delete() }
        val name = File(info.fileName).name.ifEmpty { "LemonGalaxy-${info.version}.apk" }
        val dest = dir.resolve(name)
        _state.value = UpdateState.Downloading(info, 0, info.size.toLong())
        client.downloadUpdate(
            info, dest.path,
            object : TransferListener {
                override fun onProgress(done: ULong, total: ULong) {
                    scope.launch {
                        if (_state.value is UpdateState.Downloading) {
                            _state.value = UpdateState.Downloading(info, done.toLong(), if (total == 0uL) info.size.toLong() else total.toLong())
                        }
                    }
                }

                override fun onFinished(error: String?) {
                    scope.launch {
                        _state.value = if (error == null) UpdateState.Downloaded(info, dest) else UpdateState.Failed(str(R.string.update_download_failed, error), info)
                    }
                }
            },
        )
    }

    fun openDialog() {
        if (_state.value is UpdateState.Available || _state.value is UpdateState.Downloading || _state.value is UpdateState.Downloaded) _dialog.value = true
    }

    fun dismissDialog() {
        _dialog.value = false
    }

    companion object {
        const val COMPONENT = "mobile"
        const val PLATFORM = "android"
    }
}
