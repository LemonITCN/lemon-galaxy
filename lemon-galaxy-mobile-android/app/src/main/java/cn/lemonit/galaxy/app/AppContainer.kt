package cn.lemonit.galaxy.app

import android.content.Context
import cn.lemonit.galaxy.core.CoreProvider
import cn.lemonit.galaxy.core.bridge.ClientListener
import cn.lemonit.galaxy.core.bridge.ClientState
import cn.lemonit.galaxy.core.bridge.GalaxyClientInterface
import cn.lemonit.galaxy.core.bridge.GalaxyCore
import cn.lemonit.galaxy.core.bridge.SecureStore
import cn.lemonit.galaxy.core.platform.AndroidSecureStore
import cn.lemonit.galaxy.data.AppSettings
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.launch

/**
 * 应用级依赖：内核、账号、各功能的控制器。界面只从这里拿东西，不直接接触具体内核实现。
 */
class AppContainer(
    val appContext: Context?,
    val core: GalaxyCore,
    store: SecureStore,
    val settings: AppSettings,
) {
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    private val _messages = MutableSharedFlow<String>(extraBufferCapacity = 8)
    /** 一次性提示（Snackbar）。 */
    val messages: SharedFlow<String> = _messages.asSharedFlow()
    fun toast(text: String) {
        if (text.isNotEmpty()) _messages.tryEmit(text)
    }

    val prompts = SecretPromptBroker()

    private val listener = object : ClientListener {
        override fun onState(state: ClientState) {
            scope.launch { account.onClientState(state) }
        }

        override fun onDevicesChanged() {
            scope.launch { hosts.reload() }
        }

        override fun onVaultChanged() {
            scope.launch { hosts.reload() }
        }

        override fun onSessionRevoked() {
            scope.launch {
                connections.disconnectAll()
                terminals.closeAll()
                account.onSessionRevoked()
                appContext?.getString(cn.lemonit.galaxy.R.string.session_revoked)?.let(::toast)
            }
        }
    }

    val client: GalaxyClientInterface = core.newGalaxyClient(store, listener)
    val account = AccountController(client, settings, scope)
    val hosts = HostsController(client, scope, ::toast)
    val connections = ConnectionManager(client, prompts)
    val terminals = TerminalController(scope, connections, appContext) { hosts.data.value.targetName(it) }
    val transfers = TransferManager(appContext, scope, connections)
    val agents = AgentController(
        client, scope,
        configs = appContext?.let { cn.lemonit.galaxy.data.PrefsConversationConfigStore.create(it) } ?: MemoryConversationConfigStore(),
        resolveTargets = { names ->
            val d = hosts.data.value
            names.mapNotNull { n ->
                d.devices.firstOrNull { d.deviceName(it.id) == n || it.hostname == n }
                    ?.let { cn.lemonit.galaxy.core.bridge.AgentTarget(cn.lemonit.galaxy.core.bridge.AgentTargetKind.DEVICE, it.id) }
            }
        },
        defaultMode = { settings.prefs.value.defaultAiMode },
    )

    init {
        cn.lemonit.galaxy.core.platform.LanDiscovery.onDiscoveryChanged = { active -> runCatching { client.setLanDiscovery(active) } }
    }

    /** 界面语言变化时同步给内核（影响内核返回的错误文字）。 */
    fun setCoreLocale(zh: Boolean) {
        runCatching { core.coreSetLocale(if (zh) "zh-CN" else "en") }
    }

    fun onSignedIn() {
        hosts.reload()
        hosts.refresh()
        agents.loadModels()
        agents.refreshList()
    }

    fun signOut() {
        terminals.closeAll()
        connections.disconnectAll()
        account.logout()
    }

    companion object {
        fun create(context: Context): AppContainer {
            val app = context.applicationContext
            return AppContainer(app, CoreProvider.get(app), AndroidSecureStore(app), AppSettings.create(app))
        }
    }
}
