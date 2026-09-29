<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import AIconButton from './AIconButton.vue'

const props = withDefaults(defineProps<{
  value: string
  label: string
  copiedLabel: string
  layout?: 'single-line' | 'block'
  disabled?: boolean
  copyState?: 'idle' | 'copied'
  copiedIcon?: string
}>(), {
  layout: 'single-line',
  disabled: false,
})

const emit = defineEmits<{ copy: [event: MouseEvent] }>()
const localCopied = ref(false)
const isCopied = computed(() => props.copyState ? props.copyState === 'copied' : localCopied.value)
let copiedTimer: number | undefined

function copy(event: MouseEvent) {
  emit('copy', event)
  if (props.copyState) return
  localCopied.value = true
  if (copiedTimer) window.clearTimeout(copiedTimer)
  copiedTimer = window.setTimeout(() => { localCopied.value = false }, 1600)
}

onBeforeUnmount(() => {
  if (copiedTimer) window.clearTimeout(copiedTimer)
})
</script>

<template>
  <div class="a-copy-code command-box" :class="`a-copy-code--${props.layout}`">
    <code v-if="props.layout === 'single-line'" :title="props.value">{{ props.value }}</code>
    <pre v-else><code>{{ props.value }}</code></pre>
    <span v-if="props.layout === 'single-line'" class="a-copy-code-action">
      <AIconButton class="a-copy-code-button" :class="{ 'is-copied': isCopied }" :icon="isCopied && props.copiedIcon ? props.copiedIcon : 'copy'" size="small" :label="isCopied ? props.copiedLabel : props.label" :disabled="props.disabled" @click="copy" />
    </span>
    <span v-else class="a-copy-code-action a-copy-code-action--floating">
      <AIconButton class="a-copy-code-button" :class="{ 'is-copied': isCopied }" :icon="isCopied && props.copiedIcon ? props.copiedIcon : 'copy'" size="small" :label="isCopied ? props.copiedLabel : props.label" :disabled="props.disabled" @click="copy" />
    </span>
  </div>
</template>

<style scoped>
.a-copy-code{position:relative;min-width:0;border:1px solid #202b3e;border-radius:11px;background:#0e1625;color:#f6f8fc;overflow:hidden}
.a-copy-code--single-line{height:46px;display:flex;align-items:center;gap:8px;padding-left:14px}
.command-box code{font-family:var(--font-mono)}
.a-copy-code--single-line>code{display:block;min-width:0;flex:1;color:#d8e4ff;font-size:var(--font-size-caption);line-height:18px;white-space:pre;overflow:hidden;text-overflow:ellipsis}
.a-copy-code-action{width:44px;height:44px;display:grid;place-items:center;flex:0 0 44px}
.a-copy-code :deep(.a-copy-code-button){color:#dbe4f8;border-color:#39445c;background:#20283a;box-shadow:none}
.a-copy-code :deep(.a-copy-code-button:hover:not(:disabled)){color:#fff;border-color:#4a5873;background:#29344a;box-shadow:none}
.a-copy-code--block pre{max-height:320px;margin:0;padding:16px 64px 16px 16px;overflow:auto;color:#d8e4ff;font:var(--font-size-caption)/1.7 var(--font-mono);white-space:pre-wrap;overflow-wrap:anywhere}
.a-copy-code-action--floating{position:absolute;z-index:2;top:8px;right:8px}
</style>
