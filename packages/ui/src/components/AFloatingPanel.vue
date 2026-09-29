<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from 'vue'

const props = withDefaults(defineProps<{
  open: boolean
  anchor: HTMLElement | null
  id?: string | undefined
  tone?: 'dark' | 'surface'
  width?: number
  compact?: boolean
  interactive?: boolean
  role?: 'tooltip' | 'dialog' | 'menu'
  ariaLabel?: string | undefined
  positionKey?: string | number | null
  panelClass?: string
}>(), { tone: 'dark', compact: false, interactive: false, role: 'tooltip' })

const emit = defineEmits<{ close: []; pointerEnter: []; pointerLeave: [] }>()

const panel = ref<HTMLElement | null>(null)
const placement = ref<'top' | 'bottom'>('top')
const position = ref<Record<string, string>>({ left: '16px', top: '16px' })

function updatePosition() {
  if (!props.open || !props.anchor || !panel.value) return
  const anchorRect = props.anchor.getBoundingClientRect()
  const panelRect = panel.value.getBoundingClientRect()
  const gutter = 16
  const gap = 8
  const availableAbove = anchorRect.top - gutter
  const availableBelow = window.innerHeight - anchorRect.bottom - gutter
  placement.value = availableAbove >= panelRect.height + gap || availableAbove >= availableBelow ? 'top' : 'bottom'
  const centeredLeft = anchorRect.left + anchorRect.width / 2 - panelRect.width / 2
  const left = Math.min(Math.max(centeredLeft, gutter), Math.max(gutter, window.innerWidth - panelRect.width - gutter))
  const desiredTop = placement.value === 'top' ? anchorRect.top - panelRect.height - gap : anchorRect.bottom + gap
  const top = Math.min(Math.max(desiredTop, gutter), Math.max(gutter, window.innerHeight - panelRect.height - gutter))
  const arrowLeft = Math.min(Math.max(anchorRect.left + anchorRect.width / 2 - left, 14), panelRect.width - 14)
  position.value = {
    left: `${Math.round(left)}px`,
    top: `${Math.round(top)}px`,
    '--a-floating-arrow-left': `${Math.round(arrowLeft)}px`,
  }
}

function stopPositioning() {
  window.removeEventListener('resize', updatePosition)
  window.removeEventListener('scroll', updatePosition, true)
  document.removeEventListener('pointerdown', handleOutsidePointerDown)
}

function handleOutsidePointerDown(event: PointerEvent) {
  if (!props.interactive || !(event.target instanceof Node)) return
  if (panel.value?.contains(event.target) || props.anchor?.contains(event.target)) return
  emit('close')
}

async function startPositioning() {
  if (typeof window === 'undefined') return
  stopPositioning()
  if (!props.open) return
  window.addEventListener('resize', updatePosition)
  window.addEventListener('scroll', updatePosition, true)
  if (props.interactive) document.addEventListener('pointerdown', handleOutsidePointerDown)
  await nextTick()
  updatePosition()
}

watch(() => [props.open, props.anchor, props.interactive], startPositioning, { immediate: true })
watch(() => props.positionKey, updatePosition, { flush: 'post' })
onBeforeUnmount(stopPositioning)
</script>

<template>
  <Teleport to="body">
    <Transition name="a-floating-panel">
      <div v-if="open" :id="id" ref="panel" class="a-floating-panel" :class="[panelClass, `is-${tone}`, `is-${placement}`, { 'is-compact': compact, 'is-interactive': interactive }]" :style="[width ? { width: `${width}px` } : {}, position]" :role="role" :aria-label="ariaLabel" @mouseenter="emit('pointerEnter')" @mouseleave="emit('pointerLeave')">
        <slot />
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.a-floating-panel{position:fixed;z-index:300;width:max-content;max-width:calc(100vw - 32px);max-height:calc(100vh - 32px);padding:10px 11px;border:1px solid rgba(255,255,255,.1);border-radius:10px;color:#fff;background:#202433;box-shadow:0 14px 34px rgba(8,12,24,.26);font-size:var(--font-size-body);font-weight:500;line-height:1.5;white-space:normal;pointer-events:none}.a-floating-panel.is-interactive{overflow:auto;pointer-events:auto}.a-floating-panel.is-surface{color:var(--text-soft);border-color:var(--line-strong);background:var(--surface);box-shadow:var(--shadow-lg)}.a-floating-panel.is-compact{padding:6px 8px;border-radius:7px;font-size:var(--font-size-caption);font-weight:650;white-space:nowrap}.a-floating-panel:after{content:"";position:absolute;left:var(--a-floating-arrow-left);width:8px;height:8px;border-color:inherit;background:inherit;transform:translateX(-50%) rotate(45deg)}.a-floating-panel.is-top:after{top:100%;margin-top:-4px;border-right:1px solid;border-bottom:1px solid}.a-floating-panel.is-bottom:after{bottom:100%;margin-bottom:-4px;border-left:1px solid;border-top:1px solid}.a-floating-panel-enter-active,.a-floating-panel-leave-active{transition:opacity .14s ease,transform .14s ease}.a-floating-panel-enter-from,.a-floating-panel-leave-to{opacity:0;transform:translateY(4px)}
@media (prefers-reduced-motion:reduce){.a-floating-panel{transition:none}}
</style>
