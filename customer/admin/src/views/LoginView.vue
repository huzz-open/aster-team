<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { ABrandMark, AButton, ACopyCode, AModal, APasswordInput, useToast } from '@aster/ui'
import { copyText, request } from '@aster/sdk'
import { locale, setLocale, t, type Locale } from '../i18n'

const router = useRouter()
const route = useRoute()
const email = ref(String(route.query.email || ''))
const password = ref('')
const loading = ref(false)
const forgotOpen = ref(false)
const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const message = computed(() => route.query.password_changed === '1' ? tx('密码已修改，请使用新密码重新登录。', 'Password changed. Sign in again with your new password.') : '')
const resetCommand = 'sudo aster-team-cli password reset-admin'
const toast = useToast()

async function copyResetCommand() {
  await copyText(resetCommand)
}

async function submit() {
  loading.value = true
  try {
    const result = await request<{ password_change_required?: boolean }>('/api/admin/auth/login', {
      method: 'POST', body: JSON.stringify({ email: email.value, password: password.value }),
    })
    await router.replace(result.password_change_required ? '/change-password' : String(route.query.redirect || '/overview'))
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('登录失败', 'Sign-in failed')) }
  finally { loading.value = false }
}

onMounted(async () => {
  if (!import.meta.env.DEV || route.query.local_login !== '1') return
  loading.value = true
  try {
    const response = await fetch('/__aster_local_admin_credentials', { cache: 'no-store' })
    const credentials = await response.json() as { email?: string; password?: string; error?: string }
    if (!response.ok || !credentials.email || !credentials.password) throw new Error(credentials.error || tx('本地管理员凭据不完整', 'Local administrator credentials are incomplete'))
    email.value = credentials.email
    password.value = credentials.password
    await submit()
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('本地自动登录失败', 'Local automatic sign-in failed'))
    loading.value = false
  }
})
</script>

<template>
  <div class="login-page">
    <form class="login-card" @submit.prevent="submit">
      <div class="login-toolbar"><div class="brand"><ABrandMark class="brand-mark" label="" /><div><strong>Aster Team</strong><small>{{ t('workspace') }}</small></div></div><button class="login-language" type="button" :aria-label="locale === 'zh-CN' ? 'Switch to English' : '切换为中文'" :title="locale === 'zh-CN' ? 'Switch to English' : '切换为中文'" @click="setLocale((locale === 'zh-CN' ? 'en-US' : 'zh-CN') as Locale)">{{ locale === 'zh-CN' ? 'EN' : '中文' }}</button></div>
      <h2>{{ tx('登录管理端', 'Sign in to Admin') }}</h2>
      <div class="form">
        <label class="field"><span>{{ tx('管理员邮箱', 'Administrator email') }}</span><input v-model="email" type="email" autocomplete="username" required></label>
        <div class="field"><span class="login-field-heading"><span>{{ tx('密码', 'Password') }}</span><button type="button" class="login-forgot-link" @click="forgotOpen = true">{{ tx('忘记密码？', 'Forgot password?') }}</button></span><APasswordInput v-model="password" :aria-label="tx('密码', 'Password')" autocomplete="current-password" required :show-label="tx('显示密码', 'Show password')" :hide-label="tx('隐藏密码', 'Hide password')" /></div>
        <div v-if="message" class="notice" role="status">{{ message }}</div>
        <AButton type="submit" :loading="loading">{{ tx('进入管理端', 'Open Admin') }}</AButton>
      </div>
    </form>
    <AModal :open="forgotOpen" :title="tx('重置管理员密码', 'Reset administrator password')" :close-label="tx('关闭', 'Close')" compact-header @close="forgotOpen = false">
      <div class="login-reset-guide">
        <p>{{ tx('登录 Control 主机后运行下面的命令，按提示选择管理员账号并输入两次新密码。成功后该账号的现有会话会全部失效。', 'Sign in to the Control host and run this command. Select the administrator account and enter the new password twice. Existing sessions for that account will be revoked.') }}</p>
        <ACopyCode :value="resetCommand" :label="tx('复制', 'Copy')" :copied-label="tx('已复制', 'Copied')" @copy="copyResetCommand" />
      </div>
    </AModal>
  </div>
</template>
