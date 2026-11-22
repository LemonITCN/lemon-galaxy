package cn.lemonit.galaxy.ui.ai

import android.text.format.DateUtils
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.AddComment
import androidx.compose.material.icons.outlined.ChatBubbleOutline
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.PanTool
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.app.ConversationListItem
import cn.lemonit.galaxy.app.DateGroup
import cn.lemonit.galaxy.app.dateGroup
import cn.lemonit.galaxy.app.filterConversations
import cn.lemonit.galaxy.app.groupByDate
import cn.lemonit.galaxy.ui.components.Clock
import cn.lemonit.galaxy.ui.components.ConfirmDialog
import cn.lemonit.galaxy.ui.components.EmptyState
import cn.lemonit.galaxy.ui.components.GalaxyTopBar
import cn.lemonit.galaxy.ui.theme.Galaxy

/** AI 入口：对话列表（右上新对话、搜索、按今天 / 昨天 / 最近 7 天 / 更早分组，长按删除）。 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ConversationListScreen(
    items: List<ConversationListItem>,
    onOpen: (String) -> Unit,
    onNew: () -> Unit,
    onDelete: (String) -> Unit,
    selectedId: String? = null,
    initialQuery: String = "",
) {
    var query by rememberSaveable { mutableStateOf(initialQuery) }
    var deleting by remember { mutableStateOf<ConversationListItem?>(null) }
    val now = Clock.now()
    val groups = groupByDate(filterConversations(items, query), { it.updatedAt }, now)
    Scaffold(
        topBar = {
            GalaxyTopBar(
                stringResource(R.string.ai_title),
                actions = { IconButton(onClick = onNew) { Icon(Icons.Outlined.AddComment, stringResource(R.string.ai_new)) } },
            )
        },
    ) { pad ->
        Column(Modifier.padding(pad).fillMaxSize()) {
            TextField(
                query, { query = it },
                placeholder = { Text(stringResource(R.string.search)) },
                leadingIcon = { Icon(Icons.Rounded.Search, null) },
                trailingIcon = if (query.isNotEmpty()) ({ IconButton(onClick = { query = "" }) { Icon(Icons.Rounded.Close, stringResource(R.string.clear)) } }) else null,
                singleLine = true,
                shape = RoundedCornerShape(12.dp),
                colors = TextFieldDefaults.colors(
                    focusedIndicatorColor = Color.Transparent,
                    unfocusedIndicatorColor = Color.Transparent,
                    focusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                    unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                ),
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
            )
            when {
                items.isEmpty() -> EmptyState(
                    Icons.Outlined.ChatBubbleOutline, stringResource(R.string.ai_empty),
                    action = { Button(onClick = onNew) { Text(stringResource(R.string.ai_new)) } },
                )
                groups.isEmpty() -> EmptyState(Icons.Rounded.Search, stringResource(R.string.search_empty))
                else -> LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 12.dp)) {
                    groups.forEach { (g, list) ->
                        item("g-${g.name}") {
                            Text(
                                stringResource(dateGroupLabel(g)),
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(start = 24.dp, end = 24.dp, top = 14.dp, bottom = 4.dp),
                            )
                        }
                        items(list, key = { it.id }) { c ->
                            Row(
                                Modifier
                                    .fillMaxWidth()
                                    .padding(horizontal = 8.dp)
                                    .clip(RoundedCornerShape(12.dp))
                                    .background(if (c.id == selectedId) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent)
                                    .combinedClickable(onClick = { onOpen(c.id) }, onLongClick = { deleting = c })
                                    .heightIn(min = 52.dp)
                                    .padding(horizontal = 16.dp, vertical = 10.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Text(
                                    c.title.ifBlank { stringResource(R.string.ai_new) },
                                    style = MaterialTheme.typography.bodyLarge,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                    modifier = Modifier.weight(1f),
                                )
                                Spacer(Modifier.width(12.dp))
                                Text(
                                    rowTime(c.updatedAt, now),
                                    style = MaterialTheme.typography.labelMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                                when {
                                    c.waiting -> {
                                        Spacer(Modifier.width(10.dp))
                                        Icon(Icons.Rounded.PanTool, stringResource(R.string.ai_waiting), tint = Galaxy.colors.riskUnknown, modifier = Modifier.size(15.dp))
                                    }
                                    c.running -> {
                                        Spacer(Modifier.width(10.dp))
                                        CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 1.5.dp)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    deleting?.let { c ->
        ConfirmDialog(
            title = stringResource(R.string.ai_delete),
            message = stringResource(R.string.ai_delete_message),
            confirmText = stringResource(R.string.delete),
            destructive = true,
            onConfirm = { onDelete(c.id) },
            onDismiss = { deleting = null },
        )
    }
}

/** 行尾时间：今天、昨天显示时刻，更早显示日期。 */
@Composable
private fun rowTime(at: Long, now: Long): String {
    val context = LocalContext.current
    return when (dateGroup(at, now)) {
        DateGroup.TODAY, DateGroup.YESTERDAY -> DateUtils.formatDateTime(context, at, DateUtils.FORMAT_SHOW_TIME)
        else -> DateUtils.formatDateTime(context, at, DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_ABBREV_MONTH or DateUtils.FORMAT_NO_YEAR)
    }
}
