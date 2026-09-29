import { createRouter, createWebHistory } from 'vue-router'
import { APIError, licenseFeatureRetained, request, type PublicLicenseState } from '@aster/sdk'
import MemberLayout from './views/MemberLayout.vue'
import LoginView from './views/LoginView.vue'
import HomeView from './views/HomeView.vue'
import KeysView from './views/KeysView.vue'
import UsageView from './views/UsageView.vue'
import ConsumptionLogsView from './views/ConsumptionLogsView.vue'
import ModelsView from './views/ModelsView.vue'
import QuotaView from './views/QuotaView.vue'
import AccountView from './views/AccountView.vue'
import ChangePasswordView from './views/ChangePasswordView.vue'
import LicenseRequiredView from './views/LicenseRequiredView.vue'
import PasswordView from './views/PasswordView.vue'
import { memberLicense } from './license-status'

type MemberProfile = { password_change_required: boolean; license_state: string }
const usableLicenseStates = new Set(['active', 'expired'])

const router = createRouter({ history: createWebHistory(), routes: [
  { path: '/login', component: LoginView, meta: { public: true } },
  { path: '/change-password', component: ChangePasswordView },
  { path: '/license-required', component: LicenseRequiredView, meta: { public: true } },
  { path: '/', component: MemberLayout, children: [
    { path: '', redirect: '/home' },
    { path: 'home', component: HomeView },
    { path: 'keys', component: KeysView },
    { path: 'usage', component: UsageView },
    { path: 'logs', component: ConsumptionLogsView },
    { path: 'models', component: ModelsView },
    { path: 'quota', component: QuotaView },
    { path: 'account', component: AccountView },
    { path: 'account/password', component: PasswordView },
  ] },
] })
router.beforeEach(async (to) => {
  try {
    const license = await request<PublicLicenseState>('/api/public/license-state')
    memberLicense.value = license
    if (!licenseFeatureRetained(license, 'member')) {
      const reason = license.state === 'active' && license.available ? 'feature' : undefined
      return to.path === '/license-required' && to.query.reason === reason
        ? true
        : { path: '/license-required', query: { reason } }
    }
    if (to.path === '/license-required') return { path: '/login' }
  } catch { memberLicense.value = null /* retain authentication flow when Control is unavailable */ }
  if (to.meta.public) return true
  try {
    const profile = await request<MemberProfile>('/api/member/me')
    if (profile.password_change_required && to.path !== '/change-password') return { path: '/change-password' }
    if (!profile.password_change_required && !usableLicenseStates.has(profile.license_state) && to.path !== '/license-required') return { path: '/license-required' }
    if (!profile.password_change_required && usableLicenseStates.has(profile.license_state) && (to.path === '/change-password' || to.path === '/license-required')) return { path: '/home' }
    return true
  } catch (error) {
    if (error instanceof APIError && error.code === 'FEATURE_NOT_LICENSED') return { path: '/license-required', query: { reason: 'feature' } }
    return { path: '/login', query: { redirect: to.fullPath } }
  }
})
export default router
