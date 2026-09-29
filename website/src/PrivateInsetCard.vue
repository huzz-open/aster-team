<script setup lang="ts">
import { useId } from 'vue'

const glowId = `private-inset-glow-${useId()}`
</script>

<template>
  <svg class="private-inset-card" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
    <defs>
      <filter :id="glowId" x="-25%" y="-55%" width="150%" height="210%">
        <feGaussianBlur stdDeviation="3.2" result="blur" />
        <feMerge><feMergeNode in="blur" /><feMergeNode in="SourceGraphic" /></feMerge>
      </filter>
    </defs>
    <path d="M10 3h80l9 9v78l-9 9H10l-9-9V12Z" fill="#080617" stroke="#32215e" stroke-width="1.5" />
    <path d="M10 1h80l9 9v75l-10 9H11L1 85V10Z" fill="#1b123b" stroke="#8c5ae0" stroke-width="1.2" />
    <path d="M12 5h76l7 7v70l-8 7H13l-8-7V12Z" fill="#211548" stroke="#be81ff" stroke-opacity=".35" stroke-width=".7" />
    <path d="M12 6h76" stroke="#e1c3ff" stroke-opacity=".52" stroke-width="1.1" stroke-linecap="round" />
    <path class="private-inset-card__gap" d="m6 81 8 8h72l8-8" fill="none" stroke="#a65cff" stroke-width="3.2" stroke-linecap="round" stroke-linejoin="round" opacity=".82" :filter="`url(#${glowId})`" />
  </svg>
</template>
