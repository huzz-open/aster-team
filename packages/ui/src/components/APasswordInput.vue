<script setup lang="ts">
import { computed, ref, useAttrs } from 'vue'
import AIcon from './AIcon.vue'

defineOptions({ inheritAttrs: false })

const props = withDefaults(defineProps<{
  modelValue?: string
  showLabel?: string
  hideLabel?: string
}>(), {
  modelValue: '',
  showLabel: '显示密码',
  hideLabel: '隐藏密码',
})

const emit = defineEmits<{ 'update:modelValue': [value: string] }>()
const attrs = useAttrs()
const visible = ref(false)
const actionLabel = computed(() => visible.value ? props.hideLabel : props.showLabel)

function update(event: Event) {
  emit('update:modelValue', (event.target as HTMLInputElement).value)
}
</script>

<template>
  <div class="a-password-input">
    <input
      v-bind="attrs"
      :value="modelValue"
      :type="visible ? 'text' : 'password'"
      @input="update"
    >
    <button
      class="a-password-toggle"
      type="button"
      :aria-label="actionLabel"
      :title="actionLabel"
      :aria-pressed="visible"
      @click="visible = !visible"
    >
      <AIcon :name="visible ? 'eye-off' : 'eye'" :size="17" />
    </button>
  </div>
</template>

<style scoped>
.a-password-input{position:relative;width:100%}
.a-password-input input{padding-right:44px}
.a-password-toggle{position:absolute;top:50%;right:6px;width:32px;height:32px;display:grid;place-items:center;padding:0;color:var(--muted);border:0;border-radius:8px;background:transparent;transform:translateY(-50%)}
.a-password-toggle:hover{color:var(--accent);background:var(--accent-soft)}
.a-password-toggle:focus-visible{outline:0;box-shadow:var(--focus-ring)}
</style>
