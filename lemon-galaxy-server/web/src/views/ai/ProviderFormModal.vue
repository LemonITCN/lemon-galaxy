<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import * as api from '@/api'
import type { AiProvider, AiProviderKind } from '@/api'
import InfoTip from '@/components/InfoTip.vue'
import { t } from '@/locales'
import { toast } from '@/utils/feedback'
import { providerKindLabel } from '@/utils/labels'

import { OPENCODE_GO_URL } from './constants'

// 新增 / 编辑大模型供应商。API Key 只写不读：编辑时留空表示不修改。

const open = defineModel<boolean>('open', { default: false })
const props = defineProps<{ provider?: AiProvider | null }>()
const emit = defineEmits<{ saved: [provider: AiProvider] }>()

const editing = computed(() => !!props.provider)
const form = reactive({ name: '', kind: 'opencode_go' as AiProviderKind, base_url: '', api_key: '', enabled: true })
const errors = reactive({ name: '', base_url: '', api_key: '' })
const busy = ref(false)

watch(open, (v) => {
  if (!v) return
  Object.assign(errors, { name: '', base_url: '', api_key: '' })
  const p = props.provider
  Object.assign(form, {
    name: p?.name ?? 'OpenCode Go',
    kind: p?.kind ?? 'opencode_go',
    base_url: p ? (p.kind === 'opencode_go' && p.base_url === OPENCODE_GO_URL ? '' : p.base_url) : '',
    api_key: '',
    enabled: p?.enabled ?? true,
  })
})

const kindOptions = computed(() =>
  (['opencode_go', 'openai_compatible'] as const).map((k) => ({ value: k, label: providerKindLabel(k) })),
)

function onKind(k: AiProviderKind) {
  // 名称还是另一种类型的默认名时跟着换
  if (!editing.value && (form.name === '' || form.name === providerKindLabel(k === 'opencode_go' ? 'openai_compatible' : 'opencode_go'))) {
    form.name = k === 'opencode_go' ? 'OpenCode Go' : ''
  }
}

async function submit() {
  Object.assign(errors, { name: '', base_url: '', api_key: '' })
  if (!form.name.trim()) errors.name = t('validation.required', { field: t('aiProviders.form.name') })
  if (form.kind === 'openai_compatible' && !/^https?:\/\/\S+$/.test(form.base_url.trim()))
    errors.base_url = t('validation.required', { field: t('aiProviders.form.baseUrl') })
  if (!editing.value && !form.api_key.trim()) errors.api_key = t('validation.required', { field: t('aiProviders.form.apiKey') })
  if (errors.name || errors.base_url || errors.api_key) return

  busy.value = true
  try {
    const baseUrl = form.base_url.trim()
    let p: AiProvider
    if (props.provider) {
      p = await api.updateProvider(props.provider.id, {
        name: form.name.trim(),
        base_url: baseUrl || (form.kind === 'opencode_go' ? OPENCODE_GO_URL : undefined),
        api_key: form.api_key.trim() || undefined,
        enabled: form.enabled,
      })
      toast.success(t('common.saved'))
    } else {
      p = await api.createProvider({
        name: form.name.trim(),
        kind: form.kind,
        base_url: baseUrl || undefined,
        api_key: form.api_key.trim(),
        enabled: form.enabled,
      })
      toast.success(t('common.created'))
    }
    open.value = false
    emit('saved', p)
  } catch {
    /* 已统一提示 */
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <a-modal
    v-model:open="open"
    :title="editing ? t('aiProviders.form.editTitle') : t('aiProviders.form.createTitle')"
    :width="520"
    :ok-text="editing ? t('common.save') : t('common.create')"
    :confirm-loading="busy"
    destroy-on-hidden
    @ok="submit"
  >
    <a-form layout="vertical" class="form">
      <a-form-item :label="t('aiProviders.form.kind')" required>
        <a-radio-group
          v-model:value="form.kind"
          :options="kindOptions"
          option-type="button"
          button-style="solid"
          :disabled="editing"
          @change="onKind(form.kind)"
        />
      </a-form-item>
      <a-form-item :label="t('aiProviders.form.name')" required :validate-status="errors.name ? 'error' : undefined" :help="errors.name || undefined">
        <a-input v-model:value="form.name" :maxlength="64" />
      </a-form-item>
      <a-form-item
        :required="form.kind === 'openai_compatible'"
        :validate-status="errors.base_url ? 'error' : undefined"
        :help="errors.base_url || undefined"
      >
        <template #label>
          {{ t('aiProviders.form.baseUrl') }}
          <InfoTip :text="t('aiProviders.form.baseUrlTip')" />
        </template>
        <a-input
          v-model:value="form.base_url"
          class="mono-input"
          :placeholder="form.kind === 'opencode_go' ? t('aiProviders.form.baseUrlDefault', { url: OPENCODE_GO_URL }) : 'https://api.example.com/v1'"
        />
      </a-form-item>
      <a-form-item :required="!editing" :validate-status="errors.api_key ? 'error' : undefined" :help="errors.api_key || undefined">
        <template #label>
          {{ t('aiProviders.form.apiKey') }}
          <InfoTip :text="t('aiProviders.form.apiKeyTip')" />
          <a-tag v-if="editing && provider?.has_api_key" :bordered="false" color="success" class="key-tag">{{ t('aiProviders.apiKeySet') }}</a-tag>
        </template>
        <a-input-password
          v-model:value="form.api_key"
          class="mono-input"
          autocomplete="off"
          :placeholder="editing ? t('aiProviders.form.apiKeyKeep') : 'sk-...'"
        />
      </a-form-item>
      <a-form-item :label="t('aiProviders.form.enabled')" class="switch-cell last">
        <a-switch v-model:checked="form.enabled" />
      </a-form-item>
    </a-form>
  </a-modal>
</template>

<style scoped>
.form {
  margin-top: 8px;
}
.last {
  margin-bottom: 0;
}
.key-tag {
  margin-left: 6px;
}
.mono-input,
.mono-input :deep(input) {
  font-family: var(--lg-mono);
}
</style>
