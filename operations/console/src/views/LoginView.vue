<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { ABrandMark, AButton, APasswordInput, useToast } from '@aster/ui'
import { login } from '../api/client'

const route = useRoute()
const router = useRouter()
const email = ref('')
const password = ref('')
const loading = ref(false)
const toast = useToast()

async function submit() {
  loading.value = true
  try {
    await login(email.value, password.value)
    await router.replace(String(route.query.redirect || '/overview'))
  } catch (value) {
    toast.error(value instanceof Error ? value.message : '登录失败')
  } finally {
    loading.value = false
  }
}

onMounted(async () => {
  if (!import.meta.env.DEV || route.query.local_login !== '1') return
  loading.value = true
  try {
    const response = await fetch('/__aster_local_admin_credentials', { cache: 'no-store' })
    const credentials = await response.json() as { email?: string; password?: string; error?: string }
    if (!response.ok || !credentials.email || !credentials.password) throw new Error(credentials.error || '本地管理员凭据不完整')
    email.value = credentials.email
    password.value = credentials.password
    await submit()
  } catch (value) {
    toast.error(value instanceof Error ? value.message : '本地自动登录失败')
    loading.value = false
  }
})
</script>

<template>
  <div class="login-page">
    <form class="login-card" @submit.prevent="submit">
      <div class="brand"><ABrandMark class="brand-mark" label="" /><div><strong>Aster Team</strong><small>内部运营系统</small></div></div>
      <div class="eyebrow">Operations Console</div>
      <h1>运营与授权管理</h1>
      <p>客户、试用、合同、授权和交付数据只保存在 Aster Team 内部环境。</p>
      <div class="form operations-login-form">
        <label class="field"><span>操作员邮箱</span><input v-model="email" type="email" autocomplete="username" required></label>
        <div class="field"><span>密码</span><APasswordInput v-model="password" aria-label="密码" autocomplete="current-password" required /></div>
        <AButton type="submit" :loading="loading">进入运营系统</AButton>
      </div>
    </form>
  </div>
</template>
