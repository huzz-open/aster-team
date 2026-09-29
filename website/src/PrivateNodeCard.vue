<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{ variant: 'control' | 'data' | 'runner' | 'upstream' }>()
const glassId = computed(() => `private-node-glass-${props.variant}`)
const baseId = computed(() => `private-node-base-${props.variant}`)
const edgeId = computed(() => `private-node-edge-${props.variant}`)
const glowId = computed(() => `private-node-glow-${props.variant}`)
const faceGlowId = computed(() => `private-node-face-glow-${props.variant}`)
const sheenId = computed(() => `private-node-sheen-${props.variant}`)
const isPurple = computed(() => props.variant === 'upstream')
</script>

<template>
  <svg class="private-node-card" :class="`private-node-card--${variant}`" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
    <defs>
      <linearGradient :id="glassId" x1="0" y1="0" x2="1" y2="1">
        <template v-if="isPurple">
          <stop offset="0" stop-color="#52368e" stop-opacity=".82" />
          <stop offset=".38" stop-color="#2a1a58" stop-opacity=".92" />
          <stop offset="1" stop-color="#130d2d" stop-opacity=".98" />
        </template>
        <template v-else>
          <stop offset="0" stop-color="#326d8d" stop-opacity=".72" />
          <stop offset=".36" stop-color="#173f5d" stop-opacity=".88" />
          <stop offset="1" stop-color="#08172b" stop-opacity=".98" />
        </template>
      </linearGradient>
      <linearGradient :id="baseId" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0" :stop-color="isPurple ? '#24154d' : '#0b233a'" />
        <stop offset="1" stop-color="#040a16" />
      </linearGradient>
      <linearGradient :id="edgeId" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="#effcff" />
        <stop offset=".32" :stop-color="isPurple ? '#d7b2ff' : '#a7f6ff'" />
        <stop offset=".72" :stop-color="isPurple ? '#a45cff' : '#39e8ef'" />
        <stop offset="1" :stop-color="isPurple ? '#6934c8' : '#238fae'" />
      </linearGradient>
      <linearGradient :id="sheenId" x1="0" y1="0" x2=".76" y2="1">
        <stop offset="0" stop-color="#ffffff" stop-opacity=".3" />
        <stop offset=".18" stop-color="#bff8ff" stop-opacity=".1" />
        <stop offset=".48" stop-color="#ffffff" stop-opacity="0" />
        <stop offset="1" :stop-color="isPurple ? '#a769ff' : '#3de8e3'" stop-opacity=".1" />
      </linearGradient>
      <filter :id="glowId" x="-45%" y="-180%" width="190%" height="460%">
        <feGaussianBlur stdDeviation="3.4" result="blur" />
        <feMerge><feMergeNode in="blur" /><feMergeNode in="SourceGraphic" /></feMerge>
      </filter>
      <filter :id="faceGlowId" x="-20%" y="-22%" width="140%" height="145%">
        <feGaussianBlur stdDeviation="1.25" result="blur" />
        <feMerge><feMergeNode in="blur" /><feMergeNode in="SourceGraphic" /></feMerge>
      </filter>
    </defs>

    <path d="M8 13h84l8 8v70l-8 9H8l-8-9V21Z" :fill="`url(#${baseId})`" stroke="#142b47" stroke-width="1.15" />
    <path d="M7 8h86l6 6v73l-6 7H7l-6-7V14Z" :fill="isPurple ? '#2d1762' : '#103854'" :stroke="isPurple ? '#5e2db0' : '#246680'" stroke-width=".85" />
    <path d="M7 4h86l6 6v73l-6 7H7l-6-7V10Z" :fill="isPurple ? '#432278' : '#15536b'" opacity=".88" />
    <path class="private-node-card__gap" d="M4 78 10 86h80l6-8" fill="none" :stroke="isPurple ? '#b666ff' : '#48f6f0'" stroke-width="4.2" stroke-linecap="round" stroke-linejoin="round" opacity=".9" :filter="`url(#${glowId})`" />
    <path d="M7 1h86l6 6v70l-7 7H8l-7-7V7Z" :fill="`url(#${glassId})`" :stroke="`url(#${edgeId})`" stroke-width="1.18" :filter="`url(#${faceGlowId})`" />
    <path d="M8 3h83l5 5v67l-6 6H10l-6-6V9Z" :fill="`url(#${sheenId})`" stroke="#d9fbff" stroke-opacity=".18" stroke-width=".48" />
    <path d="M8 3h83l5 5" fill="none" stroke="#ffffff" stroke-opacity=".72" stroke-width=".82" stroke-linecap="round" />
    <path d="M4 75 10 81h80l6-6" fill="none" :stroke="isPurple ? '#dfb0ff' : '#b8fbff'" stroke-opacity=".72" stroke-width=".82" />
    <path d="M10 76h80" :stroke="isPurple ? '#b96cff' : '#64fff5'" stroke-opacity=".2" stroke-width="7" stroke-linecap="round" />

    <template v-if="variant === 'data'">
      <path d="M5 18c18 0 27 42 46 42s29-42 44-42M5 27c17 0 28 40 46 40s29-40 44-40" fill="none" stroke="#75e4ee" stroke-opacity=".12" stroke-width=".42" />
    </template>
    <template v-else-if="variant === 'control'">
      <path d="M10 73h79" stroke="#62efe7" stroke-opacity=".08" stroke-width="7" stroke-linecap="round" />
    </template>
    <template v-else-if="variant === 'upstream'">
      <path d="M9 9h82" stroke="#c49aff" stroke-opacity=".11" stroke-width="6" stroke-linecap="round" />
    </template>
  </svg>
</template>
