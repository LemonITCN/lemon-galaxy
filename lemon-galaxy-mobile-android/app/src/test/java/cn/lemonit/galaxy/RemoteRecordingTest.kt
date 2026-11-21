package cn.lemonit.galaxy

import cn.lemonit.galaxy.app.ConnectionManager
import cn.lemonit.galaxy.app.SecretPromptBroker
import cn.lemonit.galaxy.core.bridge.ClientListener
import cn.lemonit.galaxy.core.bridge.ClientState
import cn.lemonit.galaxy.core.bridge.CoreConfig
import cn.lemonit.galaxy.core.bridge.DeviceKind
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.RecordingInfo
import cn.lemonit.galaxy.core.mock.MockCore
import cn.lemonit.galaxy.core.mock.MockData
import cn.lemonit.galaxy.core.platform.MemorySecureStore
import cn.lemonit.galaxy.data.VideoQuality
import cn.lemonit.galaxy.ui.remote.RemoteDesktopController
import cn.lemonit.galaxy.ui.remote.RemoteStatus
import cn.lemonit.galaxy.ui.remote.formatElapsed
import cn.lemonit.galaxy.ui.remote.recordingFileName
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
import java.nio.file.Files
import java.util.Calendar
import java.util.TimeZone
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executors

/** 远程桌面录像：开始、手动停止弹保存提示；换屏幕、切后台、离开时自动停并交出文件；开始到一半被停不留半截文件。 */
class RemoteRecordingTest {
    private val executor = Executors.newSingleThreadExecutor()
    private val main = executor.asCoroutineDispatcher()
    private val scope = CoroutineScope(SupervisorJob() + main)
    private val saved = CopyOnWriteArrayList<Pair<RecordingInfo, Boolean>>()
    private val errors = CopyOnWriteArrayList<String>()
    private val dir: File = Files.createTempDirectory("galaxy-rec").toFile()

    @After
    fun tearDown() {
        scope.cancel()
        executor.shutdownNow()
        dir.deleteRecursively()
    }

    private fun controller() = runBlocking {
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
                MockData.DEV_OFFICE_PC, "Office PC", Platform.WINDOWS, conn, scope, VideoQuality.AUTO, false,
                audioWanted = false,
                recordingDir = { dir },
                onRecordingSaved = { info, auto -> saved.add(info to auto) },
                onRecordingError = { errors.add(it) },
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

    private fun startStreaming(c: RemoteDesktopController) {
        onMain { c.start() }
        // 单元测试里没有解码器回报尺寸：等画面打开（displayId 有值且通道已建）
        waitFor("打开画面") { c.ui.value.displayId != null && c.ui.value.status != RemoteStatus.FAILED }
        Thread.sleep(500)
    }

    private fun record(c: RemoteDesktopController) {
        onMain { c.startRecording() }
        waitFor("开始录像") { c.ui.value.recording }
        assertFalse(c.ui.value.recordingBusy)
        Thread.sleep(700)
    }

    @Test
    fun manualStopOffersResultAndAutoStopsOnLifecycle() {
        val c = controller()
        startStreaming(c)

        // 手动开始、停止：交出文件，不是自动停的
        record(c)
        onMain { c.toggleRecording() }
        assertFalse(c.ui.value.recording)
        assertEquals(1, saved.size)
        val (info, auto) = saved[0]
        assertFalse(auto)
        val f = File(info.paths.single())
        assertTrue(f.name, f.name.startsWith("LemonGalaxy-Office PC-") && f.name.endsWith(".mp4"))
        assertEquals(dir, f.parentFile)
        assertTrue(f.length() > 0)

        // 切后台：自动停
        record(c)
        onMain { c.setForeground(false) }
        assertFalse(c.ui.value.recording)
        assertEquals(2, saved.size)
        assertTrue(saved[1].second)
        onMain { c.setForeground(true) }

        // 换屏幕：先停录像再重开画面（弹保存提示）
        record(c)
        onMain { c.switchDisplay(2u) }
        waitFor("换屏幕后停录像") { saved.size == 3 }
        assertFalse(saved[2].second)
        assertFalse(c.ui.value.recording)
        waitFor("新画面打开") { c.ui.value.displayId == 2u }
        Thread.sleep(500)

        // 离开远程桌面：自动停
        record(c)
        onMain { c.stop() }
        assertEquals(4, saved.size)
        assertTrue(saved[3].second)
        assertTrue(errors.toString(), errors.isEmpty())
    }

    @Test
    fun stoppingWhileStartingLeavesNoFile() {
        val c = controller()
        startStreaming(c)
        onMain {
            c.startRecording()
            // 开始到一半就离开
            c.stopRecording(auto = true)
        }
        Thread.sleep(600)
        assertFalse(c.ui.value.recording)
        assertFalse(c.ui.value.recordingBusy)
        assertTrue(saved.isEmpty())
        assertTrue(dir.listFiles().orEmpty().isEmpty())
        onMain { c.stop() }
        assertTrue(saved.isEmpty())
    }

    @Test
    fun fileNameAndElapsedFormat() {
        val cal = Calendar.getInstance(TimeZone.getDefault()).apply { set(2026, Calendar.NOVEMBER, 21, 21, 30, 5) }
        assertEquals("LemonGalaxy-OFFICE-PC-20261121-213005.mp4", recordingFileName("OFFICE-PC", cal.timeInMillis))
        assertEquals("LemonGalaxy-a-b-c-d-20261121-213005.mp4", recordingFileName("a/b\\c:d", cal.timeInMillis))
        assertEquals("LemonGalaxy-device-20261121-213005.mp4", recordingFileName("  ", cal.timeInMillis))
        assertEquals("00:00", formatElapsed(0))
        assertEquals("01:23", formatElapsed(83_400))
        assertEquals("59:59", formatElapsed(3_599_000))
        assertEquals("1:00:00", formatElapsed(3_600_000))
        assertEquals("12:05:09", formatElapsed((12 * 3600 + 5 * 60 + 9) * 1000L))
    }
}
