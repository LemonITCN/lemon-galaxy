<script setup lang="ts">
import { ref, watch } from 'vue'
import * as api from '@/api'
import type { User } from '@/api'
import PasswordField from '@/components/PasswordField.vue'
import { t } from '@/locales'
import { toast } from '@/utils/feedback'

// 重置用户密码：醒目提示重置的后果，勾选「已了解」后才能确认
const open = defineModel<boolean>('open', { default: false })
const props = defineProps<{ user: User | null }>()
const emit = defineEmits<{ done: [user: User] }>()

const password = ref('')
const ack = ref(false)
const busy = ref(false)
const error = ref('')
const MIN = 8

watch(open, (v) => {
  if (v) {
    password.value = ''
    ack.value = false
    error.value = ''
  }
})

async function submit() {
  if (!props.user) return
  error.value = ''
  if (password.value.length < MIN) {
    error.value = t('validation.passwordShort', { n: MIN })
    return
  }
  busy.value = true
  try {
    const u = await api.resetUserPassword(props.user.id, password.value)
    toast.success(t('users.reset.done'))
    open.value = false
    emit('done', u)
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
    :title="t('users.reset.title') + (user ? ' · ' + user.username : '')"
    :width="480"
    :ok-text="t('users.reset.ok')"
    :ok-button-props="{ danger: true, disabled: !ack || !password }"
    :confirm-loading="busy"
    destroy-on-hidden
    @ok="submit"
  >
    <a-alert type="error" show-icon :title="t('users.reset.warnTitle')" :description="t('users.reset.warnDesc')" class="warn" />
    <a-form layout="vertical">
      <a-form-item :label="t('users.reset.newPassword')" required :validate-status="error ? 'error' : undefined" :help="error || undefined">
        <PasswordField v-model:value="password" />
      </a-form-item>
      <a-form-item class="last">
        <a-checkbox v-model:checked="ack">{{ t('users.reset.ack') }}</a-checkbox>
      </a-form-item>
    </a-form>
  </a-modal>
</template>

<style scoped>
.warn {
  margin: 8px 0 18px;
}
.last {
  margin-bottom: 0;
}
</style>
