<script setup lang="ts">
import { ref } from 'vue'
import * as api from '@/api'
import type { User } from '@/api'
import { t } from '@/locales'
import { confirm, toast } from '@/utils/feedback'
import TypeConfirmModal from '@/components/TypeConfirmModal.vue'
import ResetPasswordModal from './ResetPasswordModal.vue'
import type { UserAction } from './userStatus'

// 用户的危险操作集中在这里（禁用/启用、重置密码、重置两步验证、注销会话、删除），列表页和详情页共用
const emit = defineEmits<{ changed: [user: User]; deleted: [id: number]; refresh: [] }>()

const target = ref<User | null>(null)
const resetOpen = ref(false)
const deleteOpen = ref(false)
const deleteBusy = ref(false)

const nameOf = (u: User) => u.display_name || u.username

async function run(action: UserAction, u: User) {
  target.value = u
  switch (action) {
    case 'toggle_disabled': {
      const disabling = !u.disabled
      const ok = await confirm({
        title: t(disabling ? 'users.confirmDisableTitle' : 'users.confirmEnableTitle', { name: nameOf(u) }),
        content: disabling ? t('users.confirmDisableDesc') : undefined,
        okText: disabling ? t('users.actDisable') : t('users.actEnable'),
        danger: disabling,
      })
      if (!ok) return
      try {
        const r = await api.updateUser(u.id, { disabled: disabling })
        toast.success(disabling ? t('users.disabledDone') : t('users.enabledDone'))
        emit('changed', r)
      } catch {
        /* 已统一提示 */
      }
      return
    }
    case 'reset_password':
      resetOpen.value = true
      return
    case 'reset_totp': {
      const ok = await confirm({
        title: t('users.confirmResetTotpTitle', { name: nameOf(u) }),
        content: t('users.confirmResetTotpDesc'),
        okText: t('users.actResetTotp'),
        danger: true,
      })
      if (!ok) return
      try {
        await api.resetUserTotp(u.id)
        toast.success(t('users.resetTotpDone'))
        emit('refresh')
      } catch {
        /* 已统一提示 */
      }
      return
    }
    case 'revoke_sessions': {
      const ok = await confirm({
        title: t('users.confirmRevokeTitle', { name: nameOf(u) }),
        content: t('users.confirmRevokeDesc'),
        okText: t('users.actRevokeSessions'),
        danger: true,
      })
      if (!ok) return
      try {
        await api.revokeUserSessions(u.id)
        toast.success(t('users.revokeDone'))
        emit('refresh')
      } catch {
        /* 已统一提示 */
      }
      return
    }
    case 'delete':
      deleteOpen.value = true
      return
  }
}

async function doDelete() {
  if (!target.value) return
  deleteBusy.value = true
  try {
    await api.deleteUser(target.value.id)
    toast.success(t('common.deleted'))
    deleteOpen.value = false
    emit('deleted', target.value.id)
  } catch {
    /* 已统一提示 */
  } finally {
    deleteBusy.value = false
  }
}

defineExpose({ run })
</script>

<template>
  <ResetPasswordModal v-model:open="resetOpen" :user="target" @done="(u: User) => emit('changed', u)" />
  <TypeConfirmModal
    v-model:open="deleteOpen"
    :title="t('users.deleteTitle')"
    :description="t('users.deleteDesc')"
    :expected="target?.username ?? ''"
    :hint="t('users.deleteTypeHint', { name: target?.username ?? '' })"
    :ok-text="t('users.deleteOk')"
    :busy="deleteBusy"
    @confirm="doDelete"
  />
</template>
