package cn.lemonit.galaxy.core

import android.content.Context
import android.os.Build
import cn.lemonit.galaxy.BuildConfig
import cn.lemonit.galaxy.core.bridge.CoreConfig
import cn.lemonit.galaxy.core.bridge.DeviceKind
import cn.lemonit.galaxy.core.bridge.GalaxyCore
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.mock.MockCore

/**
 * 界面拿内核实现的唯一入口。
 *
 * - 默认（`galaxy.core=rust`，BuildConfig.USE_MOCK_CORE = false）：通过反射加载 [RUST_ADAPTER]
 *   （src/rust/java，把 uniffi 生成的 uniffi.galaxy_ffi.* 包成 bridge 接口），APK 里带三个 ABI 的 libgalaxy_ffi.so。
 * - `-Pgalaxy.core=mock` 或 `-Pgalaxy.skipRust=true`：模拟实现 MockCore，不需要 .so（截图测试也用它）。
 * 界面代码不直接引用任何具体实现。
 */
object CoreProvider {
    const val RUST_ADAPTER = "cn.lemonit.galaxy.core.rust.UniffiCore"

    @Volatile
    private var core: GalaxyCore? = null

    /** 测试或预览时可以注入别的实现。 */
    fun install(instance: GalaxyCore) {
        core = instance
    }

    fun get(context: Context): GalaxyCore {
        core?.let { return it }
        return synchronized(this) {
            core ?: create(context.applicationContext).also { core = it }
        }
    }

    val isMock: Boolean get() = core is MockCore

    private fun create(context: Context): GalaxyCore {
        val instance: GalaxyCore = if (BuildConfig.USE_MOCK_CORE) {
            MockCore(assets = { name -> context.assets.open(name).use { it.readBytes() } })
        } else {
            val cls = Class.forName(RUST_ADAPTER)
            cls.getDeclaredConstructor().newInstance() as GalaxyCore
        }
        instance.coreInit(configFor(context))
        return instance
    }

    fun configFor(context: Context): CoreConfig {
        val locale = context.resources.configuration.locales[0]
        val pkg = context.packageManager.getPackageInfo(context.packageName, 0)
        return CoreConfig(
            dataDir = context.filesDir.resolve("core").apply { mkdirs() }.path,
            cacheDir = context.cacheDir.resolve("core").apply { mkdirs() }.path,
            logLevel = if (BuildConfig.DEBUG) "debug" else "info",
            platform = Platform.ANDROID,
            deviceKind = DeviceKind.MOBILE,
            appVersion = pkg.versionName ?: "0",
            osVersion = "Android ${Build.VERSION.RELEASE}",
            hostname = Build.MODEL ?: "Android",
            locale = if (locale.language == "zh") "zh-CN" else "en",
        )
    }
}
