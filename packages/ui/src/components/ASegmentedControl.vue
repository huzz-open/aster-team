<script setup lang="ts">
import { nextTick, useId } from 'vue'
import AIcon from './AIcon.vue'

export type SegmentedOption = {
  value: string | number
  label: string
  icon?: string | undefined
  disabled?: boolean | undefined
}

const props = withDefaults(defineProps<{
  modelValue: string | number
  options: readonly SegmentedOption[]
  label: string
  size?: 'small' | 'medium'
  stretch?: boolean
}>(), { size: 'medium', stretch: false })

const emit = defineEmits<{
  'update:modelValue': [value: string | number]
  change: [value: string | number]
}>()

const id = `a-segmented-${useId()}`
function isSelected(value: string | number) { return String(value) === String(props.modelValue) }
function isTabStop(value: string | number, index: number) {
  if (isSelected(value)) return true
  const hasSelection = props.options.some(option => isSelected(option.value))
  if (hasSelection) return false
  return props.options.findIndex(option => !option.disabled) === index
}
function choose(option: SegmentedOption) {
  if (option.disabled || isSelected(option.value)) return
  emit('update:modelValue', option.value)
  emit('change', option.value)
}
function move(event: KeyboardEvent, index: number) {
  const forward = event.key === 'ArrowRight' || event.key === 'ArrowDown'
  const backward = event.key === 'ArrowLeft' || event.key === 'ArrowUp'
  if (!forward && !backward && event.key !== 'Home' && event.key !== 'End') return
  event.preventDefault()
  const group = (event.currentTarget as HTMLElement | null)?.parentElement
  let cursor = event.key === 'Home' ? -1 : event.key === 'End' ? 0 : index
  const direction: 1 | -1 = event.key === 'End' || backward ? -1 : 1
  for (let count = 0; count < props.options.length; count += 1) {
    cursor = (cursor + direction + props.options.length) % props.options.length
    const option = props.options[cursor]
    if (option && !option.disabled) {
      choose(option)
      nextTick(() => group?.querySelectorAll<HTMLButtonElement>('[role="radio"]')[cursor]?.focus())
      return
    }
  }
}
</script>

<template>
  <div :id="id" class="a-segmented" :class="[`a-segmented--${size}`, { 'is-stretched': stretch }]" role="radiogroup" :aria-label="label">
    <button v-for="(option,index) in options" :key="String(option.value)" type="button" role="radio"
      :class="{ 'is-selected': isSelected(option.value) }" :aria-checked="isSelected(option.value)"
      :tabindex="isTabStop(option.value,index) ? 0 : -1" :disabled="option.disabled" @click="choose(option)" @keydown="move($event,index)">
      <AIcon v-if="option.icon" :name="option.icon" :size="14" /><span>{{ option.label }}</span>
    </button>
  </div>
</template>
