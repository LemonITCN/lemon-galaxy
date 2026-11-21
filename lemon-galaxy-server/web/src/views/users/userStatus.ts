// 用户状态标签与「更多」菜单（列表页和详情页共用）
import { h } from 'vue'
import { DeleteOutlined, KeyOutlined, LogoutOutlined, SafetyOutlined, StopOutlined, CheckCircleOutlined } from '@antdv-next/icons'
import type { User } from '@/api'
import { t } from '@/locales'
import type { Tone } from '@/utils/labels'

export function userStatusTags(u: User): { tone: Tone; text: string }[] {
  const tags: { tone: Tone; text: string }[] = []
  if (u.disabled) tags.push({ tone: 'danger', text: t('users.statusDisabled') })
  if (u.must_change_password) tags.push({ tone: 'warning', text: t('users.statusMustChange') })
  if (!u.account_initialized) tags.push({ tone: 'neutral', text: t('users.statusUninitialized') })
  if (u.totp_enabled) tags.push({ tone: 'success', text: t('users.statusTotp') })
  else if (u.require_totp) tags.push({ tone: 'info', text: t('users.statusRequireTotp') })
  if (!tags.length) tags.push({ tone: 'success', text: t('users.statusNormal') })
  return tags
}

export type UserAction = 'toggle_disabled' | 'reset_password' | 'reset_totp' | 'revoke_sessions' | 'delete'

export function userMenuItems(u: User) {
  return [
    {
      key: 'toggle_disabled',
      icon: () => h(u.disabled ? CheckCircleOutlined : StopOutlined),
      label: u.disabled ? t('users.actEnable') : t('users.actDisable'),
    },
    { key: 'reset_password', icon: () => h(KeyOutlined), label: t('users.actResetPassword') },
    { key: 'reset_totp', icon: () => h(SafetyOutlined), label: t('users.actResetTotp'), disabled: !u.totp_enabled },
    { key: 'revoke_sessions', icon: () => h(LogoutOutlined), label: t('users.actRevokeSessions') },
    { type: 'divider' as const },
    { key: 'delete', icon: () => h(DeleteOutlined), label: t('users.actDelete'), danger: true },
  ]
}
