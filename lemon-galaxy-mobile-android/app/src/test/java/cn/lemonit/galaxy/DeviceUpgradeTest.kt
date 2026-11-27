package cn.lemonit.galaxy

import cn.lemonit.galaxy.app.DeviceUpgrade
import cn.lemonit.galaxy.app.DeviceUpgradeController
import cn.lemonit.galaxy.core.bridge.ClientListener
import cn.lemonit.galaxy.core.bridge.ClientState
import cn.lemonit.galaxy.core.bridge.CoreConfig
import cn.lemonit.galaxy.core.bridge.DeviceKind
import cn.lemonit.galaxy.core.bridge.GalaxyClientInterface
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.mock.MockCore
import cn.lemonit.galaxy.core.mock.MockData
import cn.lemonit.galaxy.core.platform.MemorySecureStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger

/** 远程升级被控端的界面状态：按内核回调推进，结束后刷新列表，进行中不重复发起。 */
class DeviceUpgradeTest {
    private lateinit var client: GalaxyClientInterface
    private lateinit var upgrades: DeviceUpgradeController
    private val refreshed = AtomicInteger()

    @Before
    fun setUp() {
        val core = MockCore { name -> File("src/mock/assets/$name").readBytes() }
        core.coreInit(CoreConfig("/tmp", "/tmp", "debug", Platform.ANDROID, DeviceKind.MOBILE, "0.1.0", "Android 16", "Pixel", "zh-CN"))
        client = core.newGalaxyClient(MemorySecureStore(), object : ClientListener {
            override fun onState(state: ClientState) = Unit
            override fun onDevicesChanged() = Unit
            override fun onVaultChanged() = Unit
            override fun onSessionRevoked() = Unit
        })
        runBlocking { client.login("galaxy.example.com:7443#sha256-q3Vb1oX9mH2c7uY0kN4t", "alice", "pw", null) }
        // 内核回调直接在回调线程上改状态（界面里是主线程）
        upgrades = DeviceUpgradeController(client, CoroutineScope(Dispatchers.Unconfined)) { refreshed.incrementAndGet() }
    }

    private fun device(id: String) = client.devices().first { it.id == id }

    @Test
    fun progressFollowsCoreAndRefreshesWhenDone() {
        val seen = CopyOnWriteArrayList<Class<*>>()
        val web = device(MockData.DEV_WEB_01)
        upgrades.upgrade(web)
        assertEquals(DeviceUpgrade.Requesting(MockData.UPGRADE_TO), upgrades.state(web.id))
        // 进行中再点一次：忽略（不会再发一次请求）
        upgrades.upgrade(web)
        waitFor(15_000) {
            upgrades.state(web.id)?.let { s -> if (seen.lastOrNull() != s.javaClass) seen.add(s.javaClass) }
            upgrades.state(web.id) is DeviceUpgrade.Done
        }
        assertEquals(
            listOf(DeviceUpgrade.Requesting::class.java, DeviceUpgrade.Installing::class.java, DeviceUpgrade.Restarting::class.java, DeviceUpgrade.Done::class.java),
            seen.toList(),
        )
        assertEquals(DeviceUpgrade.Done(MockData.UPGRADE_TO, alreadyLatest = false), upgrades.state(web.id))
        assertEquals(1, refreshed.get())
        assertEquals(MockData.UPGRADE_TO, device(web.id).appVersion)
    }

    @Test
    fun offlineFailsAndAlreadyLatestIsReported() {
        val render = device(MockData.DEV_RENDER)
        upgrades.upgrade(render)
        waitFor { upgrades.state(render.id) is DeviceUpgrade.Failed }
        assertTrue((upgrades.state(render.id) as DeviceUpgrade.Failed).reason.isNotBlank())

        val mac = device(MockData.DEV_MACBOOK)
        upgrades.upgrade(mac)
        waitFor { upgrades.state(mac.id) is DeviceUpgrade.Done }
        assertEquals(DeviceUpgrade.Done("0.1.0", alreadyLatest = true), upgrades.state(mac.id))
        assertEquals(2, refreshed.get())
    }

    @Test
    fun clearDropsLateCallbacks() {
        val web = device(MockData.DEV_WEB_01)
        upgrades.upgrade(web)
        upgrades.clear()
        // 内核这一轮照样走完，但界面不再显示
        Thread.sleep(1_500)
        assertEquals(null, upgrades.state(web.id))
        assertEquals(0, refreshed.get())
    }

    private fun waitFor(timeoutMs: Long = 5_000, cond: () -> Boolean) {
        val end = System.currentTimeMillis() + timeoutMs
        while (!cond()) {
            if (System.currentTimeMillis() > end) fail("timed out")
            Thread.sleep(20)
        }
    }
}
