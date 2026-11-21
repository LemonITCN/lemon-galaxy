<script setup lang="ts">
import { computed, h, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import {
  ApiOutlined,
  AuditOutlined,
  BarChartOutlined,
  CheckOutlined,
  CloudDownloadOutlined,
  DashboardOutlined,
  DesktopOutlined,
  KeyOutlined,
  LogoutOutlined,
  MenuFoldOutlined,
  MenuUnfoldOutlined,
  MoonOutlined,
  RobotOutlined,
  SafetyCertificateOutlined,
  SettingOutlined,
  SunOutlined,
  TeamOutlined,
  TranslationOutlined,
  BgColorsOutlined,
} from '@antdv-next/icons'
import * as api from '@/api'
import BrandLogo from '@/components/BrandLogo.vue'
import ChangePasswordModal from '@/components/ChangePasswordModal.vue'
import { LOCALES, currentLocale, setLocale, t, tx } from '@/locales'
import { prefs, setTheme, toggleTheme } from '@/stores/prefs'
import { clearSession, session } from '@/stores/session'
import { crumb } from '@/stores/crumb'
import { initials } from '@/utils/format'
import { toast } from '@/utils/feedback'

const route = useRoute()
const router = useRouter()

const icon = (c: any) => () => h(c)

const menuItems = computed(() => [
  { key: '/admin', icon: icon(DashboardOutlined), label: t('nav.overview') },
  {
    type: 'group' as const,
    key: 'g-manage',
    label: t('nav.groupManage'),
    children: [
      { key: '/admin/users', icon: icon(TeamOutlined), label: t('nav.users') },
      { key: '/admin/devices', icon: icon(DesktopOutlined), label: t('nav.devices') },
      { key: '/admin/connections', icon: icon(ApiOutlined), label: t('nav.connections') },
    ],
  },
  {
    type: 'group' as const,
    key: 'g-ai',
    label: t('nav.groupAi'),
    children: [
      { key: '/admin/ai/providers', icon: icon(RobotOutlined), label: t('nav.aiProviders') },
      { key: '/admin/ai/usage', icon: icon(BarChartOutlined), label: t('nav.aiUsage') },
    ],
  },
  {
    type: 'group' as const,
    key: 'g-system',
    label: t('nav.groupSystem'),
    children: [
      { key: '/admin/audit', icon: icon(AuditOutlined), label: t('nav.audit') },
      { key: '/admin/settings', icon: icon(SettingOutlined), label: t('nav.settings') },
      { key: '/admin/updates', icon: icon(CloudDownloadOutlined), label: t('nav.updates') },
      { key: '/admin/admins', icon: icon(SafetyCertificateOutlined), label: t('nav.admins') },
    ],
  },
])

const selected = computed(() => [route.meta.menu ?? route.path])

function onMenu(info: { key: string | number }) {
  const key = String(info.key)
  if (key !== route.path) router.push(key)
}

// 顶栏面包屑：分组 / 页面 / 详情名
const crumbs = computed(() => {
  const list: string[] = []
  if (route.meta.groupKey) list.push(tx(route.meta.groupKey))
  if (route.meta.titleKey) list.push(tx(route.meta.titleKey))
  if (crumb.value && route.name === 'user') list.push(crumb.value)
  return list
})

// 版本号（左下角）
const version = ref('')
onMounted(async () => {
  try {
    version.value = (await api.serverInfo()).version
  } catch {
    /* 已统一提示 */
  }
})

// 个人菜单：修改密码、语言、主题、退出
const passwordOpen = ref(false)
const accountItems = computed(() => [
  { key: 'password', icon: icon(KeyOutlined), label: t('account.changePassword') },
  {
    key: 'lang',
    icon: icon(TranslationOutlined),
    label: t('lang.label'),
    children: LOCALES.map((l) => ({
      key: 'lang:' + l.code,
      label: l.name,
      extra: l.code === currentLocale.value.code ? h(CheckOutlined) : undefined,
    })),
  },
  {
    key: 'theme',
    icon: icon(BgColorsOutlined),
    label: t('theme.toggle'),
    children: (['light', 'dark'] as const).map((m) => ({
      key: 'theme:' + m,
      label: t(m === 'light' ? 'theme.light' : 'theme.dark'),
      extra: prefs.theme === m ? h(CheckOutlined) : undefined,
    })),
  },
  { type: 'divider' as const },
  { key: 'logout', icon: icon(LogoutOutlined), label: t('account.logout'), danger: true },
])

async function onAccount(info: { key: string | number }) {
  const key = String(info.key)
  if (key === 'password') passwordOpen.value = true
  else if (key.startsWith('lang:')) setLocale(key.slice(5))
  else if (key.startsWith('theme:')) setTheme(key.slice(6) as 'light' | 'dark')
  else if (key === 'logout') {
    try {
      await api.logout()
    } catch {
      /* 已过期也照样退出 */
    }
    clearSession()
    toast.success(t('account.loggedOut'))
    router.replace({ name: 'login' })
  }
}

const adminName = computed(() => session.admin?.username ?? '')
</script>

<template>
  <a-layout class="shell">
    <a-layout-sider
      v-model:collapsed="prefs.siderCollapsed"
      :width="228"
      :collapsed-width="68"
      collapsible
      :trigger="null"
      theme="dark"
      breakpoint="lg"
      class="sider"
    >
      <div class="brand" @click="router.push('/admin')">
        <BrandLogo :size="28" ring-color="#FFFFFF" />
        <div v-if="!prefs.siderCollapsed" class="brand-text">
          <div class="brand-name">LemonGalaxy</div>
          <div class="brand-sub">{{ t('brand.console') }}</div>
        </div>
      </div>
      <div class="menu-wrap">
        <a-menu mode="inline" theme="dark" :items="menuItems" :selected-keys="selected" @click="onMenu" />
      </div>
      <div class="sider-foot" :class="{ collapsed: prefs.siderCollapsed }">
        <span v-if="!prefs.siderCollapsed && version" class="ver">v{{ version }}</span>
        <a-tooltip :title="prefs.siderCollapsed ? t('nav.expand') : t('nav.collapse')" placement="right">
          <button
            type="button"
            class="sider-btn"
            :aria-label="prefs.siderCollapsed ? t('nav.expand') : t('nav.collapse')"
            @click="prefs.siderCollapsed = !prefs.siderCollapsed"
          >
            <MenuUnfoldOutlined v-if="prefs.siderCollapsed" />
            <MenuFoldOutlined v-else />
          </button>
        </a-tooltip>
      </div>
    </a-layout-sider>

    <a-layout class="main">
      <header class="header">
        <nav class="crumbs">
          <template v-for="(c, i) in crumbs" :key="i">
            <span v-if="i > 0" class="sep">/</span>
            <span :class="{ last: i === crumbs.length - 1 }">{{ c }}</span>
          </template>
        </nav>
        <div class="spacer" />
        <a-tooltip :title="t('theme.toggle')">
          <button type="button" class="icon-btn" :aria-label="t('theme.toggle')" @click="toggleTheme">
            <SunOutlined v-if="prefs.theme === 'dark'" />
            <MoonOutlined v-else />
          </button>
        </a-tooltip>
        <a-dropdown :trigger="['click']" placement="bottomRight" :menu="{ items: accountItems, selectable: false }" @menu-click="onAccount">
          <button type="button" class="account">
            <span class="avatar">{{ initials(adminName) }}</span>
            <span class="who">{{ adminName }}</span>
          </button>
        </a-dropdown>
      </header>
      <a-layout-content class="content">
        <router-view v-slot="{ Component }">
          <component :is="Component" :key="route.path" />
        </router-view>
      </a-layout-content>
    </a-layout>

    <ChangePasswordModal v-model:open="passwordOpen" />
  </a-layout>
</template>

<style scoped>
.shell {
  min-height: 100vh;
}
.sider {
  position: sticky !important;
  top: 0;
  height: 100vh;
  background: var(--lg-sider-bg) !important;
  border-right: 1px solid var(--lg-sider-border);
}
.sider :deep(.ant-layout-sider-children) {
  display: flex;
  flex-direction: column;
}
.brand {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 60px;
  padding: 0 20px;
  cursor: pointer;
  flex: none;
  color: #fff;
}
.sider.ant-layout-sider-collapsed .brand {
  justify-content: center;
  padding: 0;
}
.brand-name {
  font-weight: 700;
  font-size: 15px;
  letter-spacing: -0.01em;
  line-height: 1.2;
  color: #f4f6f8;
}
.brand-sub {
  font-size: 11px;
  color: var(--lg-sider-text-2);
  letter-spacing: 0.02em;
  margin-top: 1px;
}
.menu-wrap {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
  padding: 4px 0 12px;
  scrollbar-width: thin;
}
.menu-wrap :deep(.ant-menu) {
  border-inline-end: 0 !important;
}
.menu-wrap :deep(.ant-menu-item-group-title) {
  text-transform: uppercase;
  letter-spacing: 0.06em;
  padding-top: 14px;
}
.sider-foot {
  flex: none;
  display: flex;
  align-items: center;
  gap: 8px;
  height: 48px;
  padding: 0 14px 0 20px;
  border-top: 1px solid var(--lg-sider-border);
}
.sider-foot.collapsed {
  justify-content: center;
  padding: 0;
}
.ver {
  flex: 1;
  font-size: 11.5px;
  color: var(--lg-sider-text-2);
  font-family: var(--lg-mono);
}
.sider-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  border: 0;
  border-radius: 7px;
  background: transparent;
  color: var(--lg-sider-text);
  cursor: pointer;
  font-size: 15px;
}
.sider-btn:hover {
  background: rgba(255, 255, 255, 0.07);
  color: #fff;
}
.main {
  min-width: 0;
}
.header {
  display: flex;
  align-items: center;
  gap: 6px;
  position: sticky;
  top: 0;
  z-index: 20;
  height: 56px;
  padding: 0 20px 0 28px;
  background: color-mix(in srgb, var(--lg-surface) 88%, transparent);
  backdrop-filter: saturate(180%) blur(10px);
  border-bottom: 1px solid var(--lg-border-soft);
}
.crumbs {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13.5px;
  color: var(--lg-text-3);
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
}
.crumbs .last {
  color: var(--lg-text);
  font-weight: 550;
  overflow: hidden;
  text-overflow: ellipsis;
}
.crumbs .sep {
  color: var(--lg-border);
}
.spacer {
  flex: 1;
}
.icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: var(--lg-text-2);
  cursor: pointer;
  font-size: 15px;
}
.icon-btn:hover {
  background: var(--lg-hover);
  color: var(--lg-text);
}
.account {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  height: 38px;
  padding: 0 10px 0 4px;
  border: 0;
  border-radius: 9px;
  background: transparent;
  cursor: pointer;
  color: var(--lg-text);
  font-size: 13.5px;
  font-weight: 550;
}
.account:hover {
  background: var(--lg-hover);
}
.account .avatar {
  width: 28px;
  height: 28px;
  border-radius: 50%;
  font-size: 11.5px;
}
.content {
  min-width: 0;
}
</style>
