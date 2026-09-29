<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { AButton, AModal } from '@aster/ui'
import { OPERATIONS_SESSION_EXPIRED_EVENT } from '../session-expiry'

const AUTO_REDIRECT_DELAY_MS = 1_500
const router = useRouter()
const open = ref(false)
let redirectTimer: number | undefined

function clearRedirectTimer(): void {
  if (redirectTimer !== undefined) window.clearTimeout(redirectTimer)
  redirectTimer = undefined
}

async function redirectToLogin(): Promise<void> {
  clearRedirectTimer()
  const currentRoute = router.currentRoute.value
  const redirect = currentRoute.meta.public ? undefined : currentRoute.fullPath
  open.value = false
  await router.replace({ path: '/login', query: redirect ? { redirect } : {} })
}

function handleSessionExpired(): void {
  if (open.value) return
  document.cookie = 'aster_operations_csrf=; Max-Age=0; Path=/; SameSite=Strict'
  open.value = true
  redirectTimer = window.setTimeout(() => void redirectToLogin(), AUTO_REDIRECT_DELAY_MS)
}

onMounted(() => window.addEventListener(OPERATIONS_SESSION_EXPIRED_EVENT, handleSessionExpired))
onBeforeUnmount(() => {
  window.removeEventListener(OPERATIONS_SESSION_EXPIRED_EVENT, handleSessionExpired)
  clearRedirectTimer()
})
</script>

<template>
  <AModal
    :open="open"
    title="登录状态已过期"
    description="当前登录会话已失效，即将自动返回登录页，请重新登录。"
    close-disabled
  >
    <div class="form-actions">
      <AButton autofocus @click="redirectToLogin">立即重新登录</AButton>
    </div>
  </AModal>
</template>
