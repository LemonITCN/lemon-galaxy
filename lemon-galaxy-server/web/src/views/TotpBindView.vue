<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import * as api from '@/api'
import AuthShell from '@/components/AuthShell.vue'
import CopyText from '@/components/CopyText.vue'
import InfoTip from '@/components/InfoTip.vue'
import { t } from '@/locales'
import { clearSession, loadSession } from '@/stores/session'
import { toast } from '@/utils/feedback'

// 强制绑定两步验证：管理员第一次登录后必须完成
const router = useRouter()
const setup = ref<api.TotpSetupResult | null>(null)
const loadError = ref('')
const code = ref('')
const otpKey = ref(0)
const error = ref('')
const busy = ref(false)

async function load() {
  loadError.value = ''
  try {
    setup.value = await api.totpSetup()
  } catch (e) {
    loadError.value = api.errorText(e)
  }
}
onMounted(load)

/** 密钥每 4 位一组显示，复制时用原文 */
const secretGrouped = computed(() => (setup.value?.secret ?? '').replace(/(.{4})/g, '$1 ').trim())

async function submit() {
  if (code.value.length !== 6) return
  error.value = ''
  busy.value = true
  try {
    await api.totpVerify(code.value)
    await loadSession(true)
    toast.success(t('totpBind.done'))
    router.replace('/admin')
  } catch (e) {
    error.value = api.errorText(e)
    code.value = ''
    otpKey.value++
  } finally {
    busy.value = false
  }
}

function onComplete(v: string) {
  code.value = v
  if (v.length === 6) nextTick(submit)
}

async function switchAccount() {
  try {
    await api.logout()
  } catch {
    /* 忽略 */
  }
  clearSession()
  router.replace({ name: 'login' })
}
</script>

<template>
  <AuthShell :title="t('totpBind.title')" :subtitle="t('totpBind.subtitle')" :width="440">
    <a-result v-if="loadError" status="error" :title="loadError">
      <template #extra>
        <a-button @click="load">{{ t('common.retry') }}</a-button>
      </template>
    </a-result>
    <div v-else-if="!setup" class="loading"><a-spin /></div>
    <template v-else>
      <div class="step">
        <div class="step-title">
          <span class="num">1</span>{{ t('totpBind.scan') }}
          <InfoTip :text="t('totpBind.scanTip')" />
        </div>
        <div class="qr-row">
          <div class="qr">
            <a-qrcode :value="setup.otpauth_uri" :size="168" :bordered="false" color="#111418" bg-color="#FFFFFF" error-level="M" />
          </div>
        </div>
        <div class="secret-label">{{ t('totpBind.secret') }}</div>
        <CopyText :text="setup.secret" :display="secretGrouped" mono block />
      </div>
      <div class="step">
        <div class="step-title"><span class="num">2</span>{{ t('totpBind.code') }}</div>
        <a-input-otp :key="otpKey" v-model:value="code" :length="6" size="large" input-mode="numeric" @change="onComplete" />
        <a-alert v-if="error" type="error" show-icon :title="error" class="err" />
      </div>
      <a-button type="primary" block size="large" :loading="busy" :disabled="code.length !== 6" @click="submit">
        {{ t('totpBind.submit') }}
      </a-button>
    </template>
    <template #below>
      <a-button type="link" @click="switchAccount">{{ t('totpBind.switchAccount') }}</a-button>
    </template>
  </AuthShell>
</template>

<style scoped>
.loading {
  display: flex;
  justify-content: center;
  padding: 40px 0;
}
.step {
  margin-bottom: 22px;
}
.step-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-weight: 600;
  margin-bottom: 12px;
}
.num {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border-radius: 50%;
  background: var(--lg-brand-bg);
  color: var(--lg-brand-text);
  font-size: 12px;
  font-weight: 700;
}
.qr-row {
  display: flex;
  justify-content: center;
  margin-bottom: 14px;
}
.qr {
  padding: 10px;
  background: #fff;
  border-radius: 10px;
  border: 1px solid var(--lg-border-soft);
}
.secret-label {
  font-size: 12.5px;
  color: var(--lg-text-3);
  margin-bottom: 6px;
}
.step :deep(.ant-otp) {
  justify-content: space-between;
  width: 100%;
}
.err {
  margin-top: 12px;
}
</style>
