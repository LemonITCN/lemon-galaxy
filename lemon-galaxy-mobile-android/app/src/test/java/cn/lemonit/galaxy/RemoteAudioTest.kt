package cn.lemonit.galaxy

import cn.lemonit.galaxy.app.ConnectionManager
import cn.lemonit.galaxy.app.SecretPromptBroker
import cn.lemonit.galaxy.core.bridge.ClientListener
import cn.lemonit.galaxy.core.bridge.ClientState
import cn.lemonit.galaxy.core.bridge.CoreConfig
import cn.lemonit.galaxy.core.bridge.DeviceKind
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.mock.MockAudioPlayback
import cn.lemonit.galaxy.core.mock.MockCore
import cn.lemonit.galaxy.core.mock.MockData
import cn.lemonit.galaxy.core.platform.MemorySecureStore
import cn.lemonit.galaxy.data.VideoQuality
import cn.lemonit.galaxy.ui.remote.RemoteDesktopController
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executors

/** 远程桌面的声音开关：画面打开后自动开始、开关、前后台、离开时停掉，开到一半被关掉不漏停。 */
class RemoteAudioTest {
    // 控制器的状态只在一个线程上读写（正式运行时是主线程）
    private val executor = Executors.newSingleThreadExecutor()
    private val main = executor.asCoroutineDispatcher()
    private val scope = CoroutineScope(SupervisorJob() + main)
    private val errors = CopyOnWriteArrayList<String>()

    @After
    fun tearDown() {
        scope.cancel()
        executor.shutdownNow()
    }

    private fun controller(audioWanted: Boolean) = runBlocking {
        val core = MockCore { name -> File("src/mock/assets/$name").readBytes() }
        core.coreInit(CoreConfig("/tmp", "/tmp", "debug", Platform.ANDROID, DeviceKind.MOBILE, "0.1.0", "Android 16", "Pixel", "zh-CN"))
        val client = core.newGalaxyClient(MemorySecureStore(), object : ClientListener {
            override fun onState(state: ClientState) = Unit
            override fun onDevicesChanged() = Unit
            override fun onVaultChanged() = Unit
            override fun onSessionRevoked() = Unit
        })
        client.login("galaxy.example.com:7443#sha256-q3Vb1oX9mH2c7uY0kN4t", "alice", "pw", null)
        val conn = ConnectionManager(client, SecretPromptBroker())
        withContext(main) {
            RemoteDesktopController(
                MockData.DEV_OFFICE_PC, "OFFICE-PC", Platform.WINDOWS, conn, scope, VideoQuality.AUTO, false,
                audioWanted = audioWanted,
                onAudioError = { errors.add(it) },
            )
        }
    }

    private fun onMain(block: () -> Unit) = runBlocking { withContext(main) { block() } }

    private fun waitFor(what: String, timeoutMs: Long = 5_000, cond: () -> Boolean) {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            if (cond()) return
            Thread.sleep(20)
        }
        throw AssertionError("$timeoutMs ms 内没有$what")
    }

    @Test
    fun soundFollowsDisplayToggleAndLifecycle() {
        val before = MockAudioPlayback.playing.get()
        val c = controller(audioWanted = true)
        onMain { c.start() }
        waitFor("开始播放声音") { c.ui.value.audioOn }
        assertTrue(c.ui.value.audioSupported)
        assertEquals(before + 1, MockAudioPlayback.playing.get())

        // 用户关掉、再打开
        onMain { c.setAudio(false) }
        assertFalse(c.ui.value.audioOn)
        assertEquals(before, MockAudioPlayback.playing.get())
        onMain { c.setAudio(true) }
        waitFor("重新开始播放") { c.ui.value.audioOn }

        // 切后台停掉，回前台重新开始
        onMain { c.setForeground(false) }
        assertFalse(c.ui.value.audioOn)
        assertEquals(before, MockAudioPlayback.playing.get())
        onMain { c.setForeground(true) }
        waitFor("回前台后重新播放") { c.ui.value.audioOn }

        // 开到一半被关掉：迟到的那一路要停掉，不能漏
        onMain {
            c.setAudio(false)
            c.setAudio(true)
            c.setAudio(false)
        }
        Thread.sleep(600)
        assertFalse(c.ui.value.audioOn)
        assertFalse(c.ui.value.audioBusy)
        assertEquals(before, MockAudioPlayback.playing.get())

        // 离开远程桌面
        onMain { c.setAudio(true) }
        waitFor("再次播放") { c.ui.value.audioOn }
        onMain { c.stop() }
        assertFalse(c.ui.value.audioOn)
        assertEquals(before, MockAudioPlayback.playing.get())
        assertTrue(errors.toString(), errors.isEmpty())
    }

    @Test
    fun soundStaysOffWhenTurnedOffInSettings() {
        val before = MockAudioPlayback.playing.get()
        val c = controller(audioWanted = false)
        onMain { c.start() }
        waitFor("拿到对端能力") { c.ui.value.audioSupported && c.ui.value.displayId != null }
        Thread.sleep(400)
        assertFalse(c.ui.value.audioOn)
        assertEquals(before, MockAudioPlayback.playing.get())
        onMain { c.stop() }
    }
}
