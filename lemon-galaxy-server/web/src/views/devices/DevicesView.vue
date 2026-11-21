<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { SearchOutlined } from '@antdv-next/icons'
import * as api from '@/api'
import { DEVICE_KINDS, type Device, type DeviceKind } from '@/api'
import PlatformIcon from '@/components/PlatformIcon.vue'
import StatusBadge from '@/components/StatusBadge.vue'
import UserSelect from '@/components/UserSelect.vue'
import { t } from '@/locales'
import { toast } from '@/utils/feedback'
import { dateTime, fromNow, shortId } from '@/utils/format'
import { accountStatusLabel, deviceKindLabel, platformLabel } from '@/utils/labels'
import DeviceDrawer from './DeviceDrawer.vue'

const route = useRoute()
const router = useRouter()

const rows = ref<Device[]>([])
const total = ref(0)
const loading = ref(false)

const initialUser = Number(route.query.user_id)
const q = reactive({
  q: '',
  userId: Number.isFinite(initialUser) && initialUser > 0 ? initialUser : (undefined as number | undefined),
  online: route.query.online === 'true' ? 'true' : route.query.online === 'false' ? 'false' : (undefined as string | undefined),
  kind: undefined as DeviceKind | undefined,
  page: 1,
  pageSize: 20,
})

async function load() {
  loading.value = true
  try {
    const r = await api.listDevices({
      q: q.q.trim() || undefined,
      user_id: q.userId,
      online: q.online === undefined ? undefined : q.online === 'true',
      kind: q.kind,
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
  () => [q.userId, q.online, q.kind],
  () => {
    q.page = 1
    // 筛选条件写回地址栏，刷新或分享链接时保持一致
    const query: Record<string, string> = {}
    if (q.userId) query.user_id = String(q.userId)
    if (q.online) query.online = q.online
    router.replace({ query })
    load()
  },
)

const onlineOptions = computed(() => [
  { value: 'true', label: t('devices.online') },
  { value: 'false', label: t('devices.offline') },
])
const kindOptions = computed(() => DEVICE_KINDS.map((k) => ({ value: k, label: deviceKindLabel(k) })))

const pagination = computed(() => ({
  current: q.page,
  pageSize: q.pageSize,
  total: total.value,
  showSizeChanger: true,
  showTotal: (n: number) => t('common.total', { n }),
}))
function onTable(p: { current?: number; pageSize?: number }) {
  q.page = p.current ?? 1
  q.pageSize = p.pageSize ?? 20
  load()
}

const columns = computed(() => [
  { title: t('devices.colDevice'), key: 'device' },
  { title: t('devices.colKind'), key: 'kind', width: 110 },
  { title: t('devices.colSystem'), key: 'system', width: 180 },
  { title: t('devices.colAccounts'), key: 'accounts', width: 200 },
  { title: t('devices.colStatus'), key: 'status', width: 150 },
  { title: t('devices.colVersion'), key: 'version', width: 100 },
  { title: '', key: 'actions', width: 70, align: 'right' as const, fixed: 'right' as const },
])

const accountTagColor: Record<string, string | undefined> = { active: undefined, needs_relogin: 'warning', revoked: 'default' }

// 详情抽屉
const drawerOpen = ref(false)
const current = ref<Device | null>(null)
function openDevice(d: Device) {
  current.value = d
  drawerOpen.value = true
}
function onChanged(d: Device) {
  const i = rows.value.findIndex((r) => r.id === d.id)
  if (i >= 0) rows.value[i] = d
}
function onDeleted() {
  load()
}

async function remove(d: Device) {
  try {
    await api.deleteDevice(d.id)
    toast.success(t('common.deleted'))
    load()
  } catch {
    /* 已统一提示 */
  }
}

function customRow(record: Device) {
  return { class: 'clickable-row', onClick: () => openDevice(record) }
}
</script>

<template>
  <div class="page">
    <div class="page-head">
      <h1>{{ t('devices.title') }}</h1>
      <span class="count">{{ total }}</span>
    </div>
    <div class="panel">
      <div class="toolbar">
        <a-input v-model:value="q.q" allow-clear :placeholder="t('devices.searchPlaceholder')" style="width: 260px">
          <template #prefix><SearchOutlined class="muted" /></template>
        </a-input>
        <UserSelect v-model:value="q.userId" :placeholder="t('devices.filterUser')" :width="200" />
        <a-select v-model:value="q.online" :options="onlineOptions" allow-clear :placeholder="t('devices.filterOnline')" style="width: 130px" />
        <a-select v-model:value="q.kind" :options="kindOptions" allow-clear :placeholder="t('devices.filterKind')" style="width: 140px" />
      </div>
      <a-table
        :columns="columns"
        :data-source="rows"
        row-key="id"
        :loading="loading"
        :pagination="pagination"
        :scroll="{ x: 1020 }"
        :on-row="customRow"
        @change="onTable"
      >
        <template #bodyCell="{ column, record }">
          <template v-if="column.key === 'device'">
            <div class="cell-main">
              <PlatformIcon :platform="record.platform" boxed />
              <div class="min0">
                <div class="name">{{ record.hostname || '—' }}</div>
                <div class="sub mono">{{ shortId(record.id) }}</div>
              </div>
            </div>
          </template>
          <template v-else-if="column.key === 'kind'">
            <a-tag :bordered="false">{{ deviceKindLabel(record.kind) }}</a-tag>
          </template>
          <template v-else-if="column.key === 'system'">
            <div class="min0">
              <div class="nowrap">{{ platformLabel(record.platform) }} <span class="muted">{{ record.arch }}</span></div>
              <div class="sub-line">{{ record.os_version }}</div>
            </div>
          </template>
          <template v-else-if="column.key === 'accounts'">
            <div class="tag-row">
              <a-tooltip v-for="a in record.accounts" :key="a.user_id" :title="accountStatusLabel(a.status)">
                <a-tag :color="accountTagColor[a.status]" :bordered="false" class="acct" :class="{ revoked: a.status === 'revoked' }">
                  {{ a.username }}
                </a-tag>
              </a-tooltip>
              <StatusBadge v-if="record.assist_mode" tone="info" :text="t('devices.assist')" :dot="false" />
              <span v-if="!record.accounts.length && !record.assist_mode" class="muted">—</span>
            </div>
          </template>
          <template v-else-if="column.key === 'status'">
            <StatusBadge v-if="record.online" tone="success" :text="t('devices.online')" />
            <a-tooltip v-else :title="dateTime(record.last_seen_at, true)">
              <span class="muted nowrap">{{ t('devices.lastSeen', { time: fromNow(record.last_seen_at) }) }}</span>
            </a-tooltip>
            <div v-if="record.last_ip" class="sub-line mono" :title="t('devices.colIp')">{{ record.last_ip }}</div>
          </template>
          <template v-else-if="column.key === 'version'">
            <span class="mono secondary">{{ record.app_version || '—' }}</span>
          </template>
          <template v-else-if="column.key === 'actions'">
            <div @click.stop>
              <a-popconfirm :title="t('devices.confirmDelete')" :ok-text="t('common.delete')" :ok-button-props="{ danger: true }" placement="topRight" @confirm="remove(record as Device)">
                <a-button type="link" size="small" danger>{{ t('common.delete') }}</a-button>
              </a-popconfirm>
            </div>
          </template>
        </template>
      </a-table>
    </div>

    <DeviceDrawer v-model:open="drawerOpen" :device="current" @changed="onChanged" @deleted="onDeleted" />
  </div>
</template>

<style scoped>
.min0 {
  min-width: 0;
}
.sub-line {
  font-size: 12px;
  color: var(--lg-text-3);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.acct {
  margin-inline-end: 0;
  font-family: var(--lg-mono);
  font-size: 12px;
}
.acct.revoked {
  text-decoration: line-through;
  opacity: 0.7;
}
</style>
