<script setup lang="ts">
import { onMounted, ref } from 'vue'

declare const __ASTER_DOCS_LOCAL__: boolean
const localDocs = __ASTER_DOCS_LOCAL__
type Model = { id: string; public_name: string; display_name: string; provider?: string; enabled: boolean; available?: boolean }
const props = defineProps<{ language: 'zh' | 'en' }>()
const tx = (zh: string, en: string) => props.language === 'zh' ? zh : en
const items = ref<Model[]>([])
const audience = ref<'member' | 'admin'>('member')
const state = ref<'loading' | 'ready' | 'error'>('loading')

onMounted(async () => {
  if (!localDocs) return
  try {
    let response = await fetch('/api/member/models', { credentials: 'same-origin' })
    if (!response.ok) {
      response = await fetch('/api/admin/models', { credentials: 'same-origin' })
      audience.value = 'admin'
    }
    if (!response.ok) throw new Error(`HTTP ${response.status}`)
    const data = await response.json() as { items?: Model[] }
    if (!Array.isArray(data.items)) throw new Error('Invalid model list')
    items.value = data.items
    state.value = 'ready'
  } catch {
    state.value = 'error'
  }
})
</script>

<template>
  <section v-if="localDocs" class="instance-model-list" aria-live="polite">
    <div class="instance-model-head">
      <h2>{{ audience === 'admin' ? tx('当前实例模型', 'Models in this installation') : tx('当前成员授权模型', 'Models granted to this member') }}</h2>
      <a href="/models" target="_self">{{ audience === 'admin' ? tx('模型管理', 'Model management') : tx('成员模型列表', 'Member models') }}</a>
    </div>
    <p v-if="state === 'loading'">{{ tx('正在读取当前成员的模型…', 'Loading models for this member…') }}</p>
    <p v-else-if="state === 'error'">{{ tx('暂时无法读取模型，请登录后刷新。', 'Could not load models. Sign in and refresh.') }}</p>
    <p v-else-if="!items.length">{{ tx('当前账号尚未获得模型权限。', 'No models are granted to this account yet.') }}</p>
    <div v-else class="instance-model-table">
      <table>
        <thead><tr><th>{{ tx('公开模型 ID', 'Public model ID') }}</th><th>{{ tx('名称', 'Name') }}</th><th>{{ tx('提供商', 'Provider') }}</th><th>{{ tx('当前状态', 'Current status') }}</th></tr></thead>
        <tbody><tr v-for="item in items" :key="item.id"><td><code>{{ item.public_name }}</code></td><td>{{ item.display_name }}</td><td>{{ item.provider || '—' }}</td><td>{{ !item.enabled ? tx('已停用', 'Disabled') : item.available === false ? tx('暂不可用', 'Temporarily unavailable') : tx('可用', 'Available') }}</td></tr></tbody>
      </table>
    </div>
  </section>
</template>
