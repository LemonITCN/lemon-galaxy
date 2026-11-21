<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { PlusOutlined } from '@antdv-next/icons'
import * as api from '@/api'
import type { AdminUser } from '@/api'
import InfoTip from '@/components/InfoTip.vue'
import PasswordField from '@/components/PasswordField.vue'
import StatusBadge from '@/components/StatusBadge.vue'
import { t } from '@/locales'
import { session } from '@/stores/session'
import { toast } from '@/utils/feedback'
import { dateTime, fromNow, initials, validUsername } from '@/utils/format'

const rows = ref<AdminUser[]>([])
const loading = ref(false)

async function load() {
  loading.value = true
  try {
    rows.value = (await api.listAdmins()).items
  } catch {
    /* 已统一提示 */
  } finally {
    loading.value = false
  }
}
onMounted(load)

const columns = computed(() => [
  { title: t('admins.colAdmin'), key: 'who' },
  { title: t('admins.colTotp'), key: 'totp', width: 140 },
  { title: t('admins.colLastLogin'), key: 'login', width: 180 },
  { title: t('admins.colCreated'), key: 'created', width: 170 },
  { title: '', key: 'actions', width: 220, align: 'right' as const },
])

const isSelf = (a: AdminUser) => a.id === session.admin?.id

async function resetTotp(a: AdminUser) {
  try {
    await api.resetAdminTotp(a.id)
    toast.success(t('admins.resetTotpDone'))
    load()
  } catch {
    /* 已统一提示 */
  }
}

async function remove(a: AdminUser) {
  try {
    await api.deleteAdmin(a.id)
    toast.success(t('common.deleted'))
    rows.value = rows.value.filter((x) => x.id !== a.id)
  } catch {
    /* 已统一提示 */
  }
}

// 新建
const form = reactive({ open: false, username: '', password: '', busy: false, errUser: '', errPw: '' })
const MIN = 10
function openCreate() {
  Object.assign(form, { open: true, username: '', password: '', errUser: '', errPw: '' })
}
async function create() {
  form.errUser = validUsername(form.username.trim()) ? '' : t('validation.username')
  form.errPw = form.password.length >= MIN ? '' : t('validation.passwordShort', { n: MIN })
  if (form.errUser || form.errPw) return
  form.busy = true
  try {
    const a = await api.createAdmin(form.username.trim(), form.password)
    rows.value = [...rows.value, a]
    toast.success(t('common.created'))
    form.open = false
  } catch {
    /* 已统一提示 */
  } finally {
    form.busy = false
  }
}
</script>

<template>
  <div class="page">
    <div class="page-head">
      <h1>{{ t('admins.title') }}</h1>
      <span class="count">{{ rows.length }}</span>
      <span class="spacer" />
      <a-button type="primary" @click="openCreate">
        <template #icon><PlusOutlined /></template>
        {{ t('admins.create') }}
      </a-button>
    </div>
    <div class="panel">
      <a-table :columns="columns" :data-source="rows" row-key="id" :loading="loading" :pagination="false" :scroll="{ x: 860 }">
        <template #bodyCell="{ column, record }">
          <template v-if="column.key === 'who'">
            <div class="cell-main">
              <span class="avatar">{{ initials(record.username) }}</span>
              <span class="name mono">{{ record.username }}</span>
              <a-tag v-if="isSelf(record as AdminUser)" :bordered="false" color="success">{{ t('common.me') }}</a-tag>
            </div>
          </template>
          <template v-else-if="column.key === 'totp'">
            <StatusBadge :tone="record.totp_enabled ? 'success' : 'warning'" :text="record.totp_enabled ? t('admins.totpBound') : t('admins.totpPending')" />
          </template>
          <template v-else-if="column.key === 'login'">
            <a-tooltip v-if="record.last_login_at" :title="dateTime(record.last_login_at, true)">
              <span class="secondary">{{ fromNow(record.last_login_at) }}</span>
            </a-tooltip>
            <span v-else class="muted">{{ t('common.never') }}</span>
          </template>
          <template v-else-if="column.key === 'created'">
            <span class="secondary">{{ dateTime(record.created_at) }}</span>
          </template>
          <template v-else-if="column.key === 'actions'">
            <a-popconfirm
              v-if="record.totp_enabled"
              :title="t('admins.confirmResetTotp', { name: record.username })"
              :ok-text="t('admins.resetTotp')"
              :ok-button-props="{ danger: true }"
              placement="topRight"
              @confirm="resetTotp(record as AdminUser)"
            >
              <a-button type="link" size="small">{{ t('admins.resetTotp') }}</a-button>
            </a-popconfirm>
            <a-tooltip :title="isSelf(record as AdminUser) ? t('admins.cannotDeleteSelf') : undefined">
              <a-popconfirm
                :disabled="isSelf(record as AdminUser)"
                :title="t('admins.confirmDelete', { name: record.username })"
                :ok-text="t('common.delete')"
                :ok-button-props="{ danger: true }"
                placement="topRight"
                @confirm="remove(record as AdminUser)"
              >
                <a-button type="link" size="small" danger :disabled="isSelf(record as AdminUser)">{{ t('common.delete') }}</a-button>
              </a-popconfirm>
            </a-tooltip>
          </template>
        </template>
      </a-table>
    </div>

    <a-modal v-model:open="form.open" :title="t('admins.formTitle')" :width="440" :ok-text="t('common.create')" :confirm-loading="form.busy" destroy-on-hidden @ok="create">
      <a-form layout="vertical" class="form">
        <a-form-item :label="t('admins.username')" required :validate-status="form.errUser ? 'error' : undefined" :help="form.errUser || undefined">
          <a-input v-model:value="form.username" :maxlength="32" autocomplete="off" />
        </a-form-item>
        <a-form-item required class="last" :validate-status="form.errPw ? 'error' : undefined" :help="form.errPw || undefined">
          <template #label>
            {{ t('admins.password') }}
            <InfoTip :text="t('admins.passwordTip')" />
          </template>
          <PasswordField v-model:value="form.password" />
        </a-form-item>
      </a-form>
    </a-modal>
  </div>
</template>

<style scoped>
.form {
  margin-top: 8px;
}
.last {
  margin-bottom: 0;
}
</style>
