<script setup lang="ts">
import { ref } from 'vue'
import { CloseOutlined, PaperClipOutlined } from '@antdv-next/icons'
import { bytes } from '@/utils/format'

// 选一个本地文件（不自动上传）
const file = defineModel<File | null>('file', { default: null })
defineProps<{ accept?: string; buttonText: string }>()
const input = ref<HTMLInputElement>()

function onChange(e: Event) {
  const f = (e.target as HTMLInputElement).files?.[0]
  if (f) file.value = f
  if (input.value) input.value.value = ''
}
</script>

<template>
  <div class="pick">
    <input ref="input" type="file" class="hidden" :accept="accept" @change="onChange" />
    <a-button @click="input?.click()">
      <template #icon><PaperClipOutlined /></template>
      {{ buttonText }}
    </a-button>
    <div v-if="file" class="chosen">
      <span class="name" :title="file.name">{{ file.name }}</span>
      <span class="size">{{ bytes(file.size) }}</span>
      <button type="button" class="x" aria-label="remove" @click="file = null"><CloseOutlined /></button>
    </div>
  </div>
</template>

<style scoped>
.pick {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}
.hidden {
  display: none;
}
.chosen {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  flex: 1;
  font-size: 13px;
}
.name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--lg-mono);
  font-size: 12.5px;
}
.size {
  flex: none;
  color: var(--lg-text-3);
}
.x {
  flex: none;
  border: 0;
  background: transparent;
  color: var(--lg-text-3);
  cursor: pointer;
  padding: 2px;
  border-radius: 4px;
}
.x:hover {
  color: var(--lg-danger);
  background: var(--lg-hover);
}
</style>
