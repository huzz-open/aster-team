<script setup lang="ts">
import AButton from './AButton.vue'
import AIcon from './AIcon.vue'
withDefaults(defineProps<{ title: string; text: string; actionLabel?: string }>(), { actionLabel: '' })
defineEmits<{ action: [] }>()
</script>

<template><section class="locked-state" role="status"><div class="locked-state-mark"><AIcon name="lock" :size="26" /></div><div class="locked-state-copy"><span class="eyebrow">License required</span><h1>{{ title }}</h1><p>{{ text }}</p></div><AButton v-if="actionLabel" @click="$emit('action')">{{ actionLabel }}</AButton></section></template>
