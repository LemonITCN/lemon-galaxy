<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import type { Dayjs } from 'dayjs'
import * as api from '@/api'
import type { UsageGroupBy, UsageRow } from '@/api'
import HBarList from '@/components/HBarList.vue'
import type { BarRow } from '@/components/HBarList.vue'
import TrendChart from '@/components/TrendChart.vue'
import type { ChartPoint } from '@/components/TrendChart.vue'
import UserSelect from '@/components/UserSelect.vue'
import { t } from '@/locales'
import { compact, dayjs, num, rangePresets } from '@/utils/format'

const rows = ref<UsageRow[]>([])
const loading = ref(false)
const q = reactive({
  range: [dayjs().subtract(29, 'day').startOf('day'), dayjs().endOf('day')] as [Dayjs, Dayjs],
  userId: undefined as number | undefined,
  groupBy: 'day' as UsageGroupBy,
})

async function load() {
  loading.value = true
  try {
    const r = await api.aiUsage({
      from: q.range?.[0]?.startOf('day').valueOf(),
      to: q.range?.[1]?.endOf('day').valueOf(),
      user_id: q.userId,
      group_by: q.groupBy,
    })
    rows.value = r.items
  } catch {
    rows.value = []
  } finally {
    loading.value = false
  }
}
onMounted(load)
watch(() => [q.range, q.userId, q.groupBy], load)

const presets = computed(() =>
  rangePresets({ today: t('aiUsage.today'), last7: t('aiUsage.last7'), last30: t('aiUsage.last30'), thisMonth: t('aiUsage.thisMonth') }),
)

const groupOptions = computed(() => [
  { value: 'day', label: t('aiUsage.byDay') },
  { value: 'user', label: t('aiUsage.byUser') },
  { value: 'model', label: t('aiUsage.byModel') },
])

const totals = computed(() =>
  rows.value.reduce(
    (s, r) => ({
      requests: s.requests + r.requests,
      prompt: s.prompt + r.prompt_tokens,
      completion: s.completion + r.completion_tokens,
      total: s.total + r.total_tokens,
    }),
    { requests: 0, prompt: 0, completion: 0, total: 0 },
  ),
)

const tiles = computed(() => [
  { key: 'req', label: t('aiUsage.requests'), value: num(totals.value.requests) },
  { key: 'in', label: t('aiUsage.promptTokens'), value: compact(totals.value.prompt), full: num(totals.value.prompt) },
  { key: 'out', label: t('aiUsage.completionTokens'), value: compact(totals.value.completion), full: num(totals.value.completion) },
  { key: 'all', label: t('aiUsage.totalTokens'), value: compact(totals.value.total), full: num(totals.value.total) },
])

/** 按日期时补齐没有用量的日子，图表才连续 */
const dayPoints = computed<ChartPoint[]>(() => {
  const map = new Map(rows.value.map((r) => [r.key, r.total_tokens]))
  const [from, to] = q.range ?? []
  if (!from || !to || to.diff(from, 'day') > 120) {
    return [...rows.value].sort((a, b) => a.key.localeCompare(b.key)).map((r) => ({ label: dayjs(r.key).format('MM-DD'), title: r.key, value: r.total_tokens }))
  }
  const out: ChartPoint[] = []
  for (let d = from.startOf('day'); !d.isAfter(to, 'day'); d = d.add(1, 'day')) {
    const key = d.format('YYYY-MM-DD')
    out.push({ label: d.format('MM-DD'), title: key, value: map.get(key) ?? 0 })
  }
  return out
})

const TOP = 12
const barRows = computed<BarRow[]>(() =>
  [...rows.value]
    .sort((a, b) => b.total_tokens - a.total_tokens)
    .slice(0, TOP)
    .map((r) => ({ key: r.key, label: r.key, value: r.total_tokens, sub: t('aiUsage.requests') + ' ' + num(r.requests) })),
)

const tableRows = computed(() =>
  q.groupBy === 'day' ? [...rows.value].sort((a, b) => b.key.localeCompare(a.key)) : [...rows.value].sort((a, b) => b.total_tokens - a.total_tokens),
)

const columns = computed(() => [
  {
    title: q.groupBy === 'day' ? t('aiUsage.byDay') : q.groupBy === 'user' ? t('aiUsage.byUser') : t('aiUsage.byModel'),
    key: 'key',
  },
  { title: t('aiUsage.requests'), key: 'requests', width: 120, align: 'right' as const },
  { title: t('aiUsage.promptTokens'), key: 'prompt', width: 140, align: 'right' as const },
  { title: t('aiUsage.completionTokens'), key: 'completion', width: 140, align: 'right' as const },
  { title: t('aiUsage.totalTokens'), key: 'total', width: 140, align: 'right' as const },
  { title: t('aiUsage.share'), key: 'share', width: 170 },
])

function share(r: UsageRow) {
  return totals.value.total ? (r.total_tokens / totals.value.total) * 100 : 0
}
</script>

<template>
  <div class="page">
    <div class="page-head">
      <h1>{{ t('aiUsage.title') }}</h1>
    </div>

    <div class="panel filters">
      <div class="toolbar">
        <a-range-picker v-model:value="q.range" :presets="presets" :allow-clear="false" />
        <UserSelect v-model:value="q.userId" :placeholder="t('aiUsage.allUsers')" :width="200" />
        <span class="spacer" />
        <a-segmented v-model:value="q.groupBy" :options="groupOptions" />
      </div>
      <div class="tiles">
        <div v-for="tile in tiles" :key="tile.key" class="tile">
          <div class="tile-label">{{ tile.label }}</div>
          <a-tooltip :title="tile.full">
            <div class="tile-value">{{ tile.value }}</div>
          </a-tooltip>
        </div>
      </div>
    </div>

    <section class="panel">
      <div class="panel-head">
        {{ t('aiUsage.chartTitle') }}
        <span v-if="q.groupBy !== 'day' && rows.length > TOP" class="muted top">{{ t('aiUsage.topN', { n: TOP }) }}</span>
      </div>
      <div class="panel-body">
        <a-spin :spinning="loading">
          <a-empty v-if="!rows.length && !loading" :description="t('aiUsage.empty')" class="empty" />
          <TrendChart v-else-if="q.groupBy === 'day'" :points="dayPoints" type="bar" :format="compact" :height="220" />
          <HBarList v-else :rows="barRows" :format="compact" />
        </a-spin>
      </div>
    </section>

    <section class="panel">
      <a-table :columns="columns" :data-source="tableRows" row-key="key" :loading="loading" :pagination="{ pageSize: 50, hideOnSinglePage: true }" :scroll="{ x: 900 }">
        <template #bodyCell="{ column, record }">
          <template v-if="column.key === 'key'">
            <span :class="{ mono: q.groupBy !== 'day' }">{{ record.key }}</span>
          </template>
          <template v-else-if="column.key === 'requests'"><span class="num">{{ num(record.requests) }}</span></template>
          <template v-else-if="column.key === 'prompt'"><span class="num">{{ num(record.prompt_tokens) }}</span></template>
          <template v-else-if="column.key === 'completion'"><span class="num">{{ num(record.completion_tokens) }}</span></template>
          <template v-else-if="column.key === 'total'"><span class="num strong">{{ num(record.total_tokens) }}</span></template>
          <template v-else-if="column.key === 'share'">
            <div class="share">
              <div class="share-track"><div class="share-bar" :style="{ width: share(record as UsageRow) + '%' }" /></div>
              <span class="num">{{ share(record as UsageRow).toFixed(1) }}%</span>
            </div>
          </template>
        </template>
      </a-table>
    </section>
  </div>
</template>

<style scoped>
.filters {
  margin-bottom: 16px;
}
.tiles {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
}
.tile {
  padding: 14px 18px;
}
.tile + .tile {
  border-left: 1px solid var(--lg-border-soft);
}
.tile-label {
  font-size: 12.5px;
  color: var(--lg-text-3);
}
.tile-value {
  display: inline-block;
  font-size: 22px;
  font-weight: 650;
  font-variant-numeric: tabular-nums;
  margin-top: 2px;
}
.top {
  font-weight: 400;
  font-size: 12.5px;
}
.empty {
  padding: 24px 0;
}
.num {
  font-variant-numeric: tabular-nums;
}
.strong {
  font-weight: 600;
}
.share {
  display: flex;
  align-items: center;
  gap: 10px;
}
.share-track {
  flex: 1;
  height: 6px;
  border-radius: 3px;
  background: var(--lg-neutral-bg);
  overflow: hidden;
}
.share-bar {
  height: 100%;
  background: var(--lg-chart-1);
  border-radius: 3px;
}
.share .num {
  width: 48px;
  text-align: right;
  font-size: 12.5px;
  color: var(--lg-text-2);
}
@media (max-width: 900px) {
  .tiles {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
