<script lang="ts">
export interface BarRow {
  key: string
  label: string
  value: number
  sub?: string
}
</script>

<script setup lang="ts">
// 横向条形排行（用量按用户 / 模型分组时用）
import { computed } from 'vue'

const props = withDefaults(defineProps<{ rows: BarRow[]; format?: (n: number) => string; color?: string }>(), {
  format: (n: number) => String(n),
  color: 'var(--lg-chart-1)',
})
const max = computed(() => Math.max(1, ...props.rows.map((r) => r.value)))
</script>

<template>
  <div class="hbars">
    <div v-for="r in rows" :key="r.key" class="row">
      <div class="label" :title="r.label">
        <span class="name">{{ r.label }}</span>
        <span v-if="r.sub" class="sub">{{ r.sub }}</span>
      </div>
      <div class="track">
        <div class="bar" :style="{ width: (r.value / max) * 100 + '%', background: color }" />
      </div>
      <div class="value">{{ format(r.value) }}</div>
    </div>
  </div>
</template>

<style scoped>
.hbars {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.row {
  display: grid;
  grid-template-columns: minmax(120px, 220px) 1fr 84px;
  align-items: center;
  gap: 12px;
}
.label {
  display: flex;
  align-items: baseline;
  gap: 6px;
  min-width: 0;
}
.name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
}
.sub {
  flex: none;
  font-size: 11.5px;
  color: var(--lg-text-3);
}
.track {
  height: 10px;
  border-radius: 5px;
  background: var(--lg-neutral-bg);
  overflow: hidden;
}
.bar {
  height: 100%;
  border-radius: 5px;
  min-width: 2px;
}
.value {
  text-align: right;
  font-size: 13px;
  font-variant-numeric: tabular-nums;
  color: var(--lg-text-2);
}
</style>
