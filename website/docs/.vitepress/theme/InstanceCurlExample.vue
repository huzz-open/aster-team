<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { copyText } from './copyText'

declare const __ASTER_DOCS_LOCAL__: boolean
const localDocs = __ASTER_DOCS_LOCAL__
type Kind = 'responses' | 'chat' | 'generate' | 'edit'
type Model = { id: string; public_name: string; enabled: boolean }
type Capabilities = { protocols: string[] }
type Docs = { public_api_base_url: string }
const props = defineProps<{ language: 'zh' | 'en'; kind: Kind }>()
const tx = (zh: string, en: string) => props.language === 'zh' ? zh : en
const model = ref('')
const apiBase = ref('')
const state = ref<'loading' | 'ready' | 'unavailable' | 'error'>('loading')
const copied = ref(false)
const protocol = { responses: 'responses', chat: 'chat_completions', generate: 'images/generations', edit: 'images/edits' } as const
const path = { responses: '/v1/responses', chat: '/v1/chat/completions', generate: '/v1/images/generations', edit: '/v1/images/edits' } as const

async function getJson<T>(url: string): Promise<T> {
  const response = await fetch(url, { credentials: 'same-origin' })
  if (!response.ok) throw new Error(`HTTP ${response.status}`)
  return response.json() as Promise<T>
}

onMounted(async () => {
  if (!localDocs) return
  try {
    let docs: Docs
    let listing: { items: Model[] }
    let capabilitiesPath = '/api/member/models'
    try {
      [docs, listing] = await Promise.all([
        getJson<Docs>(`/api/member/docs?locale=${props.language === 'zh' ? 'zh-CN' : 'en-US'}`),
        getJson<{ items: Model[] }>('/api/member/models'),
      ])
    } catch {
      [docs, listing] = await Promise.all([
        getJson<Docs>('/api/admin/settings'),
        getJson<{ items: Model[] }>('/api/admin/models'),
      ])
      capabilitiesPath = '/api/admin/models'
    }
    apiBase.value = docs.public_api_base_url.replace(/\/$/, '')
    if (!apiBase.value || !Array.isArray(listing.items)) throw new Error('Invalid API documentation')
    let failed = false
    for (const item of listing.items.filter(item => item.enabled)) {
      try {
        const capabilities = await getJson<Capabilities>(`${capabilitiesPath}/${encodeURIComponent(item.id)}/capabilities`)
        if (capabilities.protocols.includes(protocol[props.kind])) {
          model.value = item.public_name
          state.value = 'ready'
          return
        }
      } catch { failed = true }
    }
    state.value = failed ? 'error' : 'unavailable'
  } catch {
    state.value = 'error'
  }
})

const example = computed(() => {
  const url = `${apiBase.value}${path[props.kind]}`
  const lines = [`curl --fail-with-body '${url}' \\`, '  -H "Authorization: Bearer $ASTER_API_KEY" \\']
  if (props.kind === 'edit') {
    lines.push(`  -F "model=${model.value}" \\`, `  -F "prompt=${tx('把背景改成夜晚', 'Replace the background with a night scene')}" \\`, '  -F "image=@source.png"')
  } else {
    const payload = props.kind === 'responses' ? { model: model.value, input: tx('你好', 'Hello') }
      : props.kind === 'chat' ? { model: model.value, messages: [{ role: 'user', content: tx('你好', 'Hello') }] }
        : { model: model.value, prompt: tx('一只站在雨夜街头的猫', 'A cat on a rainy city street at night'), n: 1 }
    lines.push('  -H "Content-Type: application/json" \\', `  -d '${JSON.stringify(payload)}'`)
  }
  return lines.join('\n')
})

async function copy() {
  try {
    await copyText(example.value)
    copied.value = true
    window.setTimeout(() => { copied.value = false }, 1500)
  } catch { copied.value = false }
}
</script>

<template>
  <div v-if="localDocs" class="instance-curl-example" aria-live="polite">
    <p v-if="state === 'loading'">{{ tx('正在加载调用示例…', 'Loading request example…') }}</p>
    <p v-else-if="state === 'error'">{{ tx('暂时无法读取调用示例；请登录后刷新。', 'Could not load an example. Sign in and refresh.') }}</p>
    <p v-else-if="state === 'unavailable'">{{ tx('当前没有获授权且支持此操作的模型。', 'No granted model supports this operation.') }}</p>
    <template v-else>
      <div class="model-guide-code"><button type="button" :aria-label="tx('复制调用示例', 'Copy request example')" @click="copy">{{ copied ? tx('已复制', 'Copied') : tx('复制', 'Copy') }}</button><pre><code>{{ example }}</code></pre></div>
    </template>
  </div>
</template>
