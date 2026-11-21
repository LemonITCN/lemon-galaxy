<script setup lang="ts">
import { nextTick, onMounted, reactive, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { LockOutlined, UserOutlined } from '@antdv-next/icons'
import * as api from '@/api'
import AuthShell from '@/components/AuthShell.vue'
import { t } from '@/locales'
import { applyLogin, checkNeedsSetup } from '@/stores/session'

// 登录：用户名 + 密码；需要两步验证时进入第二步输入 6 位验证码
const route = useRoute()
const router = useRouter()

const step = ref<'password' | 'totp'>('password')
const form = reactive({ username: '', password: '', code: '' })
const error = ref('')
const busy = ref(false)
const otpKey = ref(0)

onMounted(async () => {
  if (await checkNeedsSetup()) router.replace({ name: 'setup' })
})

async function submit() {
  error.value = ''
  if (!form.username.trim() || !form.password) {
    error.value = t('validation.required', { field: !form.username.trim() ? t('login.username') : t('login.password') })
    return
  }
  busy.value = true
  try {
    const r = await api.login(form.username.trim(), form.password, step.value === 'totp' ? form.code : undefined)
    applyLogin(r)
    if (r.stage === 'totp_setup') {
      router.replace({ name: 'totp-setup' })
      return
    }
    const redirect = typeof route.query.redirect === 'string' && route.query.redirect.startsWith('/admin') ? route.query.redirect : '/admin'
    router.replace(redirect)
  } catch (e) {
    if (e instanceof api.ApiError && e.code === 'need_totp') {
      if (step.value === 'totp') error.value = e.message
      step.value = 'totp'
      form.code = ''
      otpKey.value++
    } else {
      error.value = api.errorText(e)
      if (step.value === 'totp') {
        form.code = ''
        otpKey.value++
      }
    }
  } finally {
    busy.value = false
  }
}

function onOtpComplete(v: string) {
  form.code = v
  if (v.length === 6) nextTick(submit)
}

function back() {
  step.value = 'password'
  form.code = ''
  error.value = ''
}
</script>

<template>
  <AuthShell
    :title="step === 'password' ? t('login.title') : t('login.totpTitle')"
    :subtitle="step === 'password' ? t('login.subtitle') : t('login.totpDesc')"
    :width="380"
  >
    <a-form v-if="step === 'password'" layout="vertical" @submit.prevent="submit">
      <a-form-item :label="t('login.username')">
        <a-input v-model:value="form.username" size="large" autocomplete="username" autofocus>
          <template #prefix><UserOutlined class="muted" /></template>
        </a-input>
      </a-form-item>
      <a-form-item :label="t('login.password')">
        <a-input-password v-model:value="form.password" size="large" autocomplete="current-password">
          <template #prefix><LockOutlined class="muted" /></template>
        </a-input-password>
      </a-form-item>
      <a-alert v-if="error" type="error" show-icon :title="error" class="err" />
      <a-button type="primary" html-type="submit" block size="large" :loading="busy">{{ t('login.submit') }}</a-button>
    </a-form>

    <div v-else class="totp">
      <a-input-otp :key="otpKey" v-model:value="form.code" :length="6" size="large" auto-focus input-mode="numeric" @change="onOtpComplete" />
      <a-alert v-if="error" type="error" show-icon :title="error" class="err top" />
      <a-button type="primary" block size="large" :loading="busy" :disabled="form.code.length !== 6" class="go" @click="submit">
        {{ t('login.totpSubmit') }}
      </a-button>
      <a-button type="link" block class="back" @click="back">{{ t('login.backToPassword') }}</a-button>
    </div>
  </AuthShell>
</template>

<style scoped>
.err {
  margin-bottom: 16px;
}
.err.top {
  margin: 16px 0 0;
}
.totp {
  display: flex;
  flex-direction: column;
  align-items: stretch;
}
.totp :deep(.ant-otp) {
  justify-content: space-between;
  width: 100%;
}
.go {
  margin-top: 20px;
}
.back {
  margin-top: 6px;
}
</style>
