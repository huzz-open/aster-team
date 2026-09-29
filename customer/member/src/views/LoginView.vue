<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { ABrandMark, AButton, AModal, APasswordInput, useToast } from '@aster/ui'
import { request } from '@aster/sdk'
import { locale, setLocale, t } from '../i18n'
const router = useRouter(), route = useRoute()
const email = ref(String(route.query.email || '')), password = ref(''), loading = ref(false), forgotOpen = ref(false)
const message = ref(route.query.password_changed === '1' ? t('passwordChanged') : '')
const toast = useToast()
async function submit() { loading.value = true; message.value = ''; try { const result = await request<{ password_change_required: boolean }>('/api/member/auth/login', { method: 'POST', body: JSON.stringify({ email: email.value, password: password.value }) }); await router.push(result.password_change_required ? '/change-password' : String(route.query.redirect || '/home')) } catch (value) { toast.error(value instanceof Error ? value.message : t('loginFailed')) } finally { loading.value = false } }

onMounted(async () => {
  if (!import.meta.env.DEV || route.query.local_login !== '1') return
  loading.value = true
  try {
    const response = await fetch('/__aster_local_admin_credentials', { cache: 'no-store' })
    if (response.status === 503) {
      loading.value = false
      return
    }
    const credentials = await response.json() as { email?: string; password?: string; error?: string }
    if (!response.ok || !credentials.email || !credentials.password) throw new Error(credentials.error || t('loginFailed'))
    email.value = credentials.email
    password.value = credentials.password
    await submit()
  } catch (value) {
    toast.error(value instanceof Error ? value.message : t('loginFailed'))
    loading.value = false
  }
})
</script>
<template><div class="login-page"><form class="login-card" @submit.prevent="submit"><div class="login-toolbar"><div class="brand"><ABrandMark class="brand-mark" label="" /><div><strong>Aster Team</strong><small>{{ t('loginBadge') }}</small></div></div><button class="login-language" type="button" :aria-label="locale === 'zh-CN' ? 'Switch to English' : '切换为中文'" :title="locale === 'zh-CN' ? 'Switch to English' : '切换为中文'" @click="setLocale(locale === 'zh-CN' ? 'en-US' : 'zh-CN')">{{ locale === 'zh-CN' ? 'EN' : '中文' }}</button></div><h1>{{ t('loginTitle') }}</h1><div class="form"><label class="field"><span>{{ t('memberEmail') }}</span><input v-model="email" type="email" autocomplete="username" required></label><div class="field"><span class="login-field-heading"><span>{{ t('password') }}</span><button type="button" class="login-forgot-link" @click="forgotOpen = true">{{ t('forgotPassword') }}</button></span><APasswordInput v-model="password" :aria-label="t('password')" autocomplete="current-password" required :show-label="t('showPassword')" :hide-label="t('hidePassword')" /></div><div v-if="message" class="notice" role="status">{{ message }}</div><AButton type="submit" :loading="loading">{{ loading ? t('signingIn') : t('enterMember') }}</AButton></div></form><AModal :open="forgotOpen" :title="t('forgotPasswordTitle')" :close-label="t('close')" compact-header @close="forgotOpen = false"><div class="login-reset-guide"><p>{{ t('forgotPasswordGuide') }}</p></div></AModal></div></template>
