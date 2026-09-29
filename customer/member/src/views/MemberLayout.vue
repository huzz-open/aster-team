<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { AConsoleShell, type ConsoleNavSection } from '@aster/ui'
import { request, type User } from '@aster/sdk'
import { locale, setLocale, t, type Locale } from '../i18n'
import { memberLicense } from '../license-status'

const router = useRouter()
const user = ref<User | null>(null)
const sections = computed<ConsoleNavSection[]>(() => [
  { label: t('console'), items: [
    { to: '/home', label: t('dashboard'), icon: 'dashboard' },
    { to: '/keys', label: t('keys'), icon: 'key' },
    { to: '/usage', label: t('usage'), icon: 'chart' },
    { to: '/logs', label: t('consumptionLogs'), icon: 'audit' },
    { to: '/models', label: t('modelMarketplace'), icon: 'model' },
  ] },
  { label: t('personal'), items: [
    { to: '/quota', label: locale.value === 'en-US' ? 'Billing center' : '费用中心', icon: 'payment' },
    { to: '/account', label: t('account'), icon: 'user' },
  ] },
])
const labels = computed(() => ({ notice: t('notice'), theme: t('theme'), language: t('language'), profile: t('profile'), logout: t('logout'), collapse: t('collapse'), allClear: t('allClear') }))
const documentationUrl = computed(() => `${window.location.origin}/docs/${locale.value === 'en-US' ? 'en' : 'zh-cn'}/`)
const quickLinks = computed(() => [{ href: documentationUrl.value, label: t('docs'), icon: 'file', keepVisibleOnMobile: true }])
const profileLinks = computed(() => [{ to: '/account/password', label: t('changePassword'), icon: 'lock' }])

onMounted(async () => { user.value = await request('/api/member/me') })
async function logout() { await request('/api/member/auth/logout', { method: 'POST' }); await router.push('/login') }
</script>

<template>
  <AConsoleShell :subtitle="t('memberConsole')" :sections="sections" :quick-links="quickLinks" :profile-links="profileLinks" :user-name="user?.display_name || ''" :user-meta="user?.email || ''" :locale="locale" show-locale :labels="labels" @locale-change="setLocale($event as Locale)" @logout="logout">
    <div v-if="memberLicense?.state === 'expired'" class="notice" role="status">{{ locale === 'en-US' ? 'Your subscription has expired. Existing data and key revocation remain available. Contact your administrator to renew.' : '订阅已到期，可查看已有数据和撤销 Key。请联系管理员续订。' }}</div>
    <RouterView />
  </AConsoleShell>
</template>
