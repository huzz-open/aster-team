<script setup lang="ts">
import { dismissToast, toastState } from '../toast'
import AIcon from './AIcon.vue'

const icons = { success: 'check', warning: 'alert', error: 'error', info: 'info' } as const
</script>

<template>
  <Teleport to="body">
    <div class="toast-stack" aria-live="polite" aria-atomic="false">
      <TransitionGroup name="toast">
        <div v-for="toast in toastState.toasts" :key="toast.id" class="toast-item" :class="toast.tone" :role="toast.tone === 'error' ? 'alert' : 'status'">
          <span class="toast-icon" aria-hidden="true"><AIcon :name="icons[toast.tone]" :size="17" /></span>
          <span>{{ toast.message }}</span>
          <button type="button" aria-label="关闭提示" @click="dismissToast(toast.id)">×</button>
        </div>
      </TransitionGroup>
    </div>
  </Teleport>
</template>

<style scoped>
.toast-stack{position:fixed;z-index:3000;top:18px;left:50%;display:grid;width:min(520px,calc(100vw - 28px));gap:9px;transform:translateX(-50%);pointer-events:none}
.toast-item{display:grid;grid-template-columns:28px 1fr auto;align-items:center;gap:10px;padding:11px 12px;border:1px solid var(--line);border-radius:12px;background:var(--surface);color:var(--text);box-shadow:0 16px 44px rgba(15,23,42,.18);font-size:var(--font-size-body);line-height:1.5;pointer-events:auto}
.toast-icon{width:28px;height:28px;display:grid;place-items:center;border-radius:8px;color:var(--accent);background:var(--accent-soft)}
.toast-item.success{border-color:color-mix(in srgb,var(--positive) 28%,var(--line))}.toast-item.success .toast-icon{color:var(--positive);background:var(--positive-soft)}
.toast-item.warning{border-color:color-mix(in srgb,var(--warning) 32%,var(--line))}.toast-item.warning .toast-icon{color:var(--warning);background:var(--warning-soft)}
.toast-item.error{border-color:color-mix(in srgb,var(--danger) 30%,var(--line))}.toast-item.error .toast-icon{color:var(--danger);background:var(--danger-soft)}
.toast-item button{display:grid;width:26px;height:26px;padding:0;place-items:center;border:0;border-radius:7px;background:transparent;color:var(--muted);font:600 var(--font-size-title)/1 var(--font-ui);cursor:pointer}
.toast-item button:hover{background:var(--accent-soft);color:var(--text)}
.toast-enter-active,.toast-leave-active{transition:opacity .18s ease,transform .18s ease}
.toast-enter-from,.toast-leave-to{opacity:0;transform:translateY(-10px)}
</style>
