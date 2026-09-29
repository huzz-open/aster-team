<script setup lang="ts">
import { onMounted, reactive, ref } from 'vue'
import { useRouter } from 'vue-router'
import { ABrandMark, AButton, APasswordInput, useToast } from '@aster/ui'
import { request } from '@aster/sdk'
import { t } from '../i18n'

const router = useRouter()
const toast = useToast()
const email = ref('')
const form = reactive({ current_password: '', new_password: '', confirm_password: '' })
const validationError = ref('')
const saving = ref(false)

onMounted(async () => {
  try {
    const profile = await request<{ email: string }>('/api/member/me')
    email.value = profile.email
  } catch (value) {
    toast.error(value instanceof Error ? value.message : t('loginFailed'))
  }
})

async function submit() {
  validationError.value = ''
  if (form.new_password !== form.confirm_password) {
    validationError.value = t('mismatch')
    return
  }
  saving.value = true
  try {
    await request('/api/member/auth/password', {
      method: 'POST',
      body: JSON.stringify({ current_password: form.current_password, new_password: form.new_password }),
    })
    await router.replace({ path: '/login', query: { email: email.value, password_changed: '1' } })
  } catch (value) {
    toast.error(value instanceof Error ? value.message : t('changeFailed'))
  } finally {
    saving.value = false
  }
}

async function logout() {
  await request('/api/member/auth/logout', { method: 'POST' })
  await router.replace('/login')
}
</script>

<template>
  <div class="login-page">
    <form class="login-card" @submit.prevent="submit">
      <div class="brand"><ABrandMark class="brand-mark" label="" /><div><strong>Aster Team</strong><small>{{ t('securitySetup') }}</small></div></div>
      <h1>{{ t('firstPasswordTitle') }}</h1>
      <p>{{ t('firstPasswordDesc') }}</p>
      <div class="notice">{{ t('memberAccount', { email }) }}</div>
      <div class="form">
        <div class="field"><span>{{ t('initialPassword') }}</span><APasswordInput v-model="form.current_password" :aria-label="t('initialPassword')" autocomplete="current-password" required autofocus /></div>
        <div class="field"><span>{{ t('newPassword') }}</span><APasswordInput v-model="form.new_password" :aria-label="t('newPassword')" minlength="12" autocomplete="new-password" required /><small>{{ t('initialRule') }}</small></div>
        <div class="field"><span>{{ t('confirmPassword') }}</span><APasswordInput v-model="form.confirm_password" :aria-label="t('confirmPassword')" minlength="12" autocomplete="new-password" required /></div>
        <div v-if="validationError" class="error" role="alert">{{ validationError }}</div>
        <AButton type="submit" :loading="saving">{{ t('saveAndLogin') }}</AButton>
        <AButton icon="logout" variant="ghost" @click="logout">{{ t('signOut') }}</AButton>
      </div>
    </form>
  </div>
</template>
