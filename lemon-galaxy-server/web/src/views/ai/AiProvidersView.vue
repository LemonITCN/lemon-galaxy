<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { DeleteOutlined, EditOutlined, ExperimentOutlined, PlusOutlined, RobotOutlined, SyncOutlined } from '@antdv-next/icons'
import * as api from '@/api'
import type { AiModel, AiProvider } from '@/api'
import StatusBadge from '@/components/StatusBadge.vue'
import { t } from '@/locales'
import { toast } from '@/utils/feedback'
import { contextLength, fromNow } from '@/utils/format'
import { providerKindLabel } from '@/utils/labels'
import ProviderFormModal from './ProviderFormModal.vue'
import ProviderTestModal from './ProviderTestModal.vue'

const providers = ref<AiProvider[]>([])
const loading = ref(false)
const busy = ref<Record<string, boolean>>({})

async function load() {
  loading.value = true
  try {
    providers.value = (await api.listProviders()).items
  } catch {
    /* 已统一提示 */
  } finally {
    loading.value = false
  }
}
onMounted(load)

function replace(p: AiProvider) {
  const i = providers.value.findIndex((x) => x.id === p.id)
  if (i >= 0) providers.value[i] = p
  else providers.value.push(p)
}

async function withBusy(key: string, fn: () => Promise<void>) {
  busy.value = { ...busy.value, [key]: true }
  try {
    await fn()
  } catch {
    /* 已统一提示 */
  } finally {
    busy.value = { ...busy.value, [key]: false }
  }
}

const toggleProvider = (p: AiProvider, enabled: boolean) =>
  withBusy('en' + p.id, async () => replace(await api.updateProvider(p.id, { enabled })))

const refresh = (p: AiProvider) =>
  withBusy('rf' + p.id, async () => {
    replace(await api.refreshProviderModels(p.id))
    toast.success(t('aiProviders.refreshed'))
  })

const toggleModel = (p: AiProvider, m: AiModel, enabled: boolean) =>
  withBusy(`m${p.id}:${m.id}`, async () => {
    const items = p.models.map((x) => ({ id: x.id, enabled: x.id === m.id ? enabled : x.enabled }))
    replace(await api.setProviderModels(p.id, items))
  })

const remove = (p: AiProvider) =>
  withBusy('rm' + p.id, async () => {
    await api.deleteProvider(p.id)
    providers.value = providers.value.filter((x) => x.id !== p.id)
    toast.success(t('common.deleted'))
  })

// 弹窗
const formOpen = ref(false)
const editing = ref<AiProvider | null>(null)
const testOpen = ref(false)
const testing = ref<AiProvider | null>(null)

function openCreate() {
  editing.value = null
  formOpen.value = true
}
function openEdit(p: AiProvider) {
  editing.value = p
  formOpen.value = true
}
function openTest(p: AiProvider) {
  testing.value = p
  testOpen.value = true
}

const modelColumns = computed(() => [
  { title: t('aiProviders.colModel'), key: 'model' },
  { title: t('aiProviders.colCaps'), key: 'caps', width: 200 },
  { title: t('aiProviders.colContext'), key: 'ctx', width: 110, align: 'right' as const },
  { title: t('aiProviders.colEnabled'), key: 'enabled', width: 90, align: 'center' as const },
])

const enabledCount = (p: AiProvider) => p.models.filter((m) => m.enabled).length
</script>

<template>
  <div class="page">
    <div class="page-head">
      <h1>{{ t('aiProviders.title') }}</h1>
      <span class="count">{{ providers.length }}</span>
      <span class="spacer" />
      <a-button type="primary" @click="openCreate">
        <template #icon><PlusOutlined /></template>
        {{ t('aiProviders.create') }}
      </a-button>
    </div>

    <div v-if="loading && !providers.length" class="panel loading"><a-spin /></div>

    <div v-else-if="!providers.length" class="panel empty">
      <a-empty :description="t('aiProviders.empty')">
        <a-button type="primary" @click="openCreate">
          <template #icon><PlusOutlined /></template>
          {{ t('aiProviders.create') }}
        </a-button>
      </a-empty>
    </div>

    <section v-for="p in providers" :key="p.id" class="panel provider" :class="{ off: !p.enabled }">
      <div class="prov-head">
        <span class="prov-icon"><RobotOutlined /></span>
        <div class="prov-title">
          <div class="prov-name">
            {{ p.name }}
            <a-tag :bordered="false">{{ providerKindLabel(p.kind) }}</a-tag>
          </div>
          <div class="prov-meta">
            <span class="mono">{{ p.base_url }}</span>
            <span class="dot-sep" />
            <span>{{ t('aiProviders.apiKey') }}</span>
            <StatusBadge :tone="p.has_api_key ? 'success' : 'danger'" :text="p.has_api_key ? t('aiProviders.apiKeySet') : t('aiProviders.apiKeyMissing')" />
            <template v-if="p.models_updated_at">
              <span class="dot-sep" />
              <span>{{ t('aiProviders.modelsUpdated', { time: fromNow(p.models_updated_at) }) }}</span>
            </template>
          </div>
        </div>
        <span class="spacer" />
        <a-switch :checked="p.enabled" :loading="busy['en' + p.id]" @change="(v: any) => toggleProvider(p, !!v)" />
        <a-button size="small" :loading="busy['rf' + p.id]" @click="refresh(p)">
          <template #icon><SyncOutlined /></template>
          {{ t('aiProviders.refreshModels') }}
        </a-button>
        <a-button size="small" :disabled="!enabledCount(p)" @click="openTest(p)">
          <template #icon><ExperimentOutlined /></template>
          {{ t('aiProviders.test') }}
        </a-button>
        <a-tooltip :title="t('common.edit')">
          <a-button size="small" :aria-label="t('common.edit')" @click="openEdit(p)"><EditOutlined /></a-button>
        </a-tooltip>
        <a-popconfirm
          :title="t('aiProviders.confirmDelete', { name: p.name })"
          :ok-text="t('common.delete')"
          :ok-button-props="{ danger: true }"
          placement="topRight"
          @confirm="remove(p)"
        >
          <a-tooltip :title="t('common.delete')">
            <a-button size="small" danger :loading="busy['rm' + p.id]" :aria-label="t('common.delete')"><DeleteOutlined /></a-button>
          </a-tooltip>
        </a-popconfirm>
      </div>
      <div class="models-bar muted">{{ t('aiProviders.modelsCount', { enabled: enabledCount(p), total: p.models.length }) }}</div>
      <a-table
        :columns="modelColumns"
        :data-source="p.models"
        row-key="id"
        size="small"
        :pagination="false"
        :scroll="{ x: 640 }"
        :locale="{ emptyText: t('aiProviders.noModels') }"
      >
        <template #bodyCell="{ column, record }">
          <template v-if="column.key === 'model'">
            <div class="model">
              <span class="mname">{{ record.name || record.id }}</span>
              <span v-if="record.name && record.name !== record.id" class="mono muted">{{ record.id }}</span>
            </div>
          </template>
          <template v-else-if="column.key === 'caps'">
            <div class="tag-row">
              <StatusBadge v-if="record.vision" tone="info" :text="t('aiProviders.capVision')" :dot="false" />
              <StatusBadge v-if="record.tools" tone="brand" :text="t('aiProviders.capTools')" :dot="false" />
              <span v-if="!record.vision && !record.tools" class="muted">—</span>
            </div>
          </template>
          <template v-else-if="column.key === 'ctx'">
            <span class="mono">{{ contextLength(record.context_length) }}</span>
          </template>
          <template v-else-if="column.key === 'enabled'">
            <a-switch
              size="small"
              :checked="record.enabled"
              :loading="busy[`m${p.id}:${record.id}`]"
              @change="(v: any) => toggleModel(p, record as AiModel, !!v)"
            />
          </template>
        </template>
      </a-table>
    </section>

    <ProviderFormModal v-model:open="formOpen" :provider="editing" @saved="replace" />
    <ProviderTestModal v-model:open="testOpen" :provider="testing" />
  </div>
</template>

<style scoped>
.loading,
.empty {
  display: flex;
  justify-content: center;
  padding: 48px 0;
}
.provider {
  overflow: hidden;
}
.provider.off .prov-icon {
  background: var(--lg-neutral-bg);
  color: var(--lg-text-3);
}
.prov-head {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  padding: 14px 16px;
}
.prov-icon {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: 9px;
  background: var(--lg-brand-bg);
  color: var(--lg-brand-text);
  font-size: 17px;
  margin-right: 4px;
}
.prov-title {
  min-width: 0;
}
.prov-name {
  display: flex;
  align-items: center;
  gap: 8px;
  font-weight: 600;
  font-size: 15px;
}
.prov-meta {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 3px;
  font-size: 12.5px;
  color: var(--lg-text-3);
}
.dot-sep {
  width: 3px;
  height: 3px;
  border-radius: 50%;
  background: var(--lg-text-3);
  opacity: 0.6;
  margin: 0 2px;
}
.spacer {
  flex: 1;
}
.models-bar {
  padding: 8px 16px;
  font-size: 12.5px;
  border-top: 1px solid var(--lg-border-soft);
  background: var(--lg-surface-2);
}
.model {
  display: flex;
  align-items: baseline;
  gap: 10px;
  min-width: 0;
}
.mname {
  font-weight: 550;
  white-space: nowrap;
}
</style>
