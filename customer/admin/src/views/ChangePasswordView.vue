<script setup lang="ts">
import { onMounted, reactive, ref } from 'vue'
import { useRouter } from 'vue-router'
import { ABrandMark, AButton, APasswordInput, useToast } from '@aster/ui'
import { request } from '@aster/sdk'
import { locale } from '../i18n'

const router = useRouter()
const toast = useToast()
const email = ref('')
const form = reactive({ current_password: '', new_password: '', confirm_password: '' })
const validationError = ref('')
const saving = ref(false)
const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
onMounted(async () => {
  try { email.value = (await request<{ email: string }>('/api/admin/me')).email }
  catch (value) { toast.error(value instanceof Error ? value.message : tx('请求失败', 'Request failed')) }
})
async function submit() {
  validationError.value = ''
  if (form.new_password !== form.confirm_password) { validationError.value = tx('两次输入的新密码不一致', 'The new passwords do not match'); return }
  saving.value = true
  try {
    await request('/api/admin/auth/password', { method: 'POST', body: JSON.stringify({ current_password: form.current_password, new_password: form.new_password }) })
    await router.replace({ path: '/login', query: { email: email.value, password_changed: '1' } })
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('密码修改失败', 'Could not change password')) }
  finally { saving.value = false }
}
async function logout() { await request('/api/admin/auth/logout', { method: 'POST' }); await router.replace('/login') }
</script>

<template>
  <div class="login-page"><form class="login-card" @submit.prevent="submit">
    <div class="brand"><ABrandMark class="brand-mark" label="" /><div><strong>Aster Team</strong><small>{{ tx('管理员安全设置', 'Administrator security') }}</small></div></div>
    <h1>{{ tx('先设置你的密码', 'Set your password first') }}</h1><p>{{ tx('当前使用安装器生成的临时密码。完成修改前，管理功能将保持锁定。', 'You are using an installer-generated temporary password. Admin features stay locked until it is changed.') }}</p>
    <div class="notice">{{ tx('管理员账号', 'Administrator') }}：{{ email }}</div>
    <div class="form">
      <div class="field"><span>{{ tx('当前临时密码', 'Current temporary password') }}</span><APasswordInput v-model="form.current_password" :aria-label="tx('当前临时密码', 'Current temporary password')" autocomplete="current-password" required autofocus :show-label="tx('显示密码', 'Show password')" :hide-label="tx('隐藏密码', 'Hide password')" /></div>
      <div class="field"><span>{{ tx('新密码', 'New password') }}</span><APasswordInput v-model="form.new_password" :aria-label="tx('新密码', 'New password')" minlength="12" autocomplete="new-password" required :show-label="tx('显示密码', 'Show password')" :hide-label="tx('隐藏密码', 'Hide password')" /><small>{{ tx('至少 12 个字符，不能与当前密码相同。', 'Use at least 12 characters and a value different from the current password.') }}</small></div>
      <div class="field"><span>{{ tx('确认新密码', 'Confirm new password') }}</span><APasswordInput v-model="form.confirm_password" :aria-label="tx('确认新密码', 'Confirm new password')" minlength="12" autocomplete="new-password" required :show-label="tx('显示密码', 'Show password')" :hide-label="tx('隐藏密码', 'Hide password')" /></div>
      <div v-if="validationError" class="error" role="alert">{{ validationError }}</div><AButton type="submit" :loading="saving">{{ tx('修改密码并重新登录', 'Change password and sign in again') }}</AButton>
      <AButton icon="logout" variant="ghost" @click="logout">{{ tx('退出登录', 'Sign out') }}</AButton>
    </div>
  </form></div>
</template>
