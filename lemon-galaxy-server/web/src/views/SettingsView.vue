<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { onBeforeRouteLeave } from 'vue-router'
import * as api from '@/api'
import type { Settings } from '@/api'
import InfoTip from '@/components/InfoTip.vue'
import { t } from '@/locales'
import { confirm, toast } from '@/utils/feedback'
import { validIpOrCidr } from '@/utils/format'
import { updateSourceLabel } from '@/utils/labels'

const saved = ref<Settings | null>(null)
const form = ref<Settings | null>(null)
const loading = ref(false)
const busy = ref(false)

async function load() {
  loading.value = true
  try {
    const s = await api.getSettings()
    saved.value = s
    form.value = structuredClone(s)
  } catch {
    /* 已统一提示 */
  } finally {
    loading.value = false
  }
}
onMounted(load)

/** 只提交改过的字段 */
const changes = computed<Partial<Settings>>(() => {
  const a = saved.value
  const b = form.value
  if (!a || !b) return {}
  const out: Record<string, unknown> = {}
  for (const k of Object.keys(b) as (keyof Settings)[]) {
    if (JSON.stringify(a[k]) !== JSON.stringify(b[k])) out[k] = b[k]
  }
  return out as Partial<Settings>
})
const dirty = computed(() => Object.keys(changes.value).length > 0)

const badIps = computed(() => (form.value?.admin_ip_allowlist ?? []).filter((s) => !validIpOrCidr(s)))

function revert() {
  if (saved.value) form.value = structuredClone(saved.value)
}

async function save() {
  if (!form.value || !dirty.value) return
  if (badIps.value.length) {
    toast.error(t('validation.ipOrCidr', { value: badIps.value[0] }))
    return
  }
  const body = { ...changes.value }
  for (const k of ['relay_session_kbps', 'relay_total_mbps', 'login_lockout_threshold', 'login_lockout_minutes', 'session_idle_days', 'audit_retention_days', 'update_check_hours'] as const) {
    if (k in body) (body as Record<string, unknown>)[k] = Math.max(0, Math.floor(Number(body[k]) || 0))
  }
  if (body.server_name !== undefined) body.server_name = body.server_name.trim()
  if (body.public_host !== undefined) body.public_host = body.public_host.trim()
  if (body.update_github_repo !== undefined) body.update_github_repo = body.update_github_repo.trim()

  // 白名单从空变成有内容：提醒确认包含自己当前的网络
  if (body.admin_ip_allowlist && body.admin_ip_allowlist.length && !(saved.value?.admin_ip_allowlist.length ?? 0)) {
    const ok = await confirm({ title: t('settings.allowlistConfirmTitle'), content: t('settings.allowlistConfirmDesc'), okText: t('common.save') })
    if (!ok) return
  }

  busy.value = true
  try {
    const s = await api.updateSettings(body)
    saved.value = s
    form.value = structuredClone(s)
    toast.success(t('common.saved'))
  } catch {
    /* 已统一提示 */
  } finally {
    busy.value = false
  }
}

onBeforeRouteLeave(async () => {
  if (!dirty.value) return true
  return confirm({ title: t('common.unsavedChanges'), okText: t('common.ok') })
})

const sourceOptions = computed(() => (['github', 'manual'] as const).map((s) => ({ value: s, label: updateSourceLabel(s) })))
</script>

<template>
  <div class="page settings">
    <div class="page-head">
      <h1>{{ t('settings.title') }}</h1>
    </div>

    <div v-if="!form" class="panel loading">
      <a-spin v-if="loading" />
      <a-button v-else @click="load">{{ t('common.retry') }}</a-button>
    </div>

    <template v-else>
      <section class="panel">
        <div class="panel-head">{{ t('settings.groupBasic') }}</div>
        <div class="rows">
          <div class="row">
            <label>{{ t('settings.serverName') }} <InfoTip :text="t('settings.serverNameTip')" /></label>
            <a-input v-model:value="form.server_name" :maxlength="64" style="width: 320px" />
          </div>
          <div class="row">
            <label>{{ t('settings.publicHost') }} <InfoTip :text="t('settings.publicHostTip')" :width="340" /></label>
            <a-input v-model:value="form.public_host" class="mono-input" placeholder="galaxy.example.com" style="width: 320px" />
          </div>
        </div>
      </section>

      <section class="panel">
        <div class="panel-head">{{ t('settings.groupRelay') }}</div>
        <div class="rows">
          <div class="row">
            <label>{{ t('settings.relayEnabled') }} <InfoTip :text="t('settings.relayEnabledTip')" :width="340" /></label>
            <div class="switch-cell"><a-switch v-model:checked="form.relay_enabled" /></div>
          </div>
          <div class="row">
            <label>{{ t('settings.relaySessionKbps') }} <InfoTip :text="t('settings.zeroUnlimited')" /></label>
            <a-space-compact style="width: 220px">
              <a-input-number v-model:value="form.relay_session_kbps" :min="0" :step="512" :disabled="!form.relay_enabled" style="width: 100%" />
              <a-space-addon>{{ t('units.kbps') }}</a-space-addon>
            </a-space-compact>
            <span v-if="!form.relay_session_kbps" class="hint">{{ t('common.unlimited') }}</span>
          </div>
          <div class="row">
            <label>{{ t('settings.relayTotalMbps') }} <InfoTip :text="t('settings.zeroUnlimited')" /></label>
            <a-space-compact style="width: 220px">
              <a-input-number v-model:value="form.relay_total_mbps" :min="0" :step="10" :disabled="!form.relay_enabled" style="width: 100%" />
              <a-space-addon>{{ t('units.mbps') }}</a-space-addon>
            </a-space-compact>
            <span v-if="!form.relay_total_mbps" class="hint">{{ t('common.unlimited') }}</span>
          </div>
        </div>
      </section>

      <section class="panel">
        <div class="panel-head">{{ t('settings.groupSecurity') }}</div>
        <div class="rows">
          <div class="row">
            <label>{{ t('settings.lockoutThreshold') }} <InfoTip :text="t('settings.lockoutThresholdTip')" /></label>
            <a-space-compact style="width: 220px">
              <a-input-number v-model:value="form.login_lockout_threshold" :min="1" :max="100" style="width: 100%" />
              <a-space-addon>{{ t('units.times') }}</a-space-addon>
            </a-space-compact>
          </div>
          <div class="row">
            <label>{{ t('settings.lockoutMinutes') }}</label>
            <a-space-compact style="width: 220px">
              <a-input-number v-model:value="form.login_lockout_minutes" :min="1" style="width: 100%" />
              <a-space-addon>{{ t('units.minutes') }}</a-space-addon>
            </a-space-compact>
          </div>
          <div class="row">
            <label>{{ t('settings.sessionIdleDays') }} <InfoTip :text="t('settings.sessionIdleDaysTip')" /></label>
            <a-space-compact style="width: 220px">
              <a-input-number v-model:value="form.session_idle_days" :min="1" style="width: 100%" />
              <a-space-addon>{{ t('units.days') }}</a-space-addon>
            </a-space-compact>
          </div>
          <div class="row">
            <label>{{ t('settings.auditRetentionDays') }}</label>
            <a-space-compact style="width: 220px">
              <a-input-number v-model:value="form.audit_retention_days" :min="1" style="width: 100%" />
              <a-space-addon>{{ t('units.days') }}</a-space-addon>
            </a-space-compact>
          </div>
          <div class="row">
            <label>{{ t('settings.adminIpAllowlist') }} <InfoTip :text="t('settings.adminIpAllowlistTip')" :width="340" /></label>
            <div class="allow">
              <a-select
                v-model:value="form.admin_ip_allowlist"
                mode="tags"
                :open="false"
                :token-separators="[',', ' ', '\n']"
                :placeholder="t('settings.adminIpAllowlistPlaceholder')"
                :status="badIps.length ? 'error' : undefined"
                :suffix-icon="null"
                class="lg-mono-select"
                style="width: 100%"
              />
              <div v-if="badIps.length" class="err">{{ t('validation.ipOrCidr', { value: badIps[0] }) }}</div>
            </div>
          </div>
        </div>
      </section>

      <section class="panel">
        <div class="panel-head">{{ t('settings.groupUpdate') }}</div>
        <div class="rows">
          <div class="row">
            <label>{{ t('settings.updateSource') }} <InfoTip :text="t('settings.updateSourceTip')" :width="360" /></label>
            <div class="switch-cell">
              <a-radio-group v-model:value="form.update_source" :options="sourceOptions" option-type="button" />
            </div>
          </div>
          <div class="row">
            <label>{{ t('settings.updateGithubRepo') }}</label>
            <a-input
              v-model:value="form.update_github_repo"
              class="mono-input"
              placeholder="owner/repo"
              :disabled="form.update_source !== 'github'"
              style="width: 320px"
            />
          </div>
          <div class="row">
            <label>{{ t('settings.updateCheckHours') }}</label>
            <a-space-compact style="width: 220px">
              <a-input-number v-model:value="form.update_check_hours" :min="1" :max="720" :disabled="form.update_source !== 'github'" style="width: 100%" />
              <a-space-addon>{{ t('units.hours') }}</a-space-addon>
            </a-space-compact>
          </div>
        </div>
      </section>

      <div class="save-bar" :class="{ show: dirty }">
        <span class="dirty">{{ t('common.unsavedChanges') }}</span>
        <span class="spacer" />
        <a-button :disabled="!dirty || busy" @click="revert">{{ t('common.revert') }}</a-button>
        <a-button type="primary" :disabled="!dirty" :loading="busy" @click="save">{{ t('common.save') }}</a-button>
      </div>
    </template>
  </div>
</template>

<style scoped>
.settings {
  max-width: 1040px;
  margin: 0;
}
.loading {
  display: flex;
  justify-content: center;
  padding: 48px 0;
}
.rows {
  padding: 4px 0;
}
.row {
  display: grid;
  grid-template-columns: 240px minmax(0, 1fr);
  align-items: center;
  justify-items: start;
  gap: 0 16px;
  padding: 12px 18px;
}
.row + .row {
  border-top: 1px solid var(--lg-border-soft);
}
.row label {
  grid-column: 1;
  grid-row: 1 / span 2;
  display: flex;
  align-items: center;
  gap: 2px;
  color: var(--lg-text);
  font-size: 13.5px;
  font-weight: 500;
}
.row .hint {
  grid-column: 2;
  margin-top: 4px;
  font-size: 12px;
  color: var(--lg-text-3);
}
.row > :nth-child(2) {
  grid-column: 2;
}
.allow {
  width: 100%;
  max-width: 520px;
}
.err {
  margin-top: 4px;
  font-size: 12.5px;
  color: var(--lg-danger);
}
.mono-input {
  font-family: var(--lg-mono);
}
.save-bar {
  position: sticky;
  bottom: 16px;
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 16px;
  padding: 10px 14px 10px 18px;
  background: var(--lg-surface);
  border: 1px solid var(--lg-border);
  border-radius: 10px;
  box-shadow: 0 10px 30px rgba(16, 24, 40, 0.12);
  opacity: 0;
  transform: translateY(8px);
  pointer-events: none;
  transition:
    opacity 0.15s,
    transform 0.15s;
}
.save-bar.show {
  opacity: 1;
  transform: none;
  pointer-events: auto;
}
.dirty {
  font-size: 13px;
  font-weight: 550;
  color: var(--lg-warning);
}
.spacer {
  flex: 1;
}
@media (max-width: 900px) {
  .row {
    grid-template-columns: 1fr;
    gap: 8px;
  }
  .row label,
  .row > :nth-child(2),
  .row .hint {
    grid-column: 1;
    grid-row: auto;
  }
}
</style>
