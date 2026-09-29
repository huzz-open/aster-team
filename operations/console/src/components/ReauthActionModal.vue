<script setup lang="ts">
import { ref, watch } from 'vue'
import { AButton, AModal, APasswordInput } from '@aster/ui'

const props = defineProps<{ open: boolean; title: string; description?: string; confirmLabel?: string; error?: string; busy?: boolean }>()
const emit = defineEmits<{ close: []; submit: [password: string] }>()
const password = ref('')
watch(() => props.open, open => { if (!open) password.value = '' })
function submit() { if (password.value) { emit('submit', password.value); password.value = '' } }
</script>

<template><AModal :open="open" :title="title" :close-disabled="busy" @close="emit('close')"><form class="form" @submit.prevent="submit"><div v-if="description" class="notice">{{ description }}</div><p v-if="error" class="reauth-error" role="alert">{{ error }}</p><div class="field"><span>操作密码</span><APasswordInput v-model="password" aria-label="当前操作员密码" autocomplete="current-password" required autofocus :disabled="busy" /></div><div class="form-actions"><AButton type="button" variant="secondary" :disabled="busy" @click="emit('close')">取消</AButton><AButton type="submit" :loading="busy" :disabled="!password">{{ confirmLabel || '确认执行' }}</AButton></div></form></AModal></template>

<style scoped>.reauth-error{margin:0;color:var(--danger)}</style>
