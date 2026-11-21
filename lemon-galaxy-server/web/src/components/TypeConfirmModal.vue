<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { t } from '@/locales'

// 需要输入指定文字才能确认的危险操作（删除用户等）
const props = defineProps<{ title: string; description?: string; expected: string; hint: string; okText?: string; busy?: boolean }>()
const open = defineModel<boolean>('open', { default: false })
const emit = defineEmits<{ confirm: [] }>()
const typed = ref('')
watch(open, (v) => {
  if (v) typed.value = ''
})
const match = computed(() => typed.value.trim() === props.expected)
function ok() {
  if (match.value) emit('confirm')
}
</script>

<template>
  <a-modal
    v-model:open="open"
    :title="title"
    :width="440"
    :ok-text="okText ?? t('common.delete')"
    :ok-button-props="{ danger: true, disabled: !match }"
    :confirm-loading="busy"
    destroy-on-hidden
    @ok="ok"
  >
    <a-alert v-if="description" type="error" show-icon :title="description" class="alert" />
    <div class="hint">{{ hint }}</div>
    <a-input v-model:value="typed" class="mono-input" autocomplete="off" @press-enter="ok" />
  </a-modal>
</template>

<style scoped>
.alert {
  margin-bottom: 14px;
}
.hint {
  margin-bottom: 8px;
  color: var(--lg-text-2);
  font-size: 13px;
}
.mono-input :deep(input),
.mono-input {
  font-family: var(--lg-mono);
}
</style>
