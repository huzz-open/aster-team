<script setup lang="ts">
defineProps<{ name: string; size?: number }>()
</script>

<template>
  <svg class="a-icon" :width="size || 18" :height="size || 18" viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <template v-if="name === 'dashboard'"><rect x="3" y="3" width="7" height="7" rx="2"/><rect x="14" y="3" width="7" height="7" rx="2"/><rect x="3" y="14" width="7" height="7" rx="2"/><rect x="14" y="14" width="7" height="7" rx="2"/></template>
    <template v-else-if="name === 'tokens'"><ellipse cx="12" cy="5.5" rx="7.5" ry="3"/><path d="M4.5 5.5v5c0 1.7 3.4 3 7.5 3s7.5-1.3 7.5-3v-5"/><path d="M4.5 10.5v5c0 1.7 3.4 3 7.5 3s7.5-1.3 7.5-3v-5"/></template>
    <template v-else-if="name === 'trend-up'"><path d="m4 17 5-5 4 4 7-8"/><path d="M15 8h5v5"/></template>
    <template v-else-if="name === 'bolt'"><path d="m13 2-8 12h7l-1 8 8-12h-7z"/></template>
    <template v-else-if="name === 'pie'"><path d="M11 3a9 9 0 1 0 9 9h-9z"/><path d="M14 3.5A8.5 8.5 0 0 1 20.5 10H14z"/></template>
    <template v-else-if="name === 'target'"><circle cx="12" cy="12" r="8.5"/><circle cx="12" cy="12" r="4.5"/><circle cx="12" cy="12" r="1" fill="currentColor" stroke="none"/><path d="M12 1v2m0 18v2M1 12h2m18 0h2"/></template>
    <template v-else-if="name === 'share'"><circle cx="8" cy="8" r="3"/><circle cx="17" cy="7" r="2.5"/><path d="M2.5 20a5.5 5.5 0 0 1 11 0m1-7a4.5 4.5 0 0 1 7 3.8"/><path d="m15.5 18 2 2 3.5-4"/></template>
    <template v-else-if="name === 'cloud'"><path d="M6.5 19h11a4 4 0 0 0 .5-8 6 6 0 0 0-11.5-1.5A4.8 4.8 0 0 0 6.5 19z"/></template>
    <template v-else-if="name === 'cache'"><ellipse cx="10" cy="5" rx="6.5" ry="2.5"/><path d="M3.5 5v4c0 1.4 2.9 2.5 6.5 2.5 1.2 0 2.3-.1 3.2-.4M3.5 9v4c0 1.4 2.9 2.5 6.5 2.5"/><path d="M18.5 13.5v-3l2.5 2.5-2.5 2.5v-2a4.5 4.5 0 0 0-7.8 3M12.5 19.5v3L10 20l2.5-2.5v2a4.5 4.5 0 0 0 7.8-3"/></template>
    <template v-else-if="name === 'key'"><circle cx="8" cy="15" r="4"/><path d="m11 12 8-8m-2 2 2 2m-5 1 2 2"/></template>
    <template v-else-if="name === 'chart'"><path d="M4 20V10m6 10V4m6 16v-7m4 7H2"/></template>
    <template v-else-if="name === 'file'"><path d="M6 3h8l4 4v14H6z"/><path d="M14 3v5h5M9 13h6m-6 4h6"/></template>
    <template v-else-if="name === 'file-plus'"><path d="M6 3h8l4 4v14H6z"/><path d="M14 3v5h5M12 11v6m-3-3h6"/></template>
    <template v-else-if="name === 'user'"><circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/></template>
    <template v-else-if="name === 'users'"><circle cx="9" cy="8" r="3"/><path d="M3 20a6 6 0 0 1 12 0m2-9a3 3 0 0 1 0 6m0-12a3 3 0 0 1 0 6"/></template>
    <template v-else-if="name === 'ticket'"><path d="M4 6h16v4a2 2 0 0 0 0 4v4H4v-4a2 2 0 0 0 0-4z"/><path d="M12 8v8"/></template>
    <template v-else-if="name === 'model'"><path d="m12 3 8 4.5-8 4.5-8-4.5z"/><path d="m4 12.5 8 4.5 8-4.5M4 17l8 4 8-4"/></template>
    <template v-else-if="name === 'image'"><rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="8" cy="9" r="1.5"/><path d="m4 17 5-5 3 3 3-3 5 5"/></template>
    <template v-else-if="name === 'server'"><rect x="3" y="4" width="18" height="6" rx="2"/><rect x="3" y="14" width="18" height="6" rx="2"/><circle cx="7" cy="7" r=".5" fill="currentColor"/><circle cx="7" cy="17" r=".5" fill="currentColor"/></template>
    <template v-else-if="name === 'shield'"><path d="M12 3 20 6v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z"/><path d="m9 12 2 2 4-5"/></template>
    <template v-else-if="name === 'settings'"><circle cx="12" cy="12" r="3"/><path d="M19 12a7 7 0 0 0-.1-1l2-1.5-2-3.4-2.4 1a8 8 0 0 0-1.8-1L14.4 3h-4.8l-.4 3.1a8 8 0 0 0-1.8 1L5 6.1 3 9.5 5.1 11a7 7 0 0 0 0 2L3 14.5 5 18l2.4-1a8 8 0 0 0 1.8 1l.4 3h4.8l.4-3a8 8 0 0 0 1.8-1l2.4 1 2-3.5-2.1-1.5a7 7 0 0 0 .1-1z"/></template>
    <template v-else-if="name === 'maintenance'"><path d="M14.7 6.3a4 4 0 0 0-5-5L12 3.6 9.6 6 7.3 3.7a4 4 0 0 0 5 5L4 17a2.1 2.1 0 0 0 3 3l7.7-8.3a4 4 0 0 0 5-5L17.4 9 15 6.6z"/></template>
    <template v-else-if="name === 'customer'"><path d="M4 20V7l8-4 8 4v13M8 20v-5h8v5M8 9h.01M12 9h.01M16 9h.01"/></template>
    <template v-else-if="name === 'order'"><path d="M6 3h12v18H6zM9 8h6m-6 4h6m-6 4h4"/></template>
    <template v-else-if="name === 'trial'"><path d="M9 3h6m-5 0v5l-5 9a3 3 0 0 0 3 4h8a3 3 0 0 0 3-4l-5-9V3M8 15h8"/></template>
    <template v-else-if="name === 'sync'"><path d="M20 7h-6V1m-10 16h6v6M19 7a8 8 0 0 0-13-2M5 17a8 8 0 0 0 13 2"/></template>
    <template v-else-if="name === 'package'"><path d="m4 7 8-4 8 4-8 4zM4 7v10l8 4 8-4V7M12 11v10"/></template>
    <template v-else-if="name === 'audit'"><path d="M5 3h14v18H5zM8 8h8m-8 4h8m-8 4h5"/></template>
    <template v-else-if="name === 'menu'"><path d="M4 7h16M4 12h16M4 17h16"/></template>
    <template v-else-if="name === 'collapse'"><path d="m14 6-6 6 6 6"/></template>
    <template v-else-if="name === 'sun'"><circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M4.9 4.9l1.4 1.4m11.4 11.4 1.4 1.4M2 12h2m16 0h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></template>
    <template v-else-if="name === 'moon'"><path d="M20 15a8 8 0 0 1-11-11 9 9 0 1 0 11 11z"/></template>
    <template v-else-if="name === 'bell'"><path d="M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9M9 21h6"/></template>
    <template v-else-if="name === 'globe'"><circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a15 15 0 0 1 0 18M12 3a15 15 0 0 0 0 18"/></template>
    <template v-else-if="name === 'logout'"><path d="M10 4H4v16h6m5-4 4-4-4-4m4 4H9"/></template>
    <template v-else-if="name === 'chevron'"><path d="m9 6 6 6-6 6"/></template>
    <template v-else-if="name === 'close'"><path d="m6 6 12 12M18 6 6 18"/></template>
    <template v-else-if="name === 'plus'"><path d="M12 5v14M5 12h14"/></template>
    <template v-else-if="name === 'lock'"><rect x="5" y="10" width="14" height="11" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/></template>
    <template v-else-if="name === 'eye'"><path d="M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6S2.5 12 2.5 12z"/><circle cx="12" cy="12" r="2.7"/></template>
    <template v-else-if="name === 'eye-off'"><path d="M3 3l18 18M10.7 6.1A10.8 10.8 0 0 1 12 6c6 0 9.5 6 9.5 6a14.8 14.8 0 0 1-2.1 2.8M6.1 7.1C3.8 8.9 2.5 12 2.5 12s3.5 6 9.5 6a9.7 9.7 0 0 0 3-.5M9.9 9.9a3 3 0 0 0 4.2 4.2"/></template>
    <template v-else-if="name === 'inbox'"><path d="M4 5h16v14H4zM4 14h5l2 3h2l2-3h5"/></template>
    <template v-else-if="name === 'copy'"><rect x="9" y="9" width="11" height="11" rx="2"/><path d="M17 9V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3"/></template>
    <template v-else-if="name === 'trash'"><path d="M4 7h16M9 7V4h6v3m3 0-1 14H7L6 7m4 4v6m4-6v6"/></template>
    <template v-else-if="name === 'power'"><path d="M12 3v9m5.7-6.7a9 9 0 1 1-11.4 0"/></template>
    <template v-else-if="name === 'test'"><path d="M9 3h6m-5 0v5l-5 9a3 3 0 0 0 3 4h8a3 3 0 0 0 3-4l-5-9V3M8 15h8"/></template>
    <template v-else-if="name === 'download'"><path d="M12 3v12m-5-5 5 5 5-5M4 21h16"/></template>
    <template v-else-if="name === 'upload'"><path d="M12 16V4m-5 5 5-5 5 5M4 20h16"/></template>
    <template v-else-if="name === 'pause'"><path d="M9 5v14M15 5v14"/></template>
    <template v-else-if="name === 'play'"><path d="m8 5 11 7-11 7z"/></template>
    <template v-else-if="name === 'ban'"><circle cx="12" cy="12" r="9"/><path d="m6 6 12 12"/></template>
    <template v-else-if="name === 'payment'"><rect x="3" y="5" width="18" height="14" rx="2"/><path d="M3 10h18m-14 5h4"/></template>
    <template v-else-if="name === 'refund'"><path d="M9 7H5v-4M5 7a8 8 0 1 1-1 8"/></template>
    <template v-else-if="name === 'calendar'"><rect x="3" y="5" width="18" height="16" rx="2"/><path d="M7 3v4m10-4v4M3 10h18"/></template>
    <template v-else-if="name === 'alert'"><path d="M12 3 2.5 20h19zM12 9v5m0 3h.01"/></template>
    <template v-else-if="name === 'error'"><circle cx="12" cy="12" r="9"/><path d="m9 9 6 6m0-6-6 6"/></template>
    <template v-else-if="name === 'info'"><circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10h.01"/></template>
    <template v-else-if="name === 'edit'"><path d="m4 20 4.5-1 10-10a2 2 0 0 0-3-3l-10 10zM14 7l3 3"/></template>
    <template v-else-if="name === 'check'"><path d="m5 13 4 4L19 7"/></template>
    <template v-else-if="name === 'search'"><circle cx="11" cy="11" r="7"/><path d="m16 16 5 5"/></template>
    <template v-else><circle cx="12" cy="12" r="9"/><path d="M12 8v5m0 3h.01"/></template>
  </svg>
</template>

<style scoped>
.a-icon { display: block; flex: 0 0 auto; stroke: currentColor; stroke-width: 1.75; stroke-linecap: round; stroke-linejoin: round; }
</style>
