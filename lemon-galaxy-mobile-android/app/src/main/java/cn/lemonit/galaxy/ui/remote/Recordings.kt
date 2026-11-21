package cn.lemonit.galaxy.ui.remote

import android.content.ClipData
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import androidx.annotation.RequiresApi
import androidx.core.content.FileProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.io.IOException

/**
 * 远程桌面录像的存放、分享与保存到相册。
 * 存在应用专属的外部目录（不需要存储权限，卸载时一起删）；分享经 FileProvider 授权读取。
 */
object Recordings {
    private const val SUB = "LemonGalaxy"

    /** 应用专属外部存储的 Movies/LemonGalaxy；取不到外部存储时退回内部 files/Recordings。 */
    fun dir(context: Context): File =
        context.getExternalFilesDir(Environment.DIRECTORY_MOVIES)?.resolve(SUB) ?: context.filesDir.resolve("Recordings")

    /** 给人看的保存位置：去掉内部存储根前缀。 */
    fun displayPath(path: String): String {
        val root = runCatching { Environment.getExternalStorageDirectory()?.path }.getOrNull()
        return if (root != null && path.startsWith("$root/")) path.removePrefix("$root/") else path
    }

    /** Android 10 起写 MediaStore 不需要权限；8、9 上不提供「保存到相册」。 */
    val canSaveToGallery: Boolean get() = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q

    fun share(context: Context, paths: List<String>, chooserTitle: String) {
        val authority = context.packageName + ".updates"
        val uris = paths.map { FileProvider.getUriForFile(context, authority, File(it)) }
        if (uris.isEmpty()) return
        val intent = if (uris.size == 1) {
            Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_STREAM, uris[0])
        } else {
            Intent(Intent.ACTION_SEND_MULTIPLE).putParcelableArrayListExtra(Intent.EXTRA_STREAM, ArrayList(uris))
        }
        intent.type = "video/mp4"
        intent.clipData = ClipData.newUri(context.contentResolver, "video", uris[0]).apply { uris.drop(1).forEach { addItem(ClipData.Item(it)) } }
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        val chooser = Intent.createChooser(intent, chooserTitle)
        if (context !is android.app.Activity) chooser.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        context.startActivity(chooser)
    }

    /** 复制到系统相册的 Movies/LemonGalaxy（IS_PENDING 写完再公开）。 */
    @RequiresApi(Build.VERSION_CODES.Q)
    suspend fun saveToGallery(context: Context, paths: List<String>) = withContext(Dispatchers.IO) {
        val resolver = context.contentResolver
        val collection = MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        paths.forEach { p ->
            val f = File(p)
            val values = ContentValues().apply {
                put(MediaStore.Video.Media.DISPLAY_NAME, f.name)
                put(MediaStore.Video.Media.MIME_TYPE, "video/mp4")
                put(MediaStore.Video.Media.RELATIVE_PATH, Environment.DIRECTORY_MOVIES + "/" + SUB)
                put(MediaStore.Video.Media.IS_PENDING, 1)
            }
            val uri = resolver.insert(collection, values) ?: throw IOException("MediaStore insert failed")
            try {
                val out = resolver.openOutputStream(uri) ?: throw IOException("MediaStore open failed")
                out.use { o -> f.inputStream().use { it.copyTo(o) } }
                resolver.update(uri, ContentValues().apply { put(MediaStore.Video.Media.IS_PENDING, 0) }, null, null)
            } catch (e: Exception) {
                resolver.delete(uri, null, null)
                throw e
            }
        }
    }
}
