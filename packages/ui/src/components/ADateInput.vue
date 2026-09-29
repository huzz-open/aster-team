<script setup lang="ts">
import { ref } from 'vue'
import AIcon from './AIcon.vue'

withDefaults(defineProps<{
  modelValue?: string
  min?: string
  max?: string
  required?: boolean
  disabled?: boolean
  ariaLabel?: string
}>(), { modelValue: '', min: '', max: '', required: false, disabled: false, ariaLabel: '' })

const emit = defineEmits<{
  'update:modelValue': [value: string]
  change: [value: string]
}>()
const input = ref<HTMLInputElement | null>(null)

function update(event: Event) {
  const value = (event.target as HTMLInputElement).value
  emit('update:modelValue', value)
  emit('change', value)
}
function openPicker() {
  if (!input.value || input.value.disabled) return
  try { input.value.showPicker() } catch { input.value.focus() }
}
</script>

<template>
  <div class="a-date-input" :class="{ 'is-disabled': disabled }" @click="openPicker">
    <AIcon name="calendar" :size="15" />
    <input ref="input" type="date" :value="modelValue" :min="min || undefined" :max="max || undefined" :required="required" :disabled="disabled" :aria-label="ariaLabel || undefined" @input="update">
  </div>
</template>

<style scoped>
.a-date-input{min-width:0;height:var(--control-height);display:grid;grid-template-columns:auto minmax(0,1fr);align-items:center;gap:7px;padding:0 9px;color:var(--text-soft);border:1px solid var(--line-strong);border-radius:var(--control-radius);background:linear-gradient(180deg,var(--surface),color-mix(in srgb,var(--surface) 88%,var(--surface-2)));box-shadow:inset 0 1px 0 color-mix(in srgb,#fff 55%,transparent);cursor:pointer;transition:border-color .14s ease,box-shadow .14s ease,background .14s ease}.a-date-input:hover{border-color:color-mix(in srgb,var(--accent) 35%,var(--line-strong));background:var(--surface)}.a-date-input:focus-within{color:var(--accent);border-color:var(--accent);box-shadow:var(--focus-ring);background:var(--surface)}.a-date-input>.a-icon{color:var(--muted)}.a-date-input:focus-within>.a-icon{color:var(--accent)}.a-date-input input{width:100%;min-width:0;min-height:0;padding:0;border:0;border-radius:0;color:var(--text);background:transparent;box-shadow:none;font-size:var(--font-size-body);line-height:1;cursor:pointer}.a-date-input input:hover,.a-date-input input:focus,.a-date-input input:focus-visible{border:0;outline:0;box-shadow:none}.a-date-input input::-webkit-calendar-picker-indicator{width:0;margin:0;padding:0;opacity:0}.a-date-input.is-disabled{cursor:not-allowed;opacity:.55}.a-date-input.is-disabled input{cursor:not-allowed}
</style>
