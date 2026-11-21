import { createRouter, createWebHistory, type RouteLocationNormalized } from 'vue-router'
import { onAuthError } from '@/api'
import { checkNeedsSetup, clearSession, loadSession, session } from '@/stores/session'
import { t } from '@/locales'
import { toast } from '@/utils/feedback'
import type { MessageKey } from '@/locales'

declare module 'vue-router' {
  interface RouteMeta {
    /** 页面标题（语言包键） */
    titleKey?: MessageKey
    /** 侧栏分组（语言包键），用于顶栏面包屑 */
    groupKey?: MessageKey
    /** 侧栏高亮哪一项（详情页高亮所属列表） */
    menu?: string
    /** 不需要登录 */
    public?: boolean
  }
}

export const router = createRouter({
  history: createWebHistory('/'),
  routes: [
    { path: '/', redirect: '/admin' },
    { path: '/setup', name: 'setup', component: () => import('@/views/SetupView.vue'), meta: { public: true, titleKey: 'setup.title' } },
    { path: '/admin/login', name: 'login', component: () => import('@/views/LoginView.vue'), meta: { public: true, titleKey: 'login.title' } },
    {
      path: '/admin/totp-setup',
      name: 'totp-setup',
      component: () => import('@/views/TotpBindView.vue'),
      meta: { titleKey: 'totpBind.title' },
    },
    {
      path: '/admin',
      component: () => import('@/layouts/AdminLayout.vue'),
      children: [
        { path: '', name: 'overview', component: () => import('@/views/OverviewView.vue'), meta: { titleKey: 'nav.overview', menu: '/admin' } },
        {
          path: 'users',
          name: 'users',
          component: () => import('@/views/users/UsersView.vue'),
          meta: { titleKey: 'nav.users', groupKey: 'nav.groupManage', menu: '/admin/users' },
        },
        {
          path: 'users/:id(\\d+)',
          name: 'user',
          component: () => import('@/views/users/UserDetailView.vue'),
          meta: { titleKey: 'nav.users', groupKey: 'nav.groupManage', menu: '/admin/users' },
        },
        {
          path: 'devices',
          name: 'devices',
          component: () => import('@/views/devices/DevicesView.vue'),
          meta: { titleKey: 'nav.devices', groupKey: 'nav.groupManage', menu: '/admin/devices' },
        },
        {
          path: 'connections',
          name: 'connections',
          component: () => import('@/views/ConnectionsView.vue'),
          meta: { titleKey: 'nav.connections', groupKey: 'nav.groupManage', menu: '/admin/connections' },
        },
        {
          path: 'ai/providers',
          name: 'ai-providers',
          component: () => import('@/views/ai/AiProvidersView.vue'),
          meta: { titleKey: 'nav.aiProviders', groupKey: 'nav.groupAi', menu: '/admin/ai/providers' },
        },
        {
          path: 'ai/usage',
          name: 'ai-usage',
          component: () => import('@/views/ai/AiUsageView.vue'),
          meta: { titleKey: 'nav.aiUsage', groupKey: 'nav.groupAi', menu: '/admin/ai/usage' },
        },
        {
          path: 'audit',
          name: 'audit',
          component: () => import('@/views/AuditView.vue'),
          meta: { titleKey: 'nav.audit', groupKey: 'nav.groupSystem', menu: '/admin/audit' },
        },
        {
          path: 'settings',
          name: 'settings',
          component: () => import('@/views/SettingsView.vue'),
          meta: { titleKey: 'nav.settings', groupKey: 'nav.groupSystem', menu: '/admin/settings' },
        },
        {
          path: 'updates',
          name: 'updates',
          component: () => import('@/views/UpdatesView.vue'),
          meta: { titleKey: 'nav.updates', groupKey: 'nav.groupSystem', menu: '/admin/updates' },
        },
        {
          path: 'admins',
          name: 'admins',
          component: () => import('@/views/AdminsView.vue'),
          meta: { titleKey: 'nav.admins', groupKey: 'nav.groupSystem', menu: '/admin/admins' },
        },
      ],
    },
    { path: '/:pathMatch(.*)*', redirect: '/admin' },
  ],
  scrollBehavior: () => ({ top: 0 }),
})

function loginRedirect(to: RouteLocationNormalized) {
  const redirect = to.fullPath.startsWith('/admin') && to.fullPath !== '/admin' ? to.fullPath : undefined
  return { name: 'login', query: redirect ? { redirect } : {} }
}

router.beforeEach(async (to) => {
  await loadSession()

  if (to.meta.public) {
    if (to.name === 'login' && session.stage === 'done') return { path: (to.query.redirect as string) || '/admin' }
    if (to.name === 'login' && session.stage === 'totp_setup') return { name: 'totp-setup' }
    return true
  }

  if (to.name === 'totp-setup') {
    if (session.stage === 'totp_setup') return true
    if (session.stage === 'done') return { path: '/admin' }
  } else {
    if (session.stage === 'done') return true
    if (session.stage === 'totp_setup') return { name: 'totp-setup' }
  }

  // 没登录：服务器还没初始化就去初始化页
  if (await checkNeedsSetup()) return { name: 'setup' }
  return loginRedirect(to)
})

// 登录失效 / 服务器要求初始化：回到对应页面
let expiredNotified = 0
onAuthError((err) => {
  const current = router.currentRoute.value
  if (err.code === 'setup_required') {
    clearSession()
    session.needsSetup = true
    if (current.name !== 'setup') router.replace({ name: 'setup' })
    return
  }
  if (current.meta.public) return
  clearSession()
  if (Date.now() - expiredNotified > 3000) {
    expiredNotified = Date.now()
    toast.warning(t('errors.sessionExpired'))
  }
  router.replace(loginRedirect(current))
})
