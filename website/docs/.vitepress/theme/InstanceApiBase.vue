<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { copyText } from './copyText'

declare const __ASTER_DOCS_LOCAL__: boolean
const localDocs = __ASTER_DOCS_LOCAL__
const props = defineProps<{ language: 'zh' | 'en' }>()
const tx = (zh: string, en: string) => props.language === 'zh' ? zh : en
const baseUrl = ref('')
const state = ref<'loading' | 'ready' | 'error'>('loading')
const copied = ref(false)

onMounted(async () => {
  if (!localDocs) return
  try {
    let response = await fetch(`/api/member/docs?locale=${props.language === 'zh' ? 'zh-CN' : 'en-US'}`, { credentials: 'same-origin' })
    if (!response.ok) response = await fetch('/api/admin/settings', { credentials: 'same-origin' })
    if (!response.ok) throw new Error(`HTTP ${response.status}`)
    const data = await response.json() as { public_api_base_url?: string }
    if (!data.public_api_base_url) throw new Error('Missing public API base URL')
    baseUrl.value = data.public_api_base_url.replace(/\/$/, '')
    state.value = 'ready'
  } catch {
    state.value = 'error'
  }
})

async function copy() {
  try {
    await copyText(baseUrl.value)
    copied.value = true
    window.setTimeout(() => { copied.value = false }, 1500)
  } catch { copied.value = false }
}
</script>

<template>
  <div v-if="localDocs" class="instance-api-base" aria-live="polite">
    <span>{{ tx('当前实例 API 基础地址', 'API base URL for this installation') }}</span>
    <span v-if="state === 'loading'">{{ tx('正在读取…', 'Loading…') }}</span>
    <span v-else-if="state === 'error'">{{ tx('无法读取，请登录后刷新。', 'Unavailable. Sign in and refresh.') }}</span>
    <template v-else><code>{{ baseUrl }}</code><button type="button" :aria-label="tx('复制 API 地址', 'Copy API URL')" @click="copy">{{ copied ? tx('已复制', 'Copied') : tx('复制', 'Copy') }}</button></template>
  </div>
</template>
