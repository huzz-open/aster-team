import { reactive } from 'vue'

export const DEFAULT_TOAST_DURATION_MS = 4_000

export type ToastTone = 'success' | 'warning' | 'error' | 'info'
export type Toast = { id: number; message: string; tone: ToastTone }

const toasts = reactive<Toast[]>([])
let nextID = 1
let defaultDuration = DEFAULT_TOAST_DURATION_MS

export function configureToastDefaultDuration(duration: number): void {
  defaultDuration = Math.max(1, duration)
}

export function dismissToast(id: number): void {
  const index = toasts.findIndex(item => item.id === id)
  if (index >= 0) toasts.splice(index, 1)
}

export function showToast(message: string, tone: ToastTone = 'info', duration = defaultDuration): number {
  const normalized = message.trim()
  if (!normalized) return 0
  const id = nextID++
  toasts.push({ id, message: normalized, tone })
  window.setTimeout(() => dismissToast(id), Math.max(1, duration))
  return id
}

export function useToast() {
  return {
    success: (message: string, duration?: number) => showToast(message, 'success', duration),
    warning: (message: string, duration?: number) => showToast(message, 'warning', duration),
    error: (message: string, duration?: number) => showToast(message, 'error', duration),
    info: (message: string, duration?: number) => showToast(message, 'info', duration),
  }
}

export const toastState = { toasts }
