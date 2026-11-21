package cn.lemonit.galaxy.app

import cn.lemonit.galaxy.core.bridge.DeviceInfo
import cn.lemonit.galaxy.core.bridge.DeviceKind
import cn.lemonit.galaxy.core.bridge.GalaxyClientInterface
import cn.lemonit.galaxy.core.bridge.VaultData
import cn.lemonit.galaxy.core.bridge.VaultEntry
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class HostsData(
    val devices: List<DeviceInfo> = emptyList(),
    val vault: List<VaultEntry> = emptyList(),
    /** 能远程开机的离线设备（内核 canWake 为真） */
    val wakeable: Set<String> = emptySet(),
) {
    fun entry(id: String) = vault.firstOrNull { it.id == id }
    fun device(id: String) = devices.firstOrNull { it.id == id }

    /** 设备离线（局域网里也没发现）且内核说能远程开机 */
    fun canWake(id: String): Boolean = id in wakeable && device(id)?.let { !it.online && !it.lanSeen } == true
    fun metaFor(deviceId: String) = vault.firstOrNull { (it.data as? VaultData.DeviceMeta)?.deviceId == deviceId }
    val folders get() = vault.filter { it.data is VaultData.Folder }
    val credentials get() = vault.filter { it.data is VaultData.Credential }
    val snippets get() = vault.filter { it.data is VaultData.Snippet }
    val sshHosts get() = vault.filter { it.data is VaultData.SshHost }

    fun deviceName(id: String): String {
        val alias = (metaFor(id)?.data as? VaultData.DeviceMeta)?.alias
        return alias?.takeIf { it.isNotBlank() } ?: device(id)?.hostname ?: id.take(8)
    }

    fun targetName(t: TargetRef): String = when (t) {
        is TargetRef.Device -> deviceName(t.deviceId)
        is TargetRef.Ssh -> (entry(t.hostId)?.data as? VaultData.SshHost)?.label ?: t.hostId
    }

    /** 文件夹的完整路径（用于移动对话框与搜索结果）。 */
    fun folderPath(folderId: String?): String {
        val parts = mutableListOf<String>()
        var cur = folderId
        var guard = 0
        while (cur != null && guard++ < 32) {
            val e = entry(cur) ?: break
            parts.add(0, (e.data as? VaultData.Folder)?.name ?: break)
            cur = e.folderId
        }
        return parts.joinToString(" / ")
    }
}

/** 树里的一个可连接条目。 */
sealed class HostItem {
    abstract val key: String
    abstract val folderId: String?
    abstract val title: String
    abstract val tags: List<String>
    abstract val favorite: Boolean
    abstract val sort: Long
    abstract val searchText: String

    data class Device(val device: DeviceInfo, val meta: VaultEntry?) : HostItem() {
        private val m get() = meta?.data as? VaultData.DeviceMeta
        override val key get() = "device:${device.id}"
        override val folderId get() = meta?.folderId
        override val title get() = m?.alias?.takeIf { it.isNotBlank() } ?: device.hostname
        override val tags get() = m?.tags ?: emptyList()
        override val favorite get() = m?.favorite ?: false
        override val sort get() = meta?.sort ?: Long.MAX_VALUE
        override val searchText get() = listOf(title, device.hostname, device.osVersion, m?.note.orEmpty()).plus(tags).joinToString("\n").lowercase()
    }

    data class Ssh(val entry: VaultEntry, val host: VaultData.SshHost) : HostItem() {
        override val key get() = "ssh:${entry.id}"
        override val folderId get() = entry.folderId
        override val title get() = host.label
        override val tags get() = host.tags
        override val favorite get() = host.favorite
        override val sort get() = entry.sort
        override val searchText get() = listOf(host.label, host.host, host.username, host.note).plus(tags).joinToString("\n").lowercase()
        val address get() = "${host.username}@${host.host}" + if (host.port.toInt() != 22) ":${host.port}" else ""
    }

    data class Rdp(val entry: VaultEntry, val host: VaultData.RdpHost) : HostItem() {
        override val key get() = "rdp:${entry.id}"
        override val folderId get() = entry.folderId
        override val title get() = host.label
        override val tags get() = host.tags
        override val favorite get() = host.favorite
        override val sort get() = entry.sort
        override val searchText get() = listOf(host.label, host.host, host.username, host.note).plus(tags).joinToString("\n").lowercase()
        val address get() = (if (host.username.isNotBlank()) "${host.username}@" else "") + host.host + if (host.port.toInt() != 3389) ":${host.port}" else ""
    }
}

/** 列表里的一行：文件夹或条目。 */
data class TreeRow(
    val depth: Int,
    val folder: VaultEntry? = null,
    val item: HostItem? = null,
    val expanded: Boolean = false,
    val count: Int = 0,
    /** 搜索 / 筛选结果里显示所在文件夹路径。 */
    val path: String? = null,
) {
    val key: String get() = folder?.let { "folder:${it.id}" } ?: item!!.key
}

object HostTree {
    fun items(data: HostsData): List<HostItem> {
        val metaByDevice = data.vault.mapNotNull { e -> (e.data as? VaultData.DeviceMeta)?.let { it.deviceId to e } }.toMap()
        // 手机端只能主控、不能被控，不出现在可连接列表里
        val devices = data.devices.filter { it.kind != DeviceKind.MOBILE }.map { HostItem.Device(it, metaByDevice[it.id]) }
        val ssh = data.vault.mapNotNull { e -> (e.data as? VaultData.SshHost)?.let { HostItem.Ssh(e, it) } }
        val rdp = data.vault.mapNotNull { e -> (e.data as? VaultData.RdpHost)?.let { HostItem.Rdp(e, it) } }
        return devices + ssh + rdp
    }

    fun allTags(data: HostsData): List<String> = items(data).flatMap { it.tags }.distinct().sorted()

    /**
     * 生成列表行。query / tag / favoritesOnly 任一生效时返回平铺结果（带文件夹路径），否则返回按展开状态折叠的树。
     */
    fun rows(
        data: HostsData,
        query: String = "",
        tag: String? = null,
        favoritesOnly: Boolean = false,
        expandedOverride: Map<String, Boolean> = emptyMap(),
    ): List<TreeRow> {
        val all = items(data)
        val q = query.trim().lowercase()
        if (q.isNotEmpty() || tag != null || favoritesOnly) {
            return all.asSequence()
                .filter { q.isEmpty() || q.split(' ').all { part -> it.searchText.contains(part) } }
                .filter { tag == null || tag in it.tags }
                .filter { !favoritesOnly || it.favorite }
                .sortedWith(compareBy<HostItem>({ !it.favorite }, { it.title.lowercase() }))
                .map { TreeRow(0, item = it, path = data.folderPath(it.folderId).ifEmpty { null }) }
                .toList()
        }
        val folderIds = data.folders.map { it.id }.toSet()
        val foldersByParent = data.folders.groupBy { it.folderId?.takeIf { p -> p in folderIds } }
        val itemsByFolder = all.groupBy { it.folderId?.takeIf { f -> f in folderIds } }
        val out = mutableListOf<TreeRow>()

        fun countIn(folderId: String): Int =
            (itemsByFolder[folderId]?.size ?: 0) + (foldersByParent[folderId]?.sumOf { countIn(it.id) } ?: 0)

        fun walk(parent: String?, depth: Int) {
            foldersByParent[parent].orEmpty().sortedBy { it.sort }.forEach { f ->
                val folder = f.data as VaultData.Folder
                val expanded = expandedOverride[f.id] ?: folder.expanded
                out.add(TreeRow(depth, folder = f, expanded = expanded, count = countIn(f.id)))
                if (expanded) walk(f.id, depth + 1)
            }
            itemsByFolder[parent].orEmpty()
                .sortedWith(
                    compareBy<HostItem>(
                        { if (it is HostItem.Device && it.meta == null) 1 else 0 },
                        { it.sort },
                        { !(it is HostItem.Device && it.device.online) },
                        { it.title.lowercase() },
                    ),
                )
                .forEach { out.add(TreeRow(depth, item = it)) }
        }
        walk(null, 0)
        return out
    }

    /** 某个文件夹及其所有子孙文件夹的 id（移动时不能移进自己）。 */
    fun descendants(data: HostsData, folderId: String): Set<String> {
        val result = mutableSetOf(folderId)
        var changed = true
        while (changed) {
            changed = false
            data.folders.forEach { if (it.folderId in result && result.add(it.id)) changed = true }
        }
        return result
    }
}

/** 设备与主机：设备列表 + 加密库条目的增删改。 */
class HostsController(
    private val client: GalaxyClientInterface,
    private val scope: CoroutineScope,
    private val onError: (String) -> Unit,
) {
    private val _data = MutableStateFlow(HostsData())
    val data: StateFlow<HostsData> = _data.asStateFlow()
    private val _refreshing = MutableStateFlow(false)
    val refreshing: StateFlow<Boolean> = _refreshing.asStateFlow()

    fun reload() {
        val devices = runCatching { client.devices() }.getOrDefault(emptyList())
        val wakeable = devices.filter { !it.online && !it.lanSeen && runCatching { client.canWake(it.id) }.getOrDefault(false) }.map { it.id }.toSet()
        _data.value = HostsData(devices, runCatching { client.vault() }.getOrDefault(emptyList()), wakeable)
    }

    /** 远程开机：返回代发设备名（空字符串表示本机），失败抛出。 */
    suspend fun wake(deviceId: String): List<String> = client.wakeDevice(deviceId)

    fun refresh() {
        scope.launch {
            _refreshing.value = true
            try {
                client.refreshDevices()
            } catch (e: Exception) {
                onError(e.userMessage())
            }
            reload()
            _refreshing.value = false
        }
    }

    private fun launchSafe(block: suspend () -> Unit) {
        scope.launch {
            try {
                block()
            } catch (e: Exception) {
                e.userMessage().takeIf { it.isNotEmpty() }?.let(onError)
            }
            reload()
        }
    }

    fun save(entry: VaultEntry, done: (VaultEntry) -> Unit = {}) = launchSafe { done(client.vaultSave(entry)) }

    fun saveData(id: String?, folderId: String?, data: VaultData) {
        val existing = id?.let { _data.value.entry(it) }
        save(
            existing?.copy(data = data, folderId = folderId)
                ?: VaultEntry("", folderId, 0, data, System.currentTimeMillis()),
        )
    }

    fun delete(id: String) = launchSafe { client.vaultDelete(id) }

    fun removeDevice(deviceId: String) = launchSafe {
        client.removeDevice(deviceId)
        _data.value.metaFor(deviceId)?.let { client.vaultDelete(it.id) }
    }

    fun setFolderExpanded(folder: VaultEntry, expanded: Boolean) {
        val f = folder.data as? VaultData.Folder ?: return
        // 先改本地，界面立即响应
        _data.value = _data.value.copy(vault = _data.value.vault.map { if (it.id == folder.id) it.copy(data = f.copy(expanded = expanded)) else it })
        launchSafe { client.vaultSave(folder.copy(data = f.copy(expanded = expanded))) }
    }

    /** 设备的别名、标签、收藏、所在文件夹存在 DeviceMeta 里；还没有就新建一条。 */
    fun updateDeviceMeta(deviceId: String, folderId: String? = null, moveFolder: Boolean = false, block: (VaultData.DeviceMeta) -> VaultData.DeviceMeta) {
        val existing = _data.value.metaFor(deviceId)
        val meta = existing?.data as? VaultData.DeviceMeta ?: VaultData.DeviceMeta(deviceId, null, emptyList(), "", false)
        val entry = existing ?: VaultEntry("", null, 0, meta, System.currentTimeMillis())
        save(entry.copy(data = block(meta), folderId = if (moveFolder) folderId else entry.folderId))
    }

    fun toggleFavorite(item: HostItem) {
        when (item) {
            is HostItem.Device -> updateDeviceMeta(item.device.id) { it.copy(favorite = !it.favorite) }
            is HostItem.Ssh -> save(item.entry.copy(data = item.host.copy(favorite = !item.host.favorite)))
            is HostItem.Rdp -> save(item.entry.copy(data = item.host.copy(favorite = !item.host.favorite)))
        }
    }

    fun move(item: HostItem, folderId: String?) {
        when (item) {
            is HostItem.Device -> updateDeviceMeta(item.device.id, folderId, moveFolder = true) { it }
            is HostItem.Ssh -> save(item.entry.copy(folderId = folderId))
            is HostItem.Rdp -> save(item.entry.copy(folderId = folderId))
        }
    }

    fun moveFolder(folder: VaultEntry, parent: String?) = save(folder.copy(folderId = parent))
}

/** 列表条目 → AI 对话目标。RDP 主机暂不支持。 */
object AgentTargetOf {
    fun of(item: HostItem): cn.lemonit.galaxy.core.bridge.AgentTarget? = when (item) {
        is HostItem.Device -> cn.lemonit.galaxy.core.bridge.AgentTarget(cn.lemonit.galaxy.core.bridge.AgentTargetKind.DEVICE, item.device.id)
        is HostItem.Ssh -> cn.lemonit.galaxy.core.bridge.AgentTarget(cn.lemonit.galaxy.core.bridge.AgentTargetKind.SSH_HOST, item.entry.id)
        is HostItem.Rdp -> null
    }
}
