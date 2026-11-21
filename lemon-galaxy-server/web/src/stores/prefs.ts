// 界面偏好：主题、侧栏折叠。只是本机浏览器上的便利设置，存 localStorage（读写失败就用默认值）。
import { computed, reactive, watch } from 'vue'

export type ThemeMode = 'light' | 'dark'

const THEME_KEY = 'lg-admin-theme'
const SIDER_KEY = 'lg-admin-sider'

function read(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function write(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {
    /* 隐私模式 */
  }
}

function initialTheme(): ThemeMode {
  const saved = read(THEME_KEY)
  if (saved === 'light' || saved === 'dark') return saved
  return window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
}

export const prefs = reactive({
  theme: initialTheme(),
  siderCollapsed: read(SIDER_KEY) === '1',
})

watch(
  () => prefs.theme,
  (t) => {
    document.documentElement.dataset.theme = t
    document.documentElement.style.colorScheme = t
    write(THEME_KEY, t)
  },
  { immediate: true },
)

watch(
  () => prefs.siderCollapsed,
  (v) => write(SIDER_KEY, v ? '1' : '0'),
)

export const isDark = computed(() => prefs.theme === 'dark')

export function setTheme(t: ThemeMode) {
  prefs.theme = t
}

export function toggleTheme() {
  prefs.theme = prefs.theme === 'dark' ? 'light' : 'dark'
}
