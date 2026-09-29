<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import AFloatingPanel from './AFloatingPanel.vue'
import AIcon from './AIcon.vue'

const props = withDefaults(defineProps<{
  text: string
  label?: string
  tone?: 'dark' | 'surface'
  width?: number
  interactive?: boolean
}>(), {
  tone: 'dark',
  width: 0,
  interactive: false,
})

const trigger = ref<HTMLElement | null>(null)
const hovered = ref(false)
const panelHovered = ref(false)
const focused = ref(false)
const open = computed(() => hovered.value || panelHovered.value || focused.value)
let closeTimer: ReturnType<typeof setTimeout> | undefined

function cancelClose() {
  if (closeTimer === undefined) return
  clearTimeout(closeTimer)
  closeTimer = undefined
}

function setHovered(value: boolean) {
  cancelClose()
  if (value) {
    hovered.value = true
    return
  }
  if (!props.interactive) {
    hovered.value = false
    return
  }
  closeTimer = setTimeout(() => {
    hovered.value = false
    closeTimer = undefined
  }, 100)
}

function setPanelHovered(value: boolean) {
  cancelClose()
  panelHovered.value = value
  if (!value) hovered.value = false
}
function setFocused(value: boolean) { focused.value = value }
function close() {
  cancelClose()
  hovered.value = false
  panelHovered.value = false
  focused.value = false
}

onBeforeUnmount(cancelClose)
</script>

<template>
  <span ref="trigger" class="a-info-tip" tabindex="0" :aria-label="label || text" @mouseenter="setHovered(true)" @mouseleave="setHovered(false)" @focus="setFocused(true)" @blur="setFocused(false)">
    <AIcon name="info" :size="17" />
  </span>
  <AFloatingPanel :open="open" :anchor="trigger" :tone="tone" :width="width" :interactive="interactive" @close="close" @pointer-enter="setPanelHovered(true)" @pointer-leave="setPanelHovered(false)"><slot>{{ text }}</slot></AFloatingPanel>
</template>

<style scoped>
.a-info-tip{position:relative;width:24px;height:24px;display:inline-grid;place-items:center;flex:0 0 auto;color:var(--muted);border-radius:7px;cursor:help;outline:none;transition:color .14s ease,background .14s ease,box-shadow .14s ease}.a-info-tip:hover,.a-info-tip:focus-visible{color:var(--accent);background:var(--accent-soft)}.a-info-tip:focus-visible{box-shadow:0 0 0 2px color-mix(in srgb,var(--accent) 22%,transparent)}
@media (prefers-reduced-motion:reduce){.a-info-tip{transition:none}}
</style>
