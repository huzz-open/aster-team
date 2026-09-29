<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{ variant: 'user' | 'admin' }>()
const fillId = computed(() => `private-portal-fill-${props.variant}`)
const edgeId = computed(() => `private-portal-edge-${props.variant}`)
const glowId = computed(() => `private-portal-glow-${props.variant}`)
const faceGlowId = computed(() => `private-portal-face-glow-${props.variant}`)
const sheenId = computed(() => `private-portal-sheen-${props.variant}`)
</script>

<template>
  <svg class="private-portal-card" viewBox="0 0 250 142" preserveAspectRatio="none" aria-hidden="true">
    <defs>
      <linearGradient :id="fillId" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" :stop-color="variant === 'admin' ? '#315284' : '#2a6685'" stop-opacity=".78" />
        <stop offset=".48" stop-color="#102b45" stop-opacity=".92" />
        <stop offset="1" stop-color="#081426" />
      </linearGradient>
      <linearGradient :id="edgeId" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="#effcff" />
        <stop offset=".38" :stop-color="variant === 'admin' ? '#9bb8ff' : '#a8f9ff'" />
        <stop offset="1" :stop-color="variant === 'admin' ? '#6476e5' : '#31cad6'" />
      </linearGradient>
      <linearGradient :id="sheenId" x1="0" y1="0" x2=".8" y2="1">
        <stop offset="0" stop-color="#ffffff" stop-opacity=".34" />
        <stop offset=".22" stop-color="#c6f7ff" stop-opacity=".1" />
        <stop offset=".52" stop-color="#ffffff" stop-opacity="0" />
        <stop offset="1" :stop-color="variant === 'admin' ? '#7082ea' : '#43e8e1'" stop-opacity=".08" />
      </linearGradient>
      <filter :id="glowId" x="-40%" y="-120%" width="180%" height="340%">
        <feGaussianBlur stdDeviation="7" result="blur" />
        <feMerge><feMergeNode in="blur" /><feMergeNode in="SourceGraphic" /></feMerge>
      </filter>
      <filter :id="faceGlowId" x="-15%" y="-25%" width="130%" height="150%">
        <feGaussianBlur stdDeviation="1.8" result="blur" />
        <feMerge><feMergeNode in="blur" /><feMergeNode in="SourceGraphic" /></feMerge>
      </filter>
    </defs>
    <path d="M18 14h214l17 17v94l-17 16H18L1 125V31Z" fill="#040b17" stroke="#17334f" stroke-width="2" />
    <path d="M18 8h214l15 15v96l-15 15H18L3 119V23Z" :fill="variant === 'admin' ? '#111f48' : '#0d334d'" :stroke="variant === 'admin' ? '#354d93' : '#226a82'" stroke-width="1.5" />
    <path d="M17 4h216l13 13v97l-13 13H17L4 114V17Z" :fill="variant === 'admin' ? '#27365e' : '#155069'" opacity=".86" />
    <path class="private-portal-card__gap" d="M8 106 20 119h210l12-13" fill="none" :stroke="variant === 'admin' ? '#7d8fff' : '#4afff4'" stroke-width="7" stroke-linecap="round" stroke-linejoin="round" opacity=".9" :filter="`url(#${glowId})`" />
    <path d="M17 1h216l13 13v90l-14 13H18L4 104V14Z" :fill="`url(#${fillId})`" :stroke="`url(#${edgeId})`" stroke-width="2.1" :filter="`url(#${faceGlowId})`" />
    <path d="M19 5h212l10 10v86l-11 11H20L9 102V16Z" :fill="`url(#${sheenId})`" stroke="#e5fbff" stroke-opacity=".2" stroke-width=".8" />
    <path d="M20 5h210l10 10" fill="none" stroke="#ffffff" stroke-opacity=".7" stroke-width="1.4" stroke-linecap="round" />
    <path d="m9 101 12 11h208l12-11" fill="none" :stroke="variant === 'admin' ? '#b6c0ff' : '#bdffff'" stroke-opacity=".78" stroke-width="1.5" />
    <path d="M30 104h190" :stroke="variant === 'admin' ? '#6f7fe2' : '#4beadd'" stroke-opacity=".25" stroke-width="13" stroke-linecap="round" />
  </svg>
</template>
