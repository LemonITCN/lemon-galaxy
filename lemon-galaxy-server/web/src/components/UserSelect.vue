<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import * as api from '@/api'

// 用户下拉（按输入远程搜索）。v-model:value 是用户 ID。
const value = defineModel<number | undefined>('value')
const props = defineProps<{ placeholder?: string; width?: number }>()
const options = ref<{ value: number; label: string }[]>([])
const loading = ref(false)
let timer: number | undefined
let seq = 0

async function search(q: string) {
  const mySeq = ++seq
  loading.value = true
  try {
    const r = await api.listUsers({ q: q || undefined, page: 1, page_size: 20 })
    if (mySeq !== seq) return
    const list = r.items.map((u) => ({ value: u.id, label: u.display_name ? `${u.display_name} (${u.username})` : u.username }))
    // 保留当前选中项，避免选中后标签变成数字
    const cur = options.value.find((o) => o.value === value.value)
    if (cur && !list.some((o) => o.value === cur.value)) list.unshift(cur)
    options.value = list
  } catch {
    /* 已统一提示 */
  } finally {
    if (mySeq === seq) loading.value = false
  }
}

function onSearch(q: string) {
  clearTimeout(timer)
  timer = window.setTimeout(() => search(q.trim()), 250)
}

async function ensureLabel(id: number | undefined) {
  if (id === undefined || options.value.some((o) => o.value === id)) return
  try {
    const u = await api.getUser(id, true)
    options.value = [{ value: u.id, label: u.display_name ? `${u.display_name} (${u.username})` : u.username }, ...options.value]
  } catch {
    /* 用户可能已删除 */
  }
}

onMounted(async () => {
  await search('')
  await ensureLabel(value.value)
})
watch(value, (v) => ensureLabel(v))
</script>

<template>
  <a-select
    v-model:value="value"
    :show-search="{ filterOption: false, onSearch }"
    allow-clear
    :options="options"
    :loading="loading"
    :placeholder="props.placeholder"
    :style="{ width: (props.width ?? 200) + 'px' }"
    @clear="search('')"
  />
</template>
