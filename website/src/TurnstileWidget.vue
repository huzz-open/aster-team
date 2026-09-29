<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { loadTurnstile } from './turnstile-script'

type TurnstileOptions = {
  sitekey: string
  action: string
  theme: 'dark'
  size: 'flexible'
  appearance: 'interaction-only'
  retry: 'auto'
  'retry-interval': number
  callback: (token: string) => void
  'error-callback': (code: string) => boolean
  'expired-callback': () => void
  'timeout-callback': () => void
}

type TurnstileApi = {
  render: (element: HTMLElement, options: TurnstileOptions) => string
  reset: (widgetId: string) => void
  remove: (widgetId: string) => void
}

declare global {
  interface Window {
    turnstile?: TurnstileApi
  }
}

const props = defineProps<{ sitekey: string }>()
const emit = defineEmits<{ token: [value: string]; error: [code: string] }>()
const root = ref<HTMLElement | null>(null)
let widgetId: string | null = null
let disposed = false
function clearToken() {
  emit('token', '')
}

async function mountWidget() {
  if (!root.value || !props.sitekey) return
  try {
    await loadTurnstile()
    if (disposed) return
    if (!root.value || !window.turnstile) throw new Error('Turnstile API is unavailable')
    widgetId = window.turnstile.render(root.value, {
      sitekey: props.sitekey,
      action: 'trial_request',
      theme: 'dark',
      size: 'flexible',
      appearance: 'interaction-only',
      retry: 'auto',
      'retry-interval': 8000,
      callback: token => { if (!disposed) emit('token', token) },
      'error-callback': code => {
        if (disposed) return true
        clearToken()
        emit('error', code)
        return true
      },
      'expired-callback': () => { if (!disposed) { clearToken(); emit('error', 'expired') } },
      'timeout-callback': () => { if (!disposed) { clearToken(); emit('error', 'timeout') } },
    })
  } catch {
    if (!disposed) emit('error', 'load_failed')
  }
}

function reset() {
  clearToken()
  if (widgetId && window.turnstile) window.turnstile.reset(widgetId)
}

defineExpose({ reset })

onMounted(() => { void mountWidget() })
onBeforeUnmount(() => {
  disposed = true
  if (widgetId && window.turnstile) window.turnstile.remove(widgetId)
  widgetId = null
})
</script>

<template>
  <div ref="root" class="turnstile-widget" aria-live="polite"></div>
</template>
