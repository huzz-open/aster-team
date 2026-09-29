<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import AIcon from './AIcon.vue'

export type SelectOption = {
  value: string | number
  label: string
  description?: string | undefined
  disabled?: boolean | undefined
}

const props = withDefaults(defineProps<{
  modelValue?: string | number | null
  options: readonly SelectOption[]
  placeholder?: string
  disabled?: boolean
  required?: boolean
  searchable?: boolean
  searchPlaceholder?: string
  emptyLabel?: string
  name?: string
  ariaLabel?: string
  popupMinWidth?: number
  align?: 'start' | 'center'
}>(), {
  modelValue: '', placeholder: '请选择', disabled: false, required: false, searchable: false,
  searchPlaceholder: '搜索选项', emptyLabel: '没有匹配的选项', name: '', ariaLabel: '', popupMinWidth: 180, align: 'start',
})
const emit = defineEmits<{
  'update:modelValue': [value: string | number]
  change: [value: string | number]
  open: []
  close: []
}>()

const root = ref<HTMLElement | null>(null)
const trigger = ref<HTMLButtonElement | null>(null)
const popup = ref<HTMLElement | null>(null)
const searchInput = ref<HTMLInputElement | null>(null)
const open = ref(false)
const query = ref('')
const activeIndex = ref(-1)
const invalid = ref(false)
const popupStyle = ref<Record<string, string>>({})
const instanceID = `a-select-${Math.random().toString(36).slice(2, 10)}`

const selected = computed(() => props.options.find(option => String(option.value) === String(props.modelValue)))
const showSearch = computed(() => props.searchable || props.options.length > 8)
const filtered = computed(() => {
  const needle = query.value.trim().toLocaleLowerCase()
  return needle ? props.options.filter(option => `${option.label} ${option.description || ''}`.toLocaleLowerCase().includes(needle)) : [...props.options]
})
const selectedLabel = computed(() => selected.value?.label || props.placeholder)

function availableIndex(start: number, direction: 1 | -1) {
  if (!filtered.value.length) return -1
  let index = start
  for (let count = 0; count < filtered.value.length; count += 1) {
    index = (index + direction + filtered.value.length) % filtered.value.length
    if (!filtered.value[index]?.disabled) return index
  }
  return -1
}
function setInitialActive() {
  const selectedIndex = filtered.value.findIndex(option => String(option.value) === String(props.modelValue) && !option.disabled)
  activeIndex.value = selectedIndex >= 0 ? selectedIndex : availableIndex(-1, 1)
}
function updatePosition() {
  if (!trigger.value || !open.value) return
  const rect = trigger.value.getBoundingClientRect()
  const margin = 8
  const viewportPadding = 10
  const availableBelow = window.innerHeight - rect.bottom - viewportPadding
  const availableAbove = rect.top - viewportPadding
  const estimated = Math.min(320, 16 + filtered.value.length * 42 + (showSearch.value ? 54 : 0))
  const placeAbove = availableBelow < Math.min(180, estimated) && availableAbove > availableBelow
  const maxHeight = Math.max(120, Math.min(320, (placeAbove ? availableAbove : availableBelow) - margin))
  const width = Math.min(Math.max(rect.width, props.popupMinWidth), window.innerWidth - 2 * viewportPadding)
  popupStyle.value = {
    left: `${Math.max(viewportPadding, Math.min(rect.left, window.innerWidth - width - viewportPadding))}px`,
    top: placeAbove ? `${Math.max(viewportPadding, rect.top - maxHeight - margin)}px` : `${rect.bottom + margin}px`,
    width: `${width}px`,
    maxHeight: `${maxHeight}px`,
    transformOrigin: placeAbove ? 'bottom center' : 'top center',
  }
}
async function openMenu() {
  if (props.disabled || open.value) return
  open.value = true
  query.value = ''
  setInitialActive()
  emit('open')
  await nextTick()
  updatePosition()
  if (showSearch.value) searchInput.value?.focus()
}
function closeMenu(restoreFocus = false) {
  if (!open.value) return
  open.value = false
  query.value = ''
  emit('close')
  if (restoreFocus) nextTick(() => trigger.value?.focus())
}
function choose(option: SelectOption) {
  if (option.disabled) return
  emit('update:modelValue', option.value)
  emit('change', option.value)
  invalid.value = false
  clearValidationOwner()
  closeMenu(true)
}
function move(direction: 1 | -1) {
  activeIndex.value = availableIndex(activeIndex.value, direction)
  nextTick(() => document.getElementById(`${instanceID}-option-${activeIndex.value}`)?.scrollIntoView({ block: 'nearest' }))
}
function onKeydown(event: KeyboardEvent) {
  if (props.disabled) return
  if (!open.value) {
    if (['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(event.key)) { event.preventDefault(); openMenu() }
    return
  }
  if (event.key === 'ArrowDown') { event.preventDefault(); move(1) }
  else if (event.key === 'ArrowUp') { event.preventDefault(); move(-1) }
  else if (event.key === 'Home') { event.preventDefault(); activeIndex.value = availableIndex(-1, 1) }
  else if (event.key === 'End') { event.preventDefault(); activeIndex.value = availableIndex(0, -1) }
  else if (event.key === 'Enter' || (event.key === ' ' && !showSearch.value)) {
    event.preventDefault(); const option = filtered.value[activeIndex.value]; if (option) choose(option)
  } else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); closeMenu(true) }
  else if (event.key === 'Tab') closeMenu()
}
function onDocumentPointerDown(event: PointerEvent) {
  const node = event.target as Node
  if (!root.value?.contains(node) && !popup.value?.contains(node)) closeMenu()
}
function onProxyInvalid(event: Event) {
  event.preventDefault()
  invalid.value = true
  const form = root.value?.closest('form')
  const owner = form?.dataset.asterSelectInvalidOwner
  if (owner && owner !== instanceID) return
  if (form) form.dataset.asterSelectInvalidOwner = instanceID
  trigger.value?.focus()
  openMenu()
}
function clearValidationOwner() {
  const form = root.value?.closest('form')
  if (form?.dataset.asterSelectInvalidOwner === instanceID) delete form.dataset.asterSelectInvalidOwner
}

watch(() => props.modelValue, () => { invalid.value = false; clearValidationOwner() })
watch(filtered, () => setInitialActive())
onMounted(() => {
  document.addEventListener('pointerdown', onDocumentPointerDown, true)
  window.addEventListener('resize', updatePosition)
  window.addEventListener('scroll', updatePosition, true)
})
onBeforeUnmount(() => {
  clearValidationOwner()
  document.removeEventListener('pointerdown', onDocumentPointerDown, true)
  window.removeEventListener('resize', updatePosition)
  window.removeEventListener('scroll', updatePosition, true)
})
defineExpose({ focus: () => trigger.value?.focus(), close: closeMenu })
</script>

<template>
  <div ref="root" class="a-select" :class="[`is-align-${align}`, { 'is-open': open, 'is-disabled': disabled, 'is-invalid': invalid }]">
    <button ref="trigger" type="button" class="a-select-trigger" role="combobox" aria-haspopup="listbox"
      :aria-label="ariaLabel || undefined" :aria-expanded="open" :aria-controls="instanceID" :aria-required="required || undefined"
      :aria-invalid="invalid || undefined" :aria-activedescendant="open && activeIndex >= 0 ? `${instanceID}-option-${activeIndex}` : undefined"
      :disabled="disabled" @click="open ? closeMenu() : openMenu()" @keydown="onKeydown">
      <span :class="{ placeholder: !selected }">{{ selectedLabel }}</span><AIcon class="a-select-chevron" name="chevron" :size="14" />
    </button>
    <input v-if="required || name" class="a-select-proxy" type="text" :name="name || undefined" :value="modelValue ?? ''" :required="required" tabindex="-1" aria-hidden="true" :aria-label="`${ariaLabel || placeholder}选择状态`" autocomplete="off" @invalid="onProxyInvalid">
    <Teleport to="body">
      <Transition name="a-select-popover">
        <div v-if="open" :id="`${instanceID}-popup`" ref="popup" class="a-select-popup" :class="`is-align-${align}`" :style="popupStyle" @keydown="onKeydown">
          <label v-if="showSearch" class="a-select-search"><AIcon name="search" :size="15" /><input ref="searchInput" v-model="query" type="search" role="searchbox" :aria-label="searchPlaceholder" :aria-controls="instanceID" :aria-activedescendant="activeIndex >= 0 ? `${instanceID}-option-${activeIndex}` : undefined" :placeholder="searchPlaceholder" autocomplete="off"></label>
          <div :id="instanceID" class="a-select-options" role="listbox" :aria-label="ariaLabel || undefined">
            <div v-for="(option,index) in filtered" :id="`${instanceID}-option-${index}`" :key="String(option.value)" class="a-select-option"
              :class="{ 'is-active': index === activeIndex, 'is-selected': String(option.value) === String(modelValue), 'is-disabled': option.disabled }"
              role="option" :aria-selected="String(option.value) === String(modelValue)" :aria-disabled="option.disabled || undefined"
              @pointermove="!option.disabled && (activeIndex=index)" @pointerdown.prevent @click="choose(option)">
              <span class="a-select-option-copy"><strong>{{ option.label }}</strong><small v-if="option.description">{{ option.description }}</small></span>
              <AIcon v-if="String(option.value) === String(modelValue)" name="check" :size="15" />
            </div>
            <div v-if="!filtered.length" class="a-select-empty">{{ emptyLabel }}</div>
          </div>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>
