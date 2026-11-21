<script setup lang="ts">
import { InfoCircleOutlined } from '@antdv-next/icons'

// 说明文字收进小 i 图标，点开看（界面上不铺说明小字）
withDefaults(defineProps<{ text?: string; title?: string; placement?: string; width?: number }>(), {
  placement: 'bottomLeft',
  width: 320,
})
</script>

<template>
  <a-popover trigger="click" :placement="placement as any" :title="title" :styles="{ container: { maxWidth: width + 'px' } }">
    <template #content>
      <div class="info-tip"><slot>{{ text }}</slot></div>
    </template>
    <button type="button" class="info-btn" aria-label="info" @click.stop>
      <InfoCircleOutlined />
    </button>
  </a-popover>
</template>

<style scoped>
.info-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border: 0;
  padding: 0;
  margin: 0 2px;
  border-radius: 5px;
  background: transparent;
  color: var(--lg-text-3);
  cursor: pointer;
  font-size: 13px;
  vertical-align: -3px;
}
.info-btn:hover {
  color: var(--lg-brand-text);
  background: var(--lg-hover);
}
.info-tip {
  line-height: 1.65;
  font-size: 13px;
  color: var(--lg-text-2);
  white-space: pre-line;
}
</style>
