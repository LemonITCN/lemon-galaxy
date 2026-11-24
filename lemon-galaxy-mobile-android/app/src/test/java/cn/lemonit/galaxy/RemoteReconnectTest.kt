package cn.lemonit.galaxy

import cn.lemonit.galaxy.app.ConnectionManager
import cn.lemonit.galaxy.app.SecretPromptBroker
import cn.lemonit.galaxy.core.bridge.ClientListener
import cn.lemonit.galaxy.core.bridge.ClientState
import cn.lemonit.galaxy.core.bridge.CoreConfig
import cn.lemonit.galaxy.core.bridge.DeviceKind
import cn.lemonit.galaxy.core.bridge.GalaxyClientInterface
import cn.lemonit.galaxy.core.bridge.GalaxyException
import cn.lemonit.galaxy.core.bridge.PeerSessionInterface
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.mock.MockCore
import cn.lemonit.galaxy.core.mock.MockData
import cn.lemonit.galaxy.core.mock.MockPeerSession
import cn.lemonit.galaxy.core.platform.MemorySecureStore
import cn.lemonit.galaxy.data.VideoQuality
import cn.lemonit.galaxy.ui.remote.RECONNECT_DELAYS_MS
import cn.lemonit.galaxy.ui.remote.RECONNECT_PICTURE_TIMEOUT_MS
import cn.lemonit.galaxy.ui.remote.RemoteDesktopController
import cn.lemonit.galaxy.ui.remote.RemoteStatus
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Test
import java.io.File
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicInteger

/**
 * 远程桌面断线自动重连：画面断了先在原会话上重开、会话断了重新连接（同一块屏）；
 * 后台不重连、回前台再连；自己停的不重连；按间隔重试都不成显示失败，重试从头再来。
 */
class RemoteReconnectTest {
    private val executor = Executors.newSingleThreadExecutor()
    private val main = executor.asCoroutineDispatcher()
    private val scope = CoroutineScope(SupervisorJob() + main)

    @After
    fun tearDown() {
        scope.cancel()
        executor.shutdownNow()
    }

    /** 可以让连接失败的客户端（模拟设备离线）。 */
    private class FlakyClient(private val inner: GalaxyClientInterface) : GalaxyClientInterface by inner {
        @Volatile var offline = false
        val connects = AtomicInteger()

        override suspend fun connect(deviceId: String): PeerSessionInterface {
            connects.incrementAndGet()
            if (offline) throw GalaxyException.Network("设备离线")
            return inner.connect(deviceId)
        }
    }

    private lateinit var client: FlakyClient
    private lateinit var conn: ConnectionManager

    private fun controller(delays: List<Long> = listOf(150, 150, 150), pictureTimeoutMs: Long = 5_000) = runBlocking {
        val core = MockCore { name -> File("src/mock/assets/$name").readBytes() }
        core.coreInit(CoreConfig("/tmp", "/tmp", "debug", Platform.ANDROID, DeviceKind.MOBILE, "0.1.0", "Android 16", "Pixel", "zh-CN"))
        val inner = core.newGalaxyClient(MemorySecureStore(), object : ClientListener {
            override fun onState(state: ClientState) = Unit
            override fun onDevicesChanged() = Unit
            override fun onVaultChanged() = Unit
            override fun onSessionRevoked() = Unit
        })
        inner.login("galaxy.example.com:7443#sha256-q3Vb1oX9mH2c7uY0kN4t", "alice", "pw", null)
        client = FlakyClient(inner)
        conn = ConnectionManager(client, SecretPromptBroker())
        withContext(main) {
            RemoteDesktopController(
                MockData.DEV_OFFICE_PC, "OFFICE-PC", Platform.WINDOWS, conn, scope, VideoQuality.AUTO, false,
                reconnectDelaysMs = delays,
                pictureTimeoutMs = pictureTimeoutMs,
            )
        }
    }

    private fun onMain(block: () -> Unit) = runBlocking { withContext(main) { block() } }

    private fun waitFor(what: String, timeoutMs: Long = 5_000, cond: () -> Boolean) {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            if (cond()) return
            Thread.sleep(10)
        }
        throw AssertionError("$timeoutMs ms 内没有$what")
    }

    private fun peer() = conn.existingPeer(MockData.DEV_OFFICE_PC)

    private fun streamingOn(c: RemoteDesktopController, display: UInt) {
        onMain { c.start() }
        waitFor("出画面") { c.ui.value.status == RemoteStatus.STREAMING }
        onMain { c.switchDisplay(display) }
        waitFor("换到第 $display 块屏") { c.ui.value.displayId == display && c.ui.value.status == RemoteStatus.STREAMING }
        // 旧画面迟到的关闭通知（自己停的）不触发重连
        Thread.sleep(400)
        assertEquals(RemoteStatus.STREAMING, c.ui.value.status)
    }

    @Test
    fun scheduleIsAboutTwoMinutes() {
        assertEquals(listOf(1_000L, 2_000L, 4_000L, 8_000L, 15_000L), RECONNECT_DELAYS_MS.take(5))
        assertEquals(10, RECONNECT_DELAYS_MS.size)
        assertEquals(105_000L, RECONNECT_DELAYS_MS.sum())
        assertEquals(20_000L, RECONNECT_PICTURE_TIMEOUT_MS)
    }

    @Test
    fun openedWithoutPictureTimesOutAndRetries() {
        // 共 3 次，打开后最多等 600 ms 出画面
        val c = controller(pictureTimeoutMs = 600)
        streamingOn(c, 2u)
        val mock = peer()!!.session as MockPeerSession
        val opened = mock.openedDisplays

        // 被控端卡住：重开的画面打开成功但一直不出画面，一直停在重连中
        mock.silentDisplays = true
        mock.endDisplayStreams("画面没有响应")
        waitFor("第 1 次重开") { mock.openedDisplays == opened + 1 }
        Thread.sleep(300)
        assertEquals(RemoteStatus.RECONNECTING, c.ui.value.status)
        // 被控端恢复：等满了算这次失败，接着第 2 次重试就连上
        mock.silentDisplays = false
        waitFor("第 2 次重开后出画面") { c.ui.value.status == RemoteStatus.STREAMING }
        assertEquals(opened + 2, mock.openedDisplays)
        assertEquals(2u, c.ui.value.displayId)

        // 又卡住：一帧都没显示过，次数接着用（只剩 1 次），等满了直接失败
        mock.silentDisplays = true
        mock.endDisplayStreams("画面没有响应")
        waitFor("次数用完后失败") { c.ui.value.status == RemoteStatus.FAILED }
        assertEquals("画面没有响应", c.ui.value.error)
        assertEquals(opened + 3, mock.openedDisplays)
        assertEquals(1, client.connects.get())
        onMain { c.stop() }
    }

    @Test
    fun streamEndedReopensSameDisplayOnSameSession() {
        val c = controller()
        streamingOn(c, 2u)
        val session = peer()
        (session!!.session as MockPeerSession).endDisplayStreams("画面没有响应")
        waitFor("进入重连") { c.ui.value.status == RemoteStatus.RECONNECTING }
        assertNull(c.ui.value.error)
        waitFor("重开画面") { c.ui.value.status == RemoteStatus.STREAMING }
        assertEquals(2u, c.ui.value.displayId)
        assertSame(session, peer())
        assertEquals(1, client.connects.get())
        onMain { c.stop() }
    }

    @Test
    fun sessionClosedReconnectsWithNewSession() {
        val c = controller()
        streamingOn(c, 2u)
        val session = peer()
        conn.disconnect(MockData.DEV_OFFICE_PC)
        waitFor("进入重连") { c.ui.value.status == RemoteStatus.RECONNECTING }
        waitFor("重新连上") { c.ui.value.status == RemoteStatus.STREAMING }
        assertEquals(2u, c.ui.value.displayId)
        assertNotSame(session, peer())
        assertEquals(2, client.connects.get())
        onMain { c.stop() }
    }

    @Test
    fun backgroundWaitsForForeground() {
        val c = controller()
        streamingOn(c, 1u)
        onMain { c.setForeground(false) }
        conn.disconnect(MockData.DEV_OFFICE_PC)
        waitFor("进入重连") { c.ui.value.status == RemoteStatus.RECONNECTING }
        Thread.sleep(800)
        // 后台不连
        assertEquals(RemoteStatus.RECONNECTING, c.ui.value.status)
        assertEquals(1, client.connects.get())
        onMain { c.setForeground(true) }
        waitFor("回前台后重新连上") { c.ui.value.status == RemoteStatus.STREAMING }
        assertEquals(1u, c.ui.value.displayId)
        assertEquals(2, client.connects.get())
        onMain { c.stop() }
    }

    @Test
    fun leavingNeverReconnects() {
        val c = controller()
        streamingOn(c, 1u)
        onMain { c.stop() }
        conn.disconnect(MockData.DEV_OFFICE_PC)
        Thread.sleep(800)
        assertEquals(1, client.connects.get())
        assertNull(peer())
    }

    /** 结束当前画面并等它重连回来（在原会话上重开，状态回到 STREAMING）。 */
    private fun endAndReopen(c: RemoteDesktopController) {
        (peer()!!.session as MockPeerSession).endDisplayStreams("画面没有响应")
        waitFor("进入重连") { c.ui.value.status == RemoteStatus.RECONNECTING }
        waitFor("重开画面") { c.ui.value.status == RemoteStatus.STREAMING }
    }

    @Test
    fun attemptsResetOnlyAfterFirstFrameShown() {
        // 共 3 次
        val c = controller()
        streamingOn(c, 2u)

        // 重开了画面但一帧没显示就又断：接着用剩下的次数
        endAndReopen(c)
        endAndReopen(c)
        // 第 2 次重开后显示出了第一帧：次数清零（回主线程处理完再继续）
        c.decoder.frameShown()
        onMain { }
        endAndReopen(c)
        endAndReopen(c)
        endAndReopen(c)
        // 3 次用完、一帧都没显示又断了：不再重试，直接失败并显示原因
        (peer()!!.session as MockPeerSession).endDisplayStreams("画面没有响应")
        waitFor("次数用完后失败") { c.ui.value.status == RemoteStatus.FAILED }
        assertEquals("画面没有响应", c.ui.value.error)
        assertEquals(2u, c.ui.value.displayId)
        assertEquals(1, client.connects.get())

        // 点重试从头来
        onMain { c.retry() }
        waitFor("重试后重开") { c.ui.value.status == RemoteStatus.STREAMING }
        onMain { c.stop() }
    }

    @Test
    fun givesUpAfterScheduleAndRetryStartsOver() {
        val c = controller()
        streamingOn(c, 2u)
        client.offline = true
        conn.disconnect(MockData.DEV_OFFICE_PC)
        waitFor("进入重连") { c.ui.value.status == RemoteStatus.RECONNECTING }
        waitFor("重试都不成后失败") { c.ui.value.status == RemoteStatus.FAILED }
        assertEquals("设备离线", c.ui.value.error)
        // 第一次连上 + 3 次重连
        assertEquals(4, client.connects.get())
        Thread.sleep(500)
        assertEquals(4, client.connects.get())

        // 点重试：从头按间隔再来，连上回到原来那块屏
        client.offline = false
        onMain { c.retry() }
        assertEquals(RemoteStatus.RECONNECTING, c.ui.value.status)
        waitFor("重试后连上") { c.ui.value.status == RemoteStatus.STREAMING }
        assertEquals(2u, c.ui.value.displayId)
        assertNull(c.ui.value.error)
        onMain { c.stop() }
    }
}
