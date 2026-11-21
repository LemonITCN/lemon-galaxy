<script setup lang="ts">
import { reactive, ref, watch } from 'vue'
import * as api from '@/api'
import { t } from '@/locales'
import { toast } from '@/utils/feedback'
import PasswordField from '@/components/PasswordField.vue'

// 管理员修改自己的密码
const open = defineModel<boolean>('open', { default: false })
const form = reactive({ old: '', next: '', confirm: '' })
const error = ref('')
const busy = ref(false)
const MIN = 8

watch(open, (v) => {
  if (v) {
    Object.assign(form, { old: '', next: '', confirm: '' })
    error.value = ''
  }
})

async function submit() {
  error.value = ''
  if (!form.old || !form.next) {
    error.value = t('validation.required', { field: !form.old ? t('password.old') : t('password.new') })
    return
  }
  if (form.next.length < MIN) {
    error.value = t('validation.passwordShort', { n: MIN })
    return
  }
  if (form.next !== form.confirm) {
    error.value = t('validation.passwordMismatch')
    return
  }
  busy.value = true
  try {
    await api.changePassword(form.old, form.next)
    toast.success(t('password.changed'))
    open.value = false
  } catch (e) {
    error.value = api.errorText(e)
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <a-modal v-model:open="open" :title="t('password.title')" :width="420" :confirm-loading="busy" :ok-text="t('common.save')" destroy-on-hidden @ok="submit">
    <a-form layout="vertical" class="form" @submit.prevent="submit">
      <a-form-item :label="t('password.old')" required>
        <a-input-password v-model:value="form.old" autocomplete="current-password" />
      </a-form-item>
      <a-form-item :label="t('password.new')" required>
        <PasswordField v-model:value="form.next" />
      </a-form-item>
      <a-form-item :label="t('password.confirm')" required class="last">
        <a-input-password v-model:value="form.confirm" autocomplete="new-password" />
      </a-form-item>
    </a-form>
    <a-alert v-if="error" type="error" show-icon :title="error" class="err" />
  </a-modal>
</template>

<style scoped>
.form {
  margin-top: 8px;
}
.last {
  margin-bottom: 0;
}
.err {
  margin-top: 14px;
}
</style>
