package cn.lemonit.galaxy.app

import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.AiModel
import cn.lemonit.galaxy.core.bridge.AiModels
import cn.lemonit.galaxy.core.bridge.ContextRef
import cn.lemonit.galaxy.core.bridge.ContextRefKind
import cn.lemonit.galaxy.core.bridge.DeviceKind
import cn.lemonit.galaxy.core.bridge.Platform
import cn.lemonit.galaxy.core.bridge.VaultData
import java.time.Instant
import java.time.ZoneId
import java.time.temporal.ChronoUnit

/*
 * AI 聊天页（docs/ai-chat-ui.md）里与界面无关的规则：可用状态、默认模型与档位、@ 引用解析、
 * 引用的构造与去重、对话按日期分组。全部是纯函数，JVM 单元测试覆盖。
 */

// ---------- 第 5 节：不可用时的提示 ----------

/** ai_models() 的加载结果。 */
sealed class AiModelsState {
    data object Loading : AiModelsState()
    data class Failed(val message: String) : AiModelsState()
    data class Loaded(val models: AiModels) : AiModelsState()
}

enum class AiAvailability { LOADING, FAILED, DISABLED, NO_MODELS, READY }

/** 输入框只有 READY 时可用。[userAiEnabled] 为登录用户的 ai_enabled（未知时为 null）。 */
fun aiAvailability(state: AiModelsState, userAiEnabled: Boolean?): AiAvailability = when {
    userAiEnabled == false -> AiAvailability.DISABLED
    state is AiModelsState.Loading -> AiAvailability.LOADING
    state is AiModelsState.Failed -> AiAvailability.FAILED
    state is AiModelsState.Loaded && !state.models.enabled -> AiAvailability.DISABLED
    state is AiModelsState.Loaded && state.models.items.isEmpty() -> AiAvailability.NO_MODELS
    else -> AiAvailability.READY
}

// ---------- 第 2、3 节：模型与档位 ----------

/** 输入框里可选的四个档位（off 只作上限用）。 */
val SELECTABLE_MODES = listOf(AiMode.READ_ONLY, AiMode.CONFIRM_ALL, AiMode.CONFIRM_DANGEROUS, AiMode.FULL_TRUST)

/** 档位不超过管理员上限；存下来的是 off（不可选）时回到默认的「危险操作确认」再取上限。 */
fun capMode(mode: AiMode, max: AiMode): AiMode {
    val m = if (mode == AiMode.OFF) AiMode.CONFIRM_DANGEROUS else mode
    if (max == AiMode.OFF) return AiMode.READ_ONLY
    return if (m.ordinal > max.ordinal) max else m
}

/** 高于管理员上限的档位在菜单里置灰。 */
fun modeAllowed(mode: AiMode, max: AiMode): Boolean = mode != AiMode.OFF && mode.ordinal <= max.ordinal

/** 默认模型：本机记住的上次选择，没有或已不可用时取列表第一个。[providerId] 为 -1 表示没选过。 */
fun pickDefaultModel(items: List<AiModel>, providerId: Long, modelId: String): AiModel? =
    items.firstOrNull { providerId >= 0 && it.providerId == providerId.toULong() && it.id == modelId } ?: items.firstOrNull()

/** 模型菜单按供应商分组（保持服务器给出的顺序）。 */
fun groupModels(items: List<AiModel>): List<Pair<String, List<AiModel>>> =
    items.groupBy { it.providerId }.values.map { it.first().providerName to it }

// ---------- 第 2.1 节：@ 引用 ----------

/** 文本框里正在输入的 @ 引用：[start] 是 @ 的位置，[end] 是光标，[query] 是两者之间的过滤文字。 */
data class MentionQuery(val start: Int, val end: Int, val query: String)

/**
 * 光标前面最近的一个 @，且 @ 到光标之间没有空白时，返回正在输入的引用。
 * @ 前面紧挨着字母数字或 . _ - 时（如邮箱 a@b.com）不算。
 */
fun findMention(text: String, cursor: Int): MentionQuery? {
    if (cursor < 1 || cursor > text.length) return null
    var i = cursor - 1
    while (i >= 0) {
        val c = text[i]
        if (c == '@') {
            val prev = if (i > 0) text[i - 1] else ' '
            val glued = prev.isAsciiWordChar()
            return if (glued) null else MentionQuery(i, cursor, text.substring(i + 1, cursor))
        }
        if (c.isWhitespace()) return null
        i--
    }
    return null
}

private fun Char.isAsciiWordChar() = this in 'a'..'z' || this in 'A'..'Z' || this in '0'..'9' || this == '.' || this == '_' || this == '-'

/** 选中候选后删掉 @ 与过滤文字，返回新文本与光标位置。 */
fun removeMention(text: String, m: MentionQuery): Pair<String, Int> {
    val end = m.end.coerceAtMost(text.length)
    return text.removeRange(m.start, end) to m.start
}

/** @ 候选：账号里的设备（不含本机与手机）与 SSH 主机。 */
data class MentionCandidate(
    val ref: ContextRef,
    /** 设备的平台；SSH 主机为 null。 */
    val platform: Platform?,
    val online: Boolean,
    val subtitle: String,
)

fun mentionCandidates(data: HostsData): List<MentionCandidate> {
    val devices = data.devices
        .filter { it.kind != DeviceKind.MOBILE && !it.isSelf }
        .map { d ->
            MentionCandidate(
                ContextRef(ContextRefKind.DEVICE, "", d.id, data.deviceName(d.id)),
                d.platform, d.online || d.lanSeen, d.osVersion,
            )
        }
    val hosts = data.vault.mapNotNull { e ->
        (e.data as? VaultData.SshHost)?.let { h ->
            val port = if (h.port.toInt() != 22) ":${h.port}" else ""
            MentionCandidate(ContextRef(ContextRefKind.SSH_HOST, "", e.id, h.label), null, true, "${h.username}@${h.host}$port")
        }
    }
    return devices + hosts
}

/** 按输入的文字过滤（不分大小写，名称或副标题包含即可），名称以它开头的排前面。 */
fun filterCandidates(all: List<MentionCandidate>, query: String): List<MentionCandidate> {
    val q = query.trim().lowercase()
    if (q.isEmpty()) return all
    return all.filter { it.ref.label.lowercase().contains(q) || it.subtitle.lowercase().contains(q) }
        .sortedBy { if (it.ref.label.lowercase().startsWith(q)) 0 else 1 }
}

// ---------- 第 2.2 节：引用 ----------

object Refs {
    fun device(id: String, label: String) = ContextRef(ContextRefKind.DEVICE, "", id, label)
    fun sshHost(entryId: String, label: String) = ContextRef(ContextRefKind.SSH_HOST, "", entryId, label)

    /** 文件 / 文件夹：target_id 为所在设备 ID 或 SSH 主机条目 ID，id 为路径，标签显示文件名。 */
    fun path(target: TargetRef, path: String, folder: Boolean) = ContextRef(
        if (folder) ContextRefKind.FOLDER else ContextRefKind.FILE,
        when (target) {
            is TargetRef.Device -> target.deviceId
            is TargetRef.Ssh -> target.hostId
        },
        path,
        fileName(path),
    )

    fun snippet(entryId: String, label: String) = ContextRef(ContextRefKind.SNIPPET, "", entryId, label)
    fun portForward(entryId: String, label: String) = ContextRef(ContextRefKind.PORT_FORWARD, "", entryId, label)
    fun activity(label: String) = ContextRef(ContextRefKind.ACTIVITY, "", "", label)

    /** 路径的最后一段；根目录显示 "/"。 */
    fun fileName(path: String): String {
        val p = path.trimEnd('/', '\\')
        if (p.isEmpty()) return path.ifEmpty { "/" }
        val i = maxOf(p.lastIndexOf('/'), p.lastIndexOf('\\'))
        return if (i >= 0) p.substring(i + 1).ifEmpty { p } else p
    }

    /** 加引用（同一个资源只留一个）。 */
    fun add(refs: List<ContextRef>, ref: ContextRef): List<ContextRef> =
        if (refs.any { it.kind == ref.kind && it.targetId == ref.targetId && it.id == ref.id }) refs else refs + ref

    fun addAll(refs: List<ContextRef>, more: List<ContextRef>): List<ContextRef> = more.fold(refs, ::add)

    /** 文件 / 文件夹所在设备或主机的名字（长按标签时显示）。 */
    fun placeName(data: HostsData, ref: ContextRef): String =
        (data.entry(ref.targetId)?.data as? VaultData.SshHost)?.label ?: data.deviceName(ref.targetId)
}

/** 文本与引用都为空时发送按钮置灰。 */
fun canSend(text: String, refs: List<ContextRef>): Boolean = text.isNotBlank() || refs.isNotEmpty()

// ---------- 第 1 节：对话列表 ----------

enum class DateGroup { TODAY, YESTERDAY, LAST_7_DAYS, EARLIER }

/** 按自然日分组：今天 / 昨天 / 最近 7 天（2–6 天前）/ 更早。 */
fun dateGroup(at: Long, now: Long, zone: ZoneId = ZoneId.systemDefault()): DateGroup {
    val day = Instant.ofEpochMilli(at).atZone(zone).toLocalDate()
    val today = Instant.ofEpochMilli(now).atZone(zone).toLocalDate()
    val diff = ChronoUnit.DAYS.between(day, today)
    return when {
        diff <= 0 -> DateGroup.TODAY
        diff == 1L -> DateGroup.YESTERDAY
        diff < 7 -> DateGroup.LAST_7_DAYS
        else -> DateGroup.EARLIER
    }
}

/** 按更新时间倒序后分组，空组不出现。 */
fun <T> groupByDate(items: List<T>, at: (T) -> Long, now: Long, zone: ZoneId = ZoneId.systemDefault()): List<Pair<DateGroup, List<T>>> {
    val sorted = items.sortedByDescending(at)
    val byGroup = sorted.groupBy { dateGroup(at(it), now, zone) }
    return DateGroup.entries.mapNotNull { g -> byGroup[g]?.let { g to it } }
}

/** 对话列表的搜索：标题包含输入的文字（不分大小写）。 */
fun filterConversations(items: List<ConversationListItem>, query: String): List<ConversationListItem> {
    val q = query.trim().lowercase()
    return if (q.isEmpty()) items else items.filter { it.title.lowercase().contains(q) }
}
