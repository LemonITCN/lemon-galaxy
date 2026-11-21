// 枚举值到界面文字与颜色的映射（文字都走语言包）。
import { t, tx } from '@/locales'
import { AI_MODES, AUDIT_ACTIONS, type AiMode, type ConnectionPath, type DeviceAccountStatus } from '@/api/types'

export type Tone = 'success' | 'warning' | 'danger' | 'info' | 'neutral' | 'brand'

export const aiModeLabel = (m: string) => tx(`enums.aiMode.${m}`, m)
export const aiModeDesc = (m: string) => tx(`enums.aiModeDesc.${m}`, '')
export const deviceKindLabel = (k: string) => tx(`enums.deviceKind.${k}`, k)
export const platformLabel = (p: string) => tx(`enums.platform.${p}`, p)
export const connKindLabel = (k: string) => tx(`enums.connKind.${k}`, k)
export const connPathLabel = (p: string) => tx(`enums.connPath.${p}`, p)
export const actorTypeLabel = (a: string) => tx(`enums.actorType.${a}`, a)
export const actionLabel = (a: string) => tx(`enums.action.${a}`, a)
export const targetTypeLabel = (a: string) => tx(`enums.targetType.${a}`, a)
export const providerKindLabel = (k: string) => tx(`enums.providerKind.${k}`, k)
export const updateSourceLabel = (s: string) => tx(`enums.updateSource.${s}`, s)
export const accountStatusLabel = (s: string) => tx(`enums.accountStatus.${s}`, s)

export function aiModeOptions() {
  return AI_MODES.map((m) => ({ value: m, label: aiModeLabel(m) }))
}

export const aiModeTone: Record<AiMode, Tone> = {
  off: 'neutral',
  read_only: 'info',
  confirm_all: 'info',
  confirm_dangerous: 'brand',
  full_trust: 'warning',
}

export const accountStatusTone: Record<DeviceAccountStatus, Tone> = {
  active: 'success',
  needs_relogin: 'warning',
  revoked: 'neutral',
}

export const connPathTone: Record<ConnectionPath, Tone> = {
  connecting: 'neutral',
  lan: 'success',
  direct: 'brand',
  relay_udp: 'info',
  relay_tcp: 'warning',
  closed: 'neutral',
}

export function actionTone(a: string): Tone {
  if (a.endsWith('_failed') || a === 'user.locked') return 'danger'
  if (a.endsWith('delete') || a.endsWith('revoke') || a.startsWith('user.reset') || a === 'user.revoke_sessions') return 'warning'
  if (a.endsWith('create') || a === 'device.register' || a === 'update.upload') return 'success'
  if (a.endsWith('login') || a.endsWith('logout') || a.startsWith('connection.')) return 'neutral'
  return 'info'
}

/** 审计动作下拉：按前缀分组 */
export function actionOptions() {
  const groups = new Map<string, { value: string; label: string }[]>()
  for (const a of AUDIT_ACTIONS) {
    const g = a.split('.')[0]
    if (!groups.has(g)) groups.set(g, [])
    groups.get(g)!.push({ value: a, label: actionLabel(a) })
  }
  return [...groups.entries()].map(([g, options]) => ({ label: tx(`enums.actionGroup.${g}`, g), options }))
}

export function requiredMsg(field: string) {
  return t('validation.required', { field })
}
