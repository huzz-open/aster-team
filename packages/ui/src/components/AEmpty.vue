<script setup lang="ts">
import AIcon from './AIcon.vue'
withDefaults(defineProps<{ title: string; text?: string; icon?: string }>(), { icon: 'inbox', text: '' })
</script>
<template><div class="empty"><div class="empty-mark"><AIcon :name="icon" :size="21" /></div><strong>{{ title }}</strong><p v-if="text">{{ text }}</p><slot /></div></template>
