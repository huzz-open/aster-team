<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, useId, watch } from 'vue'
import AIconButton from './AIconButton.vue'
const props = withDefaults(defineProps<{ open: boolean; title: string; description?: string; closeLabel?: string; closeDisabled?: boolean; compactHeader?: boolean; wide?: boolean }>(), { closeLabel: '关闭', closeDisabled: false, compactHeader: false, wide: false })
const emit = defineEmits<{ close: [] }>()
const card = ref<HTMLElement | null>(null)
const titleHeading = ref<HTMLElement | null>(null)
const titleID = `modal-${useId()}`
let previousFocus: HTMLElement | null = null
let previousOverflow = ''

function focusableElements() {
  return Array.from(card.value?.querySelectorAll<HTMLElement>('button:not([disabled]),a[href],input:not([disabled]):not([hidden]):not([tabindex="-1"]),select:not([disabled]):not([tabindex="-1"]),textarea:not([disabled]):not([tabindex="-1"]),[tabindex]:not([tabindex="-1"])') || [])
}
function initialFocusTarget() {
  return card.value?.querySelector<HTMLElement>('[autofocus]:not([disabled])')
    || titleHeading.value
    || focusableElements()[0]
}
function requestClose() {
  if (!props.closeDisabled) emit('close')
}
function handleKeydown(event: KeyboardEvent) {
  if (event.key !== 'Tab') return
  const elements = focusableElements()
  if (!elements.length) return
  const first = elements[0]!
  const last = elements[elements.length - 1]!
  if (!(document.activeElement instanceof HTMLElement) || !elements.includes(document.activeElement)) {
    event.preventDefault()
    ;(event.shiftKey ? last : first).focus()
    return
  }
  if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus() }
  else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus() }
}
function handleDocumentKeydown(event: KeyboardEvent) {
  if (event.key !== 'Escape' || !props.open) return
  const cards = Array.from(document.querySelectorAll<HTMLElement>('.modal-card'))
  if (cards.at(-1) !== card.value) return
  event.preventDefault()
  event.stopPropagation()
  requestClose()
}
watch(() => props.open, async (open) => {
  if (open) {
    previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null
    previousOverflow = document.body.style.overflow
    document.body.style.overflow = 'hidden'
    document.addEventListener('keydown', handleDocumentKeydown)
    await nextTick()
    initialFocusTarget()?.focus()
  } else {
    document.removeEventListener('keydown', handleDocumentKeydown)
    document.body.style.overflow = previousOverflow
    previousFocus?.focus()
  }
}, { immediate: true })
onBeforeUnmount(() => {
  document.removeEventListener('keydown', handleDocumentKeydown)
  document.body.style.overflow = previousOverflow
})
</script>

<template>
  <Teleport to="body">
    <Transition name="a-modal">
      <div v-if="open" class="modal-backdrop">
        <section ref="card" class="modal-card" :class="{ 'is-compact-header': compactHeader, 'is-wide': wide }" role="dialog" aria-modal="true" :aria-labelledby="titleID" @keydown="handleKeydown">
          <header class="modal-head">
            <div><h2 :id="titleID" ref="titleHeading" tabindex="-1">{{ title }}</h2><p v-if="description">{{ description }}</p></div>
            <AIconButton data-modal-close icon="close" :label="closeLabel" :disabled="closeDisabled" @click="requestClose" />
          </header>
          <slot />
        </section>
      </div>
    </Transition>
  </Teleport>
</template>
