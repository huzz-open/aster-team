<script setup lang="ts">
withDefaults(defineProps<{
  icon: 'disable-user' | 'enable-user'
  label: string
  variant?: 'neutral' | 'accent' | 'danger'
  disabled?: boolean
}>(), { variant: 'neutral', disabled: false })

defineEmits<{ click: [event: MouseEvent] }>()
</script>

<template>
  <button type="button" class="a-icon-button a-icon-button--small" :class="`a-icon-button--${variant}`" :disabled="disabled" :aria-label="label" :title="label" @click="$emit('click', $event)">
    <span class="raster-action-icon" :class="`is-${icon}`" aria-hidden="true" />
  </button>
</template>

<style scoped>
.raster-action-icon{display:block;width:20px;height:20px;background-image:url('../assets/action-icons-sprite.png');background-repeat:no-repeat;background-size:80px 40px}
.is-disable-user{background-position:0 0}
.is-enable-user{background-position:-20px 0}
</style>
