package cn.lemonit.galaxy.rust

import cn.lemonit.galaxy.core.bridge.ClientListener
import cn.lemonit.galaxy.core.bridge.ClientState
import cn.lemonit.galaxy.core.bridge.CoreConfig
import cn.lemonit.galaxy.core.bridge.DeviceKind
import cn.lemonit.galaxy.core.bridge.GalaxyCore
import cn.lemonit.galaxy.core.bridge.GalaxyException
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.UpgradeListener
import cn.lemonit.galaxy.core.platform.MemorySecureStore
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.BeforeClass
import org.junit.Test
import java.io.File
import java.nio.file.Files
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * 真内核适配层冒烟测试：在主机 JVM 上通过 JNA 加载主机版 galaxy_ffi（与安卓 .so 同源），
 * 验证生成的绑定能加载（含 uniffi 校验和）、适配层的类型转换、错误映射与本地化消息。
 * 不连服务器；-Pgalaxy.skipRust=true 或找不到主机动态库时跳过。
 */
class UniffiCoreSmokeTest {
    companion object {
        private var core: GalaxyCore? = null

        @BeforeClass
        @JvmStatic
        fun load() {
            if (System.getProperty("galaxy.skipRust") == "true") return
            val dir = System.getProperty("galaxy.hostLibDir") ?: return
            val lib = listOf("libgalaxy_ffi.dylib", "libgalaxy_ffi.so", "galaxy_ffi.dll").map { File(dir, it) }.firstOrNull { it.isFile } ?: return
            System.setProperty("jna.library.path", lib.parentFile.path)
            val cls = runCatching { Class.forName("cn.lemonit.galaxy.core.rust.UniffiCore") }.getOrNull() ?: return
            val c = cls.getDeclaredConstructor().newInstance() as GalaxyCore
            val data = Files.createTempDirectory("galaxy-smoke").toFile()
            c.coreInit(CoreConfig(data.resolve("data").path, data.resolve("cache").path, "info", Platform.ANDROID, DeviceKind.MOBILE, "0.1.0", "test", "smoke", "zh-CN"))
            core = c
        }
    }

    private fun newClient(states: MutableList<ClientState> = mutableListOf()) = core!!.newGalaxyClient(MemorySecureStore(), object : ClientListener {
        override fun onState(state: ClientState) {
            states.add(state)
        }

        override fun onDevicesChanged() = Unit
        override fun onVaultChanged() = Unit
        override fun onSessionRevoked() = Unit
    })

    @Test
    fun loadsAndStartsLoggedOut() {
        assumeTrue("主机版 galaxy_ffi 不可用，跳过", core != null)
        assertTrue(core!!.coreVersion().isNotBlank())
        val client = newClient()
        assertEquals(ClientState.LOGGED_OUT, client.state())
        assertNull(client.currentUser())
        assertFalse(client.restore())
        assertTrue(client.devices().isEmpty())
        assertTrue(client.deviceId().isNotEmpty())
    }

    @Test
    fun loginErrorsAreMappedAndLocalized() = runBlocking {
        assumeTrue("主机版 galaxy_ffi 不可用，跳过", core != null)
        val client = newClient()
        // 格式不对的地址：内核直接报错，不发请求
        val e1 = runCatching { withTimeout(20_000) { client.login("not-an-address", "alice", "pw", null) } }.exceptionOrNull()
        assertTrue("应抛 GalaxyException，实际 $e1", e1 is GalaxyException)
        assertTrue((e1 as GalaxyException).message!!.isNotBlank())
        assertFalse("消息不应带 uniffi 的 message= 前缀", e1.message!!.startsWith("message="))
        // 连不上的服务器：网络错误
        core!!.coreSetLocale("en")
        val e2 = runCatching { withTimeout(30_000) { client.login("127.0.0.1:1#sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", "alice", "pw", null) } }.exceptionOrNull()
        assertTrue("应抛 GalaxyException，实际 $e2", e2 is GalaxyException)
        assertTrue(e2!!.message!!.isNotBlank())
        core!!.coreSetLocale("zh-CN")
    }

    @Test
    fun wakeBindingsWithoutServer() = runBlocking {
        assumeTrue("主机版 galaxy_ffi 不可用，跳过", core != null)
        val client = newClient()
        // 没登录、设备列表为空：不能远程开机，硬开报错（走 GalaxyException 映射）
        assertFalse(client.canWake("00112233445566778899aabbccddeeff"))
        val e = runCatching { withTimeout(10_000) { client.wakeDevice("00112233445566778899aabbccddeeff") } }.exceptionOrNull()
        assertTrue("应抛 GalaxyException，实际 $e", e is GalaxyException)
    }

    @Test
    fun upgradeDeviceCallbackWithoutServer() {
        assumeTrue("主机版 galaxy_ffi 不可用，跳过", core != null)
        val client = newClient()
        // 没登录：立即返回，经监听器以失败结束（验证回调接口经 JNA 接通）
        val done = CountDownLatch(1)
        var started = false
        var result: Pair<String?, String?>? = null
        client.upgradeDevice("00112233445566778899aabbccddeeff", object : UpgradeListener {
            override fun onStarted(version: String) {
                started = true
            }

            override fun onOffline() = Unit

            override fun onFinished(version: String?, error: String?) {
                result = version to error
                done.countDown()
            }
        })
        assertTrue("20 秒内没有回调 onFinished", done.await(20, TimeUnit.SECONDS))
        assertFalse(started)
        assertNull(result!!.first)
        assertTrue("应带失败原因，实际 $result", !result!!.second.isNullOrBlank())
    }
}
