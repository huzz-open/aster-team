<script setup lang="ts">
import AButton from './AButton.vue'
import AModal from './AModal.vue'

withDefaults(defineProps<{
  open: boolean
  title: string
  text: string
  confirmLabel?: string
  cancelLabel?: string
  danger?: boolean
  busy?: boolean
  confirmIcon?: string | undefined
}>(), { confirmLabel: '确认', cancelLabel: '取消', danger: false, busy: false })
defineEmits<{ close: []; confirm: [] }>()
</script>

<template><AModal :open="open" :title="title" :close-label="cancelLabel" :close-disabled="busy" @close="$emit('close')"><p class="confirm-message">{{ text }}</p><div class="form-actions"><AButton variant="secondary" :disabled="busy" @click="$emit('close')">{{ cancelLabel }}</AButton><AButton :variant="danger ? 'danger' : 'primary'" :icon="confirmIcon" :loading="busy" @click="$emit('confirm')">{{ confirmLabel }}</AButton></div></AModal></template>
