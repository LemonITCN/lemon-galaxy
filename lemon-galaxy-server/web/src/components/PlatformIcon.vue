<script setup lang="ts">
import { computed } from 'vue'
import { AndroidFilled, AppleFilled, HarmonyOSOutlined, LinuxOutlined, QuestionCircleOutlined, WindowsFilled } from '@antdv-next/icons'
import { platformLabel } from '@/utils/labels'

// 平台图标（SVG）+ 可选文字
const props = defineProps<{ platform: string; label?: boolean; boxed?: boolean }>()
const icon = computed(() => {
  switch (props.platform) {
    case 'linux':
      return LinuxOutlined
    case 'macos':
    case 'ios':
      return AppleFilled
    case 'windows':
      return WindowsFilled
    case 'android':
      return AndroidFilled
    case 'harmonyos':
      return HarmonyOSOutlined
    default:
      return QuestionCircleOutlined
  }
})
</script>

<template>
  <span class="platform" :class="[{ boxed }, 'p-' + platform]" :title="platformLabel(platform)">
    <span class="ic"><component :is="icon" /></span>
    <span v-if="label" class="lb">{{ platformLabel(platform) }}</span>
  </span>
</template>

<style scoped>
.platform {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  white-space: nowrap;
  color: var(--lg-text-2);
}
.ic {
  display: inline-flex;
  font-size: 14px;
}
.boxed .ic {
  width: 32px;
  height: 32px;
  border-radius: 8px;
  align-items: center;
  justify-content: center;
  background: var(--lg-neutral-bg);
  color: var(--lg-text);
  font-size: 16px;
}
.lb {
  color: var(--lg-text);
}
</style>
