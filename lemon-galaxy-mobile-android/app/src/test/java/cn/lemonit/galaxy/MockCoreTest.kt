package cn.lemonit.galaxy

import cn.lemonit.galaxy.core.bridge.AgentEvent
import cn.lemonit.galaxy.core.bridge.AgentListener
import cn.lemonit.galaxy.core.bridge.AgentTarget
import cn.lemonit.galaxy.core.bridge.AgentTargetKind
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.ApprovalDecision
import cn.lemonit.galaxy.core.bridge.ClientListener
import cn.lemonit.galaxy.core.bridge.ClientState
import cn.lemonit.galaxy.core.bridge.ConversationConfig
import cn.lemonit.galaxy.core.bridge.CoreConfig
import cn.lemonit.galaxy.core.bridge.DeviceKind
import cn.lemonit.galaxy.core.bridge.GalaxyClientInterface
import cn.lemonit.galaxy.core.bridge.GalaxyException
import cn.lemonit.galaxy.core.bridge.LoginResult
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.SecretPrompt
import cn.lemonit.galaxy.core.bridge.TerminalOptions
import cn.lemonit.galaxy.core.bridge.TerminalSink
import cn.lemonit.galaxy.core.bridge.TransferListener
import cn.lemonit.galaxy.core.bridge.UpgradeListener
import cn.lemonit.galaxy.core.bridge.VaultData
import cn.lemonit.galaxy.core.bridge.VaultEntry
import cn.lemonit.galaxy.core.mock.MockAudioPlayback
import cn.lemonit.galaxy.core.mock.MockCore
import cn.lemonit.galaxy.core.mock.MockData
import cn.lemonit.galaxy.core.platform.MemorySecureStore
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** 模拟内核的行为测试：界面依赖的流程在模拟层都走得通。 */
class MockCoreTest {
    private lateinit var client: GalaxyClientInterface
    private val states = CopyOnWriteArrayList<ClientState>()
    private val store = MemorySecureStore()
    private val server = "galaxy.example.com:7443#sha256-q3Vb1oX9mH2c7uY0kN4t"

    private fun newClient(): GalaxyClientInterface {
        val core = MockCore { name -> File("src/mock/assets/$name").readBytes() }
        core.coreInit(CoreConfig("/tmp", "/tmp", "debug", Platform.ANDROID, DeviceKind.MOBILE, "0.1.0", "Android 16", "Pixel", "zh-CN"))
        return core.newGalaxyClient(store, object : ClientListener {
            override fun onState(state: ClientState) {
                states.add(state)
            }

            override fun onDevicesChanged() = Unit
            override fun onVaultChanged() = Unit
            override fun onSessionRevoked() = Unit
        })
    }

    @Before
    fun setUp() {
        client = newClient()
    }

    @Test
    fun loginVariantsAndRestore() = runBlocking<Unit> {
        assertEquals(LoginResult.NEED_TOTP, client.login(server, "totp-alice", "pw", null))
        try {
            client.login(server, "totp-alice", "pw", "000000")
            fail("wrong totp accepted")
        } catch (e: GalaxyException.Auth) {
            assertEquals("totp_invalid", e.code)
        }
        try {
            client.login(server, "alice", "wrong", null)
            fail("wrong password accepted")
        } catch (e: GalaxyException.Auth) {
            assertEquals("invalid_credentials", e.code)
        }
        assertEquals(LoginResult.MUST_CHANGE_PASSWORD, client.login(server, "new-bob", "pw", null))
        client.completeFirstLogin("long-enough-password")
        assertEquals(ClientState.ONLINE, client.state())
        // 新进程用本地会话恢复
        val again = newClient()
        assertTrue(again.restore())
        assertEquals("new-bob", again.currentUser()?.username)
        again.logout()
        assertFalse(newClient().restore())
    }

    @Test
    fun vaultCrud() = runBlocking<Unit> {
        client.login(server, "alice", "pw", null)
        val before = client.vault().size
        val saved = client.vaultSave(VaultEntry("", null, 0, VaultData.Snippet("ls", "ls -la", emptyList()), 0))
        assertTrue(saved.id.isNotEmpty())
        assertEquals(before + 1, client.vault().size)
        // 删文件夹：里面的条目移到上一级
        client.vaultDelete(MockData.F_DB)
        assertEquals(MockData.F_PROD, client.vault().first { it.id == MockData.SSH_PG }.folderId)
        client.vaultDelete(saved.id)
        // 新建的片段与删掉的 Databases 文件夹都不在了
        assertEquals(before - 1, client.vault().size)
    }

    @Test
    fun terminalEchoDetachAndAttach() = runBlocking<Unit> {
        client.login(server, "alice", "pw", null)
        val peer = client.connect(MockData.DEV_WEB_01)
        val out = StringBuffer()
        val sink = object : TerminalSink {
            override fun onOutput(data: ByteArray) {
                out.append(data.decodeToString())
            }

            override fun onExit(code: Int?) = Unit
        }
        val ch = peer.openTerminal(TerminalOptions(null, null, 80u, 24u, emptyList()), sink)
        ch.write("whoami\r".toByteArray())
        waitFor { out.contains("demo\r\n") }
        ch.detach()
        val sessions = peer.listTerminals()
        assertTrue(sessions.any { it.id == ch.sessionId() && !it.attached })
        // 接续：回放之前的输出
        val replay = StringBuffer()
        peer.openTerminal(TerminalOptions(ch.sessionId(), null, 80u, 24u, emptyList()), object : TerminalSink {
            override fun onOutput(data: ByteArray) {
                replay.append(data.decodeToString())
            }

            override fun onExit(code: Int?) = Unit
        })
        assertTrue(replay.contains("whoami"))
        peer.disconnect()
    }

    @Test
    fun filesAndTransfer() = runBlocking<Unit> {
        client.login(server, "alice", "pw", null)
        val files = client.connect(MockData.DEV_BUILD_MAC).files()
        val home = files.home()
        files.mkdir("$home/new-dir")
        assertTrue(files.list(home).any { it.name == "new-dir" })
        files.writeText("$home/new-dir/a.txt", "hello")
        assertEquals("hello", files.readText("$home/new-dir/a.txt", 1024u))
        files.rename("$home/new-dir/a.txt", "$home/new-dir/b.txt")
        files.chmod("$home/new-dir/b.txt", 0x1C0u)
        assertEquals(0x1C0u, files.stat("$home/new-dir/b.txt").mode)
        try {
            files.remove("$home/new-dir", false)
            fail("non-empty dir removed")
        } catch (_: GalaxyException) {
        }
        files.remove("$home/new-dir", true)
        assertFalse(files.list(home).any { it.name == "new-dir" })

        val local = File.createTempFile("galaxy", ".txt")
        val done = CountDownLatch(1)
        files.download("$home/notes.txt", local.path, object : TransferListener {
            override fun onProgress(done: ULong, total: ULong) = Unit
            override fun onFinished(error: String?) {
                errorHolder = error
                done.countDown()
            }
        })
        assertTrue(done.await(10, TimeUnit.SECONDS))
        assertEquals(null, errorHolder)
        assertTrue(local.readText().isNotEmpty())
        local.delete()
    }

    @Volatile private var errorHolder: String? = "unset"

    @Test
    fun sshPromptsForHostKeyAndPassword() = runBlocking<Unit> {
        client.login(server, "alice", "pw", null)
        val asked = CopyOnWriteArrayList<String>()
        val prompt = object : SecretPrompt {
            override fun ask(prompt: String, echo: Boolean): String? {
                asked.add("ask:$echo")
                return "secret"
            }

            override fun confirmHostKey(host: String, port: UShort, keyType: String, fingerprint: String, changed: Boolean): Boolean {
                asked.add("hostkey:$changed")
                assertTrue(fingerprint.startsWith("SHA256:"))
                return true
            }
        }
        val session = client.connectSsh(MockData.SSH_PG, prompt)
        // 首次连接先确认主机密钥，再问密码（遮挡输入）
        assertEquals(listOf("hostkey:false", "ask:false"), asked.toList())
        assertEquals(MockData.SSH_PG, session.info().hostId)
        assertTrue(session.info().via.startsWith("device:"))
        assertNotNull(session.files())
        assertTrue(client.vault().any { (it.data as? VaultData.KnownHost)?.host == "10.0.3.21" })
        session.disconnect()
        assertTrue(session.isClosed())

        // 主机密钥已记录：第二次只问密码
        asked.clear()
        client.connectSsh(MockData.SSH_PG, prompt).disconnect()
        assertEquals(listOf("ask:false"), asked.toList())
    }

    @Test
    fun rdpForwardsThroughDeviceAndCloses() = runBlocking<Unit> {
        client.login(server, "alice", "pw", null)
        val t = client.openRdp(MockData.RDP_FINANCE)
        assertTrue(t.forwarded)
        assertEquals("127.0.0.1", t.host)
        assertTrue(t.forwardId.isNotEmpty())
        client.closeRdp(t.forwardId)
    }

    @Test
    fun audioOnlyOnDevicesThatAllowIt() = runBlocking<Unit> {
        client.login(server, "alice", "pw", null)
        val pc = client.connect(MockData.DEV_OFFICE_PC)
        assertTrue("audio" in pc.info().capabilities)
        val a = pc.startAudio(0.6f)
        assertTrue(a.isActive())
        a.setVolume(2f)
        a.setMuted(true)
        val m = a as MockAudioPlayback
        assertEquals(1f, m.volume)
        assertTrue(m.muted)
        a.stop()
        a.stop()
        assertFalse(a.isActive())
        // 没有画面的 Linux 被控端：能力里没有 audio，硬开也报错
        val web = client.connect(MockData.DEV_WEB_01)
        assertFalse("audio" in web.info().capabilities)
        try {
            web.startAudio(1f)
            fail("audio started on a device without the capability")
        } catch (e: GalaxyException.Permission) {
            assertTrue(e.message.orEmpty().isNotEmpty())
        }
    }

    @Test
    fun wakeOnlyOfflineDevicesWithAddress() = runBlocking<Unit> {
        client.login(server, "alice", "pw", null)
        // render-ws：离线，经 OFFICE-PC 代发；db-01：离线，本机代发
        assertTrue(client.canWake(MockData.DEV_RENDER))
        assertTrue(client.canWake(MockData.DEV_DB_01))
        // 在线设备、局域网可见但没上报网卡地址的设备、不存在的设备都不能远程开机
        assertFalse(client.canWake(MockData.DEV_OFFICE_PC))
        assertFalse(client.canWake(MockData.DEV_NAS))
        assertFalse(client.canWake("nope"))
        assertEquals(listOf("OFFICE-PC"), client.wakeDevice(MockData.DEV_RENDER))
        assertEquals(listOf(""), client.wakeDevice(MockData.DEV_DB_01))
        try {
            client.wakeDevice(MockData.DEV_NAS)
            fail("woke a device without a reported address")
        } catch (e: GalaxyException.Other) {
            assertTrue(e.message.orEmpty().isNotEmpty())
        }
    }

    @Test
    fun portForwardEntriesAreIgnoredByHostTree() {
        val data = cn.lemonit.galaxy.app.HostsData(MockData.devices(), MockData.vault())
        assertTrue(data.vault.any { it.data is VaultData.PortForward })
        val items = cn.lemonit.galaxy.app.HostTree.items(data)
        assertTrue(items.none { it.key.contains(MockData.PF_PG) })
        // 规则所在文件夹的计数只算设备与主机
        val rows = cn.lemonit.galaxy.app.HostTree.rows(data)
        assertTrue(rows.none { it.item?.key?.contains(MockData.PF_PG) == true })
    }

    @Test
    fun recordingWritesPlayableMp4() = runBlocking<Unit> {
        client.login(server, "alice", "pw", null)
        val pc = client.connect(MockData.DEV_OFFICE_PC)
        val frames = java.util.concurrent.atomic.AtomicInteger()
        val sink = object : cn.lemonit.galaxy.core.bridge.VideoSink {
            override fun onConfig(codec: cn.lemonit.galaxy.core.bridge.VideoCodec, width: UInt, height: UInt) = Unit
            override fun onFrame(data: ByteArray, ptsUs: ULong, keyframe: Boolean) {
                frames.incrementAndGet()
            }
            override fun onCursor(x: Int, y: Int, visible: Boolean) = Unit
            override fun onClosed(reason: String) = Unit
        }
        val ch = pc.openDisplay(cn.lemonit.galaxy.core.bridge.DisplayOptions(1u, 30u, 0u, listOf(cn.lemonit.galaxy.core.bridge.VideoCodec.H264), false), sink)
        val dir = java.nio.file.Files.createTempDirectory("rec").toFile()
        val file = File(dir, "a/LemonGalaxy-test.mp4")
        assertFalse(ch.isRecording())
        try {
            ch.stopRecording()
            fail("stopRecording without recording")
        } catch (e: GalaxyException.Other) {
            assertTrue(e.message.orEmpty().isNotEmpty())
        }
        ch.startRecording(file.path, withAudio = true)
        assertTrue(ch.isRecording())
        try {
            ch.startRecording(file.path, withAudio = false)
            fail("started twice")
        } catch (e: GalaxyException.Other) {
            assertTrue(e.message.orEmpty().isNotEmpty())
        }
        val start = frames.get()
        waitFor { frames.get() - start >= 20 }
        val info = ch.stopRecording()
        assertFalse(ch.isRecording())
        assertEquals(listOf(file.path), info.paths)
        assertTrue(file.length() > 1000)
        assertEquals(file.length().toULong(), info.bytes)
        val head = file.readBytes().copyOfRange(4, 8).decodeToString()
        assertEquals("ftyp", head)
        // 关画面时正在录的录像也会结束
        val file2 = File(dir, "b.mp4")
        ch.startRecording(file2.path, withAudio = false)
        val s2 = frames.get()
        waitFor { frames.get() - s2 >= 5 }
        ch.stop()
        assertFalse(ch.isRecording())
        assertTrue(file2.length() > 0)
        System.getenv("GALAXY_KEEP_RECORDING")?.let { file.copyTo(File(it), overwrite = true) }
        dir.deleteRecursively()
    }

    @Test
    fun updateCheckAndDownloadReportsResult() = runBlocking<Unit> {
        client.login(server, "alice", "pw", null)
        assertEquals(null, client.checkUpdate("mobile", "android", "", "9.9.9"))
        val info = client.checkUpdate("mobile", "android", "", "0.1.0")
        assertNotNull(info)
        val done = CountDownLatch(1)
        var progressed = false
        client.downloadUpdate(info!!, File.createTempFile("upd", ".apk").path, object : TransferListener {
            override fun onProgress(done: ULong, total: ULong) {
                progressed = true
            }

            override fun onFinished(error: String?) {
                this@MockCoreTest.updateError = error
                done.countDown()
            }
        })
        assertTrue(done.await(10, TimeUnit.SECONDS))
        assertTrue(progressed)
        // 模拟内核不提供真实安装包：以清楚的错误结束
        assertNotNull(updateError)
    }

    @Volatile private var updateError: String? = null

    /** 记下远程升级的回调顺序。 */
    private class UpgradeRecorder : UpgradeListener {
        val events = CopyOnWriteArrayList<String>()
        val done = CountDownLatch(1)

        override fun onStarted(version: String) {
            events.add("started:$version")
        }

        override fun onOffline() {
            events.add("offline")
        }

        override fun onFinished(version: String?, error: String?) {
            events.add(if (error != null) "failed" else "finished:$version")
            done.countDown()
        }
    }

    @Test
    fun upgradeDeviceGoesOfflineAndComesBackWithNewVersion() = runBlocking<Unit> {
        client.login(server, "alice", "pw", null)
        val before = client.devices().first { it.id == MockData.DEV_WEB_01 }
        assertEquals(MockData.UPGRADE_TO, before.updateVersion)
        val r = UpgradeRecorder()
        client.upgradeDevice(MockData.DEV_WEB_01, r)
        // 升级途中设备掉线
        waitFor(10_000) { "offline" in r.events }
        assertFalse(client.devices().first { it.id == MockData.DEV_WEB_01 }.online)
        assertTrue(r.done.await(15, TimeUnit.SECONDS))
        assertEquals(listOf("started:${MockData.UPGRADE_TO}", "offline", "finished:${MockData.UPGRADE_TO}"), r.events.toList())
        val after = client.devices().first { it.id == MockData.DEV_WEB_01 }
        assertTrue(after.online)
        assertEquals(MockData.UPGRADE_TO, after.appVersion)
        assertEquals("", after.updateVersion)
    }

    @Test
    fun upgradeDeviceAlreadyLatestOrOfflineEndsRightAway() = runBlocking<Unit> {
        client.login(server, "alice", "pw", null)
        // 本来就是最新：直接完成，带现在的版本，不经过开始与掉线
        val latest = UpgradeRecorder()
        client.upgradeDevice(MockData.DEV_MACBOOK, latest)
        assertTrue(latest.done.await(5, TimeUnit.SECONDS))
        assertEquals(listOf("finished:0.1.0"), latest.events.toList())
        // 离线设备：以原因结束
        val offline = UpgradeRecorder()
        client.upgradeDevice(MockData.DEV_RENDER, offline)
        assertTrue(offline.done.await(5, TimeUnit.SECONDS))
        assertEquals(listOf("failed"), offline.events.toList())
    }

    @Test
    fun sshRejectingHostKeyCancels() = runBlocking<Unit> {
        client.login(server, "alice", "pw", null)
        val result = runCatching {
            client.connectSsh(MockData.SSH_BASTION, object : SecretPrompt {
                override fun ask(prompt: String, echo: Boolean): String? = null
                override fun confirmHostKey(host: String, port: UShort, keyType: String, fingerprint: String, changed: Boolean) = false
            })
        }
        assertTrue(result.exceptionOrNull() is GalaxyException.Cancelled)
    }

    @Test
    fun agentAsksForApprovalAndSecret() = runBlocking<Unit> {
        client.login(server, "alice", "pw", null)
        val events = CopyOnWriteArrayList<AgentEvent>()
        val conv = client.newConversation(
            ConversationConfig(1u, "glm-5.2", listOf(AgentTarget(AgentTargetKind.DEVICE, MockData.DEV_WEB_01)), AiMode.CONFIRM_DANGEROUS, false, null, null),
            object : AgentListener {
                override fun onEvent(event: AgentEvent) {
                    events.add(event)
                }
            },
        )
        conv.send("clean up disk")
        waitFor(20_000) { events.any { it is AgentEvent.ToolProposed && it.needsApproval } }
        val call = events.filterIsInstance<AgentEvent.ToolProposed>().first { it.needsApproval }.call
        assertEquals(cn.lemonit.galaxy.core.bridge.RiskLevel.DANGEROUS, call.risk)
        // 只读命令自动执行，没有要确认
        assertTrue(events.filterIsInstance<AgentEvent.ToolProposed>().filter { it.call.risk == cn.lemonit.galaxy.core.bridge.RiskLevel.READ }.none { it.needsApproval })
        conv.decide(call.callId, ApprovalDecision.AllowOnce)
        waitFor { events.any { it is AgentEvent.SecretRequested } }
        val req = events.filterIsInstance<AgentEvent.SecretRequested>().first()
        conv.provideSecret(req.requestId, "hunter2")
        waitFor { events.any { it is AgentEvent.AskUser } }
        // 密码不出现在任何事件和历史里
        assertFalse(events.toString().contains("hunter2"))
        assertFalse(conv.messages().toString().contains("hunter2"))
        conv.stop()
    }

    @Test
    fun readOnlyModeDeniesChanges() = runBlocking<Unit> {
        client.login(server, "alice", "pw", null)
        val events = CopyOnWriteArrayList<AgentEvent>()
        val conv = client.newConversation(
            ConversationConfig(1u, "glm-5.2", listOf(AgentTarget(AgentTargetKind.DEVICE, MockData.DEV_WEB_01)), AiMode.READ_ONLY, false, null, null),
            object : AgentListener {
                override fun onEvent(event: AgentEvent) {
                    events.add(event)
                }
            },
        )
        conv.send("clean up disk")
        waitFor(20_000) { events.any { it is AgentEvent.ToolFinished && it.result.denied } }
        assertTrue(events.filterIsInstance<AgentEvent.ToolProposed>().none { it.needsApproval })
        conv.stop()
    }

    private fun waitFor(timeoutMs: Long = 5_000, cond: () -> Boolean) {
        val end = System.currentTimeMillis() + timeoutMs
        while (!cond()) {
            if (System.currentTimeMillis() > end) fail("timed out")
            Thread.sleep(20)
        }
    }
}
