<script setup lang="ts">
withDefaults(defineProps<{ label?: string }>(), { label: '正在加载…' })
</script>

<template><div class="loading-state" role="status" aria-live="polite"><span class="loading-state-spinner" aria-hidden="true"></span><span>{{ label }}</span></div></template>
