package cn.lemonit.galaxy.ui.hosts

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Code
import androidx.compose.material.icons.rounded.CreateNewFolder
import androidx.compose.material.icons.rounded.Dns
import androidx.compose.material.icons.rounded.ExpandMore
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material.icons.rounded.Link
import androidx.compose.material.icons.rounded.MoreVert
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Star
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.PrimaryTabRow
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.HostItem
import cn.lemonit.galaxy.app.HostTree
import cn.lemonit.galaxy.app.HostsData
import cn.lemonit.galaxy.app.TreeRow
import cn.lemonit.galaxy.core.bridge.VaultData
import cn.lemonit.galaxy.core.bridge.VaultEntry
import cn.lemonit.galaxy.ui.components.EmptyState
import cn.lemonit.galaxy.ui.components.EntryAvatar
import cn.lemonit.galaxy.ui.components.GalaxyTopBar
import cn.lemonit.galaxy.ui.components.InfoButton
import cn.lemonit.galaxy.ui.components.RdpIcon
import cn.lemonit.galaxy.ui.components.SshIcon
import cn.lemonit.galaxy.ui.components.Tag
import cn.lemonit.galaxy.ui.components.deviceKindIcon
import cn.lemonit.galaxy.ui.components.deviceStatusColor
import cn.lemonit.galaxy.ui.components.relativeTime
import cn.lemonit.galaxy.ui.theme.Galaxy

enum class HostsSubTab(val label: Int) { HOSTS(R.string.nav_hosts), CREDENTIALS(R.string.hosts_section_credentials), SNIPPETS(R.string.hosts_section_snippets) }

/** 列表页的本地筛选状态。 */
data class HostsFilter(
    val query: String = "",
    val tag: String? = null,
    val favoritesOnly: Boolean = false,
    val subTab: HostsSubTab = HostsSubTab.HOSTS,
)

/** 列表页对外的操作。 */
interface HostsActions {
    fun onFilter(f: HostsFilter)
    fun onRefresh()
    fun onItem(item: HostItem)
    fun onFolderToggle(folder: VaultEntry, expanded: Boolean)
    fun onFolderMenu(folder: VaultEntry, action: FolderAction)
    fun onAdd(kind: AddKind)
    fun onCredential(entry: VaultEntry)
    fun onSnippet(entry: VaultEntry)
}

enum class FolderAction { RENAME, NEW_SUBFOLDER, MOVE, DELETE }
enum class AddKind { FOLDER, SSH, RDP, CREDENTIAL, SNIPPET, ASSIST }

@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
fun HostsScreen(
    data: HostsData,
    filter: HostsFilter,
    refreshing: Boolean,
    actions: HostsActions,
    modifier: Modifier = Modifier,
    selectedKey: String? = null,
) {
    Scaffold(
        modifier = modifier,
        topBar = {
            GalaxyTopBar(
                title = stringResource(R.string.hosts_title),
                actions = {
                    InfoButton(stringResource(R.string.status_info))
                    IconButton(onClick = actions::onRefresh, enabled = !refreshing) {
                        Icon(Icons.Rounded.Refresh, stringResource(R.string.refresh))
                    }
                    AddMenu(actions::onAdd)
                },
            )
        },
        containerColor = MaterialTheme.colorScheme.surface,
    ) { pad ->
        Column(Modifier.padding(pad).fillMaxSize()) {
            SearchField(filter.query, { actions.onFilter(filter.copy(query = it)) }, stringResource(R.string.hosts_search))
            PrimaryTabRow(
                selectedTabIndex = filter.subTab.ordinal,
                containerColor = MaterialTheme.colorScheme.surface,
                divider = { HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant) },
            ) {
                HostsSubTab.entries.forEach { t ->
                    Tab(
                        selected = filter.subTab == t,
                        onClick = { actions.onFilter(filter.copy(subTab = t)) },
                        text = { Text(stringResource(t.label)) },
                        selectedContentColor = MaterialTheme.colorScheme.primary,
                        unselectedContentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            when (filter.subTab) {
                HostsSubTab.HOSTS -> HostsList(data, filter, actions, selectedKey)
                HostsSubTab.CREDENTIALS -> SimpleEntryList(
                    data.credentials.filter { matches(it, filter.query) }, Icons.Rounded.Key,
                    { (it.data as VaultData.Credential).label }, { (it.data as VaultData.Credential).username },
                    actions::onCredential,
                )
                HostsSubTab.SNIPPETS -> SimpleEntryList(
                    data.snippets.filter { matches(it, filter.query) }, Icons.Rounded.Code,
                    { (it.data as VaultData.Snippet).label }, { (it.data as VaultData.Snippet).command },
                    actions::onSnippet, monoSubtitle = true,
                )
            }
        }
    }
}

private fun matches(e: VaultEntry, q: String): Boolean {
    if (q.isBlank()) return true
    val text = when (val d = e.data) {
        is VaultData.Credential -> d.label + " " + d.username
        is VaultData.Snippet -> d.label + " " + d.command + " " + d.tags.joinToString(" ")
        else -> ""
    }.lowercase()
    return q.lowercase().split(' ').all { text.contains(it) }
}

@Composable
private fun AddMenu(onAdd: (AddKind) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) { Icon(Icons.Rounded.Add, stringResource(R.string.hosts_add)) }
        DropdownMenu(open, onDismissRequest = { open = false }) {
            val entries = listOf(
                Triple(AddKind.SSH, R.string.add_ssh, SshIcon),
                Triple(AddKind.RDP, R.string.add_rdp, RdpIcon),
                Triple(AddKind.FOLDER, R.string.add_folder, Icons.Rounded.CreateNewFolder),
                Triple(AddKind.CREDENTIAL, R.string.add_credential, Icons.Rounded.Key),
                Triple(AddKind.SNIPPET, R.string.add_snippet, Icons.Rounded.Code),
                Triple(AddKind.ASSIST, R.string.add_assist, Icons.Rounded.Link),
            )
            entries.forEach { (k, label, icon) ->
                DropdownMenuItem(
                    text = { Text(stringResource(label)) },
                    leadingIcon = { Icon(icon, null) },
                    onClick = {
                        open = false
                        onAdd(k)
                    },
                )
            }
        }
    }
}

@Composable
fun SearchField(value: String, onChange: (String) -> Unit, placeholder: String, modifier: Modifier = Modifier) {
    TextField(
        value = value,
        onValueChange = onChange,
        placeholder = { Text(placeholder, maxLines = 1) },
        leadingIcon = { Icon(Icons.Rounded.Search, null) },
        trailingIcon = if (value.isNotEmpty()) ({
            IconButton(onClick = { onChange("") }) { Icon(Icons.Rounded.Close, stringResource(R.string.clear)) }
        }) else null,
        singleLine = true,
        shape = RoundedCornerShape(12.dp),
        colors = TextFieldDefaults.colors(
            focusedIndicatorColor = Color.Transparent,
            unfocusedIndicatorColor = Color.Transparent,
            focusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
            unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
        ),
        modifier = modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).height(52.dp),
    )
}

@Composable
private fun HostsList(data: HostsData, filter: HostsFilter, actions: HostsActions, selectedKey: String?) {
    val rows = HostTree.rows(data, filter.query, filter.tag, filter.favoritesOnly)
    val tags = HostTree.allTags(data)
    LazyColumn(Modifier.fillMaxSize()) {
        item("chips") {
            Row(
                Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp, vertical = 6.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                FilterChip(
                    selected = !filter.favoritesOnly && filter.tag == null,
                    onClick = { actions.onFilter(filter.copy(favoritesOnly = false, tag = null)) },
                    label = { Text(stringResource(R.string.hosts_filter_all)) },
                )
                FilterChip(
                    selected = filter.favoritesOnly,
                    onClick = { actions.onFilter(filter.copy(favoritesOnly = !filter.favoritesOnly)) },
                    label = { Text(stringResource(R.string.hosts_filter_favorites)) },
                    leadingIcon = { Icon(Icons.Rounded.Star, null, Modifier.size(FilterChipDefaults.IconSize)) },
                )
                tags.forEach { t ->
                    FilterChip(
                        selected = filter.tag == t,
                        onClick = { actions.onFilter(filter.copy(tag = if (filter.tag == t) null else t)) },
                        label = { Text("#$t") },
                    )
                }
            }
        }
        if (rows.isEmpty()) {
            item("empty") {
                EmptyState(
                    Icons.Rounded.Dns,
                    stringResource(if (filter.query.isNotBlank() || filter.tag != null || filter.favoritesOnly) R.string.search_empty else R.string.hosts_empty),
                )
            }
        }
        items(rows, key = { it.key }) { row ->
            if (row.folder != null) {
                FolderRow(row, actions)
            } else {
                ItemRow(row.item!!, row.depth, row.path, selected = row.key == selectedKey) { actions.onItem(row.item) }
            }
        }
        item("bottom") { Spacer(Modifier.height(24.dp)) }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun FolderRow(row: TreeRow, actions: HostsActions) {
    val f = row.folder!!
    val folder = f.data as VaultData.Folder
    var menu by remember { mutableStateOf(false) }
    val tint = folder.color?.let { runCatching { Color(android.graphics.Color.parseColor(it)) }.getOrNull() } ?: MaterialTheme.colorScheme.onSurfaceVariant
    Row(
        Modifier
            .fillMaxWidth()
            .combinedClickable(onClick = { actions.onFolderToggle(f, !row.expanded) }, onLongClick = { menu = true })
            .padding(start = 8.dp + 20.dp * row.depth, end = 4.dp, top = 4.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            if (row.expanded) Icons.Rounded.ExpandMore else Icons.Rounded.ChevronRight,
            null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(20.dp),
        )
        Spacer(Modifier.width(6.dp))
        Icon(Icons.Rounded.Folder, null, tint = tint, modifier = Modifier.size(22.dp))
        Spacer(Modifier.width(12.dp))
        Text(folder.name, style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text(row.count.toString(), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Box {
            IconButton(onClick = { menu = true }) { Icon(Icons.Rounded.MoreVert, stringResource(R.string.more), tint = MaterialTheme.colorScheme.onSurfaceVariant) }
            DropdownMenu(menu, onDismissRequest = { menu = false }) {
                listOf(
                    FolderAction.RENAME to R.string.rename,
                    FolderAction.NEW_SUBFOLDER to R.string.folder_new,
                    FolderAction.MOVE to R.string.action_move,
                    FolderAction.DELETE to R.string.delete,
                ).forEach { (a, label) ->
                    DropdownMenuItem(text = { Text(stringResource(label)) }, onClick = {
                        menu = false
                        actions.onFolderMenu(f, a)
                    })
                }
            }
        }
    }
}

/** 条目行（设备、SSH、RDP）。 */
@Composable
fun ItemRow(item: HostItem, depth: Int, path: String?, selected: Boolean = false, onClick: () -> Unit) {
    val bg = if (selected) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent
    Row(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 8.dp, vertical = 1.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(bg)
            .clickable(onClick = onClick)
            .padding(start = 8.dp + 20.dp * depth, end = 8.dp, top = 8.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val (icon, status) = avatarFor(item)
        EntryAvatar(icon, status = status)
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(item.title, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                if (item.favorite) {
                    Spacer(Modifier.width(4.dp))
                    Icon(Icons.Rounded.Star, null, tint = Galaxy.colors.warning, modifier = Modifier.size(14.dp))
                }
            }
            Text(
                subtitleFor(item) + (path?.let { "  ·  $it" } ?: ""),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                fontFamily = if (item is HostItem.Device) null else FontFamily.Monospace,
            )
        }
        if (item.tags.isNotEmpty()) {
            Spacer(Modifier.width(8.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                item.tags.take(2).forEach { Tag(it) }
            }
        }
    }
}

@Composable
fun avatarFor(item: HostItem): Pair<ImageVector, Color?> = when (item) {
    is HostItem.Device -> deviceKindIcon(item.device.kind, item.device.platform) to deviceStatusColor(item.device)
    is HostItem.Ssh -> SshIcon to null
    is HostItem.Rdp -> RdpIcon to null
}

@Composable
fun subtitleFor(item: HostItem): String = when (item) {
    is HostItem.Device -> {
        val d = item.device
        val status = when {
            d.online -> stringResource(R.string.status_online)
            d.lanSeen -> stringResource(R.string.status_lan)
            else -> stringResource(R.string.status_last_seen, relativeTime(d.lastSeenAt))
        }
        "${d.osVersion}  ·  $status"
    }
    is HostItem.Ssh -> item.address
    is HostItem.Rdp -> "RDP  ·  " + item.address
}

@Composable
private fun SimpleEntryList(
    entries: List<VaultEntry>,
    icon: ImageVector,
    title: (VaultEntry) -> String,
    subtitle: (VaultEntry) -> String,
    onClick: (VaultEntry) -> Unit,
    monoSubtitle: Boolean = false,
) {
    if (entries.isEmpty()) {
        EmptyState(icon, stringResource(R.string.search_empty))
        return
    }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = androidx.compose.foundation.layout.PaddingValues(vertical = 8.dp)) {
        items(entries, key = { it.id }) { e ->
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 8.dp, vertical = 1.dp)
                    .clip(RoundedCornerShape(12.dp))
                    .clickable { onClick(e) }
                    .padding(horizontal = 8.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                EntryAvatar(icon)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(title(e), style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(
                        subtitle(e),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        fontFamily = if (monoSubtitle) FontFamily.Monospace else null,
                    )
                }
            }
        }
    }
}
