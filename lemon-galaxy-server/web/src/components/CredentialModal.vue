<script setup lang="ts">
import { t } from '@/locales'
import CopyText from '@/components/CopyText.vue'

// 新建用户后展示一次用户名和初始密码，方便转交
const open = defineModel<boolean>('open', { default: false })
defineProps<{ title: string; description: string; username: string; password: string }>()
</script>

<template>
  <a-modal v-model:open="open" :title="title" :width="460" :mask-closable="false" :footer="null">
    <a-alert type="info" show-icon :title="description" class="alert" />
    <div class="kv">
      <div class="k">{{ t('users.credUsername') }}</div>
      <CopyText :text="username" mono block />
      <div class="k">{{ t('users.credPassword') }}</div>
      <CopyText :text="password" mono block />
    </div>
    <div class="modal-foot">
      <a-button type="primary" @click="open = false">{{ t('common.done') }}</a-button>
    </div>
  </a-modal>
</template>

<style scoped>
.alert {
  margin: 8px 0 16px;
}
.kv {
  display: grid;
  grid-template-columns: 1fr;
  gap: 6px;
}
.k {
  font-size: 12.5px;
  color: var(--lg-text-3);
  margin-top: 4px;
}
</style>
