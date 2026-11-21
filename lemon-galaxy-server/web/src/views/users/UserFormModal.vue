<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import * as api from '@/api'
import type { AiMode, User } from '@/api'
import InfoTip from '@/components/InfoTip.vue'
import PasswordField from '@/components/PasswordField.vue'
import { t } from '@/locales'
import { toast } from '@/utils/feedback'
import { validUsername } from '@/utils/format'
import { aiModeOptions } from '@/utils/labels'

// 新建 / 编辑用户
const open = defineModel<boolean>('open', { default: false })
const props = defineProps<{ user?: User | null }>()
const emit = defineEmits<{ saved: [user: User, password?: string] }>()

const editing = computed(() => !!props.user)
const form = reactive({
  username: '',
  display_name: '',
  initial_password: '',
  note: '',
  require_totp: false,
  ai_enabled: true,
  ai_max_mode: 'confirm_dangerous' as AiMode,
})
const errors = reactive({ username: '', password: '' })
const busy = ref(false)
const MIN = 8

watch(open, (v) => {
  if (!v) return
  errors.username = ''
  errors.password = ''
  const u = props.user
  Object.assign(form, {
    username: u?.username ?? '',
    display_name: u?.display_name ?? '',
    initial_password: '',
    note: u?.note ?? '',
    require_totp: u?.require_totp ?? false,
    ai_enabled: u?.ai_enabled ?? true,
    ai_max_mode: u?.ai_max_mode ?? 'confirm_dangerous',
  })
})

const modeOptions = computed(() => aiModeOptions())

async function submit() {
  errors.username = ''
  errors.password = ''
  if (!editing.value) {
    if (!validUsername(form.username.trim())) errors.username = t('validation.username')
    if (form.initial_password.length < MIN) errors.password = t('validation.passwordShort', { n: MIN })
    if (errors.username || errors.password) return
  }
  busy.value = true
  try {
    if (props.user) {
      const u = await api.updateUser(props.user.id, {
        display_name: form.display_name.trim(),
        note: form.note.trim(),
        require_totp: form.require_totp,
        ai_enabled: form.ai_enabled,
        ai_max_mode: form.ai_max_mode,
      })
      toast.success(t('common.saved'))
      open.value = false
      emit('saved', u)
    } else {
      const u = await api.createUser({
        username: form.username.trim(),
        display_name: form.display_name.trim() || undefined,
        initial_password: form.initial_password,
        note: form.note.trim() || undefined,
        require_totp: form.require_totp,
        ai_enabled: form.ai_enabled,
        ai_max_mode: form.ai_max_mode,
      })
      open.value = false
      emit('saved', u, form.initial_password)
    }
  } catch {
    /* 已统一提示 */
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <a-modal
    v-model:open="open"
    :title="editing ? t('users.form.editTitle') : t('users.form.createTitle')"
    :width="560"
    :ok-text="editing ? t('common.save') : t('common.create')"
    :confirm-loading="busy"
    destroy-on-hidden
    @ok="submit"
  >
    <a-form layout="vertical" class="form">
      <div class="form-grid">
        <a-form-item required :validate-status="errors.username ? 'error' : undefined" :help="errors.username || undefined">
          <template #label>
            {{ t('users.form.username') }}
            <InfoTip :text="t('users.form.usernameTip')" />
          </template>
          <a-input v-model:value="form.username" :disabled="editing" :maxlength="32" autocomplete="off" class="mono-input" />
        </a-form-item>
        <a-form-item :label="t('users.form.displayName')">
          <a-input v-model:value="form.display_name" :maxlength="64" :placeholder="t('common.optional')" />
        </a-form-item>
      </div>
      <a-form-item v-if="!editing" required :validate-status="errors.password ? 'error' : undefined" :help="errors.password || undefined">
        <template #label>
          {{ t('users.form.initialPassword') }}
          <InfoTip :text="t('users.form.initialPasswordTip')" />
        </template>
        <PasswordField v-model:value="form.initial_password" />
      </a-form-item>
      <a-form-item :label="t('users.form.note')">
        <a-textarea v-model:value="form.note" :auto-size="{ minRows: 2, maxRows: 4 }" :maxlength="500" :placeholder="t('common.optional')" />
      </a-form-item>
      <div class="form-grid">
        <a-form-item class="switch-cell">
          <template #label>
            {{ t('users.form.requireTotp') }}
            <InfoTip :text="t('users.form.requireTotpTip')" />
          </template>
          <a-switch v-model:checked="form.require_totp" />
        </a-form-item>
        <a-form-item :label="t('users.form.aiEnabled')" class="switch-cell">
          <a-switch v-model:checked="form.ai_enabled" />
        </a-form-item>
      </div>
      <a-form-item class="last">
        <template #label>
          {{ t('users.form.aiMaxMode') }}
          <InfoTip :text="t('users.form.aiMaxModeTip')" :width="360" />
        </template>
        <a-select v-model:value="form.ai_max_mode" :options="modeOptions" :disabled="!form.ai_enabled" style="width: 100%" />
      </a-form-item>
    </a-form>
  </a-modal>
</template>

<style scoped>
.form {
  margin-top: 8px;
}
.last {
  margin-bottom: 0;
}
.mono-input {
  font-family: var(--lg-mono);
}
</style>
