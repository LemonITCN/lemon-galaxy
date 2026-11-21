package cn.lemonit.galaxy.app

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

enum class Tab { HOSTS, AI, FILES, SETTINGS }

/** 主界面上压栈打开的页面。手机上全屏覆盖，平板横屏时显示在右侧详情栏（远程桌面始终全屏）。 */
sealed class Route {
    data class Terminal(val target: TargetRef?) : Route()
    data class Files(val target: TargetRef) : Route()
    data class FileEditor(val target: TargetRef, val path: String) : Route()
    data class Remote(val deviceId: String) : Route()
    data class Conversation(val id: String) : Route()
    data class NewConversation(val target: TargetRef?) : Route()
    data class EditEntry(val kind: EntryKind, val id: String?, val folderId: String?) : Route()
    data object Licenses : Route()

    val fullscreen: Boolean get() = this is Remote
}

enum class EntryKind { SSH, RDP, CREDENTIAL, SNIPPET, DEVICE }

class Navigator(initialTab: Tab = Tab.HOSTS) {
    var tab by mutableStateOf(initialTab)
    val stack = mutableStateListOf<Route>()
    val top: Route? get() = stack.lastOrNull()

    fun push(route: Route) {
        // 同一个页面不重复压栈
        if (stack.lastOrNull() == route) return
        stack.add(route)
    }

    fun pop(): Boolean {
        if (stack.isEmpty()) return false
        stack.removeAt(stack.lastIndex)
        return true
    }

    fun replace(route: Route) {
        if (stack.isNotEmpty()) stack.removeAt(stack.lastIndex)
        stack.add(route)
    }

    fun selectTab(t: Tab) {
        tab = t
        stack.clear()
    }

    fun reset() {
        stack.clear()
        tab = Tab.HOSTS
    }
}
