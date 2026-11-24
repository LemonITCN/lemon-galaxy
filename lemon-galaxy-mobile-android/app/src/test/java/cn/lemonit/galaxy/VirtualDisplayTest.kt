package cn.lemonit.galaxy

import cn.lemonit.galaxy.app.ConnectionManager
import cn.lemonit.galaxy.app.SecretPromptBroker
import cn.lemonit.galaxy.core.bridge.ClientListener
import cn.lemonit.galaxy.core.bridge.ClientState
import cn.lemonit.galaxy.core.bridge.CoreConfig
import cn.lemonit.galaxy.core.bridge.DeviceKind
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.mock.MockCore
import cn.lemonit.galaxy.core.mock.MockData
import cn.lemonit.galaxy.core.mock.MockPeerSession
import cn.lemonit.galaxy.core.platform.MemorySecureStore
import cn.lemonit.galaxy.data.VideoQuality
import cn.lemonit.galaxy.ui.remote.RemoteDesktopController
import cn.lemonit.galaxy.ui.remote.RemoteStatus
import cn.lemonit.galaxy.ui.remote.VdResolution
import cn.lemonit.galaxy.ui.remote.VdSize
import cn.lemonit.galaxy.ui.remote.VirtualDisplayForm
import cn.lemonit.galaxy.ui.remote.VirtualDisplays
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executors

/** 加虚拟屏：表单输入校验（纯逻辑），以及控制器对模拟内核的加屏、切屏、删屏、会话结束清空。 */
class VirtualDisplayTest {
    // ---------------- 表单 ----------------

    @Test
    fun defaultsTo1080p60() {
        val f = VirtualDisplayForm()
        assertEquals(VdSize(1920, 1080), f.size(null))
        assertEquals(60, f.refreshHz())
        assertFalse(f.sizeInvalid)
        assertFalse(f.refreshInvalid)
        assertTrue(f.canSubmit(local = null, added = 0, max = 3, busy = false))
    }

    @Test
    fun presetsAreAllLegal() {
        VirtualDisplays.PRESETS.forEach { assertTrue(it.label, VirtualDisplays.sizeOk(it.width, it.height)) }
        VirtualDisplays.REFRESH_RATES.forEach { assertTrue("$it", VirtualDisplays.refreshOk(it)) }
        assertEquals("2560×1440", VirtualDisplayForm().selectPreset(VdSize(2560, 1440)).size(null)?.label)
    }

    @Test
    fun localScreenIsLandscapeAndInRange() {
        // 手机竖着拿也取横屏方向：长边为宽
        assertEquals(VdSize(2400, 1080), VirtualDisplays.localScreen(1080, 2400))
        assertEquals(VdSize(2400, 1080), VirtualDisplays.localScreen(2400, 1080))
        // 边界
        assertEquals(VdSize(640, 480), VirtualDisplays.localScreen(480, 640))
        assertEquals(VdSize(7680, 4320), VirtualDisplays.localScreen(4320, 7680))
        // 超出范围就不显示这一项
        assertNull(VirtualDisplays.localScreen(720, 400))
        assertNull(VirtualDisplays.localScreen(600, 479))
        assertNull(VirtualDisplays.localScreen(7681, 2160))
        assertNull(VirtualDisplays.localScreen(4400, 4400))
        assertEquals(VdSize(4400, 3000), VirtualDisplays.localScreen(3000, 4400))

        val local = VdSize(2400, 1080)
        val f = VirtualDisplayForm().selectLocal()
        assertEquals(VdResolution.Local, f.resolution)
        assertEquals(local, f.size(local))
        // 本机屏幕不可用时不能提交
        assertNull(f.size(null))
        assertFalse(f.canSubmit(local = null, added = 0, max = 3, busy = false))
        assertTrue(f.canSubmit(local = local, added = 0, max = 3, busy = false))
    }

    @Test
    fun customSizeRange() {
        val f = VirtualDisplayForm().selectCustom(null)
        fun size(w: String, h: String) = f.withWidth(w).withHeight(h)
        assertEquals(VdSize(640, 480), size("640", "480").size(null))
        assertEquals(VdSize(7680, 4320), size("7680", "4320").size(null))
        assertEquals(VdSize(3440, 1440), size("3440", "1440").size(null))
        listOf("639" to "480", "640" to "479", "7681" to "1080", "1920" to "4321", "" to "1080", "1920" to "", "0" to "0").forEach { (w, h) ->
            val g = size(w, h)
            assertNull("$w×$h", g.size(null))
            assertTrue("$w×$h 要提示范围", g.sizeInvalid)
            assertFalse("$w×$h 按钮要置灰", g.canSubmit(local = null, added = 0, max = 3, busy = false))
        }
        // 选的不是自定义时不提示
        assertFalse(size("1", "1").selectPreset(VdSize(1280, 720)).sizeInvalid)
    }

    @Test
    fun customRefreshRange() {
        val f = VirtualDisplayForm().selectCustomRefresh()
        assertNull(f.refresh)
        // 先填上原来的档位
        assertEquals("60", f.customRefresh)
        assertEquals(24, f.withCustomRefresh("24").refreshHz())
        assertEquals(240, f.withCustomRefresh("240").refreshHz())
        assertEquals(75, f.withCustomRefresh("75").refreshHz())
        listOf("23", "241", "", "0").forEach { v ->
            val g = f.withCustomRefresh(v)
            assertNull(v, g.refreshHz())
            assertTrue(v, g.refreshInvalid)
            assertFalse(v, g.canSubmit(local = null, added = 0, max = 3, busy = false))
        }
        // 回到档位
        val back = f.withCustomRefresh("999").selectRefresh(144)
        assertEquals(144, back.refreshHz())
        assertFalse(back.refreshInvalid)
    }

    @Test
    fun inputsKeepDigitsOnly() {
        val f = VirtualDisplayForm().selectCustom(null)
        assertEquals("1920", f.withWidth("19a2 0").width)
        assertEquals("1234", f.withWidth("123456").width)
        assertEquals("1080", f.withHeight("-1,080").height)
        assertEquals("144", f.selectCustomRefresh().withCustomRefresh("1445").customRefresh)
    }

    @Test
    fun customPrefillsCurrentChoice() {
        val f = VirtualDisplayForm().selectPreset(VdSize(2560, 1440)).selectCustom(null)
        assertEquals("2560", f.width)
        assertEquals("1440", f.height)
        assertEquals(VdSize(2560, 1440), f.size(null))
        // 改过的值来回切换档位后保留
        val edited = f.withWidth("3440").selectPreset(VdSize(1280, 720)).selectCustom(null)
        assertEquals(VdSize(3440, 1440), edited.size(null))
        // 本机屏幕切到自定义：填本机分辨率
        val local = VdSize(2400, 1080)
        assertEquals(local, VirtualDisplayForm().selectLocal().selectCustom(local).size(null))
    }

    @Test
    fun submitRules() {
        val f = VirtualDisplayForm()
        assertTrue(f.canSubmit(null, added = 2, max = 3, busy = false))
        // 加满、进行中、不支持
        assertFalse(f.canSubmit(null, added = 3, max = 3, busy = false))
        assertFalse(f.canSubmit(null, added = 0, max = 3, busy = true))
        assertFalse(f.canSubmit(null, added = 0, max = 0, busy = false))
    }

    // ---------------- 控制器 + 模拟内核 ----------------

    // 控制器的状态只在一个线程上读写（正式运行时是主线程）
    private val executor = Executors.newSingleThreadExecutor()
    private val main = executor.asCoroutineDispatcher()
    private val scope = CoroutineScope(SupervisorJob() + main)
    private val addErrors = CopyOnWriteArrayList<String>()
    private val removeErrors = CopyOnWriteArrayList<String>()

    @After
    fun tearDown() {
        scope.cancel()
        executor.shutdownNow()
    }

    private fun connections(): ConnectionManager = runBlocking {
        val core = MockCore { name -> File("src/mock/assets/$name").readBytes() }
        core.coreInit(CoreConfig("/tmp", "/tmp", "debug", Platform.ANDROID, DeviceKind.MOBILE, "0.1.0", "Android 16", "Pixel", "zh-CN"))
        val client = core.newGalaxyClient(MemorySecureStore(), object : ClientListener {
            override fun onState(state: ClientState) = Unit
            override fun onDevicesChanged() = Unit
            override fun onVaultChanged() = Unit
            override fun onSessionRevoked() = Unit
        })
        client.login("galaxy.example.com:7443#sha256-q3Vb1oX9mH2c7uY0kN4t", "alice", "pw", null)
        ConnectionManager(client, SecretPromptBroker())
    }

    private fun controller(conn: ConnectionManager, deviceId: String = MockData.DEV_OFFICE_PC) = runBlocking {
        withContext(main) {
            RemoteDesktopController(
                deviceId, "OFFICE-PC", Platform.WINDOWS, conn, scope, VideoQuality.AUTO, false,
                onVirtualAddFailed = { addErrors.add(it) },
                onVirtualRemoveFailed = { removeErrors.add(it) },
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
    fun addSwitchRemoveAndClearWithSession() {
        val conn = connections()
        val c = controller(conn)
        onMain { c.start() }
        waitFor("打开主屏并取到虚拟屏上限") { c.ui.value.displayId == 1u && c.ui.value.virtualMax == 3 }
        assertTrue(c.ui.value.myVirtualIds.isEmpty())

        // 不合法的值不发请求
        onMain { c.addVirtualDisplay(600, 480, 60) }
        onMain { c.addVirtualDisplay(1920, 1080, 300) }
        assertFalse(c.ui.value.virtualAdding)

        // 加一块：进行中 → 记入自己加的、出现在列表里、直接切过去
        onMain { c.addVirtualDisplay(2560, 1440, 60) }
        assertTrue(c.ui.value.virtualAdding)
        waitFor("加好并切到新屏") { c.ui.value.virtualAddedSeq == 1 && c.ui.value.displayId == c.ui.value.virtualAdded?.id }
        val first = c.ui.value.virtualAdded!!
        assertTrue(first.isVirtual)
        assertEquals(2560u, first.width)
        assertEquals(1440u, first.height)
        assertEquals(setOf(first.id), c.ui.value.myVirtualIds)
        assertTrue(c.ui.value.displays.any { it.id == first.id })
        assertFalse(c.ui.value.virtualAdding)
        // 旧画面（主屏）停下后迟到的关闭通知不能把新画面标成失败
        waitFor("新屏出画面") { c.ui.value.status == RemoteStatus.STREAMING }
        Thread.sleep(400)
        assertEquals(RemoteStatus.STREAMING, c.ui.value.status)
        assertEquals(null, c.ui.value.error)

        // 加到上限后不再发请求
        onMain { c.addVirtualDisplay(1920, 1080, 144) }
        waitFor("第二块") { c.ui.value.virtualAddedSeq == 2 && !c.ui.value.virtualAdding }
        onMain { c.addVirtualDisplay(1280, 720, 30) }
        waitFor("第三块") { c.ui.value.virtualAddedSeq == 3 && !c.ui.value.virtualAdding }
        assertTrue(c.ui.value.virtualFull)
        onMain { c.addVirtualDisplay(1280, 720, 30) }
        assertFalse(c.ui.value.virtualAdding)

        // 别人加的虚拟屏（Virtual 1）不在自己加的里：删不了
        val others = c.ui.value.displays.first { it.isVirtual && it.id !in c.ui.value.myVirtualIds }
        onMain { c.removeVirtualDisplay(others.id) }
        assertTrue(c.ui.value.virtualRemoving.isEmpty())

        // 删掉正在看的那块：切回主屏，列表与集合里都没了
        val viewing = c.ui.value.displayId!!
        assertTrue(viewing in c.ui.value.myVirtualIds)
        onMain { c.removeVirtualDisplay(viewing) }
        assertTrue(viewing in c.ui.value.virtualRemoving)
        waitFor("删掉") { c.ui.value.virtualRemovedSeq == 1 }
        assertEquals(1u, c.ui.value.displayId)
        assertFalse(c.ui.value.displays.any { it.id == viewing })
        assertEquals(2, c.ui.value.myVirtualIds.size)
        assertTrue(c.ui.value.virtualRemoving.isEmpty())
        assertFalse(c.ui.value.virtualFull)
        // 先切回主屏再删：被删那一路迟到的关闭通知不影响主屏画面
        Thread.sleep(400)
        assertEquals(RemoteStatus.STREAMING, c.ui.value.status)

        // 删一块没在看的：画面不动
        val other = c.ui.value.myVirtualIds.first()
        onMain { c.removeVirtualDisplay(other) }
        waitFor("再删一块") { c.ui.value.virtualRemovedSeq == 2 }
        assertEquals(1u, c.ui.value.displayId)
        assertEquals(1, c.ui.value.myVirtualIds.size)

        // 离开远程桌面再回来（如去了文件管理）：同一会话，自己加的还在，仍能删
        val kept = c.ui.value.myVirtualIds
        onMain { c.stop() }
        val again = controller(conn)
        onMain { again.start() }
        // 出过画面之后断开才自动重连（第一次就没打开的直接显示失败）
        waitFor("重新打开") { again.ui.value.displayId == 1u && again.ui.value.status == RemoteStatus.STREAMING }
        assertEquals(kept, again.ui.value.myVirtualIds)
        assertEquals(3, again.ui.value.virtualMax)

        // 会话断开：集合清空、入口收起
        conn.disconnect(MockData.DEV_OFFICE_PC)
        waitFor("断开后清空") { again.ui.value.myVirtualIds.isEmpty() && again.ui.value.virtualMax == 0 }
        // 被控端会自动删掉本会话加的屏：列表里也去掉
        assertFalse(again.ui.value.displays.any { it.id in kept })

        // 自动重连上是新会话：从空开始，上限重新取，回到原来那块屏
        waitFor("自动重连并重新取上限") { again.ui.value.virtualMax == 3 && again.ui.value.status == RemoteStatus.STREAMING }
        assertEquals(1u, again.ui.value.displayId)
        assertTrue(again.ui.value.myVirtualIds.isEmpty())
        assertFalse(again.ui.value.displays.any { it.id in kept })
        onMain { again.switchDisplay(2u) }
        waitFor("换到第二块屏") { again.ui.value.displayId == 2u && again.ui.value.status == RemoteStatus.STREAMING }
        onMain { again.stop() }
        assertTrue(addErrors.toString(), addErrors.isEmpty())
        assertTrue(removeErrors.toString(), removeErrors.isEmpty())
    }

    private fun mock(conn: ConnectionManager) = conn.existingPeer(MockData.DEV_OFFICE_PC)!!.session as MockPeerSession

    @Test
    fun addKeepsNewDisplayWhenHostListLags() {
        val conn = connections()
        val c = controller(conn)
        onMain { c.start() }
        waitFor("打开主屏") { c.ui.value.displayId == 1u && c.ui.value.virtualMax == 3 }
        // 被控端列表里暂时还没有刚加的屏：先把 add 返回的那块补进列表，不要滤掉
        mock(conn).listLagsBehindAdd = true
        onMain { c.addVirtualDisplay(1920, 1200, 60) }
        waitFor("加好") { c.ui.value.virtualAddedSeq == 1 && !c.ui.value.virtualAdding }
        val added = c.ui.value.virtualAdded!!
        assertTrue(added.id in c.ui.value.myVirtualIds)
        assertTrue(c.ui.value.displays.any { it.id == added.id })
        waitFor("切到新屏") { c.ui.value.displayId == added.id }
        onMain { c.stop() }
        assertTrue(addErrors.toString(), addErrors.isEmpty())
    }

    @Test
    fun refreshDropsMineGoneOnHost() {
        val conn = connections()
        val c = controller(conn)
        onMain { c.start() }
        waitFor("打开主屏") { c.ui.value.displayId == 1u && c.ui.value.virtualMax == 3 }
        onMain { c.addVirtualDisplay(1920, 1080, 60) }
        waitFor("第一块") { c.ui.value.virtualAddedSeq == 1 && !c.ui.value.virtualAdding }
        onMain { c.addVirtualDisplay(2560, 1440, 60) }
        waitFor("第二块") { c.ui.value.virtualAddedSeq == 2 && !c.ui.value.virtualAdding }
        assertEquals(2, c.ui.value.myVirtualIds.size)
        onMain { c.stop() }

        // 被控端那边虚拟屏没了（如 Windows 换控制台会话）：重新列屏后从「我加的」去掉，计数腾出来
        mock(conn).dropVirtualDisplays()
        val again = controller(conn)
        onMain { again.start() }
        waitFor("重新列屏") { again.ui.value.displayId == 1u && again.ui.value.displays.none { it.isVirtual && it.id > 100u } }
        assertTrue(again.ui.value.myVirtualIds.isEmpty())
        assertTrue(conn.existingPeer(MockData.DEV_OFFICE_PC)!!.myVirtualDisplays.value.isEmpty())
        assertFalse(again.ui.value.virtualFull)
        onMain { again.stop() }
    }

    @Test
    fun noEntryWithoutVirtualDisplaySupport() {
        // Linux 被控端（无图形界面）没有虚拟屏能力
        val conn = connections()
        val c = controller(conn, MockData.DEV_WEB_01)
        onMain { c.start() }
        waitFor("打开失败（没有图形界面）") { c.ui.value.error != null }
        assertEquals(0, c.ui.value.virtualMax)
        onMain { c.addVirtualDisplay(1920, 1080, 60) }
        assertFalse(c.ui.value.virtualAdding)
        onMain { c.stop() }
    }
}
