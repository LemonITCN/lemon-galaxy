<script setup lang="ts">
import { computed, watch } from 'vue'
import { useRoute } from 'vue-router'
import { theme } from 'antdv-next'
import { currentLocale, t, tx } from '@/locales'
import { isDark } from '@/stores/prefs'
import AppBridge from '@/components/AppBridge.vue'

// 品牌色：柠檬黄绿。浅色模式下主色按钮用深色字、链接和选中文字用加深的绿，保证对比度。
const themeConfig = computed(() => {
  const dark = isDark.value
  const brandText = dark ? '#B5E03F' : '#4C7A0C'
  const onBrand = dark ? '#0F1708' : '#13200A'
  return {
    algorithm: dark ? theme.darkAlgorithm : theme.defaultAlgorithm,
    token: {
      colorPrimary: dark ? '#9BCB2B' : '#7CB518',
      colorInfo: dark ? '#6F9CFF' : '#2A62D6',
      colorSuccess: dark ? '#4CBF7B' : '#2E9E58',
      colorWarning: dark ? '#E0A43E' : '#D48806',
      colorError: dark ? '#EE5F67' : '#D9363E',
      colorLink: brandText,
      colorLinkHover: dark ? '#C9EC55' : '#5E9410',
      colorLinkActive: dark ? '#A3D52B' : '#3D6309',
      colorPrimaryText: brandText,
      colorPrimaryTextHover: dark ? '#C9EC55' : '#5E9410',
      colorPrimaryTextActive: dark ? '#A3D52B' : '#3D6309',
      borderRadius: 7,
      borderRadiusLG: 10,
      fontSize: 14,
      controlHeight: 34,
      fontFamily:
        '-apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", "Helvetica Neue", Arial, sans-serif',
      fontFamilyCode: '"SF Mono", "JetBrains Mono", Menlo, Consolas, monospace',
      ...(dark
        ? {
            colorBgLayout: '#0F1114',
            colorBgContainer: '#16191E',
            colorBgElevated: '#1D2127',
            colorBorder: '#2E343D',
            colorBorderSecondary: '#222730',
          }
        : { colorBgLayout: '#F4F6F8', colorBorder: '#DADFE5', colorBorderSecondary: '#ECEFF3' }),
    },
    components: {
      Button: { primaryColor: onBrand, primaryShadow: 'none', defaultShadow: 'none', dangerShadow: 'none', fontWeight: 500 },
      Radio: { buttonSolidCheckedColor: onBrand },
      Tabs: { itemSelectedColor: brandText, itemHoverColor: brandText, itemActiveColor: brandText },
      Pagination: { itemActiveColor: brandText, itemActiveColorHover: brandText },
      Menu: {
        darkItemBg: 'transparent',
        darkSubMenuItemBg: 'transparent',
        darkPopupBg: '#151920',
        darkItemColor: '#A3AAB4',
        darkItemHoverColor: '#F1F3F5',
        darkItemHoverBg: 'rgba(255,255,255,0.06)',
        darkItemSelectedBg: 'rgba(163,213,43,0.15)',
        darkItemSelectedColor: '#C9EC55',
        darkGroupTitleColor: '#5F6772',
        itemBorderRadius: 7,
        itemMarginInline: 10,
        itemHeight: 38,
        groupTitleFontSize: 11.5,
        iconSize: 15,
        collapsedIconSize: 17,
      },
      Table: {
        headerBg: dark ? '#1A1E24' : '#F8FAFB',
        headerColor: dark ? '#9AA2AD' : '#5D6470',
        headerSplitColor: 'transparent',
        cellPaddingBlock: 12,
        rowHoverBg: dark ? '#1C2026' : '#F7F9FA',
      },
      Segmented: dark
        ? { trackBg: '#0F1114', itemSelectedBg: '#2A3038', itemColor: '#A2A9B3', itemHoverColor: '#E6E9ED', itemSelectedColor: '#F2F4F6' }
        : { trackBg: '#EDF0F3', itemSelectedBg: '#FFFFFF' },
      Modal: { titleFontSize: 16 },
      Drawer: { footerPaddingBlock: 12 },
      Tag: { defaultBg: dark ? '#20252C' : '#F2F4F7' },
      Descriptions: { labelColor: dark ? '#8A929D' : '#6B7280', itemPaddingBottom: 12 },
    },
  }
})

// 页面标题跟随语言
const route = useRoute()
watch(
  () => [route.meta.titleKey, currentLocale.value.code] as const,
  ([key]) => {
    const title = key ? tx(String(key), '') : ''
    document.title = title ? `${title} · LemonGalaxy` : `LemonGalaxy ${t('brand.console')}`
  },
  { immediate: true },
)
</script>

<template>
  <a-config-provider :locale="currentLocale.antd" :theme="themeConfig" :button="{ autoInsertSpace: false }">
    <a-app>
      <AppBridge>
        <router-view />
      </AppBridge>
    </a-app>
  </a-config-provider>
</template>
