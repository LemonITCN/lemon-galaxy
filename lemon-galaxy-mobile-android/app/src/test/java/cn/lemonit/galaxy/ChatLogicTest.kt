package cn.lemonit.galaxy

import cn.lemonit.galaxy.app.AiAvailability
import cn.lemonit.galaxy.app.AiModelsState
import cn.lemonit.galaxy.app.ConversationListItem
import cn.lemonit.galaxy.app.DateGroup
import cn.lemonit.galaxy.app.HostsData
import cn.lemonit.galaxy.app.Refs
import cn.lemonit.galaxy.app.TargetRef
import cn.lemonit.galaxy.app.aiAvailability
import cn.lemonit.galaxy.app.canSend
import cn.lemonit.galaxy.app.capMode
import cn.lemonit.galaxy.app.dateGroup
import cn.lemonit.galaxy.app.filterCandidates
import cn.lemonit.galaxy.app.filterConversations
import cn.lemonit.galaxy.app.findMention
import cn.lemonit.galaxy.app.groupByDate
import cn.lemonit.galaxy.app.groupModels
import cn.lemonit.galaxy.app.mentionCandidates
import cn.lemonit.galaxy.app.modeAllowed
import cn.lemonit.galaxy.app.pickDefaultModel
import cn.lemonit.galaxy.app.removeMention
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.AiModel
import cn.lemonit.galaxy.core.bridge.AiModels
import cn.lemonit.galaxy.core.bridge.ContextRefKind
import cn.lemonit.galaxy.core.mock.MockData
import cn.lemonit.galaxy.ui.ai.Markdown
import cn.lemonit.galaxy.ui.ai.MdBlock
import cn.lemonit.galaxy.ui.ai.MdSpan
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.LocalDateTime
import java.time.ZoneId

/** AI 聊天页的纯逻辑（docs/ai-chat-ui.md）。 */
class ChatLogicTest {
    private val models = listOf(
        AiModel(1u, "OpenCode Go", "glm-5.2", "GLM-5.2", false, true, 200_000u),
        AiModel(1u, "OpenCode Go", "kimi-k2.6", "Kimi K2.6", true, true, 256_000u),
        AiModel(2u, "Local", "qwen", "Qwen", true, true, 32_000u),
    )

    // ---------- @ 引用 ----------

    @Test
    fun mentionAtStartAndAfterSpace() {
        assertEquals("", findMention("@", 1)?.query)
        assertEquals("web", findMention("look @web", 9)?.query)
        assertEquals(5, findMention("look @web", 9)?.start)
    }

    @Test
    fun mentionAfterChineseWithoutSpace() {
        val t = "看看@db"
        assertEquals("db", findMention(t, t.length)?.query)
        assertEquals(2, findMention(t, t.length)?.start)
    }

    @Test
    fun noMentionForEmailOrAfterSpace() {
        assertNull(findMention("mail a@b.com", 12))
        assertNull(findMention("@web now", 8))
        assertNull(findMention("no at here", 10))
        assertNull(findMention("", 0))
    }

    @Test
    fun mentionUsesCursorNotEnd() {
        val t = "@web and more"
        assertEquals("we", findMention(t, 3)?.query)
    }

    @Test
    fun removeMentionDropsAtAndQuery() {
        val t = "看看 @we 的磁盘"
        val m = findMention(t, 6)!!
        val (out, cursor) = removeMention(t, m)
        assertEquals("看看  的磁盘", out)
        assertEquals(3, cursor)
    }

    @Test
    fun candidatesExcludeSelfAndPhonesAndIncludeSshHosts() {
        val data = HostsData(MockData.devices(MockData.NOW), MockData.vault(MockData.NOW))
        val c = mentionCandidates(data)
        assertTrue(c.none { it.ref.id == MockData.DEV_SELF })
        assertTrue(c.any { it.ref.kind == ContextRefKind.SSH_HOST && it.ref.id == MockData.SSH_GITLAB })
        // 设备显示别名
        assertEquals("web-01 (nginx)", c.first { it.ref.id == MockData.DEV_WEB_01 }.ref.label)
    }

    @Test
    fun filterCandidatesPrefersPrefix() {
        val data = HostsData(MockData.devices(MockData.NOW), MockData.vault(MockData.NOW))
        val all = mentionCandidates(data)
        assertEquals(all, filterCandidates(all, ""))
        val r = filterCandidates(all, "WEB")
        assertTrue(r.isNotEmpty())
        assertTrue(r.first().ref.label.lowercase().startsWith("web"))
        assertTrue(filterCandidates(all, "zzzz").isEmpty())
    }

    // ---------- 引用 ----------

    @Test
    fun pathRefsUseTargetAndFileName() {
        val f = Refs.path(TargetRef.Device("d1"), "/etc/nginx/nginx.conf", folder = false)
        assertEquals(ContextRefKind.FILE, f.kind)
        assertEquals("d1", f.targetId)
        assertEquals("/etc/nginx/nginx.conf", f.id)
        assertEquals("nginx.conf", f.label)
        val d = Refs.path(TargetRef.Ssh("h1"), "/var/log/", folder = true)
        assertEquals(ContextRefKind.FOLDER, d.kind)
        assertEquals("h1", d.targetId)
        assertEquals("log", d.label)
        assertEquals("/", Refs.fileName("/"))
        assertEquals("Users", Refs.fileName("C:\\Users"))
    }

    @Test
    fun addRefDeduplicates() {
        val a = Refs.device("d1", "web")
        val refs = Refs.addAll(emptyList(), listOf(a, a, Refs.activity("x"), Refs.activity("x")))
        assertEquals(2, refs.size)
        assertEquals("", refs[1].id)
        assertEquals("", refs[1].targetId)
    }

    @Test
    fun sendNeedsTextOrRefs() {
        assertFalse(canSend("  ", emptyList()))
        assertTrue(canSend("hi", emptyList()))
        assertTrue(canSend("", listOf(Refs.activity("x"))))
    }

    // ---------- 模型与档位 ----------

    @Test
    fun defaultModelIsRememberedOrFirst() {
        assertEquals("kimi-k2.6", pickDefaultModel(models, 1, "kimi-k2.6")?.id)
        // 记住的已不可用：取第一个
        assertEquals("glm-5.2", pickDefaultModel(models, 1, "gone")?.id)
        assertEquals("glm-5.2", pickDefaultModel(models, 2, "kimi-k2.6")?.id)
        // 没选过
        assertEquals("glm-5.2", pickDefaultModel(models, -1, "")?.id)
        assertNull(pickDefaultModel(emptyList(), 1, "glm-5.2"))
    }

    @Test
    fun modelsGroupedByProviderInOrder() {
        val g = groupModels(models)
        assertEquals(listOf("OpenCode Go", "Local"), g.map { it.first })
        assertEquals(2, g[0].second.size)
    }

    @Test
    fun modeCappedByAdmin() {
        assertEquals(AiMode.CONFIRM_DANGEROUS, capMode(AiMode.CONFIRM_DANGEROUS, AiMode.FULL_TRUST))
        assertEquals(AiMode.CONFIRM_ALL, capMode(AiMode.FULL_TRUST, AiMode.CONFIRM_ALL))
        assertEquals(AiMode.READ_ONLY, capMode(AiMode.CONFIRM_DANGEROUS, AiMode.READ_ONLY))
        assertEquals(AiMode.CONFIRM_DANGEROUS, capMode(AiMode.OFF, AiMode.FULL_TRUST))
        assertTrue(modeAllowed(AiMode.CONFIRM_ALL, AiMode.CONFIRM_ALL))
        assertFalse(modeAllowed(AiMode.FULL_TRUST, AiMode.CONFIRM_DANGEROUS))
    }

    // ---------- 不可用提示 ----------

    @Test
    fun availabilityStates() {
        val ok = AiModelsState.Loaded(AiModels(true, AiMode.FULL_TRUST, models))
        assertEquals(AiAvailability.LOADING, aiAvailability(AiModelsState.Loading, true))
        assertEquals(AiAvailability.FAILED, aiAvailability(AiModelsState.Failed("x"), null))
        assertEquals(AiAvailability.READY, aiAvailability(ok, true))
        assertEquals(AiAvailability.DISABLED, aiAvailability(ok, false))
        assertEquals(AiAvailability.DISABLED, aiAvailability(AiModelsState.Loaded(AiModels(false, AiMode.OFF, emptyList())), true))
        assertEquals(AiAvailability.NO_MODELS, aiAvailability(AiModelsState.Loaded(AiModels(true, AiMode.FULL_TRUST, emptyList())), true))
    }

    // ---------- 对话列表 ----------

    private val zone = ZoneId.of("Asia/Shanghai")
    private fun at(y: Int, mo: Int, d: Int, h: Int) = LocalDateTime.of(y, mo, d, h, 0).atZone(zone).toInstant().toEpochMilli()

    @Test
    fun dateGroupsByCalendarDay() {
        val now = at(2026, 11, 22, 9)
        assertEquals(DateGroup.TODAY, dateGroup(at(2026, 11, 22, 0), now, zone))
        assertEquals(DateGroup.YESTERDAY, dateGroup(at(2026, 11, 21, 23), now, zone))
        assertEquals(DateGroup.YESTERDAY, dateGroup(at(2026, 11, 21, 0), now, zone))
        assertEquals(DateGroup.LAST_7_DAYS, dateGroup(at(2026, 11, 20, 12), now, zone))
        assertEquals(DateGroup.LAST_7_DAYS, dateGroup(at(2026, 11, 16, 12), now, zone))
        assertEquals(DateGroup.EARLIER, dateGroup(at(2026, 11, 15, 12), now, zone))
        // 时钟不准时未来的也算今天
        assertEquals(DateGroup.TODAY, dateGroup(at(2026, 11, 23, 12), now, zone))
    }

    @Test
    fun groupByDateSortsAndSkipsEmptyGroups() {
        val now = at(2026, 11, 22, 9)
        val items = listOf(
            ConversationListItem("old", "Old", at(2026, 10, 1, 9)),
            ConversationListItem("t1", "Today 1", at(2026, 11, 22, 7)),
            ConversationListItem("t2", "Today 2", at(2026, 11, 22, 8)),
        )
        val g = groupByDate(items, { it.updatedAt }, now, zone)
        assertEquals(listOf(DateGroup.TODAY, DateGroup.EARLIER), g.map { it.first })
        assertEquals(listOf("t2", "t1"), g[0].second.map { it.id })
    }

    @Test
    fun searchFiltersTitles() {
        val items = listOf(ConversationListItem("a", "Nginx reload", 1), ConversationListItem("b", "磁盘清理", 2))
        assertEquals(listOf("a"), filterConversations(items, "NGINX").map { it.id })
        assertEquals(listOf("b"), filterConversations(items, "磁盘").map { it.id })
        assertEquals(2, filterConversations(items, " ").size)
    }

    // ---------- Markdown ----------

    @Test
    fun markdownBlocks() {
        val md = "# Title\n\ntext **bold**\n\n- a\n  - b\n1. one\n\n```bash\nls -la\n```\n\n| A | B |\n|---|:-:|\n| 1 | 2 |\n| 3 |\n\n> quote\n\n---"
        val b = Markdown.parse(md)
        assertEquals(MdBlock.Heading(1, "Title"), b[0])
        assertEquals(MdBlock.Paragraph("text **bold**"), b[1])
        assertEquals(MdBlock.ListItem("•", "a", 0), b[2])
        assertEquals(MdBlock.ListItem("•", "b", 1), b[3])
        assertEquals(MdBlock.ListItem("1.", "one", 0), b[4])
        assertEquals(MdBlock.Code("bash", "ls -la"), b[5])
        assertEquals(MdBlock.Table(listOf("A", "B"), listOf(listOf("1", "2"), listOf("3", ""))), b[6])
        assertEquals(MdBlock.Quote("quote"), b[7])
        assertEquals(MdBlock.Rule, b[8])
    }

    @Test
    fun markdownUnclosedCodeWhileStreaming() {
        val b = Markdown.parse("run:\n```sh\necho hi")
        assertEquals(MdBlock.Code("sh", "echo hi"), b[1])
    }

    @Test
    fun markdownInlineSpans() {
        val s = Markdown.spans("a **b** `c` *d* [e](https://x.cn) see https://y.cn/p.")
        assertEquals(
            listOf(
                MdSpan.Plain("a "), MdSpan.Bold("b"), MdSpan.Plain(" "), MdSpan.Code("c"), MdSpan.Plain(" "), MdSpan.Italic("d"),
                MdSpan.Plain(" "), MdSpan.Link("e", "https://x.cn"), MdSpan.Plain(" see "), MdSpan.Link("https://y.cn/p", "https://y.cn/p"), MdSpan.Plain("."),
            ),
            s,
        )
        // 乘号、未闭合的星号不当斜体
        assertEquals(listOf(MdSpan.Plain("2 * 3 = 6 and *x")), Markdown.spans("2 * 3 = 6 and *x"))
    }
}
