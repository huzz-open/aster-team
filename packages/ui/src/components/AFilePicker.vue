<script setup lang="ts">
import { computed, ref, useId, watch } from 'vue'
import AIcon from './AIcon.vue'
import AIconButton from './AIconButton.vue'

const props = withDefaults(defineProps<{
  modelValue?: File | null
  label: string
  hint?: string
  emptyLabel?: string
  accept?: string
  required?: boolean
  disabled?: boolean
  loading?: boolean
  compact?: boolean
  clearLabel?: string
  name?: string
  icon?: string
}>(), {
  modelValue: null,
  hint: '',
  emptyLabel: '尚未选择文件',
  accept: '',
  required: false,
  disabled: false,
  loading: false,
  compact: false,
  clearLabel: '清除文件',
  name: '',
  icon: 'upload',
})

const emit = defineEmits<{
  'update:modelValue': [value: File | null]
  select: [value: File]
  clear: []
}>()

const input = ref<HTMLInputElement | null>(null)
const action = ref<HTMLButtonElement | null>(null)
const localFile = ref<File | null>(props.modelValue)
const invalid = ref(false)
const dragActive = ref(false)
const validationMessage = ref('')
const id = `a-file-${useId()}`
const hintID = `${id}-hint`
const errorID = `${id}-error`
const file = computed(() => props.modelValue || localFile.value)
const describedBy = computed(() => [props.hint ? hintID : '', validationMessage.value ? errorID : ''].filter(Boolean).join(' ') || undefined)

watch(() => props.modelValue, value => { localFile.value = value })

function openPicker() {
  if (props.disabled || props.loading || !input.value) return
  input.value.value = ''
  input.value.click()
}

function accepts(selected: File) {
  if (!props.accept) return true
  return props.accept.split(',').map(token => token.trim().toLowerCase()).filter(Boolean).some(token => {
    if (token.startsWith('.')) return selected.name.toLowerCase().endsWith(token)
    if (token.endsWith('/*')) return selected.type.toLowerCase().startsWith(token.slice(0, -1))
    return selected.type.toLowerCase() === token
  })
}

function commitFile(selected: File) {
  if (!accepts(selected)) {
    invalid.value = true
    validationMessage.value = '文件类型不受支持，请重新选择。'
    action.value?.focus()
    return
  }
  localFile.value = selected
  invalid.value = false
  validationMessage.value = ''
  emit('update:modelValue', selected)
  emit('select', selected)
}

function selectFile(event: Event) {
  const selected = (event.target as HTMLInputElement).files?.[0] || null
  if (selected) commitFile(selected)
}

function onDrop(event: DragEvent) {
  dragActive.value = false
  if (props.disabled || props.loading) return
  const selected = event.dataTransfer?.files?.[0]
  if (selected) commitFile(selected)
}

function clearFile() {
  if (props.disabled || props.loading) return
  if (input.value) input.value.value = ''
  localFile.value = null
  invalid.value = false
  validationMessage.value = ''
  emit('update:modelValue', null)
  emit('clear')
  action.value?.focus()
}

function onInvalid(event: Event) {
  event.preventDefault()
  invalid.value = true
  validationMessage.value = '请选择一个文件。'
  action.value?.focus()
}
</script>

<template>
  <div class="a-file-picker" :class="{ 'is-compact': compact, 'is-disabled': disabled, 'is-invalid': invalid, 'is-dragging': dragActive, 'has-file': file }"
    @dragenter.prevent="dragActive = true" @dragover.prevent="dragActive = true" @dragleave.self="dragActive = false" @drop.prevent="onDrop">
    <input :id="id" ref="input" class="a-file-picker-input" type="file" :name="name || undefined" :accept="accept || undefined"
      :disabled="disabled || loading" tabindex="-1" aria-hidden="true" :aria-label="label" :aria-describedby="describedBy" @change="selectFile">
    <input class="a-file-picker-proxy" type="text" :value="file?.name || ''" :required="required" :disabled="disabled || loading"
      tabindex="-1" aria-hidden="true" :aria-label="`${label}选择状态`" @invalid="onInvalid">
    <button ref="action" class="a-file-picker-action" type="button" :disabled="disabled || loading" :aria-controls="id" :aria-invalid="invalid || undefined"
      :aria-describedby="describedBy" @click="openPicker">
      <span v-if="loading" class="a-button-spinner" aria-hidden="true"></span><AIcon v-else :name="file ? 'file' : icon" :size="compact ? 15 : 18" />
      <span class="a-file-picker-action-copy"><strong>{{ label }}</strong><small v-if="!compact">{{ file?.name || emptyLabel }}</small></span>
    </button>
    <AIconButton v-if="file && !loading" class="a-file-picker-clear" icon="close" size="small" :label="clearLabel" :disabled="disabled" @click="clearFile" />
    <small v-if="hint && !compact" :id="hintID" class="a-file-picker-hint">{{ hint }}</small>
    <small v-if="validationMessage && !compact" :id="errorID" class="a-file-picker-error" role="alert">{{ validationMessage }}</small>
  </div>
</template>
