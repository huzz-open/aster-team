<script setup lang="ts">
import { onBeforeUnmount, ref, useAttrs } from 'vue'
import AFloatingPanel from './AFloatingPanel.vue'
import AIcon from './AIcon.vue'

defineOptions({ inheritAttrs: false })
const attrs = useAttrs()
const props = withDefaults(defineProps<{
  icon: string
  label: string
  variant?: 'neutral' | 'accent' | 'danger'
  size?: 'small' | 'medium'
  disabled?: boolean
  type?: 'button' | 'submit' | 'reset'
  loading?: boolean
}>(), { variant: 'neutral', size: 'medium', disabled: false, loading: false, type: 'button' })
const emit = defineEmits<{ click: [event: MouseEvent] }>()
const trigger = ref<HTMLButtonElement | null>(null)
const longPressed = ref(false)
const tooltipVisible = ref(false)
let suppressClick = false
let longPressTimer: ReturnType<typeof setTimeout> | undefined
let tooltipTimer: ReturnType<typeof setTimeout> | undefined
const tooltipID = `a-icon-tooltip-${Math.random().toString(36).slice(2, 9)}`
function clearLongPress() { if (longPressTimer) clearTimeout(longPressTimer); longPressTimer = undefined }
function clearTooltipTimer() { if (tooltipTimer) clearTimeout(tooltipTimer); tooltipTimer = undefined }
function showTooltip(delay = 0) {
  clearTooltipTimer()
  tooltipTimer = setTimeout(() => { tooltipVisible.value = true }, delay)
}
function hideTooltip() { clearTooltipTimer(); if (!longPressed.value) tooltipVisible.value = false }
function pointerDown(event: PointerEvent) {
  if (props.disabled || props.loading || event.button !== 0) return
  if (event.pointerType === 'mouse') return
  clearLongPress()
  longPressTimer = setTimeout(() => { longPressed.value = true; suppressClick = true; showTooltip() }, 480)
}
function pointerEnd() { clearLongPress(); if (longPressed.value) setTimeout(() => { longPressed.value = false; tooltipVisible.value = false; suppressClick = false }, 650) }
function click(event: MouseEvent) {
  if (suppressClick) { event.preventDefault(); event.stopPropagation(); suppressClick = false; return }
  emit('click', event)
}
function contextMenu(event: MouseEvent) { if (longPressed.value) event.preventDefault() }
onBeforeUnmount(() => { clearLongPress(); clearTooltipTimer() })
</script>

<template>
  <button v-bind="attrs" ref="trigger" :type="type" class="a-icon-button" :class="[`a-icon-button--${variant}`, `a-icon-button--${size}`, { 'is-loading': loading, 'is-long-press': longPressed }]" :disabled="disabled || loading" :aria-label="label" :aria-describedby="tooltipVisible ? tooltipID : undefined" :aria-busy="loading || undefined" @mouseenter="showTooltip(320)" @mouseleave="hideTooltip();pointerEnd()" @focus="showTooltip()" @blur="hideTooltip" @pointerdown="pointerDown" @pointerup="pointerEnd" @pointercancel="pointerEnd" @contextmenu="contextMenu" @click="click"><span v-if="loading" class="a-button-spinner" aria-hidden="true"></span><AIcon v-else :name="icon" :size="size === 'small' ? 15 : 17" /></button>
  <AFloatingPanel :id="tooltipID" :open="tooltipVisible" :anchor="trigger" compact>{{ label }}</AFloatingPanel>
</template>
