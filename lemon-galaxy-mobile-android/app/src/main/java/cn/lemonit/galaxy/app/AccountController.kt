package cn.lemonit.galaxy.app

import cn.lemonit.galaxy.core.bridge.ClientState
import cn.lemonit.galaxy.core.bridge.GalaxyClientInterface
import cn.lemonit.galaxy.core.bridge.GalaxyClientProposedInterface
import cn.lemonit.galaxy.core.bridge.LoginResult
import cn.lemonit.galaxy.core.bridge.ServerInfo
import cn.lemonit.galaxy.core.bridge.TotpSetup
import cn.lemonit.galaxy.core.bridge.UserInfo
import cn.lemonit.galaxy.data.AppSettings
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** 登录流程的步骤。 */
enum class LoginStep { CREDENTIALS, TOTP, CHANGE_PASSWORD }

data class LoginUiState(
    val server: String = "",
    val username: String = "",
    val password: String = "",
    val totp: String = "",
    val newPassword: String = "",
    val confirmPassword: String = "",
    val step: LoginStep = LoginStep.CREDENTIALS,
    val busy: Boolean = false,
    val error: String? = null,
)

/** 账号会话：启动恢复、登录三步、改密码、两步验证、退出。 */
class AccountController(
    private val client: GalaxyClientInterface,
    private val settings: AppSettings,
    private val scope: CoroutineScope,
) {
    private val _clientState = MutableStateFlow(client.state())
    val clientState: StateFlow<ClientState> = _clientState.asStateFlow()

    private val _user = MutableStateFlow<UserInfo?>(null)
    val user: StateFlow<UserInfo?> = _user.asStateFlow()
    private val _server = MutableStateFlow<ServerInfo?>(null)
    val server: StateFlow<ServerInfo?> = _server.asStateFlow()

    /** 已进入主界面（登录成功或恢复成功）。 */
    private val _signedIn = MutableStateFlow(false)
    val signedIn: StateFlow<Boolean> = _signedIn.asStateFlow()

    private val _login = MutableStateFlow(
        LoginUiState(server = settings.prefs.value.lastServer, username = settings.prefs.value.lastUsername),
    )
    val login: StateFlow<LoginUiState> = _login.asStateFlow()

    private val _totpEnabled = MutableStateFlow<Boolean?>(null)
    val totpEnabled: StateFlow<Boolean?> = _totpEnabled.asStateFlow()

    val proposed: GalaxyClientProposedInterface? get() = client as? GalaxyClientProposedInterface

    fun onClientState(state: ClientState) {
        _clientState.value = state
    }

    fun onSessionRevoked() {
        _signedIn.value = false
        _login.update { it.copy(step = LoginStep.CREDENTIALS, password = "", busy = false) }
    }

    /** 启动时用本地保存的会话恢复登录。 */
    fun restore(): Boolean {
        val ok = runCatching { client.restore() }.getOrDefault(false)
        if (ok) enterMain()
        return ok
    }

    private fun enterMain() {
        _user.value = client.currentUser()
        _server.value = client.server()
        _signedIn.value = true
        refreshTotp()
    }

    fun edit(block: (LoginUiState) -> LoginUiState) = _login.update { block(it).copy(error = null) }

    fun submitCredentials() = run {
        val s = _login.value
        when (client.login(s.server.trim(), s.username.trim(), s.password, null)) {
            LoginResult.SUCCESS -> success()
            LoginResult.NEED_TOTP -> _login.update { it.copy(step = LoginStep.TOTP, totp = "") }
            LoginResult.MUST_CHANGE_PASSWORD -> _login.update { it.copy(step = LoginStep.CHANGE_PASSWORD) }
        }
    }

    fun submitTotp() = run {
        val s = _login.value
        when (client.login(s.server.trim(), s.username.trim(), s.password, s.totp.trim())) {
            LoginResult.SUCCESS -> success()
            LoginResult.MUST_CHANGE_PASSWORD -> _login.update { it.copy(step = LoginStep.CHANGE_PASSWORD) }
            LoginResult.NEED_TOTP -> _login.update { it.copy(totp = "") }
        }
    }

    fun submitNewPassword() = run {
        client.completeFirstLogin(_login.value.newPassword)
        success()
    }

    fun backToCredentials() = _login.update { it.copy(step = LoginStep.CREDENTIALS, totp = "", newPassword = "", confirmPassword = "", error = null) }

    private fun success() {
        val s = _login.value
        settings.update { it.copy(lastServer = s.server.trim(), lastUsername = s.username.trim()) }
        _login.update { it.copy(password = "", totp = "", newPassword = "", confirmPassword = "", step = LoginStep.CREDENTIALS) }
        enterMain()
    }

    private fun run(block: suspend () -> Unit) {
        if (_login.value.busy) return
        _login.update { it.copy(busy = true, error = null) }
        scope.launch {
            try {
                block()
                _login.update { it.copy(busy = false) }
            } catch (e: Exception) {
                _login.update { it.copy(busy = false, error = e.userMessage()) }
            }
        }
    }

    fun logout(onDone: () -> Unit = {}) {
        scope.launch {
            runCatching { client.logout() }
            _signedIn.value = false
            _user.value = null
            onDone()
        }
    }

    suspend fun changePassword(old: String, new: String) = client.changePassword(old, new)

    fun refreshTotp() {
        val p = proposed ?: return
        scope.launch { _totpEnabled.value = runCatching { p.totpEnabled() }.getOrNull() }
    }

    suspend fun totpSetup(): TotpSetup = proposed!!.totpSetup()
    suspend fun totpVerify(code: String) {
        proposed!!.totpVerify(code)
        _totpEnabled.value = true
    }

    suspend fun totpDisable(code: String) {
        proposed!!.totpDisable(code)
        _totpEnabled.value = false
    }

    companion object {
        /** 服务器地址格式：host:port#sha256-指纹（host 可以是域名、IPv4 或 [IPv6]）。 */
        private val SERVER_RE = Regex("^(\\[[0-9A-Fa-f:.]+]|[A-Za-z0-9.-]+):(\\d{1,5})#sha256-[A-Za-z0-9+/_=-]{8,}$")

        fun isValidServer(s: String): Boolean {
            val m = SERVER_RE.matchEntire(s.trim()) ?: return false
            val port = m.groupValues[2].toIntOrNull() ?: return false
            return port in 1..65535
        }

        /** 扫码结果：允许直接是地址，或 lemongalaxy://login?server=<地址> 形式。 */
        fun parseScanned(raw: String): String? {
            val t = raw.trim()
            if (isValidServer(t)) return t
            val q = runCatching { android.net.Uri.parse(t) }.getOrNull() ?: return null
            if (q.scheme == "lemongalaxy") {
                val s = q.getQueryParameter("server") ?: return null
                return s.takeIf { isValidServer(it) }
            }
            return null
        }
    }
}
