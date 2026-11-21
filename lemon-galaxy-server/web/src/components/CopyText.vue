<script setup lang="ts">
import { ref } from 'vue'
import { CheckOutlined, CopyOutlined } from '@antdv-next/icons'
import { copyText } from '@/utils/format'
import { t } from '@/locales'

// 可复制的文本（地址、指纹、密钥、ID）
const props = defineProps<{ text: string; display?: string; mono?: boolean; block?: boolean; large?: boolean; wrap?: boolean }>()
const done = ref(false)
let timer: number | undefined

async function onCopy() {
  if (!props.text) return
  if (await copyText(props.text)) {
    done.value = true
    clearTimeout(timer)
    timer = window.setTimeout(() => (done.value = false), 1400)
  }
}
</script>

<template>
  <span class="copy" :class="{ mono, block, large, wrap }">
    <span class="text" :title="text">{{ display ?? text }}</span>
    <a-tooltip :title="done ? t('common.copied') : t('common.copy')">
      <button type="button" class="btn" :aria-label="t('common.copy')" @click.stop="onCopy">
        <CheckOutlined v-if="done" class="ok" />
        <CopyOutlined v-else />
      </button>
    </a-tooltip>
  </span>
</template>

<style scoped>
.copy {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  max-width: 100%;
  min-width: 0;
  vertical-align: middle;
}
.copy.block {
  display: flex;
  background: var(--lg-code-bg);
  border: 1px solid var(--lg-border-soft);
  border-radius: 8px;
  padding: 6px 6px 6px 12px;
}
.copy.block .text {
  flex: 1;
}
.text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}
.mono .text {
  font-family: var(--lg-mono);
  font-size: 12.5px;
}
.wrap .text {
  white-space: normal;
  word-break: break-all;
}
.large .text {
  font-size: 15px;
  font-weight: 550;
  letter-spacing: 0.01em;
}
.btn {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--lg-text-3);
  cursor: pointer;
}
.btn:hover {
  color: var(--lg-brand-text);
  background: var(--lg-hover);
}
.ok {
  color: var(--lg-success);
}
</style>
