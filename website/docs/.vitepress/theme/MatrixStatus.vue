<script setup lang="ts">
import { onBeforeUnmount, ref, useId, watch } from 'vue'
import AFloatingPanel from '../../../../packages/ui/src/components/AFloatingPanel.vue'

defineProps<{ status: 'supported' | 'mapped' | 'unsupported'; label: string }>()
const root = ref<HTMLElement | null>(null)
const open = ref(false)
const panelId = useId()
let hovered = false
let pinned = false
let selecting = false
let closeTimer: ReturnType<typeof setTimeout> | undefined

function cancelClose() {
  clearTimeout(closeTimer)
  closeTimer = undefined
}
function close() {
  cancelClose()
  pinned = false
  open.value = false
}
function hasSelection() {
  const selection = window.getSelection()
  return selection && !selection.isCollapsed &&
    (contains(selection.anchorNode) || contains(selection.focusNode))
}
function contains(node: Node | null) {
  return !!node && (root.value?.contains(node) || document.getElementById(panelId)?.contains(node))
}
function scheduleClose() {
  cancelClose()
  closeTimer = setTimeout(() => {
    if (!hovered && !pinned && !selecting && !contains(document.activeElement) && !hasSelection()) close()
  }, 180)
}
function enter(event: PointerEvent) {
  if (event.pointerType === 'touch') return
  hovered = true
  cancelClose()
  open.value = true
}
function enterPanel() {
  hovered = true
  cancelClose()
}
function leave() {
  hovered = false
  scheduleClose()
}
function toggle() {
  if (pinned) close()
  else {
    pinned = true
    cancelClose()
    open.value = true
  }
}

// Only open panels listen globally. Selection can extend outside the panel
// during a drag without losing the data that the user is copying.
watch(open, (visible, _previous, cleanup) => {
  if (!visible) return
  const press = (event: PointerEvent) => { if (document.getElementById(panelId)?.contains(event.target as Node)) selecting = true }
  const release = () => { selecting = false; scheduleClose() }
  const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') close() }
  document.addEventListener('pointerdown', press)
  document.addEventListener('pointerup', release)
  document.addEventListener('pointercancel', release)
  document.addEventListener('selectionchange', scheduleClose)
  document.addEventListener('keydown', escape)
  cleanup(() => {
    document.removeEventListener('pointerdown', press)
    document.removeEventListener('pointerup', release)
    document.removeEventListener('pointercancel', release)
    document.removeEventListener('selectionchange', scheduleClose)
    document.removeEventListener('keydown', escape)
    selecting = false
  })
})
onBeforeUnmount(cancelClose)
</script>

<template>
  <div ref="root" class="matrix-status" :class="`matrix-status--${status}`"
    @pointerenter="enter" @pointerleave="leave" @focusout="scheduleClose">
    <button type="button" class="matrix-status-trigger" :aria-expanded="open" :aria-controls="panelId"
      @focus="open = true" @click="toggle">{{ label }}</button>
    <AFloatingPanel :id="panelId" :open="open" :anchor="root" :width="340" tone="surface"
      panel-class="matrix-popover" role="dialog" :aria-label="label" interactive
      @close="close" @pointer-enter="enterPanel" @pointer-leave="leave">
      <strong>{{ label }}</strong>
      <slot />
    </AFloatingPanel>
  </div>
</template>
