<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { ArrowLeftOutlined, EditOutlined, MoreOutlined } from '@antdv-next/icons'
import * as api from '@/api'
import type { AiGrant, AiMode, AiProvider, Device, User } from '@/api'
import InfoTip from '@/components/InfoTip.vue'
import PlatformIcon from '@/components/PlatformIcon.vue'
import StatusBadge from '@/components/StatusBadge.vue'
import { t } from '@/locales'
import { crumb } from '@/stores/crumb'
import { toast } from '@/utils/feedback'
import { dateTime, fromNow, groupDigits, initials, num, ungroupDigits } from '@/utils/format'
import { accountStatusLabel, accountStatusTone, aiModeDesc, aiModeOptions, deviceKindLabel, providerKindLabel } from '@/utils/labels'
import UserActionsHost from './UserActionsHost.vue'
import UserFormModal from './UserFormModal.vue'
import { userMenuItems, userStatusTags, type UserAction } from './userStatus'

const route = useRoute()
const router = useRouter()
const id = Number(route.params.id)

const user = ref<User | null>(null)
const notFound = ref(false)
const devices = ref<Device[]>([])
const devicesTotal = ref(0)
const devicesLoading = ref(false)
const providers = ref<AiProvider[]>([])

async function loadUser() {
  try {
    user.value = await api.getUser(id, true)
    crumb.value = user.value.username
    resetAiForm()
  } catch (e) {
    if (e instanceof api.ApiError && e.status === 404) notFound.value = true
    else toast.error(api.errorText(e))
  }
}

async function loadDevices() {
  devicesLoading.value = true
  try {
    const r = await api.listDevices({ user_id: id, page: 1, page_size: 50 })
    devices.value = r.items
    devicesTotal.value = r.total
  } catch {
    /* 已统一提示 */
  } finally {
    devicesLoading.value = false
  }
}

// ---------- AI ----------

interface GrantRow {
  provider: AiProvider
  on: boolean
  all: boolean
  models: string[]
}

const ai = reactive({
  enabled: false,
  mode: 'confirm_dangerous' as AiMode,
  daily: 0,
  monthly: 0,
})
const grantRows = ref<GrantRow[]>([])
const savedGrants = ref<AiGrant[]>([])
const aiBusy = ref(false)

function resetAiForm() {
  const u = user.value
  if (!u) return
  ai.enabled = u.ai_enabled
  ai.mode = u.ai_max_mode
  ai.daily = u.ai_daily_token_limit
  ai.monthly = u.ai_monthly_token_limit
  buildGrantRows()
}

function buildGrantRows() {
  grantRows.value = providers.value.map((p) => {
    const g = savedGrants.value.find((x) => x.provider_id === p.id)
    const all = !g || g.models.includes('*')
    return { provider: p, on: !!g, all: g ? all : true, models: g && !all ? [...g.models] : [] }
  })
}

async function loadAi() {
  try {
    const [p, g] = await Promise.all([api.listProviders(), api.getUserGrants(id)])
    providers.value = p.items
    savedGrants.value = g.items
    buildGrantRows()
  } catch {
    /* 已统一提示 */
  }
}

function grantsFromRows(): AiGrant[] {
  return grantRows.value
    .filter((r) => r.on && (r.all || r.models.length > 0))
    .map((r) => ({ provider_id: r.provider.id, models: r.all ? ['*'] : [...r.models] }))
}

const aiDirty = computed(() => {
  const u = user.value
  if (!u) return false
  if (ai.enabled !== u.ai_enabled || ai.mode !== u.ai_max_mode) return true
  if ((ai.daily ?? 0) !== u.ai_daily_token_limit || (ai.monthly ?? 0) !== u.ai_monthly_token_limit) return true
  const norm = (list: AiGrant[]) =>
    JSON.stringify([...list].sort((a, b) => a.provider_id - b.provider_id).map((g) => [g.provider_id, [...g.models].sort()]))
  return norm(grantsFromRows()) !== norm(savedGrants.value)
})

async function saveAi() {
  if (!user.value) return
  aiBusy.value = true
  try {
    const u = await api.updateUser(id, {
      ai_enabled: ai.enabled,
      ai_max_mode: ai.mode,
      ai_daily_token_limit: Math.max(0, Math.floor(ai.daily || 0)),
      ai_monthly_token_limit: Math.max(0, Math.floor(ai.monthly || 0)),
    })
    const g = await api.putUserGrants(id, grantsFromRows())
    user.value = u
    savedGrants.value = g.items
    resetAiForm()
    toast.success(t('userDetail.aiSaved'))
  } catch {
    /* 已统一提示 */
  } finally {
    aiBusy.value = false
  }
}

const modeOptions = computed(() => aiModeOptions())

function modelOptions(p: AiProvider) {
  return p.models.map((m) => ({ value: m.id, label: m.name || m.id, disabled: false, off: !m.enabled }))
}

// ---------- 基本信息 / 操作 ----------

const formOpen = ref(false)
const host = ref<InstanceType<typeof UserActionsHost>>()

function onMore(info: { key: string | number }) {
  if (user.value) host.value?.run(String(info.key) as UserAction, user.value)
}
function onChanged(u: User) {
  user.value = u
  crumb.value = u.username
  resetAiForm()
  loadDevices()
}

const deviceColumns = computed(() => [
  { title: t('devices.colDevice'), key: 'device' },
  { title: t('devices.colKind'), key: 'kind', width: 130 },
  { title: t('devices.binding'), key: 'binding', width: 130 },
  { title: t('devices.colStatus'), key: 'online', width: 160 },
])

function bindingOf(d: Device) {
  return d.accounts.find((a) => a.user_id === id)
}

onMounted(async () => {
  crumb.value = ''
  await loadUser()
  if (!notFound.value) {
    loadDevices()
    loadAi()
  }
})
onBeforeUnmount(() => (crumb.value = ''))
</script>

<template>
  <div class="page">
    <a-result v-if="notFound" status="404" :title="t('userDetail.notFound')">
      <template #extra>
        <a-button @click="router.push('/admin/users')">{{ t('common.back') }}</a-button>
      </template>
    </a-result>

    <template v-else>
      <div class="page-head">
        <a-button type="text" class="back" :aria-label="t('common.back')" @click="router.push('/admin/users')"><ArrowLeftOutlined /></a-button>
        <template v-if="user">
          <span class="avatar big" :class="{ off: user.disabled }">{{ initials(user.display_name || user.username) }}</span>
          <div class="title-block">
            <h1>{{ user.display_name || user.username }}</h1>
            <div class="sub mono">{{ user.username }}</div>
          </div>
          <div class="tag-row">
            <StatusBadge v-for="s in userStatusTags(user)" :key="s.text" :tone="s.tone" :text="s.text" />
          </div>
          <span class="spacer" />
          <a-button @click="formOpen = true">
            <template #icon><EditOutlined /></template>
            {{ t('common.edit') }}
          </a-button>
          <a-dropdown :trigger="['click']" placement="bottomRight" :menu="{ items: userMenuItems(user) }" @menu-click="onMore">
            <a-button>
              <template #icon><MoreOutlined /></template>
              {{ t('common.more') }}
            </a-button>
          </a-dropdown>
        </template>
        <a-skeleton v-else active :paragraph="false" :title="{ width: 240 }" />
      </div>

      <div class="grid">
        <section class="panel">
          <div class="panel-head">{{ t('userDetail.basic') }}</div>
          <div class="panel-body">
            <dl v-if="user" class="kv">
              <dt>{{ t('userDetail.username') }}</dt>
              <dd class="mono">{{ user.username }}</dd>
              <dt>{{ t('userDetail.displayName') }}</dt>
              <dd>{{ user.display_name || '—' }}</dd>
              <dt>{{ t('userDetail.note') }}</dt>
              <dd class="pre">{{ user.note || '—' }}</dd>
              <dt>{{ t('userDetail.initialized') }}</dt>
              <dd>
                <StatusBadge
                  :tone="user.account_initialized ? 'success' : 'neutral'"
                  :text="user.account_initialized ? t('userDetail.initializedYes') : t('userDetail.initializedNo')"
                />
              </dd>
              <dt>
                {{ t('userDetail.epoch') }}
                <InfoTip :text="t('userDetail.epochTip')" />
              </dt>
              <dd class="mono">{{ user.account_epoch }}</dd>
              <dt>{{ t('userDetail.totp') }}</dt>
              <dd>
                <StatusBadge :tone="user.totp_enabled ? 'success' : 'neutral'" :text="user.totp_enabled ? t('userDetail.totpOn') : t('userDetail.totpOff')" />
                <span v-if="user.require_totp" class="muted small">{{ t('userDetail.requireTotp') }}</span>
              </dd>
              <dt>{{ t('userDetail.lastLogin') }}</dt>
              <dd>
                <template v-if="user.last_login_at">{{ dateTime(user.last_login_at, true) }} <span class="muted small">{{ fromNow(user.last_login_at) }}</span></template>
                <span v-else class="muted">{{ t('common.never') }}</span>
              </dd>
              <dt>{{ t('userDetail.createdAt') }}</dt>
              <dd>{{ dateTime(user.created_at, true) }}</dd>
            </dl>
            <a-skeleton v-else active />
          </div>
        </section>

        <section class="panel">
          <div class="panel-head">
            {{ t('userDetail.ai') }}
            <span class="spacer" />
            <template v-if="aiDirty">
              <span class="dirty">{{ t('common.unsavedChanges') }}</span>
              <a-button size="small" @click="resetAiForm">{{ t('common.revert') }}</a-button>
            </template>
            <a-button size="small" type="primary" :disabled="!aiDirty" :loading="aiBusy" @click="saveAi">{{ t('common.save') }}</a-button>
          </div>
          <div class="panel-body">
            <div class="ai-form">
              <div class="row">
                <label>{{ t('userDetail.aiEnabled') }}</label>
                <div class="switch-cell"><a-switch v-model:checked="ai.enabled" /></div>
              </div>
              <div class="row">
                <label>
                  {{ t('userDetail.aiMaxMode') }}
                  <InfoTip :text="t('users.form.aiMaxModeTip')" :width="360" />
                </label>
                <div>
                  <a-select v-model:value="ai.mode" :options="modeOptions" :disabled="!ai.enabled" style="width: 220px" />
                  <div class="hint">{{ aiModeDesc(ai.mode) }}</div>
                </div>
              </div>
              <div class="row">
                <label>
                  {{ t('userDetail.dailyLimit') }}
                  <InfoTip :text="t('userDetail.limitTip')" />
                </label>
                <a-space-compact style="width: 220px">
                  <a-input-number v-model:value="ai.daily" :min="0" :step="10000" :formatter="groupDigits" :parser="ungroupDigits" :disabled="!ai.enabled" style="width: 100%" />
                  <a-space-addon>{{ t('units.tokens') }}</a-space-addon>
                </a-space-compact>
              </div>
              <div class="row">
                <label>
                  {{ t('userDetail.monthlyLimit') }}
                  <InfoTip :text="t('userDetail.limitTip')" />
                </label>
                <a-space-compact style="width: 220px">
                  <a-input-number v-model:value="ai.monthly" :min="0" :step="100000" :formatter="groupDigits" :parser="ungroupDigits" :disabled="!ai.enabled" style="width: 100%" />
                  <a-space-addon>{{ t('units.tokens') }}</a-space-addon>
                </a-space-compact>
              </div>
            </div>

            <div class="grants-head">
              {{ t('userDetail.grants') }}
              <InfoTip :text="t('userDetail.grantsTip')" :width="340" />
            </div>
            <div v-if="!providers.length" class="no-prov">
              <span class="muted">{{ t('userDetail.noProviders') }}</span>
              <a-button type="link" size="small" @click="router.push('/admin/ai/providers')">{{ t('userDetail.goProviders') }}</a-button>
            </div>
            <div v-else class="grants" :class="{ dim: !ai.enabled }">
              <div v-for="row in grantRows" :key="row.provider.id" class="grant">
                <div class="grant-top">
                  <a-checkbox v-model:checked="row.on" :disabled="!ai.enabled">
                    <span class="pname">{{ row.provider.name }}</span>
                  </a-checkbox>
                  <a-tag :bordered="false">{{ providerKindLabel(row.provider.kind) }}</a-tag>
                  <StatusBadge v-if="!row.provider.enabled" tone="neutral" :text="t('userDetail.providerDisabled')" />
                  <span class="spacer" />
                  <a-segmented
                    v-if="row.on"
                    v-model:value="row.all"
                    size="small"
                    :disabled="!ai.enabled"
                    :options="[
                      { value: true, label: t('userDetail.allModels') },
                      { value: false, label: t('userDetail.someModels') },
                    ]"
                  />
                </div>
                <a-checkbox-group v-if="row.on && !row.all" v-model:value="row.models" :disabled="!ai.enabled" class="models">
                  <a-checkbox v-for="m in modelOptions(row.provider)" :key="m.value" :value="m.value">
                    <span class="mono">{{ m.value }}</span>
                    <span v-if="m.off" class="off-tag">{{ t('userDetail.modelDisabled') }}</span>
                  </a-checkbox>
                </a-checkbox-group>
              </div>
            </div>
          </div>
        </section>
      </div>

      <section class="panel devices">
        <div class="panel-head">
          {{ t('userDetail.devices') }}
          <span class="count muted">{{ num(devicesTotal) }}</span>
          <span class="spacer" />
          <a-button type="link" size="small" @click="router.push({ path: '/admin/devices', query: { user_id: id } })">
            {{ t('userDetail.viewInDevices') }}
          </a-button>
        </div>
        <a-table
          :columns="deviceColumns"
          :data-source="devices"
          row-key="id"
          :loading="devicesLoading"
          :pagination="false"
          :scroll="{ x: 720 }"
          :locale="{ emptyText: t('userDetail.noDevices') }"
        >
          <template #bodyCell="{ column, record }">
            <template v-if="column.key === 'device'">
              <div class="cell-main">
                <PlatformIcon :platform="record.platform" boxed />
                <div class="min0">
                  <div class="name">{{ record.hostname }}</div>
                  <div class="sub">{{ record.os_version }} · {{ record.arch }}</div>
                </div>
              </div>
            </template>
            <template v-else-if="column.key === 'kind'">
              <a-tag :bordered="false">{{ deviceKindLabel(record.kind) }}</a-tag>
            </template>
            <template v-else-if="column.key === 'binding'">
              <StatusBadge
                v-if="bindingOf(record as Device)"
                :tone="accountStatusTone[bindingOf(record as Device)!.status]"
                :text="accountStatusLabel(bindingOf(record as Device)!.status)"
              />
            </template>
            <template v-else-if="column.key === 'online'">
              <StatusBadge v-if="record.online" tone="success" :text="t('devices.online')" />
              <span v-else class="muted">{{ t('devices.lastSeen', { time: fromNow(record.last_seen_at) }) }}</span>
            </template>
          </template>
        </a-table>
      </section>

      <UserFormModal v-model:open="formOpen" :user="user" @saved="onChanged" />
      <UserActionsHost ref="host" @changed="onChanged" @refresh="loadUser" @deleted="router.replace('/admin/users')" />
    </template>
  </div>
</template>

<style scoped>
.back {
  margin-left: -6px;
}
.avatar.big {
  width: 40px;
  height: 40px;
  border-radius: 10px;
  font-size: 14px;
}
.title-block {
  display: flex;
  flex-direction: column;
  margin-right: 6px;
}
.title-block .sub {
  color: var(--lg-text-3);
}
.grid {
  display: grid;
  grid-template-columns: minmax(0, 5fr) minmax(0, 7fr);
  gap: 16px;
  align-items: start;
}
.grid .panel + .panel {
  margin-top: 0;
}
.devices {
  margin-top: 16px;
}
.kv {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  gap: 12px 20px;
  margin: 0;
}
.kv dt {
  color: var(--lg-text-3);
  font-size: 13px;
  white-space: nowrap;
}
.kv dd {
  margin: 0;
  min-width: 0;
  overflow-wrap: anywhere;
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
}
.pre {
  white-space: pre-wrap;
}
.small {
  font-size: 12px;
}
.dirty {
  font-size: 12.5px;
  font-weight: 500;
  color: var(--lg-warning);
  margin-right: 4px;
}
.ai-form {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.ai-form .row {
  display: grid;
  grid-template-columns: 150px minmax(0, 1fr);
  align-items: start;
  gap: 12px;
}
.ai-form label {
  padding-top: 6px;
  color: var(--lg-text-2);
  font-size: 13.5px;
}
.ai-form .switch-cell {
  padding-top: 5px;
}
.hint {
  margin-top: 6px;
  font-size: 12.5px;
  color: var(--lg-text-3);
}
.grants-head {
  display: flex;
  align-items: center;
  gap: 4px;
  margin: 22px 0 10px;
  padding-top: 16px;
  border-top: 1px solid var(--lg-border-soft);
  font-weight: 600;
  font-size: 13.5px;
}
.no-prov {
  display: flex;
  align-items: center;
  gap: 4px;
}
.grants {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.grants.dim {
  opacity: 0.6;
}
.grant {
  border: 1px solid var(--lg-border-soft);
  border-radius: 8px;
  padding: 10px 12px;
  background: var(--lg-surface-2);
}
.grant-top {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.grant-top .spacer {
  flex: 1;
}
.pname {
  font-weight: 560;
}
.models {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: 6px 12px;
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px dashed var(--lg-border);
}
.off-tag {
  margin-left: 6px;
  font-size: 11.5px;
  padding: 0 5px;
  border-radius: 4px;
  background: var(--lg-neutral-bg);
  color: var(--lg-text-3);
}
.count {
  font-weight: 500;
  font-size: 13px;
}
.min0 {
  min-width: 0;
}
@media (max-width: 1200px) {
  .grid {
    grid-template-columns: 1fr;
  }
}
</style>
