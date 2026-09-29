<script setup lang="ts">
import { useId } from 'vue'

withDefaults(defineProps<{ size?: number; label?: string }>(), { size: 38, label: 'Aster Team' })
const id = `aster-brand-${useId().replace(/[^a-z0-9_-]/gi, '')}`
</script>

<template>
  <svg
    class="a-brand-mark"
    :width="size"
    :height="size"
    viewBox="0 0 1024 1024"
    :role="label ? 'img' : undefined"
    :aria-label="label || undefined"
    :aria-hidden="label ? undefined : 'true'"
    focusable="false"
  >
    <defs>
      <linearGradient :id="`${id}-left`" x1="470" y1="230" x2="210" y2="820" gradientUnits="userSpaceOnUse">
        <stop stop-color="#B7E8FF" />
        <stop offset="0.5" stop-color="#72ADFF" />
        <stop offset="1" stop-color="#327CFF" />
      </linearGradient>
      <linearGradient :id="`${id}-right`" x1="554" y1="230" x2="820" y2="820" gradientUnits="userSpaceOnUse">
        <stop stop-color="#B8FFE8" />
        <stop offset="0.5" stop-color="#77ECCA" />
        <stop offset="1" stop-color="#35DDBB" />
      </linearGradient>
      <linearGradient :id="`${id}-gateway`" x1="512" y1="620" x2="512" y2="820" gradientUnits="userSpaceOnUse">
        <stop stop-color="#A8F8DF" />
        <stop offset="1" stop-color="#5BE4C7" />
      </linearGradient>
    </defs>
    <rect width="1024" height="1024" fill="#061D3D" />
    <path :fill="`url(#${id}-left)`" d="M485 226c-18 0-34 10-43 27L151 790c-12 23 5 50 31 50h126c16 0 31-7 41-20l72-89c9-11 14-25 14-39v-67c0-14 5-27 15-37l35-36c10-10 15-23 15-37V241c0-8-7-15-15-15Z" />
    <path :fill="`url(#${id}-right)`" d="M539 226c18 0 34 10 43 27l291 537c12 23-5 50-31 50H716c-16 0-31-7-41-20l-72-89c-9-11-14-25-14-39v-67c0-14-5-27-15-37l-35-36c-10-10-15-23-15-37V241c0-8 7-15 15-15Z" />
    <path :fill="`url(#${id}-gateway)`" d="M512 634c-20 0-36 16-36 36v170h72V670c0-20-16-36-36-36Z" />
  </svg>
</template>
