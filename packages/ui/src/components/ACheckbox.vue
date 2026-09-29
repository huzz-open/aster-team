<script setup lang="ts">
import { computed, useId } from 'vue'
import AIcon from './AIcon.vue'

const props = withDefaults(defineProps<{
  modelValue?: boolean | string[]
  value?: string
  label?: string
  description?: string
  disabled?: boolean
  name?: string
}>(), {
  modelValue: false,
  value: '',
  label: '',
  description: '',
  disabled: false,
  name: '',
})

const emit = defineEmits<{
  'update:modelValue': [value: boolean | string[]]
  change: [value: boolean | string[]]
}>()

const id = `a-checkbox-${useId()}`
const descriptionID = `${id}-description`
const checked = computed(() => Array.isArray(props.modelValue)
  ? props.modelValue.includes(props.value)
  : Boolean(props.modelValue))

function update(event: Event) {
  const nextChecked = (event.target as HTMLInputElement).checked
  const next = Array.isArray(props.modelValue)
    ? nextChecked
      ? [...new Set([...props.modelValue, props.value])]
      : props.modelValue.filter(item => item !== props.value)
    : nextChecked
  emit('update:modelValue', next)
  emit('change', next)
}
</script>

<template>
  <label class="a-checkbox" :class="{ 'is-checked': checked, 'is-disabled': disabled }" :for="id">
    <input :id="id" class="a-checkbox-input" type="checkbox" :name="name || undefined" :value="value || undefined"
      :checked="checked" :disabled="disabled" :aria-describedby="description ? descriptionID : undefined" @change="update">
    <span class="a-checkbox-mark" aria-hidden="true"><AIcon name="check" :size="13" /></span>
    <span class="a-checkbox-copy">
      <slot><strong>{{ label }}</strong><small v-if="description" :id="descriptionID">{{ description }}</small></slot>
    </span>
  </label>
</template>
