<script setup lang="ts">
import { App } from 'antdv-next'
import { bindFeedback } from '@/utils/feedback'

// 放在 <a-app> 里面：把带主题的 message / modal 交给全局提示模块
bindFeedback(App.useApp())
</script>

<template>
  <slot />
</template>
