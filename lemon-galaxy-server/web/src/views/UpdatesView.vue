<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { CloudSyncOutlined, UploadOutlined } from '@antdv-next/icons'
import * as api from '@/api'
import type { UpdateList, UpdatePackage } from '@/api'
import CopyText from '@/components/CopyText.vue'
import FilePick from '@/components/FilePick.vue'
import InfoTip from '@/components/InfoTip.vue'
import PlatformIcon from '@/components/PlatformIcon.vue'
import StatusBadge from '@/components/StatusBadge.vue'
import { t } from '@/locales'
import { toast } from '@/utils/feedback'
import { bytes, dateTime, fromNow, shortId } from '@/utils/format'
import { updateSourceLabel } from '@/utils/labels'

const data = ref<UpdateList | null>(null)
const loading = ref(false)
const checking = ref(false)

async function load() {
  loading.value = true
  try {
    data.value = await api.listUpdates()
  } catch {
    /* 已统一提示 */
  } finally {
    loading.value = false
  }
}
onMounted(load)

async function check() {
  checking.value = true
  try {
    data.value = await api.checkUpdates()
    if (!data.value.last_error) toast.success(t('updates.checked'))
  } catch {
    /* 已统一提示 */
  } finally {
    checking.value = false
  }
}

async function remove(p: UpdatePackage) {
  try {
    await api.deleteUpdate(p.id)
    toast.success(t('common.deleted'))
    if (data.value) data.value.items = data.value.items.filter((x) => x.id !== p.id)
  } catch {
    /* 已统一提示 */
  }
}

const rows = computed(() => [...(data.value?.items ?? [])].sort((a, b) => b.created_at - a.created_at))

const columns = computed(() => [
  { title: t('updates.colComponent'), key: 'component', width: 180 },
  { title: t('updates.colPlatform'), key: 'platform', width: 150 },
  { title: t('updates.colFile'), key: 'file' },
  { title: t('updates.colSha'), key: 'sha', width: 150 },
  { title: t('updates.colSignature'), key: 'sig', width: 104 },
  { title: t('updates.colCreated'), key: 'created', width: 150 },
  { title: '', key: 'actions', width: 64, align: 'right' as const, fixed: 'right' as const },
])

// 手动上传
const uploadOpen = ref(false)
const file = ref<File | null>(null)
const sig = ref<File | null>(null)
const progress = ref(0)
const uploading = ref(false)
let abort: (() => void) | null = null

function openUpload() {
  file.value = null
  sig.value = null
  progress.value = 0
  uploadOpen.value = true
}

async function doUpload() {
  if (!file.value || !sig.value) return
  uploading.value = true
  progress.value = 0
  const h = api.uploadUpdate(file.value, sig.value, (p) => (progress.value = p))
  abort = h.abort
  try {
    const p = await h.promise
    if (data.value) data.value.items = [p, ...data.value.items.filter((x) => x.id !== p.id)]
    toast.success(t('updates.uploaded'))
    uploadOpen.value = false
  } catch {
    /* 已统一提示 */
  } finally {
    uploading.value = false
    abort = null
  }
}

function cancelUpload() {
  abort?.()
  uploadOpen.value = false
}
</script>

<template>
  <div class="page">
    <div class="page-head">
      <h1>{{ t('updates.title') }}</h1>
      <span class="count">{{ rows.length }}</span>
      <span class="last muted">
        <template v-if="data?.last_check_at">
          <a-tooltip :title="dateTime(data.last_check_at, true)">{{ t('updates.lastCheck', { time: fromNow(data.last_check_at) }) }}</a-tooltip>
        </template>
        <template v-else-if="data">{{ t('updates.neverChecked') }}</template>
      </span>
      <span class="spacer" />
      <a-button :loading="checking" @click="check">
        <template #icon><CloudSyncOutlined /></template>
        {{ t('updates.check') }}
      </a-button>
      <a-button type="primary" @click="openUpload">
        <template #icon><UploadOutlined /></template>
        {{ t('updates.upload') }}
      </a-button>
    </div>

    <a-alert v-if="data?.last_error" type="error" show-icon :title="t('updates.lastError')" :description="data.last_error" class="err" />

    <div class="panel">
      <a-table
        :columns="columns"
        :data-source="rows"
        row-key="id"
        :loading="loading"
        :pagination="{ pageSize: 50, hideOnSinglePage: true }"
        :scroll="{ x: 990 }"
        :locale="{ emptyText: t('updates.empty') }"
      >
        <template #bodyCell="{ column, record }">
          <template v-if="column.key === 'component'">
            <div class="mono strong">{{ record.component }}</div>
            <div class="sub mono">{{ record.version }}</div>
          </template>
          <template v-else-if="column.key === 'platform'">
            <PlatformIcon :platform="record.platform" label />
            <div class="sub mono">{{ record.arch }}</div>
          </template>
          <template v-else-if="column.key === 'file'">
            <div class="mono file" :title="record.file_name">{{ record.file_name }}</div>
            <div class="sub">{{ bytes(record.size) }}</div>
          </template>
          <template v-else-if="column.key === 'sha'">
            <CopyText :text="record.sha256" :display="shortId(record.sha256, 10, 6)" mono />
          </template>
          <template v-else-if="column.key === 'sig'">
            <StatusBadge :tone="record.signature_ok ? 'success' : 'danger'" :text="record.signature_ok ? t('updates.sigOk') : t('updates.sigBad')" />
          </template>
          <template v-else-if="column.key === 'created'">
            <div class="secondary nowrap">{{ dateTime(record.created_at) }}</div>
            <div class="sub">{{ updateSourceLabel(record.source) }}</div>
          </template>
          <template v-else-if="column.key === 'actions'">
            <a-popconfirm :title="t('updates.confirmDelete')" :ok-text="t('common.delete')" :ok-button-props="{ danger: true }" placement="topRight" @confirm="remove(record as UpdatePackage)">
              <a-button type="link" size="small" danger>{{ t('common.delete') }}</a-button>
            </a-popconfirm>
          </template>
        </template>
      </a-table>
    </div>

    <a-modal
      v-model:open="uploadOpen"
      :width="520"
      :mask-closable="!uploading"
      :ok-text="t('updates.uploadOk')"
      :ok-button-props="{ disabled: !file || !sig }"
      :confirm-loading="uploading"
      destroy-on-hidden
      @ok="doUpload"
      @cancel="cancelUpload"
    >
      <template #title>
        {{ t('updates.uploadTitle') }}
        <InfoTip :text="t('updates.uploadTip')" :width="340" />
      </template>
      <div class="up">
        <div class="up-row">
          <div class="up-label">{{ t('updates.file') }}</div>
          <FilePick v-model:file="file" :button-text="t('updates.chooseFile')" />
        </div>
        <div class="up-row">
          <div class="up-label">{{ t('updates.signature') }}</div>
          <FilePick v-model:file="sig" accept=".sig,.minisig,.sign" :button-text="t('updates.chooseFile')" />
        </div>
        <a-progress v-if="uploading" :percent="progress" size="small" />
      </div>
    </a-modal>
  </div>
</template>

<style scoped>
.last {
  font-size: 12.5px;
  margin-left: 4px;
}
.err {
  margin-bottom: 16px;
}
.strong {
  font-weight: 600;
}
.sub {
  margin-top: 2px;
  font-size: 12px;
  color: var(--lg-text-3);
  white-space: nowrap;
}
.file {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.up {
  display: flex;
  flex-direction: column;
  gap: 16px;
  margin-top: 12px;
}
.up-row {
  display: grid;
  grid-template-columns: 90px minmax(0, 1fr);
  align-items: center;
  gap: 12px;
}
.up-label {
  color: var(--lg-text-2);
  font-size: 13.5px;
}
</style>
