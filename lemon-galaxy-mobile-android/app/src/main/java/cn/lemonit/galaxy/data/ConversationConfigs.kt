package cn.lemonit.galaxy.data

import android.content.Context
import android.content.SharedPreferences
import cn.lemonit.galaxy.app.ConversationConfigStore
import cn.lemonit.galaxy.core.bridge.AgentTarget
import cn.lemonit.galaxy.core.bridge.AgentTargetKind
import cn.lemonit.galaxy.core.bridge.AiMode
import cn.lemonit.galaxy.core.bridge.ConversationConfig
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

/**
 * 每个 AI 对话创建时的模型、目标、档位，存本机（不含对话内容，对话内容由内核加密保存）。
 * 重新打开历史对话时交给 open_conversation。
 */
class PrefsConversationConfigStore(private val sp: SharedPreferences) : ConversationConfigStore {
    @Serializable
    private data class Stored(
        val providerId: Long,
        val modelId: String,
        val targets: List<Target>,
        val mode: String,
        val planFirst: Boolean,
        val trustMinutes: Int? = null,
        val title: String? = null,
    )

    @Serializable
    private data class Target(val kind: String, val id: String)

    private val json = Json { ignoreUnknownKeys = true }

    override fun get(id: String): ConversationConfig? {
        val raw = sp.getString(id, null) ?: return null
        return runCatching {
            val s = json.decodeFromString(Stored.serializer(), raw)
            ConversationConfig(
                providerId = s.providerId.toULong(),
                modelId = s.modelId,
                targets = s.targets.mapNotNull { t -> runCatching { AgentTarget(AgentTargetKind.valueOf(t.kind), t.id) }.getOrNull() },
                mode = runCatching { AiMode.valueOf(s.mode) }.getOrDefault(AiMode.CONFIRM_DANGEROUS),
                planFirst = s.planFirst,
                trustMinutes = s.trustMinutes?.toUInt(),
                title = s.title,
            )
        }.getOrNull()
    }

    override fun put(id: String, config: ConversationConfig) {
        val s = Stored(
            config.providerId.toLong(), config.modelId, config.targets.map { Target(it.kind.name, it.id) },
            config.mode.name, config.planFirst, config.trustMinutes?.toInt(), config.title,
        )
        sp.edit().putString(id, json.encodeToString(Stored.serializer(), s)).apply()
    }

    override fun remove(id: String) {
        sp.edit().remove(id).apply()
    }

    companion object {
        fun create(context: Context) = PrefsConversationConfigStore(context.getSharedPreferences("conversation-configs", Context.MODE_PRIVATE))
    }
}
