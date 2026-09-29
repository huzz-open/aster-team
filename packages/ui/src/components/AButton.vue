<script setup lang="ts">
import { computed } from 'vue'
import AIcon from './AIcon.vue'

type ButtonVariant = 'primary' | 'secondary' | 'danger' | 'ghost'
const props = withDefaults(defineProps<{
  variant?: ButtonVariant
  /** @deprecated Use variant. */
  kind?: ButtonVariant
  size?: 'small' | 'medium' | 'large'
  disabled?: boolean
  loading?: boolean
  type?: 'button' | 'submit' | 'reset'
  icon?: string | undefined
}>(), {
  size: 'medium',
  disabled: false,
  loading: false,
  type: 'button',
})

const resolvedVariant = computed(() => props.variant ?? props.kind ?? 'primary')
</script>

<template>
  <button :type="type" class="a-button" :class="[`a-button--${resolvedVariant}`, `a-button--${size}`, { 'is-loading': loading }]" :disabled="disabled || loading" :aria-busy="loading || undefined">
    <span v-if="loading" class="a-button-spinner" aria-hidden="true"></span><AIcon v-else-if="icon" :name="icon" :size="size === 'small' ? 14 : size === 'large' ? 18 : 16" /><slot />
  </button>
</template>
