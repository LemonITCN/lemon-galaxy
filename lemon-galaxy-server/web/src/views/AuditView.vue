<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import type { Dayjs } from 'dayjs'
import { SearchOutlined } from '@antdv-next/icons'
import * as api from '@/api'
import { ACTOR_TYPES, type ActorType, type AuditEntry } from '@/api'
import StatusBadge from '@/components/StatusBadge.vue'
import { t } from '@/locales'
import { dateTime, rangePresets } from '@/utils/format'
import { actionLabel, actionOptions, actionTone, actorTypeLabel, targetTypeLabel } from '@/utils/labels'

const rows = ref<AuditEntry[]>([])
const total = ref(0)
const loading = ref(false)
const q = reactive({
  q: '',
  actorType: undefined as ActorType | undefined,
  action: undefined as string | undefined,
  range: null as [Dayjs, Dayjs] | null,
  page: 1,
  pageSize: 30,
})

async function load() {
  loading.value = true
  try {
    const r = await api.listAudit({
      q: q.q.trim() || undefined,
      actor_type: q.actorType,
      action: q.action,
      from: q.range?.[0]?.startOf('day').valueOf(),
      to: q.range?.[1]?.endOf('day').valueOf(),
      page: q.page,
      page_size: q.pageSize,
    })
    rows.value = r.items
    total.value = r.total
  } catch {
    /* 已统一提示 */
  } finally {
    loading.value = false
  }
}
onMounted(load)

let timer: number | undefined
watch(
  () => q.q,
  () => {
    clearTimeout(timer)
    timer = window.setTimeout(() => {
      q.page = 1
      load()
    }, 300)
  },
)
watch(
  () => [q.actorType, q.action, q.range],
  () => {
    q.page = 1
    load()
  },
)

const actorOptions = computed(() => ACTOR_TYPES.map((a) => ({ value: a, label: actorTypeLabel(a) })))
const actions = computed(() => actionOptions())
const presets = computed(() => rangePresets({ today: t('aiUsage.today'), last7: t('aiUsage.last7'), last30: t('aiUsage.last30') }))

const pagination = computed(() => ({
  current: q.page,
  pageSize: q.pageSize,
  total: total.value,
  showSizeChanger: true,
  pageSizeOptions: ['30', '50', '100'],
  showTotal: (n: number) => t('common.total', { n }),
}))
function onTable(p: { current?: number; pageSize?: number }) {
  q.page = p.current ?? 1
  q.pageSize = p.pageSize ?? 30
  load()
}

const columns = computed(() => [
  { title: t('audit.colTime'), key: 'time', width: 170 },
  { title: t('audit.colActor'), key: 'actor', width: 200 },
  { title: t('audit.colAction'), key: 'action', width: 220 },
  { title: t('audit.colTarget'), key: 'target' },
  { title: t('audit.colIp'), key: 'ip', width: 150 },
])

function hasDetail(r: AuditEntry) {
  return !!r.detail && Object.keys(r.detail).length > 0
}

const actorTone: Record<string, string> = { admin: 'brand', user: 'info', device: 'neutral', system: 'warning' }
</script>

<template>
  <div class="page">
    <div class="page-head">
      <h1>{{ t('audit.title') }}</h1>
      <span class="count">{{ total }}</span>
    </div>
    <div class="panel">
      <div class="toolbar">
        <a-range-picker v-model:value="q.range" :presets="presets" />
        <a-select v-model:value="q.actorType" :options="actorOptions" allow-clear :placeholder="t('audit.allActors')" style="width: 140px" />
        <a-select
          v-model:value="q.action"
          :options="actions"
          allow-clear
          :show-search="{ optionFilterProp: 'label' }"
          :placeholder="t('audit.allActions')"
          style="width: 220px"
        />
        <a-input v-model:value="q.q" allow-clear :placeholder="t('audit.searchPlaceholder')" style="width: 240px">
          <template #prefix><SearchOutlined class="muted" /></template>
        </a-input>
      </div>
      <a-table
        :columns="columns"
        :data-source="rows"
        row-key="id"
        :loading="loading"
        :pagination="pagination"
        :scroll="{ x: 1000 }"
        :expandable="{ rowExpandable: (r: AuditEntry) => hasDetail(r) }"
        @change="onTable"
      >
        <template #bodyCell="{ column, record }">
          <template v-if="column.key === 'time'">
            <span class="secondary num">{{ dateTime(record.at, true) }}</span>
          </template>
          <template v-else-if="column.key === 'actor'">
            <div class="actor">
              <StatusBadge :tone="actorTone[record.actor_type] ?? 'neutral'" :text="actorTypeLabel(record.actor_type)" :dot="false" />
              <span class="aname" :title="record.actor_id">{{ record.actor_name || record.actor_id || '—' }}</span>
            </div>
          </template>
          <template v-else-if="column.key === 'action'">
            <a-tooltip :title="record.action">
              <StatusBadge :tone="actionTone(record.action)" :text="actionLabel(record.action)" />
            </a-tooltip>
          </template>
          <template v-else-if="column.key === 'target'">
            <span v-if="record.target_type || record.target_name" class="target">
              <span v-if="record.target_type" class="muted">{{ targetTypeLabel(record.target_type) }}</span>
              <span class="tname" :title="record.target_id">{{ record.target_name || record.target_id }}</span>
            </span>
            <span v-else class="muted">—</span>
          </template>
          <template v-else-if="column.key === 'ip'">
            <span class="mono secondary">{{ record.ip || '—' }}</span>
          </template>
        </template>
        <template #expandedRowRender="{ record }">
          <div class="detail">
            <div class="detail-title">{{ t('audit.detail') }}</div>
            <pre class="code-block">{{ JSON.stringify(record.detail, null, 2) }}</pre>
          </div>
        </template>
      </a-table>
    </div>
  </div>
</template>

<style scoped>
.num {
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.actor {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.aname {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 520;
}
.target {
  display: inline-flex;
  gap: 8px;
  align-items: baseline;
  min-width: 0;
}
.tname {
  font-weight: 520;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.detail {
  padding: 4px 8px;
}
.detail-title {
  font-size: 12.5px;
  color: var(--lg-text-3);
  margin-bottom: 6px;
}
</style>
