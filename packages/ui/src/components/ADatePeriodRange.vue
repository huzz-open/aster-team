<script setup lang="ts">
import { nextTick } from 'vue'
import ADateRange from './ADateRange.vue'
import ASegmentedControl, { type SegmentedOption } from './ASegmentedControl.vue'

const props = withDefaults(defineProps<{
  modelValue: string | number
  from?: string
  to?: string
  options: readonly SegmentedOption[]
  label: string
  customValue?: string | number
  min?: string
  max?: string
  startLabel?: string
  endLabel?: string
  locale?: 'zh-CN' | 'en-US'
  size?: 'small' | 'medium'
  maxRangeDays?: number
}>(), {
  from: '',
  to: '',
  customValue: 'custom',
  min: '',
  max: '',
  startLabel: '开始日期',
  endLabel: '结束日期',
  locale: 'zh-CN',
  size: 'small',
  maxRangeDays: 0,
})

const emit = defineEmits<{
  'update:modelValue': [value: string | number]
  'update:from': [value: string]
  'update:to': [value: string]
  change: [value: string | number]
}>()

function choosePeriod(value: string | number) {
  const preset = String(value)
  if (preset === 'all') {
    emit('update:from', '')
    emit('update:to', '')
  } else {
    const match = /^(\d+)(d|h)$/.exec(preset)
    const anchor = parseLocalDate(props.max) || new Date()
    if (match) {
      const amount = Number(match[1])
      const days = match[2] === 'h' ? Math.max(1, Math.ceil(amount / 24)) : amount
      const start = new Date(anchor.getFullYear(), anchor.getMonth(), anchor.getDate() - days + 1)
      emit('update:from', formatLocalDate(start))
      emit('update:to', formatLocalDate(anchor))
    }
  }
  emit('update:modelValue', value)
  emit('change', value)
}

async function chooseCustom() {
  emit('update:modelValue', props.customValue)
  await nextTick()
  if (props.from && props.to) emit('change', props.customValue)
}

function parseLocalDate(value: string) {
  const parts = value.split('-').map(Number)
  if (parts.length !== 3 || parts.some(part => !Number.isFinite(part))) return null
  return new Date(parts[0]!, parts[1]! - 1, parts[2]!)
}

function formatLocalDate(value: Date) {
  return `${value.getFullYear()}-${String(value.getMonth() + 1).padStart(2, '0')}-${String(value.getDate()).padStart(2, '0')}`
}
</script>

<template>
  <div class="a-date-period-range" :class="{ 'is-custom': String(modelValue) === String(customValue) }">
    <ASegmentedControl :model-value="modelValue" :options="options" :label="label" :size="size" @update:model-value="choosePeriod" />
    <ADateRange
      class="a-date-period-range__date"
      :from="from"
      :to="to"
      :min="min"
      :max="max"
      :max-range-days="maxRangeDays"
      :start-label="startLabel"
      :end-label="endLabel"
      :locale="locale"
      @update:from="emit('update:from', $event)"
      @update:to="emit('update:to', $event)"
      @change="chooseCustom"
    />
  </div>
</template>

<style scoped>
.a-date-period-range{display:flex;align-items:center;gap:8px;min-width:0}.a-date-period-range__date{flex:0 0 220px;transition:opacity .15s,border-color .15s}.a-date-period-range:not(.is-custom) .a-date-period-range__date{opacity:.72}.a-date-period-range.is-custom .a-date-period-range__date{opacity:1}
@media(max-width:720px){.a-date-period-range{align-items:stretch;flex-direction:column}.a-date-period-range__date{width:100%;max-width:100%;flex:1 1 auto}}
</style>
