// 登录状态：当前管理员、登录阶段（done / totp_setup）。CSRF 令牌交给请求封装保存在内存里。
import { reactive } from 'vue'
import * as api from '@/api'
import type { AdminStage, AdminUser, LoginResult } from '@/api'

export const session = reactive({
  admin: null as AdminUser | null,
  stage: null as AdminStage | null,
  loaded: false,
  /** 服务器还没有管理员（null 表示还没查过） */
  needsSetup: null as boolean | null,
})

/** 进页面或刷新后取当前登录状态；force 时忽略缓存重新取 */
export async function loadSession(force = false): Promise<void> {
  if (session.loaded && !force) return
  try {
    const r = await api.me()
    api.setCsrfToken(r.csrf_token)
    session.admin = r.admin ?? null
    session.stage = r.stage
  } catch (e) {
    session.admin = null
    session.stage = null
    if (e instanceof api.ApiError && e.code === 'setup_required') session.needsSetup = true
  }
  session.loaded = true
}

/** 查询服务器是否还需要初始化（结果缓存，初始化完成后会被重置） */
export async function checkNeedsSetup(): Promise<boolean> {
  if (session.needsSetup !== null) return session.needsSetup
  try {
    const r = await api.setupStatus()
    session.needsSetup = r.needs_setup
  } catch {
    session.needsSetup = false
  }
  return session.needsSetup
}

export function applyLogin(r: LoginResult) {
  api.setCsrfToken(r.csrf_token)
  session.stage = r.stage
  session.admin = r.admin ?? null
  session.loaded = true
}

export function clearSession() {
  api.setCsrfToken('')
  session.admin = null
  session.stage = null
}
