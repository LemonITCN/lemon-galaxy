<script setup lang="ts">
// 状态标记：圆点 + 文字，tone 决定颜色；dot=false 时不画圆点
withDefaults(defineProps<{ tone?: string; text: string; dot?: boolean }>(), { tone: 'neutral', dot: true })
</script>

<template>
  <span class="status" :class="'tone-' + tone" :title="text"><i v-if="dot" /><span class="t">{{ text }}</span></span>
</template>

<style scoped>
.status {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 22px;
  padding: 0 8px;
  border-radius: 999px;
  font-size: 12px;
  font-weight: 500;
  line-height: 1;
  white-space: nowrap;
  max-width: 100%;
  box-sizing: border-box;
  vertical-align: middle;
}
.t {
  overflow: hidden;
  text-overflow: ellipsis;
  line-height: 22px;
}
.status i {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
  flex: none;
}
.tone-success { color: var(--lg-success); background: var(--lg-success-bg); }
.tone-warning { color: var(--lg-warning); background: var(--lg-warning-bg); }
.tone-danger { color: var(--lg-danger); background: var(--lg-danger-bg); }
.tone-info { color: var(--lg-info); background: var(--lg-info-bg); }
.tone-neutral { color: var(--lg-neutral); background: var(--lg-neutral-bg); }
.tone-brand { color: var(--lg-brand-text); background: var(--lg-brand-bg); }
</style>
