import { createRouter, createWebHistory } from 'vue-router'
import { loadAdminProfile } from './license-status'
import { adminPageMeta } from './page-access'
import AdminLayout from './views/AdminLayout.vue'
import LoginView from './views/LoginView.vue'
import OverviewView from './views/OverviewView.vue'
import UsersView from './views/UsersView.vue'
import BillingView from './views/BillingView.vue'
import QuotaRequestsView from './views/QuotaRequestsView.vue'
import ModelsView from './views/ModelsView.vue'
import RunnerView from './views/RunnerView.vue'
import UpstreamAccountsView from './views/UpstreamAccountsView.vue'
import SettingsView from './views/SettingsView.vue'
import ChangePasswordView from './views/ChangePasswordView.vue'
import LicenseView from './views/LicenseView.vue'
import ConsumptionLogsView from './views/ConsumptionLogsView.vue'
import AuditEventsView from './views/AuditEventsView.vue'
import PasswordView from './views/PasswordView.vue'
import MaintenanceView from './views/MaintenanceView.vue'

const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/login', component: LoginView, meta: { public: true } },
    { path: '/change-password', component: ChangePasswordView },
    { path: '/', component: AdminLayout, children: [
      { path: '', redirect: '/overview' },
      { path: 'overview', component: OverviewView, meta: adminPageMeta('/overview') },
      { path: 'consumption-logs', component: ConsumptionLogsView, meta: adminPageMeta('/consumption-logs') },
      { path: 'audit-events', component: AuditEventsView, meta: adminPageMeta('/audit-events') },
      { path: 'users', component: UsersView, meta: adminPageMeta('/users') },
      { path: 'billing', component: BillingView, meta: adminPageMeta('/billing') },
      { path: 'quota-requests', component: QuotaRequestsView, meta: adminPageMeta('/quota-requests') },
      { path: 'models', component: ModelsView, meta: adminPageMeta('/models') },
      { path: 'upstream-accounts', component: UpstreamAccountsView, meta: adminPageMeta('/upstream-accounts') },
      { path: 'runner', redirect: '/runners' },
      { path: 'runners', component: RunnerView, meta: adminPageMeta('/runners') },
      { path: 'settings', component: SettingsView, meta: adminPageMeta('/settings') },
      { path: 'account/password', component: PasswordView, meta: adminPageMeta('/account/password') },
      { path: 'license', component: LicenseView, meta: adminPageMeta('/license') },
      { path: 'maintenance', component: MaintenanceView, meta: adminPageMeta('/maintenance') },
    ] },
  ],
})
router.beforeEach(async (to) => {
  if (to.meta.public) return true
  try {
    const profile = await loadAdminProfile()
    if (profile.password_change_required && to.path !== '/change-password') return { path: '/change-password' }
    if (!profile.password_change_required && to.path === '/change-password') return { path: '/overview' }
    return true
  } catch {
    return { path: '/login', query: { redirect: to.fullPath } }
  }
})
export default router
