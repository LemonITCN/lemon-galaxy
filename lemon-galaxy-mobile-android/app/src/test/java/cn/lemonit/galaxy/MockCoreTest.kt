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
import cn.lemonit.galaxy.core.bridge.VaultData
import cn.lemonit.galaxy.core.bridge.VaultEntry
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
