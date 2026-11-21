<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { CheckCircleFilled, SendOutlined } from '@antdv-next/icons'
import * as api from '@/api'
import type { AiProvider, AiTestResult } from '@/api'
import { t } from '@/locales'
import { num } from '@/utils/format'

// 测试供应商：选一个模型，服务器发一句话，看回复和耗时
const open = defineModel<boolean>('open', { default: false })
const props = defineProps<{ provider: AiProvider | null }>()

const model = ref<string>()
const busy = ref(false)
const result = ref<AiTestResult | null>(null)
const error = ref('')

const options = computed(() =>
  (props.provider?.models ?? []).filter((m) => m.enabled).map((m) => ({ value: m.id, label: m.name && m.name !== m.id ? `${m.name} (${m.id})` : m.id })),
)

watch(open, (v) => {
  if (!v) return
  result.value = null
  error.value = ''
  model.value = options.value[0]?.value
})

async function run() {
  if (!props.provider || !model.value) return
  busy.value = true
  result.value = null
  error.value = ''
  try {
    result.value = await api.testProvider(props.provider.id, model.value)
  } catch (e) {
    error.value = api.errorText(e)
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <a-modal v-model:open="open" :title="t('aiProviders.testTitle', { name: provider?.name ?? '' })" :width="560" :footer="null" destroy-on-hidden>
    <div class="bar">
      <a-select
        v-model:value="model"
        :options="options"
        :show-search="{ optionFilterProp: 'label' }"
        :placeholder="t('aiProviders.testModel')"
        style="flex: 1; min-width: 0"
      />
      <a-button type="primary" :loading="busy" :disabled="!model" @click="run">
        <template #icon><SendOutlined /></template>
        {{ t('aiProviders.testSend') }}
      </a-button>
    </div>
    <div v-if="busy" class="waiting"><a-spin size="small" /></div>
    <a-alert v-else-if="error" type="error" show-icon :title="t('aiProviders.testFailed')" :description="error" class="res" />
    <div v-else-if="result" class="res">
      <div class="res-head">
        <CheckCircleFilled class="ok" />
        <span>{{ t('aiProviders.testReply') }}</span>
        <span class="spacer" />
        <a-tag :bordered="false" class="lat">{{ t('aiProviders.testLatency', { ms: num(result.latency_ms) }) }}</a-tag>
      </div>
      <pre class="code-block reply">{{ result.reply }}</pre>
    </div>
  </a-modal>
</template>

<style scoped>
.bar {
  display: flex;
  gap: 8px;
  margin-top: 10px;
}
.waiting {
  display: flex;
  justify-content: center;
  padding: 28px 0 12px;
}
.res {
  margin-top: 16px;
}
.res-head {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
  font-weight: 550;
}
.spacer {
  flex: 1;
}
.ok {
  color: var(--lg-success);
}
.lat {
  font-variant-numeric: tabular-nums;
}
.reply {
  max-height: 280px;
  overflow: auto;
  font-family: var(--lg-font);
  font-size: 13.5px;
}
</style>
