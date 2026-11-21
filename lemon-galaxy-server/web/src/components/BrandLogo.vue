<script lang="ts">
let counter = 0
</script>

<script setup lang="ts">
// LemonGalaxy 标志：柠檬绿星核 + 两条交叉轨道 + 一颗卫星
withDefaults(defineProps<{ size?: number; tile?: boolean; ringColor?: string }>(), {
  size: 28,
  tile: false,
  ringColor: 'currentColor',
})
counter += 1
const gid = `lg-logo-${counter}`
</script>

<template>
  <svg class="brand-logo" viewBox="0 0 32 32" :width="size" :height="size" aria-hidden="true" focusable="false">
    <defs>
      <linearGradient :id="gid" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="#D6F25F" />
        <stop offset="1" stop-color="#7CB518" />
      </linearGradient>
    </defs>
    <rect v-if="tile" width="32" height="32" rx="8" fill="#13161B" />
    <ellipse
      cx="16"
      cy="16"
      rx="12.2"
      ry="5.2"
      transform="rotate(-28 16 16)"
      fill="none"
      :stroke="tile ? '#FFFFFF' : ringColor"
      stroke-opacity="0.3"
      stroke-width="1.5"
    />
    <ellipse cx="16" cy="16" rx="12.2" ry="5.2" transform="rotate(32 16 16)" fill="none" :stroke="`url(#${gid})`" stroke-width="1.5" />
    <circle cx="16" cy="16" r="4.6" :fill="`url(#${gid})`" />
    <circle cx="7.3" cy="9.7" r="1.9" fill="#D6F25F" />
  </svg>
</template>

<style scoped>
.brand-logo {
  flex: none;
  display: block;
}
</style>
