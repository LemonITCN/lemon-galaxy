// 格式化：时间一律是 Unix 毫秒；字节数按 1024 进位；数字按当前语言分组。
import dayjs from 'dayjs'
import { localeCode, t } from '@/locales'

export { dayjs }

/** 2026-11-21 14:03:22 */
export function dateTime(ms: number | null | undefined, withSeconds = false): string {
  if (!ms) return '—'
  return dayjs(ms).format(withSeconds ? 'YYYY-MM-DD HH:mm:ss' : 'YYYY-MM-DD HH:mm')
}

export function dateOnly(ms: number | null | undefined): string {
  return ms ? dayjs(ms).format('YYYY-MM-DD') : '—'
}

/** 相对时间：3 小时前（依赖当前语言，模板里调用会随语言刷新） */
export function fromNow(ms: number | null | undefined): string {
  if (!ms) return t('common.never')
  void localeCode.value
  return dayjs(ms).fromNow()
}

export function bytes(n: number | null | undefined): string {
  if (!n || n < 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB', 'PB']
  const i = Math.max(0, Math.min(units.length - 1, Math.floor(Math.log(n) / Math.log(1024))))
  const v = n / 1024 ** i
  return `${i === 0 || v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`
}

export function num(n: number | null | undefined): string {
  return new Intl.NumberFormat(localeCode.value).format(n ?? 0)
}

/** 紧凑数字：1.2K / 1.2万 */
export function compact(n: number | null | undefined): string {
  return new Intl.NumberFormat(localeCode.value, { notation: 'compact', maximumFractionDigits: 1 }).format(n ?? 0)
}

/** 时长（毫秒）：2 小时 5 分 */
export function duration(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000))
  const d = Math.floor(s / 86400)
  const h = Math.floor((s % 86400) / 3600)
  const m = Math.floor((s % 3600) / 60)
  const sec = s % 60
  if (d > 0) return t('duration.days', { d, h })
  if (h > 0) return t('duration.hours', { h, m })
  if (m > 0) return t('duration.minutes', { m, s: sec })
  return t('duration.seconds', { s: sec })
}

/** 上下文长度：200000 → 200K */
export function contextLength(n: number): string {
  if (!n) return '—'
  if (n >= 1_000_000) return `${+(n / 1_000_000).toFixed(1)}M`
  if (n >= 1000) return `${Math.round(n / 1000)}K`
  return String(n)
}

/** 长 ID 缩短显示：9f2c41ab…77e0 */
export function shortId(id: string, head = 8, tail = 4): string {
  if (!id || id.length <= head + tail + 1) return id
  return `${id.slice(0, head)}…${id.slice(-tail)}`
}

/** 头像缩写 */
export function initials(name: string): string {
  const s = (name || '').trim()
  if (!s) return '?'
  return /^[\x00-\x7f]/.test(s) ? s.slice(0, 2).toUpperCase() : s.slice(-2)
}

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text)
    return true
  } catch {
    const ta = document.createElement('textarea')
    ta.value = text
    ta.setAttribute('readonly', '')
    ta.style.position = 'fixed'
    ta.style.opacity = '0'
    document.body.appendChild(ta)
    ta.select()
    let ok = false
    try {
      ok = document.execCommand('copy')
    } catch {
      ok = false
    }
    ta.remove()
    return ok
  }
}

/** 随机强密码：去掉易混淆字符，保证含大小写字母、数字和符号 */
export function generatePassword(length = 16): string {
  const upper = 'ABCDEFGHJKLMNPQRSTUVWXYZ'
  const lower = 'abcdefghijkmnopqrstuvwxyz'
  const digits = '23456789'
  const symbols = '-_.@#%+=!'
  const all = upper + lower + digits + symbols
  const rand = (max: number) => {
    const buf = new Uint32Array(1)
    const limit = Math.floor(0xffffffff / max) * max
    do crypto.getRandomValues(buf)
    while (buf[0] >= limit)
    return buf[0] % max
  }
  const pick = (set: string) => set[rand(set.length)]
  const chars = [pick(upper), pick(lower), pick(digits), pick(symbols)]
  while (chars.length < length) chars.push(pick(all))
  for (let i = chars.length - 1; i > 0; i--) {
    const j = rand(i + 1)
    ;[chars[i], chars[j]] = [chars[j], chars[i]]
  }
  return chars.join('')
}

const USERNAME_RE = /^[A-Za-z0-9._-]{3,32}$/
export function validUsername(s: string): boolean {
  return USERNAME_RE.test(s)
}

/** 粗查 IPv4 / IPv6 地址或 CIDR 网段（最终以服务器校验为准） */
export function validIpOrCidr(s: string): boolean {
  const [ip, prefix, extra] = s.trim().split('/')
  if (extra !== undefined || !ip) return false
  const v4 = /^(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}$/.test(ip)
  const v6 = !v4 && ip.includes(':') && /^[0-9a-fA-F:.]+$/.test(ip) && ip.split('::').length <= 2
  if (!v4 && !v6) return false
  if (prefix === undefined) return true
  if (!/^\d{1,3}$/.test(prefix)) return false
  const p = Number(prefix)
  return v4 ? p <= 32 : p <= 128
}

/** 日期范围选择器的快捷选项 */
export function rangePresets(labels: { today: string; last7: string; last30: string; thisMonth?: string }) {
  type Range = [ReturnType<typeof dayjs>, ReturnType<typeof dayjs>]
  const end = dayjs().endOf('day')
  const list: { label: string; value: Range }[] = [
    { label: labels.today, value: [dayjs().startOf('day'), end] },
    { label: labels.last7, value: [dayjs().subtract(6, 'day').startOf('day'), end] },
    { label: labels.last30, value: [dayjs().subtract(29, 'day').startOf('day'), end] },
  ]
  if (labels.thisMonth) list.push({ label: labels.thisMonth, value: [dayjs().startOf('month'), end] })
  return list
}

/** InputNumber 千分位显示 */
export const groupDigits = (v: string | number | undefined) => `${v ?? ''}`.replace(/\B(?=(\d{3})+(?!\d))/g, ',')
export const ungroupDigits = (v: string | undefined) => (v ?? '').replace(/,/g, '')
