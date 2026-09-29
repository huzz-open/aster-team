<script setup lang="ts">
defineProps<{ name: 'play' | 'pause' | 'gateway' | 'github' | 'user' | 'users' | 'settings' | 'sliders' | 'user-code' | 'user-ops' | 'user-content' | 'server' | 'chart' | 'model' | 'shield' | 'check' | 'arrow' | 'download' | 'expand' | 'plus' | 'chevron-right' | 'chevron-down' | 'send' | 'copy' | 'wallet' | 'key' | 'database' | 'clock' | 'filter' | 'file' | 'calculator' | 'close'; size?: number }>()
</script>

<template>
  <svg class="site-icon" :data-icon="name" :width="size || 20" :height="size || 20" viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <template v-if="name === 'gateway'"><path d="M5 7h13m-4-4 4 4-4 4M19 17H6m4 4-4-4 4-4" /></template>
    <template v-else-if="name === 'github'"><path fill="currentColor" stroke="none" d="M12 2C6.48 2 2 6.58 2 12.23c0 4.52 2.87 8.35 6.84 9.71.5.1.68-.22.68-.49 0-.24-.01-1.05-.01-1.9-2.78.62-3.37-1.21-3.37-1.21-.45-1.18-1.11-1.49-1.11-1.49-.91-.64.07-.63.07-.63 1 .07 1.53 1.06 1.53 1.06.9 1.57 2.35 1.12 2.92.86.09-.66.35-1.12.64-1.38-2.22-.26-4.56-1.14-4.56-5.08 0-1.12.39-2.04 1.03-2.76-.1-.26-.45-1.31.1-2.72 0 0 .84-.28 2.75 1.05A9.32 9.32 0 0 1 12 6.23c.85 0 1.71.12 2.51.35 1.91-1.33 2.75-1.05 2.75-1.05.55 1.41.2 2.46.1 2.72.64.72 1.03 1.64 1.03 2.76 0 3.95-2.34 4.82-4.57 5.08.36.32.68.94.68 1.9 0 1.37-.01 2.47-.01 2.81 0 .27.18.59.69.49A10.25 10.25 0 0 0 22 12.23C22 6.58 17.52 2 12 2Z" /></template>
    <template v-else-if="name === 'user'"><circle cx="12" cy="7" r="3.5" /><path d="M5 21a7 7 0 0 1 14 0" /></template>
    <template v-else-if="name === 'users'"><circle cx="9" cy="8" r="3" /><path d="M3 20a6 6 0 0 1 12 0m2-9a3 3 0 0 1 0 6m0-12a3 3 0 0 1 0 6" /></template>
    <template v-else-if="name === 'settings'"><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.86 2.86-.06-.06A1.7 1.7 0 0 0 15 19.4a1.7 1.7 0 0 0-1 .6 1.7 1.7 0 0 0-.4 1.1V21H9.4v-.1A1.7 1.7 0 0 0 8 19.4a1.7 1.7 0 0 0-1.88.34l-.06.06-2.86-2.86.06-.06A1.7 1.7 0 0 0 3.6 15a1.7 1.7 0 0 0-.6-1 1.7 1.7 0 0 0-1.1-.4H2V9.4h-.1A1.7 1.7 0 0 0 3.6 8a1.7 1.7 0 0 0-.34-1.88l-.06-.06L6.06 3.2l.06.06A1.7 1.7 0 0 0 8 3.6a1.7 1.7 0 0 0 1-.6 1.7 1.7 0 0 0 .4-1.1V2h4.2v-.1A1.7 1.7 0 0 0 15 3.6a1.7 1.7 0 0 0 1.88-.34l.06-.06 2.86 2.86-.06.06A1.7 1.7 0 0 0 19.4 8c.13.4.34.75.6 1 .3.26.68.4 1.1.4h.1v4.2h-.1a1.7 1.7 0 0 0-1.7 1.4Z" /></template>
    <template v-else-if="name === 'sliders'"><path d="M4 6h16M4 12h16M4 18h16" /><circle cx="9" cy="6" r="2" /><circle cx="15" cy="12" r="2" /><circle cx="8" cy="18" r="2" /></template>
    <template v-else-if="name === 'user-code'"><circle cx="12" cy="7" r="3" /><path d="M6 20a6 6 0 0 1 12 0M5 10l-3 3 3 3m14-6 3 3-3 3" /></template>
    <template v-else-if="name === 'user-ops'"><circle cx="10" cy="7" r="3" /><path d="M4 20a6 6 0 0 1 12 0" /><circle cx="18" cy="15" r="3" /><path d="M18 10v2m0 6v2m-5-5h2m6 0h2" /></template>
    <template v-else-if="name === 'user-content'"><circle cx="9" cy="8" r="3" /><path d="M3 20a6 6 0 0 1 12 0m2-16 3 3-7 7-4 1 1-4 7-7Z" /></template>
    <template v-else-if="name === 'server'"><rect x="3" y="4" width="18" height="6" rx="2" /><rect x="3" y="14" width="18" height="6" rx="2" /><path d="M7 7h.01M7 17h.01M11 7h6M11 17h6" /></template>
    <template v-else-if="name === 'chart'"><path d="M4 20V10m6 10V4m6 16v-7m4 7H2" /></template>
    <template v-else-if="name === 'model'"><path d="m12 3 8 4-8 4-8-4 8-4Z" /><path d="m4 12 8 4 8-4M4 17l8 4 8-4" /></template>
    <template v-else-if="name === 'shield'"><path d="M12 3 4.5 6v5.5c0 4.5 3 7.6 7.5 9.5 4.5-1.9 7.5-5 7.5-9.5V6L12 3Z" /><path d="m9 12 2 2 4-5" /></template>
    <template v-else-if="name === 'check'"><path d="m5 12 4 4L19 6" /></template>
    <template v-else-if="name === 'arrow'"><path d="M4 12h15m-5-5 5 5-5 5" /></template>
    <template v-else-if="name === 'download'"><path d="M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5" /></template>
    <template v-else-if="name === 'expand'"><path d="M8 3H3v5m13-5h5v5M3 16v5h5m13-5v5h-5" /></template>
    <template v-else-if="name === 'plus'"><path d="M12 5v14M5 12h14" /></template>
    <template v-else-if="name === 'chevron-right'"><path d="m9 5 7 7-7 7" /></template>
    <template v-else-if="name === 'chevron-down'"><path d="m6 9 6 6 6-6" /></template>
    <template v-else-if="name === 'send'"><path d="m21 3-7.6 18-3.2-7.2L3 10.6 21 3Z" /><path d="m10.2 13.8 4.5-4.5" /></template>
    <template v-else-if="name === 'copy'"><rect x="8" y="8" width="11" height="11" rx="2" /><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3" /></template>
    <template v-else-if="name === 'wallet'"><path d="M4 6.5h14a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a3 3 0 0 1-3-3v-10a3 3 0 0 1 3-3h11" /><path d="M15 11h6v5h-6a2.5 2.5 0 0 1 0-5Z" /></template>
    <template v-else-if="name === 'key'"><circle cx="8" cy="15" r="5" /><path d="m11.5 11.5 8-8m-2 2 2 2m-5 1 2 2" /></template>
    <template v-else-if="name === 'database'"><ellipse cx="12" cy="5" rx="8" ry="3" /><path d="M4 5v7c0 1.7 3.6 3 8 3s8-1.3 8-3V5M4 12v7c0 1.7 3.6 3 8 3s8-1.3 8-3v-7" /></template>
    <template v-else-if="name === 'clock'"><circle cx="12" cy="12" r="9" /><path d="M12 7v6l4 2" /></template>
    <template v-else-if="name === 'filter'"><path d="M3 5h18l-7 8v6l-4 2v-8L3 5Z" /></template>
    <template v-else-if="name === 'file'"><path d="M6 3h8l4 4v14H6V3Z" /><path d="M14 3v5h5M9 12h6M9 16h6" /></template>
    <template v-else-if="name === 'calculator'"><rect x="4" y="2.5" width="16" height="19" rx="2" /><path d="M7.5 6h9v3h-9zM8 13h.01M12 13h.01M16 13h.01M8 17h.01M12 17h.01M16 17h.01" /></template>
    <template v-else-if="name === 'close'"><path d="m5 5 14 14M19 5 5 19" /></template>
    <template v-else-if="name === 'play'"><path d="m8 4 12 8-12 8V4Z" /></template>
    <template v-else-if="name === 'pause'"><path d="M8 4v16M16 4v16" /></template>
  </svg>
</template>

<style scoped>.site-icon{display:block;stroke:currentColor;stroke-width:1.8;stroke-linecap:round;stroke-linejoin:round}</style>
