<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { DeleteOutlined, StopOutlined } from '@antdv-next/icons'
import * as api from '@/api'
import type { Device } from '@/api'
import CopyText from '@/components/CopyText.vue'
import PlatformIcon from '@/components/PlatformIcon.vue'
import StatusBadge from '@/components/StatusBadge.vue'
import { t } from '@/locales'
import { toast } from '@/utils/feedback'
import { dateTime, fromNow } from '@/utils/format'
import { accountStatusLabel, accountStatusTone, deviceKindLabel, platformLabel } from '@/utils/labels'

// 设备详情：基本信息 + 各账号的绑定状态，可从某个账号吊销、从所有账号吊销、删除
const open = defineModel<boolean>('open', { default: false })
const props = defineProps<{ device: Device | null }>()
const emit = defineEmits<{ changed: [device: Device]; deleted: [id: string] }>()

const router = useRouter()
const dev = ref<Device | null>(null)
const busy = ref('')

watch(
  () => [open.value, props.device] as const,
  async ([o, d]) => {
    if (!o || !d) return
    dev.value = d
    try {
      dev.value = await api.getDevice(d.id)
    } catch {
      /* 已统一提示，先用列表里的数据 */
    }
  },
  { immediate: true },
)

async function revoke(userId?: number) {
  if (!dev.value) return
  busy.value = userId === undefined ? 'all' : 'u' + userId
  try {
    const d = await api.revokeDevice(dev.value.id, userId)
    dev.value = d
    toast.success(t('devices.revoked'))
    emit('changed', d)
  } catch {
    /* 已统一提示 */
  } finally {
    busy.value = ''
  }
}

async function remove() {
  if (!dev.value) return
  busy.value = 'delete'
  try {
    await api.deleteDevice(dev.value.id)
    toast.success(t('common.deleted'))
    emit('deleted', dev.value.id)
    open.value = false
  } catch {
    /* 已统一提示 */
  } finally {
    busy.value = ''
  }
}

const activeCount = computed(() => dev.value?.accounts.filter((a) => a.status !== 'revoked').length ?? 0)

const accountColumns = computed(() => [
  { title: t('devices.accountUser'), key: 'user' },
  { title: t('devices.accountStatus'), key: 'status', width: 130 },
  { title: t('devices.accountAdded'), key: 'added', width: 150 },
  { title: '', key: 'actions', width: 80, align: 'right' as const },
])

function goUser(id: number) {
  open.value = false
  router.push(`/admin/users/${id}`)
}
</script>

<template>
  <a-drawer v-model:open="open" :size="600" :title="t('devices.drawerTitle')" destroy-on-hidden>
    <template v-if="dev">
      <div class="hero">
        <PlatformIcon :platform="dev.platform" boxed />
        <div class="min0">
          <div class="host">{{ dev.hostname }}</div>
          <div class="sub">{{ platformLabel(dev.platform) }} · {{ dev.os_version }} · {{ dev.arch }}</div>
        </div>
        <span class="spacer" />
        <StatusBadge v-if="dev.online" tone="success" :text="t('devices.online')" />
        <StatusBadge v-else tone="neutral" :text="t('devices.offline')" />
      </div>

      <dl class="kv">
        <dt>{{ t('devices.id') }}</dt>
        <dd><CopyText :text="dev.id" mono /></dd>
        <dt>{{ t('devices.kind') }}</dt>
        <dd><a-tag :bordered="false">{{ deviceKindLabel(dev.kind) }}</a-tag></dd>
        <dt>{{ t('devices.appVersion') }}</dt>
        <dd class="mono">{{ dev.app_version || '—' }}</dd>
        <dt>{{ t('devices.lastSeenAt') }}</dt>
        <dd>
          <template v-if="dev.online">{{ t('devices.online') }}</template>
          <template v-else>{{ dateTime(dev.last_seen_at, true) }} <span class="muted">{{ fromNow(dev.last_seen_at) }}</span></template>
        </dd>
        <dt>{{ t('devices.lastIp') }}</dt>
        <dd class="mono">{{ dev.last_ip || '—' }}</dd>
        <dt>{{ t('devices.macs') }}</dt>
        <dd class="mono">{{ dev.macs && dev.macs.length ? dev.macs.join(', ') : '—' }}</dd>
        <dt>{{ t('devices.assistMode') }}</dt>
        <dd>{{ dev.assist_mode ? t('common.yes') : t('common.no') }}</dd>
        <dt>{{ t('devices.createdAt') }}</dt>
        <dd>{{ dateTime(dev.created_at, true) }}</dd>
      </dl>

      <div class="section-head">
        {{ t('devices.accounts') }}
        <span class="spacer" />
        <a-popconfirm
          v-if="activeCount > 0"
          :title="t('devices.confirmRevokeAll')"
          :ok-text="t('devices.revoke')"
          :ok-button-props="{ danger: true }"
          @confirm="revoke()"
        >
          <a-button size="small" danger :loading="busy === 'all'">
            <template #icon><StopOutlined /></template>
            {{ t('devices.revokeAll') }}
          </a-button>
        </a-popconfirm>
      </div>
      <a-table
        :columns="accountColumns"
        :data-source="dev.accounts"
        row-key="user_id"
        :pagination="false"
        size="small"
        :locale="{ emptyText: t('devices.noAccounts') }"
      >
        <template #bodyCell="{ column, record }">
          <template v-if="column.key === 'user'">
            <a class="mono" @click="goUser(record.user_id)">{{ record.username }}</a>
          </template>
          <template v-else-if="column.key === 'status'">
            <StatusBadge :tone="accountStatusTone[record.status as keyof typeof accountStatusTone]" :text="accountStatusLabel(record.status)" />
          </template>
          <template v-else-if="column.key === 'added'">
            <span class="secondary">{{ dateTime(record.added_at) }}</span>
          </template>
          <template v-else-if="column.key === 'actions'">
            <a-popconfirm
              v-if="record.status !== 'revoked'"
              :title="t('devices.confirmRevoke', { name: record.username })"
              :ok-text="t('devices.revoke')"
              :ok-button-props="{ danger: true }"
              placement="topRight"
              @confirm="revoke(record.user_id)"
            >
              <a-button type="link" size="small" danger :loading="busy === 'u' + record.user_id">{{ t('devices.revoke') }}</a-button>
            </a-popconfirm>
          </template>
        </template>
      </a-table>
    </template>

    <template #footer>
      <div class="foot">
        <a-popconfirm :title="t('devices.confirmDelete')" :ok-text="t('common.delete')" :ok-button-props="{ danger: true }" placement="topLeft" @confirm="remove">
          <a-button danger :loading="busy === 'delete'">
            <template #icon><DeleteOutlined /></template>
            {{ t('devices.deleteDevice') }}
          </a-button>
        </a-popconfirm>
        <span class="spacer" />
        <a-button @click="open = false">{{ t('common.close') }}</a-button>
      </div>
    </template>
  </a-drawer>
</template>

<style scoped>
.hero {
  display: flex;
  align-items: center;
  gap: 12px;
  padding-bottom: 16px;
  margin-bottom: 16px;
  border-bottom: 1px solid var(--lg-border-soft);
}
.host {
  font-size: 16px;
  font-weight: 600;
}
.sub {
  font-size: 12.5px;
  color: var(--lg-text-3);
}
.spacer {
  flex: 1;
}
.min0 {
  min-width: 0;
}
.kv {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  gap: 11px 24px;
  margin: 0;
}
.kv dt {
  color: var(--lg-text-3);
  font-size: 13px;
}
.kv dd {
  margin: 0;
  min-width: 0;
}
.section-head {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 24px 0 10px;
  font-weight: 600;
}
.foot {
  display: flex;
  align-items: center;
}
</style>
