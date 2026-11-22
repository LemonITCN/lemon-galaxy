package cn.lemonit.galaxy

import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.appcompat.app.AppCompatActivity
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_WEAK
import androidx.biometric.BiometricManager.Authenticators.DEVICE_CREDENTIAL
import androidx.biometric.BiometricPrompt
import androidx.compose.runtime.mutableStateOf
import androidx.core.content.ContextCompat
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import cn.lemonit.galaxy.ui.app.GalaxyApp

/**
 * 唯一的 Activity。AppCompatActivity 是为了应用内切换语言（AppCompatDelegate）与 BiometricPrompt。
 * 应用锁：开启后冷启动和从后台回来都要解锁；AI 任务在后台时暂停（ai-agent.md 第 1 节）。
 */
class MainActivity : AppCompatActivity() {
    private val locked = mutableStateOf(false)
    /** 正在显示的系统验证框（自动弹出时避免重复；用户点「解锁」时先取消它再重新弹） */
    private var prompt: BiometricPrompt? = null

    private val processObserver = object : DefaultLifecycleObserver {
        override fun onStart(owner: LifecycleOwner) {
            container().agents.setForeground(true)
        }

        override fun onStop(owner: LifecycleOwner) {
            container().agents.setForeground(false)
            if (container().settings.prefs.value.appLock) locked.value = true
        }
    }

    private fun container() = (application as GalaxyApplication).container

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        if (savedInstanceState == null && container().settings.prefs.value.appLock) locked.value = true
        ProcessLifecycleOwner.get().lifecycle.addObserver(processObserver)
        setContent {
            GalaxyApp(container(), locked = locked.value, onUnlock = { unlock(userInitiated = true) })
        }
    }

    override fun onResume() {
        super.onResume()
        // 只在真正回到前台后自动弹验证：在后台或刚回来还没 resume 时调用，系统会悄悄丢掉这次请求且不回调
        if (locked.value && prompt == null) unlock(userInitiated = false)
    }

    override fun onDestroy() {
        // 应用退出：关掉经设备转发的 RDP 端口
        if (isFinishing) container().rdp.closeAll()
        ProcessLifecycleOwner.get().lifecycle.removeObserver(processObserver)
        super.onDestroy()
    }

    /**
     * 弹系统验证框。`userInitiated` 为真（用户点了「解锁」）时一定重新发起：先取消可能卡住的上一次。
     * 自动弹出只在 RESUMED 状态下进行。
     */
    private fun unlock(userInitiated: Boolean = true) {
        if (!locked.value) return
        if (!lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) return
        if (!userInitiated && prompt != null) return
        prompt?.let { runCatching { it.cancelAuthentication() } }
        val p = BiometricPrompt(
            this,
            ContextCompat.getMainExecutor(this),
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    prompt = null
                    locked.value = false
                }

                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    prompt = null
                }

                override fun onAuthenticationFailed() = Unit
            },
        )
        prompt = p
        val info = BiometricPrompt.PromptInfo.Builder()
            .setTitle(getString(R.string.lock_prompt))
            .setAllowedAuthenticators(BIOMETRIC_WEAK or DEVICE_CREDENTIAL)
            .build()
        try {
            p.authenticate(info)
        } catch (e: Exception) {
            // 设备没设锁屏密码等情况：不让应用卡死在锁屏上
            android.util.Log.w("LemonGalaxy", "应用锁验证无法启动", e)
            prompt = null
            locked.value = false
        }
    }
}
