<script setup lang="ts">
import { ref } from 'vue'
import { CopyOutlined, ThunderboltOutlined } from '@antdv-next/icons'
import { copyText, generatePassword } from '@/utils/format'
import { toast } from '@/utils/feedback'
import { t } from '@/locales'

// 密码输入框：可一键生成随机强密码（生成后自动显示明文并复制）
const value = defineModel<string>('value', { default: '' })
defineProps<{ placeholder?: string; autocomplete?: string; status?: '' | 'error' | 'warning' }>()
const visible = ref(false)

async function generate() {
  value.value = generatePassword(16)
  visible.value = true
  if (await copyText(value.value)) toast.success(t('password.generated'))
}

async function copy() {
  if (value.value && (await copyText(value.value))) toast.success(t('common.copied'))
}
</script>

<template>
  <a-space-compact block>
    <a-input-password
      v-model:value="value"
      v-model:icon-visible="visible"
      :placeholder="placeholder"
      :autocomplete="autocomplete ?? 'new-password'"
      :status="status"
      class="pw"
      style="width: 100%"
    />
    <a-tooltip :title="t('password.generate')">
      <a-button :aria-label="t('password.generate')" @click="generate"><ThunderboltOutlined /></a-button>
    </a-tooltip>
    <a-tooltip :title="t('common.copy')">
      <a-button :aria-label="t('common.copy')" :disabled="!value" @click="copy"><CopyOutlined /></a-button>
    </a-tooltip>
  </a-space-compact>
</template>

<style scoped>
.pw :deep(input) {
  font-family: var(--lg-mono);
}
</style>
