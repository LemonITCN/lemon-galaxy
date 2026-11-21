<script setup lang="ts">
import { computed } from 'vue'
import { CheckOutlined, MoonOutlined, SunOutlined, TranslationOutlined } from '@antdv-next/icons'
import { h } from 'vue'
import BrandLogo from '@/components/BrandLogo.vue'
import { LOCALES, currentLocale, setLocale, t } from '@/locales'
import { prefs, toggleTheme } from '@/stores/prefs'

// 登录、初始化、绑定两步验证等页面的外壳：左上品牌，右上语言与主题，中间一张卡片
defineProps<{ title: string; subtitle?: string; width?: number }>()

const langItems = computed(() =>
  LOCALES.map((l) => ({
    key: l.code,
    label: l.name,
    extra: l.code === currentLocale.value.code ? h(CheckOutlined) : undefined,
  })),
)
function onLang(info: { key: string | number }) {
  setLocale(String(info.key))
}
</script>

<template>
  <div class="auth">
    <svg class="bg" viewBox="0 0 1200 800" preserveAspectRatio="xMidYMid slice" aria-hidden="true">
      <ellipse cx="600" cy="400" rx="560" ry="210" transform="rotate(-18 600 400)" />
      <ellipse cx="600" cy="400" rx="430" ry="150" transform="rotate(24 600 400)" />
      <ellipse cx="600" cy="400" rx="300" ry="96" transform="rotate(-6 600 400)" />
    </svg>
    <header class="top">
      <div class="brand">
        <BrandLogo :size="26" tile />
        <span class="name">LemonGalaxy</span>
      </div>
      <div class="tools">
        <a-dropdown :trigger="['click']" placement="bottomRight" :menu="{ items: langItems, selectable: false }" @menu-click="onLang">
          <button type="button" class="tool">
            <TranslationOutlined />
            <span>{{ currentLocale.name }}</span>
          </button>
        </a-dropdown>
        <a-tooltip :title="t('theme.toggle')">
          <button type="button" class="tool icon" :aria-label="t('theme.toggle')" @click="toggleTheme">
            <SunOutlined v-if="prefs.theme === 'dark'" />
            <MoonOutlined v-else />
          </button>
        </a-tooltip>
      </div>
    </header>
    <main class="center">
      <div class="card" :style="{ maxWidth: (width ?? 400) + 'px' }">
        <div class="head">
          <h1>{{ title }}</h1>
          <p v-if="subtitle" class="sub">{{ subtitle }}</p>
        </div>
        <slot />
      </div>
      <div class="below"><slot name="below" /></div>
    </main>
  </div>
</template>

<style scoped>
.auth {
  position: relative;
  min-height: 100vh;
  display: flex;
  flex-direction: column;
  background: var(--lg-bg);
  overflow: hidden;
}
.bg {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
}
.bg ellipse {
  fill: none;
  stroke: var(--lg-brand);
  stroke-opacity: 0.12;
  stroke-width: 1.2;
}
.top {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 64px;
  padding: 0 24px;
}
.brand {
  display: flex;
  align-items: center;
  gap: 10px;
}
.brand .name {
  font-weight: 700;
  font-size: 16px;
  letter-spacing: -0.01em;
}
.tools {
  display: flex;
  align-items: center;
  gap: 4px;
}
.tool {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 32px;
  padding: 0 10px;
  border: 0;
  border-radius: 7px;
  background: transparent;
  color: var(--lg-text-2);
  cursor: pointer;
  font-size: 13px;
}
.tool.icon {
  width: 32px;
  padding: 0;
  justify-content: center;
  font-size: 15px;
}
.tool:hover {
  background: var(--lg-hover);
  color: var(--lg-text);
}
.center {
  position: relative;
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 24px 16px 72px;
}
.card {
  width: 100%;
  background: var(--lg-surface);
  border: 1px solid var(--lg-border-soft);
  border-radius: 14px;
  box-shadow: 0 12px 40px rgba(16, 24, 40, 0.08);
  padding: 28px 28px 24px;
}
.head {
  margin-bottom: 22px;
}
.head h1 {
  margin: 0;
  font-size: 20px;
  font-weight: 650;
  letter-spacing: -0.01em;
}
.head .sub {
  margin: 6px 0 0;
  color: var(--lg-text-3);
  font-size: 13.5px;
}
.below {
  margin-top: 16px;
}
</style>
