package cn.lemonit.galaxy.core.rust

import android.content.Context
import android.util.Log
import androidx.annotation.Keep

/**
 * 安卓专用：把 JavaVM 与 Application Context 登记给内核（galaxy-ffi 的 src/android.rs）。
 * 内核的声音输出（cpal 的 AAudio 后端）要用它们；没登记时开始播放远端声音会报错，其余功能不受影响。
 * 包名、类名、方法名与内核导出的 JNI 函数 Java_cn_lemonit_galaxy_core_rust_NativeInit_init 一一对应，不能改；
 * release 包里靠 proguard-rules.pro 与 @Keep 保留。
 */
@Keep
object NativeInit {
    private const val TAG = "NativeInit"

    @JvmStatic
    external fun init(context: Context)

    /** 加载 libgalaxy_ffi.so（与 JNA 加载的是同一个文件）并登记；要在 core_init 之前调。失败只记日志，声音不可用。 */
    fun register(context: Context) {
        try {
            System.loadLibrary("galaxy_ffi")
            init(context.applicationContext)
        } catch (e: UnsatisfiedLinkError) {
            Log.w(TAG, "内核声音输出初始化失败，远端声音不可用", e)
        } catch (e: Exception) {
            Log.w(TAG, "内核声音输出初始化失败，远端声音不可用", e)
        }
    }
}
