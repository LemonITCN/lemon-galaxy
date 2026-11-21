<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { EditOutlined, MoreOutlined, PlusOutlined, SearchOutlined } from '@antdv-next/icons'
import * as api from '@/api'
import type { User } from '@/api'
import CredentialModal from '@/components/CredentialModal.vue'
import StatusBadge from '@/components/StatusBadge.vue'
import { t } from '@/locales'
import { dateTime, fromNow, initials } from '@/utils/format'
import { aiModeLabel, aiModeTone } from '@/utils/labels'
import UserActionsHost from './UserActionsHost.vue'
import UserFormModal from './UserFormModal.vue'
import { userMenuItems, userStatusTags, type UserAction } from './userStatus'

const router = useRouter()
const rows = ref<User[]>([])
const total = ref(0)
const loading = ref(false)
const q = reactive({ q: '', page: 1, pageSize: 20 })

async function load() {
  loading.value = true
  try {
    const r = await api.listUsers({ q: q.q.trim() || undefined, page: q.page, page_size: q.pageSize })
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
  { title: t('users.colUser'), key: 'user' },
  { title: t('users.colStatus'), key: 'status', width: 200 },
  { title: t('users.colDevices'), key: 'devices', width: 110 },
  { title: t('users.colAi'), key: 'ai', width: 160 },
  { title: t('users.colLastLogin'), key: 'login', width: 120 },
  { title: t('users.colCreated'), key: 'created', width: 160 },
  { title: '', key: 'actions', width: 84, align: 'right' as const, fixed: 'right' as const },
])

// 新建 / 编辑
const formOpen = ref(false)
const editing = ref<User | null>(null)
const cred = reactive({ open: false, username: '', password: '' })

function openCreate() {
  editing.value = null
  formOpen.value = true
}
function openEdit(u: User) {
  editing.value = u
  formOpen.value = true
}
function onSaved(u: User, password?: string) {
  if (password) {
    Object.assign(cred, { open: true, username: u.username, password })
    q.page = 1
    load()
  } else replaceRow(u)
}
function replaceRow(u: User) {
  const i = rows.value.findIndex((r) => r.id === u.id)
  if (i >= 0) rows.value[i] = u
}

// 更多操作
const host = ref<InstanceType<typeof UserActionsHost>>()
function onMore(u: User, info: { key: string | number }) {
  host.value?.run(String(info.key) as UserAction, u)
}

function customRow(record: User) {
  return { class: 'clickable-row', onClick: () => router.push(`/admin/users/${record.id}`) }
}
</script>

<template>
  <div class="page">
    <div class="page-head">
      <h1>{{ t('users.title') }}</h1>
      <span class="count">{{ total }}</span>
      <span class="spacer" />
      <a-button type="primary" @click="openCreate">
        <template #icon><PlusOutlined /></template>
        {{ t('users.create') }}
      </a-button>
    </div>
    <div class="panel">
      <div class="toolbar">
        <a-input v-model:value="q.q" allow-clear :placeholder="t('users.searchPlaceholder')" style="width: 300px">
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
        :on-row="customRow"
        @change="onTable"
      >
        <template #bodyCell="{ column, record }">
          <template v-if="column.key === 'user'">
            <div class="cell-main">
              <span class="avatar" :class="{ off: record.disabled }">{{ initials(record.display_name || record.username) }}</span>
              <div class="min0">
                <div class="name">{{ record.display_name || record.username }}</div>
                <div class="sub mono">{{ record.username }}<template v-if="record.note"> · {{ record.note }}</template></div>
              </div>
            </div>
          </template>
          <template v-else-if="column.key === 'status'">
            <div class="tag-row">
              <StatusBadge v-for="s in userStatusTags(record)" :key="s.text" :tone="s.tone" :text="s.text" />
            </div>
          </template>
          <template v-else-if="column.key === 'devices'">
            <span class="dev">
              <i class="dot" :class="{ on: record.online_device_count > 0 }" />
              {{ t('users.devicesOnline', { online: record.online_device_count, total: record.device_count }) }}
            </span>
          </template>
          <template v-else-if="column.key === 'ai'">
            <StatusBadge
              v-if="record.ai_enabled"
              :tone="aiModeTone[record.ai_max_mode as keyof typeof aiModeTone]"
              :text="aiModeLabel(record.ai_max_mode)"
              :dot="false"
            />
            <span v-else class="muted">{{ t('users.aiOff') }}</span>
          </template>
          <template v-else-if="column.key === 'login'">
            <a-tooltip v-if="record.last_login_at" :title="dateTime(record.last_login_at, true)">
              <span class="secondary">{{ fromNow(record.last_login_at) }}</span>
            </a-tooltip>
            <span v-else class="muted">{{ t('common.never') }}</span>
          </template>
          <template v-else-if="column.key === 'created'">
            <span class="secondary nowrap">{{ dateTime(record.created_at) }}</span>
          </template>
          <template v-else-if="column.key === 'actions'">
            <div class="row-actions" @click.stop>
              <a-tooltip :title="t('common.edit')">
                <a-button type="text" size="small" :aria-label="t('common.edit')" @click="openEdit(record as User)"><EditOutlined /></a-button>
              </a-tooltip>
              <a-dropdown
                :trigger="['click']"
                placement="bottomRight"
                :menu="{ items: userMenuItems(record as User) }"
                @menu-click="(info: { key: string | number }) => onMore(record as User, info)"
              >
                <a-button type="text" size="small" :aria-label="t('common.more')"><MoreOutlined /></a-button>
              </a-dropdown>
            </div>
          </template>
        </template>
      </a-table>
    </div>

    <UserFormModal v-model:open="formOpen" :user="editing" @saved="onSaved" />
    <CredentialModal
      v-model:open="cred.open"
      :title="t('users.credTitle')"
      :description="t('users.credDesc')"
      :username="cred.username"
      :password="cred.password"
    />
    <UserActionsHost ref="host" @changed="replaceRow" @deleted="load" @refresh="load" />
  </div>
</template>

<style scoped>
.min0 {
  min-width: 0;
}
.dev {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  color: var(--lg-text-2);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--lg-border);
}
.dot.on {
  background: var(--lg-success);
}
.row-actions {
  display: inline-flex;
  gap: 2px;
}
</style>
