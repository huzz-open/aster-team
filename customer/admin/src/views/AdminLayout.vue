<script setup lang="ts">
import { computed, onMounted, provide } from 'vue'
import { RouterView, useRoute, useRouter } from 'vue-router'
import { AConsoleShell, ALockedState, type ConsoleNavItem, type ConsoleNavSection } from '@aster/ui'
import { request } from '@aster/sdk'
import { locale, setLocale, t, type Locale } from '../i18n'
import { adminProfile as profile, adminProfileLoaded as profileLoaded, clearAdminProfile, loadAdminProfile, refreshLicenseProfileKey } from '../license-status'
import { adminPageAllowed, adminPageRequirement, type AdminPage } from '../page-access'

const router = useRouter()
const route = useRoute()
async function refreshLicenseProfile() {
  try { await loadAdminProfile() }
  catch { /* Keep the recovery page mounted; the shared profile fails closed. */ }
}
provide(refreshLicenseProfileKey, refreshLicenseProfile)
const license = computed(() => profile.value?.license)
const licenseStatus = computed(() => {
  const state = license.value?.state || (profileLoaded.value ? 'unavailable' : '')
  const labels = locale.value === 'en-US'
    ? { active: 'Licensed', expired: 'Subscription expired', missing: 'Unlicensed', unavailable: 'License unavailable' }
    : { active: '已授权', expired: '订阅已到期', missing: '未授权', unavailable: '许可证不可用' }
  const tone: 'positive' | 'warning' | 'danger' | 'neutral' = state === 'active' ? 'positive' : state === 'missing' ? 'neutral' : state ? 'danger' : 'neutral'
  return { label: (labels as Record<string, string>)[state] || tx('检查中', 'Checking'), tone }
})
const pageRestricted = computed(() => profileLoaded.value && !adminPageAllowed(route.meta.adminPage, license.value))
const missingFeature = computed(() => adminPageRequirement(route.meta.adminPage)?.kind === 'feature' && license.value?.state === 'active' && license.value.available)
const navItem = (to: AdminPage, label: string, icon: string): ConsoleNavItem => ({
  to, label, icon,
  locked: profileLoaded.value && !adminPageAllowed(to, license.value),
  lockedLabel: license.value?.state === 'active' && license.value.available
    ? tx('当前授权未包含此功能', 'This feature is not included in the current license')
    : tx('需要先安装有效许可证', 'Install a valid license to use this feature'),
})
const sections = computed<ConsoleNavSection[]>(() => [
  { label: t('operation'), items: [
    navItem('/overview', t('overview'), 'dashboard'),
    navItem('/consumption-logs', t('consumptionLogs'), 'audit'),
    navItem('/users', t('users'), 'users'),
    navItem('/billing', t('billing'), 'payment'),
    navItem('/quota-requests', t('quotaRequests'), 'ticket'),
  ] },
  { label: t('resources'), items: [
    navItem('/models', t('models'), 'model'),
    navItem('/upstream-accounts', t('accounts'), 'key'),
    navItem('/runners', t('runners'), 'server'),
  ] },
  { label: t('security'), items: [
    navItem('/audit-events', t('auditEvents'), 'audit'),
    navItem('/license', t('license'), 'shield'),
    navItem('/maintenance', t('maintenance'), 'server'),
    navItem('/settings', t('settings'), 'settings'),
  ] },
])
const labels = computed(() => ({ notice: t('notice'), theme: t('theme'), language: t('language'), profile: t('profile'), logout: t('logout'), collapse: t('collapse'), allClear: t('allClear') }))
const quickLinks = computed(() => [
  ...[
    { to: '/models', label: t('models'), icon: 'model' },
    { to: '/runners', label: t('runners'), icon: 'server' },
    { to: '/consumption-logs', label: t('consumptionLogs'), icon: 'audit' },
  ].filter(item => adminPageAllowed(item.to, license.value)),
  { href: `/docs/${locale.value === 'en-US' ? 'en' : 'zh-cn'}/administration/`, label: tx('文档', 'Documentation'), icon: 'file', keepVisibleOnMobile: true },
])
const profileLinks = computed(() => [{ to: '/account/password', label: tx('修改密码', 'Change password'), icon: 'lock' }])

const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
onMounted(async () => {
  if (!profileLoaded.value) await refreshLicenseProfile()
})
async function logout() { await request('/api/admin/auth/logout', { method: 'POST' }); clearAdminProfile(); await router.push('/login') }
</script>

<template>
  <AConsoleShell :subtitle="t('workspace')" :status-label="licenseStatus.label" :status-tone="licenseStatus.tone" :sections="sections" :user-name="profile?.email || ''" :user-meta="t('administrator')" :locale="locale" show-locale :quick-links="quickLinks" :profile-links="profileLinks" :labels="labels" @locale-change="setLocale($event as Locale)" @logout="logout">
    <div v-if="!profileLoaded" class="content"><div class="card">{{ tx('正在读取部署状态…', 'Loading deployment status…') }}</div></div>
    <ALockedState v-else-if="pageRestricted" :title="missingFeature ? tx('当前授权未包含此功能', 'This feature is not included') : tx('需要有效许可证', 'A valid license is required')" :text="missingFeature ? tx('请在产品授权中核对功能列表，导入包含此功能的许可证后即可使用。', 'Review your licensed features and import a license that includes this feature.') : tx('请在产品授权中查看当前状态或导入许可证，更新后无需重启服务。', 'Review the current status or import a license. Updates take effect without restarting the service.')" :action-label="tx('查看授权状态', 'View license status')" @action="router.push('/license')" />
    <template v-else>
      <div v-if="license?.state === 'expired'" class="notice" role="status">{{ tx('订阅已到期，可查看已有数据、修改密码和整理资源。新增资源与模型调用暂停，续订后恢复。', 'Your subscription has expired. Existing data, password recovery and resource cleanup remain available. New resources and model calls resume after renewal.') }}</div>
      <RouterView />
    </template>
  </AConsoleShell>
</template>

<style scoped>
:deep(.console-nav a[href$="/upstream-accounts"] > span) {
  min-width: 0;
  white-space: normal;
  line-height: 1.4;
}
</style>
