<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import * as api from '@/api'
import AuthShell from '@/components/AuthShell.vue'
import InfoTip from '@/components/InfoTip.vue'
import PasswordField from '@/components/PasswordField.vue'
import { t } from '@/locales'
import { session } from '@/stores/session'
import { validUsername } from '@/utils/format'

// 初始化：服务器还没有管理员时，用日志里打印的一次性令牌创建第一个管理员
const route = useRoute()
const router = useRouter()

const state = ref<'loading' | 'form' | 'done' | 'already'>('loading')
const tokenFromUrl = typeof route.query.token === 'string' ? route.query.token : ''
const form = reactive({ token: tokenFromUrl, username: 'admin', password: '', confirm: '' })
const error = ref('')
const busy = ref(false)
const MIN = 10

onMounted(async () => {
  try {
    const r = await api.setupStatus()
    state.value = r.needs_setup ? 'form' : 'already'
    session.needsSetup = r.needs_setup
  } catch (e) {
    state.value = 'form'
    error.value = api.errorText(e)
  }
})

const usernameBad = computed(() => !!form.username && !validUsername(form.username))

async function submit() {
  error.value = ''
  if (!form.token.trim()) return (error.value = t('validation.required', { field: t('setup.token') }))
  if (!validUsername(form.username)) return (error.value = t('validation.username'))
  if (form.password.length < MIN) return (error.value = t('validation.passwordShort', { n: MIN }))
  if (form.password !== form.confirm) return (error.value = t('validation.passwordMismatch'))
  busy.value = true
  try {
    await api.setup(form.token.trim(), form.username.trim(), form.password)
    session.needsSetup = false
    state.value = 'done'
  } catch (e) {
    error.value = api.errorText(e)
  } finally {
    busy.value = false
  }
}

function goLogin() {
  router.replace({ name: 'login' })
}
</script>

<template>
  <AuthShell :title="t('setup.title')" :subtitle="state === 'form' ? t('setup.subtitle') : undefined" :width="420">
    <div v-if="state === 'loading'" class="loading"><a-spin /></div>

    <a-result v-else-if="state === 'done'" status="success" :title="t('setup.doneTitle')" :sub-title="t('setup.doneDesc')" class="result">
      <template #extra>
        <a-button type="primary" block @click="goLogin">{{ t('setup.goLogin') }}</a-button>
      </template>
    </a-result>

    <a-result v-else-if="state === 'already'" status="info" :title="t('setup.alreadyTitle')" :sub-title="t('setup.alreadyDesc')" class="result">
      <template #extra>
        <a-button type="primary" block @click="goLogin">{{ t('setup.goLogin') }}</a-button>
      </template>
    </a-result>

    <a-form v-else layout="vertical" @submit.prevent="submit">
      <a-form-item v-if="!tokenFromUrl" required>
        <template #label>
          {{ t('setup.token') }}
          <InfoTip :text="t('setup.tokenTip')" />
        </template>
        <a-input v-model:value="form.token" class="mono-input" autocomplete="off" />
      </a-form-item>
      <a-form-item :label="t('setup.username')" required :validate-status="usernameBad ? 'error' : undefined" :help="usernameBad ? t('validation.username') : undefined">
        <a-input v-model:value="form.username" autocomplete="username" :maxlength="32" />
      </a-form-item>
      <a-form-item :label="t('setup.password')" required>
        <PasswordField v-model:value="form.password" />
      </a-form-item>
      <a-form-item :label="t('setup.confirm')" required>
        <a-input-password v-model:value="form.confirm" autocomplete="new-password" />
      </a-form-item>
      <a-alert v-if="error" type="error" show-icon :title="error" class="err" />
      <a-button type="primary" html-type="submit" block size="large" :loading="busy">{{ t('setup.submit') }}</a-button>
    </a-form>
  </AuthShell>
</template>

<style scoped>
.loading {
  display: flex;
  justify-content: center;
  padding: 32px 0;
}
.result {
  padding: 8px 0 0;
}
.err {
  margin-bottom: 16px;
}
.mono-input {
  font-family: var(--lg-mono);
}
</style>
