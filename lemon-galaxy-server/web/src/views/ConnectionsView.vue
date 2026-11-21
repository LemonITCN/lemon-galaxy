<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { ArrowRightOutlined, ReloadOutlined } from '@antdv-next/icons'
import * as api from '@/api'
import { CONNECTION_PATHS, type Connection } from '@/api'
import StatusBadge from '@/components/StatusBadge.vue'
import { t } from '@/locales'
import { toast } from '@/utils/feedback'
import { bytes, dateTime, dayjs, duration } from '@/utils/format'
import { connKindLabel, connPathLabel, connPathTone } from '@/utils/labels'

// 当前连接：每 5 秒刷新一次，可强制断开
const router = useRouter()
const rows = ref<Connection[]>([])
const loading = ref(false)
const auto = ref(true)
const updatedAt = ref(0)
const now = ref(Date.now())
let poll: number | undefined
let tick: number | undefined
let inFlight = false

async function load(manual = false) {
  if (inFlight) return
  inFlight = true
  if (manual) loading.value = true
  try {
    // 自动刷新时不弹错误提示，避免网络抖动时连续弹窗
    const r = await api.listConnections(!manual)
    rows.value = r.items
    updatedAt.value = Date.now()
  } catch {
    /* 手动刷新时已统一提示 */
  } finally {
    inFlight = false
    loading.value = false
  }
}

function startPoll() {
  clearInterval(poll)
  if (auto.value) poll = window.setInterval(() => load(false), 5000)
}

onMounted(() => {
  load(true)
  startPoll()
  tick = window.setInterval(() => (now.value = Date.now()), 1000)
})
onBeforeUnmount(() => {
  clearInterval(poll)
  clearInterval(tick)
})
watch(auto, startPoll)

const pathCounts = computed(() =>
  CONNECTION_PATHS.filter((p) => p !== 'closed')
    .map((p) => ({ path: p, n: rows.value.filter((r) => r.path === p).length }))
    .filter((x) => x.n > 0),
)

const columns = computed(() => [
  { title: t('connections.colKind'), key: 'kind', width: 110 },
  { title: t('connections.colUser'), key: 'user', width: 130 },
  { title: t('connections.colRoute'), key: 'route' },
  { title: t('connections.colPath'), key: 'path', width: 130 },
  { title: t('connections.colDuration'), key: 'duration', width: 140 },
  { title: t('connections.colRelay'), key: 'relay', width: 120, align: 'right' as const },
  { title: '', key: 'actions', width: 80, align: 'right' as const, fixed: 'right' as const },
])

async function terminate(c: Connection) {
  try {
    await api.terminateConnection(c.id)
    toast.success(t('connections.terminated'))
    rows.value = rows.value.filter((r) => r.id !== c.id)
  } catch {
    /* 已统一提示 */
  }
}
</script>

<template>
  <div class="page">
    <div class="page-head">
      <h1>{{ t('connections.title') }}</h1>
      <span class="count">{{ rows.length }}</span>
      <div class="paths">
        <StatusBadge v-for="p in pathCounts" :key="p.path" :tone="connPathTone[p.path]" :text="`${connPathLabel(p.path)} ${p.n}`" />
      </div>
      <span class="spacer" />
      <span v-if="updatedAt" class="updated muted">{{ t('connections.updatedAt', { time: dayjs(updatedAt).format('HH:mm:ss') }) }}</span>
      <label class="auto">
        <a-switch v-model:checked="auto" size="small" />
        <span>{{ t('connections.autoRefresh') }}</span>
      </label>
      <a-button :loading="loading" @click="load(true)">
        <template #icon><ReloadOutlined /></template>
        {{ t('common.refresh') }}
      </a-button>
    </div>
    <div class="panel">
      <a-table
        :columns="columns"
        :data-source="rows"
        row-key="id"
        :pagination="false"
        :scroll="{ x: 980 }"
        :locale="{ emptyText: t('connections.empty') }"
      >
        <template #bodyCell="{ column, record }">
          <template v-if="column.key === 'kind'">
            <a-tag :bordered="false" :color="record.kind === 'assist' ? 'processing' : undefined">{{ connKindLabel(record.kind) }}</a-tag>
          </template>
          <template v-else-if="column.key === 'user'">
            <a v-if="record.user_id" class="mono" @click="router.push(`/admin/users/${record.user_id}`)">{{ record.username }}</a>
            <span v-else class="muted">—</span>
          </template>
          <template v-else-if="column.key === 'route'">
            <div class="route">
              <span class="end" :title="record.from_device_id">{{ record.from_device_name || record.from_device_id }}</span>
              <ArrowRightOutlined class="arrow" />
              <span class="end" :title="record.to_device_id">{{ record.to_device_name || record.to_device_id }}</span>
            </div>
          </template>
          <template v-else-if="column.key === 'path'">
            <StatusBadge :tone="connPathTone[record.path as keyof typeof connPathTone]" :text="connPathLabel(record.path)" />
          </template>
          <template v-else-if="column.key === 'duration'">
            <a-tooltip :title="dateTime(record.started_at, true)">
              <span class="num">{{ duration(now - record.started_at) }}</span>
            </a-tooltip>
          </template>
          <template v-else-if="column.key === 'relay'">
            <span class="num" :class="{ muted: !record.relay_bytes }">{{ record.relay_bytes ? bytes(record.relay_bytes) : '—' }}</span>
          </template>
          <template v-else-if="column.key === 'actions'">
            <a-popconfirm
              v-if="record.path !== 'closed'"
              :title="t('connections.confirmTerminate')"
              :ok-text="t('connections.terminate')"
              :ok-button-props="{ danger: true }"
              placement="topRight"
              @confirm="terminate(record as Connection)"
            >
              <a-button type="link" size="small" danger>{{ t('connections.terminate') }}</a-button>
            </a-popconfirm>
          </template>
        </template>
      </a-table>
    </div>
  </div>
</template>

<style scoped>
.paths {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-left: 6px;
}
.updated {
  font-size: 12.5px;
  font-variant-numeric: tabular-nums;
}
.auto {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: var(--lg-text-2);
  cursor: pointer;
  margin: 0 6px;
}
.route {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}
.end {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 520;
}
.arrow {
  flex: none;
  color: var(--lg-text-3);
  font-size: 12px;
}
.num {
  font-variant-numeric: tabular-nums;
}
</style>
