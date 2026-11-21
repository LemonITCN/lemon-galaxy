<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import {
  ApiOutlined,
  DatabaseOutlined,
  DesktopOutlined,
  ReloadOutlined,
  SwapOutlined,
  TeamOutlined,
  WifiOutlined,
} from '@antdv-next/icons'
import * as api from '@/api'
import type { ServerInfo, TimelinePoint } from '@/api'
import CopyText from '@/components/CopyText.vue'
import InfoTip from '@/components/InfoTip.vue'
import TrendChart from '@/components/TrendChart.vue'
import type { ChartPoint } from '@/components/TrendChart.vue'
import { t } from '@/locales'
import { bytes, compact, dateTime, dayjs, duration, num } from '@/utils/format'

const router = useRouter()
const info = ref<ServerInfo | null>(null)
const days = ref<TimelinePoint[]>([])
const loading = ref(false)
const now = ref(Date.now())
let clock: number | undefined

async function load() {
  loading.value = true
  try {
    const [i, tl] = await Promise.all([api.serverInfo(), api.timeline(7)])
    info.value = i
    days.value = tl.items
  } catch {
    /* 已统一提示 */
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  load()
  clock = window.setInterval(() => (now.value = Date.now()), 30_000)
})
onBeforeUnmount(() => clearInterval(clock))

const stats = computed(() => {
  const i = info.value
  return [
    { key: 'users', icon: TeamOutlined, label: t('overview.users'), value: i ? num(i.users) : '—', to: '/admin/users' },
    { key: 'devices', icon: DesktopOutlined, label: t('overview.devices'), value: i ? num(i.devices) : '—', to: '/admin/devices' },
    {
      key: 'online',
      icon: WifiOutlined,
      label: t('overview.onlineDevices'),
      value: i ? num(i.online_devices) : '—',
      sub: i ? t('overview.ofTotal', { n: num(i.devices) }) : '',
      to: '/admin/devices?online=true',
      live: !!i && i.online_devices > 0,
    },
    { key: 'conn', icon: ApiOutlined, label: t('overview.activeConnections'), value: i ? num(i.active_connections) : '—', to: '/admin/connections' },
    { key: 'relay', icon: SwapOutlined, label: t('overview.relayTotal'), value: i ? bytes(i.relay_bytes_total) : '—' },
    { key: 'db', icon: DatabaseOutlined, label: t('overview.dbSize'), value: i ? bytes(i.db_size_bytes) : '—' },
  ]
})

function series(field: keyof Omit<TimelinePoint, 'day'>): ChartPoint[] {
  return days.value.map((d) => ({ label: dayjs(d.day).format('MM-DD'), title: d.day, value: d[field] }))
}

const charts = computed(() => {
  const sum = (f: keyof Omit<TimelinePoint, 'day'>) => days.value.reduce((s, d) => s + d[f], 0)
  return [
    { key: 'connections', title: t('overview.trendConnections'), type: 'bar' as const, color: 'var(--lg-chart-1)', points: series('connections'), format: compact, total: num(sum('connections')) },
    { key: 'relay', title: t('overview.trendRelay'), type: 'area' as const, color: 'var(--lg-chart-2)', points: series('relay_bytes'), format: bytes, total: bytes(sum('relay_bytes')) },
    { key: 'ai', title: t('overview.trendAiTokens'), type: 'bar' as const, color: 'var(--lg-chart-3)', points: series('ai_tokens'), format: compact, total: compact(sum('ai_tokens')) },
    { key: 'logins', title: t('overview.trendLogins'), type: 'area' as const, color: 'var(--lg-chart-4)', points: series('logins'), format: compact, total: num(sum('logins')) },
  ]
})

const uptime = computed(() => (info.value?.started_at ? duration(now.value - info.value.started_at) : '—'))
const hostPort = computed(() => (info.value ? (info.value.public_host ? `${info.value.public_host}:${info.value.port}` : `:${info.value.port}`) : '—'))
</script>

<template>
  <div class="page">
    <div class="page-head">
      <h1>{{ t('overview.title') }}</h1>
      <span class="spacer" />
      <a-button :loading="loading" @click="load">
        <template #icon><ReloadOutlined /></template>
        {{ t('common.refresh') }}
      </a-button>
    </div>

    <section class="panel server">
      <div class="addr-block">
        <div class="label">
          {{ t('overview.address') }}
          <InfoTip :text="t('overview.addressTip')" :width="360" />
        </div>
        <a-skeleton v-if="!info" active :paragraph="false" :title="{ width: '70%' }" />
        <CopyText v-else :text="info.address" mono block large wrap class="addr" />
        <a-alert v-if="info && !info.public_host" type="warning" show-icon :title="t('overview.noPublicHost')" class="host-warn">
          <template #action>
            <a-button size="small" type="link" @click="router.push('/admin/settings')">{{ t('overview.goSettings') }}</a-button>
          </template>
        </a-alert>
      </div>
      <dl class="facts">
        <div class="fact fp">
          <dt>{{ t('overview.fingerprint') }}</dt>
          <dd><CopyText v-if="info" :text="info.fingerprint" mono /><template v-else>—</template></dd>
        </div>
        <div class="fact">
          <dt>{{ t('overview.host') }}</dt>
          <dd class="mono">{{ hostPort }}</dd>
        </div>
        <div class="fact">
          <dt>{{ t('overview.version') }}</dt>
          <dd class="mono">{{ info ? 'v' + info.version : '—' }}</dd>
        </div>
        <div class="fact">
          <dt>{{ t('overview.uptime') }}</dt>
          <dd>
            <a-tooltip v-if="info" :title="t('overview.startedAt', { time: dateTime(info.started_at, true) })">
              <span>{{ uptime }}</span>
            </a-tooltip>
            <template v-else>—</template>
          </dd>
        </div>
      </dl>
    </section>

    <section class="stats">
      <component
        :is="s.to ? 'button' : 'div'"
        v-for="s in stats"
        :key="s.key"
        type="button"
        class="stat"
        :class="{ link: !!s.to }"
        @click="s.to && router.push(s.to)"
      >
        <span class="stat-icon" :class="{ live: s.live }"><component :is="s.icon" /></span>
        <span class="stat-body">
          <span class="stat-label">{{ s.label }}</span>
          <span class="stat-value">{{ s.value }}</span>
          <span v-if="s.sub" class="stat-sub">{{ s.sub }}</span>
        </span>
      </component>
    </section>

    <div class="section-title">{{ t('overview.trend') }}</div>
    <section class="charts">
      <div v-for="c in charts" :key="c.key" class="panel chart-card">
        <div class="chart-head">
          <span class="chart-title"><i :style="{ background: c.color }" />{{ c.title }}</span>
          <span class="chart-total">{{ t('overview.sum7', { v: c.total }) }}</span>
        </div>
        <TrendChart :points="c.points" :type="c.type" :color="c.color" :format="c.format" :binary="c.key === 'relay'" :height="150" />
      </div>
    </section>
  </div>
</template>

<style scoped>
.addr-block {
  padding: 18px 20px 16px;
  min-width: 0;
}
.label {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 13px;
  color: var(--lg-text-2);
  font-weight: 550;
  margin-bottom: 10px;
}
.addr {
  padding: 10px 8px 10px 14px !important;
}
.host-warn {
  margin-top: 12px;
}
.facts {
  display: flex;
  flex-wrap: wrap;
  gap: 10px 36px;
  margin: 0;
  padding: 12px 20px;
  border-top: 1px solid var(--lg-border-soft);
  background: var(--lg-surface-2);
  border-radius: 0 0 10px 10px;
}
.fact {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}
.fact.fp {
  flex: 1 1 380px;
}
.fact dt {
  flex: none;
  color: var(--lg-text-3);
  font-size: 12.5px;
}
.fact dd {
  margin: 0;
  min-width: 0;
  font-weight: 550;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.stats {
  display: grid;
  grid-template-columns: repeat(6, minmax(0, 1fr));
  gap: 12px;
  margin-top: 16px;
}
.stat {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  padding: 14px 14px;
  background: var(--lg-surface);
  border: 1px solid var(--lg-border-soft);
  border-radius: 10px;
  box-shadow: var(--lg-shadow);
  text-align: left;
  font: inherit;
  color: inherit;
  min-width: 0;
}
.stat.link {
  cursor: pointer;
  transition: border-color 0.15s;
}
.stat.link:hover {
  border-color: var(--lg-brand);
}
.stat-icon {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: 8px;
  background: var(--lg-neutral-bg);
  color: var(--lg-text-2);
  font-size: 15px;
}
.stat-icon.live {
  background: var(--lg-brand-bg);
  color: var(--lg-brand-text);
}
.stat-body {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.stat-label {
  font-size: 12.5px;
  color: var(--lg-text-3);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.stat-value {
  font-size: 21px;
  font-weight: 650;
  letter-spacing: -0.01em;
  line-height: 1.35;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.stat-sub {
  font-size: 12px;
  color: var(--lg-text-3);
}
.section-title {
  margin: 22px 0 10px;
  font-size: 13px;
  font-weight: 600;
  color: var(--lg-text-2);
}
.charts {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 16px;
}
.charts .panel + .panel {
  margin-top: 0;
}
.chart-card {
  padding: 14px 16px 10px;
}
.chart-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 10px;
}
.chart-title {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  font-weight: 600;
  font-size: 13.5px;
}
.chart-title i {
  width: 8px;
  height: 8px;
  border-radius: 2px;
}
.chart-total {
  font-size: 12.5px;
  color: var(--lg-text-3);
  font-variant-numeric: tabular-nums;
}
@media (min-width: 1760px) {
  .charts {
    grid-template-columns: repeat(4, minmax(0, 1fr));
  }
}
@media (max-width: 1439px) {
  .stats {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
}
@media (max-width: 900px) {
  .charts {
    grid-template-columns: 1fr;
  }
  .stats {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
