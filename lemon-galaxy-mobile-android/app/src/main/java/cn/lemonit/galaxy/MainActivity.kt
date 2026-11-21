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
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import cn.lemonit.galaxy.ui.app.GalaxyApp

/**
 * 唯一的 Activity。AppCompatActivity 是为了应用内切换语言（AppCompatDelegate）与 BiometricPrompt。
 * 应用锁：开启后冷启动和从后台回来都要解锁；AI 任务在后台时暂停（ai-agent.md 第 1 节）。
 */
class MainActivity : AppCompatActivity() {
    private val locked = mutableStateOf(false)
    private var authenticating = false

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
            GalaxyApp(container(), locked = locked.value, onUnlock = ::unlock)
        }
    }

    override fun onDestroy() {
        ProcessLifecycleOwner.get().lifecycle.removeObserver(processObserver)
        super.onDestroy()
    }

    private fun unlock() {
        if (authenticating || !locked.value) return
        authenticating = true
        val prompt = BiometricPrompt(
            this,
            ContextCompat.getMainExecutor(this),
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    authenticating = false
                    locked.value = false
                }

                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    authenticating = false
                }

                override fun onAuthenticationFailed() = Unit
            },
        )
        val info = BiometricPrompt.PromptInfo.Builder()
            .setTitle(getString(R.string.lock_prompt))
            .setAllowedAuthenticators(BIOMETRIC_WEAK or DEVICE_CREDENTIAL)
            .build()
        prompt.authenticate(info)
    }
}
