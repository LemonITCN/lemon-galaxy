// 多语言底座。
// 新增一种语言：在本目录照 zh-CN.ts 的结构加一个语言文件，然后在下面的 LOCALES 里登记一行。
import { computed, ref, watch } from 'vue'
import dayjs from 'dayjs'
import relativeTime from 'dayjs/plugin/relativeTime'
import zhCN from './zh-CN'
import en from './en'
import type { Messages } from './zh-CN'
import type { LocaleDefinition, MessageKeys } from './types'

export const LOCALES: LocaleDefinition<Messages>[] = [
  zhCN,
  en,
]

export type MessageKey = MessageKeys<Messages>

const STORAGE_KEY = 'lg-admin-locale'

dayjs.extend(relativeTime)

function detect(): string {
  try {
    const saved = localStorage.getItem(STORAGE_KEY)
    if (saved && LOCALES.some((l) => l.code === saved)) return saved
  } catch {
    /* 隐私模式下读不到 */
  }
  const wanted = (navigator.languages?.length ? navigator.languages : [navigator.language]).filter(Boolean)
  for (const w of wanted) {
    const exact = LOCALES.find((l) => l.code.toLowerCase() === w.toLowerCase())
    if (exact) return exact.code
  }
  for (const w of wanted) {
    const lower = w.toLowerCase()
    const byPrefix = LOCALES.find((l) => l.match.some((m) => lower === m || lower.startsWith(m + '-')))
    if (byPrefix) return byPrefix.code
  }
  return (LOCALES.find((l) => l.code === 'en') ?? LOCALES[0]).code
}

const code = ref(detect())

export const currentLocale = computed(() => LOCALES.find((l) => l.code === code.value) ?? LOCALES[0])

/** 当前语言代码（也是请求头 Accept-Language 的值） */
export const localeCode = computed(() => currentLocale.value.code)

export function setLocale(next: string) {
  if (!LOCALES.some((l) => l.code === next)) return
  code.value = next
  try {
    localStorage.setItem(STORAGE_KEY, next)
  } catch {
    /* 隐私模式 */
  }
}

watch(
  currentLocale,
  (l) => {
    dayjs.locale(l.dayjs)
    document.documentElement.lang = l.code
  },
  { immediate: true },
)

function lookup(messages: unknown, key: string): string | undefined {
  const parts = key.split('.')
  let node: unknown = messages
  for (let i = 0; i < parts.length; i++) {
    if (!node || typeof node !== 'object') return undefined
    const obj = node as Record<string, unknown>
    if (parts[i] in obj) {
      node = obj[parts[i]]
      continue
    }
    // 动作代码这类键本身带点（如 user.create）：剩余部分整体作为一个键再试一次
    const rest = parts.slice(i).join('.')
    return typeof obj[rest] === 'string' ? (obj[rest] as string) : undefined
  }
  return typeof node === 'string' ? node : undefined
}

function format(text: string, params?: Record<string, string | number>): string {
  if (!params) return text
  return text.replace(/\{(\w+)\}/g, (m, name: string) => (name in params ? String(params[name]) : m))
}

/** 取文案。在模板和 computed 里调用即可随语言切换自动刷新。 */
export function t(key: MessageKey, params?: Record<string, string | number>): string {
  return format(lookup(currentLocale.value.messages, key) ?? lookup(LOCALES[0].messages, key) ?? key, params)
}

/** 动态拼出来的键（枚举值等）：没有对应文案时返回 fallback（默认原样返回最后一段） */
export function tx(key: string, fallback?: string, params?: Record<string, string | number>): string {
  const hit = lookup(currentLocale.value.messages, key) ?? lookup(LOCALES[0].messages, key)
  if (hit !== undefined) return format(hit, params)
  return fallback ?? key
}
