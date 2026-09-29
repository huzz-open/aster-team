import { createRouter, createWebHistory, type RouteLocationNormalized } from 'vue-router'
import { getSession } from './api/client'
import OperationsLayout from './views/OperationsLayout.vue'
import LoginView from './views/LoginView.vue'
import OverviewView from './views/OverviewView.vue'
import CustomersView from './views/CustomersView.vue'
import CommercialPlansView from './views/CommercialPlansView.vue'
import PlanDraftsView from './views/PlanDraftsView.vue'
import PublicCatalogsView from './views/PublicCatalogsView.vue'
import PublicationsView from './views/PublicationsView.vue'
import CommercialOrdersView from './views/CommercialOrdersView.vue'
import PaidAuthorizationsView from './views/PaidAuthorizationsView.vue'
import FreeDistributionsView from './views/FreeDistributionsView.vue'
import AuditView from './views/AuditView.vue'
import ChangePasswordView from './views/ChangePasswordView.vue'
import ReleaseCenterView from './views/ReleaseCenterView.vue'
import EnvironmentUpgradesView from './views/EnvironmentUpgradesView.vue'
import ReleaseArtifactsView from './views/ReleaseArtifactsView.vue'
import ReleaseWorkflowView from './views/ReleaseWorkflowView.vue'
import SigningEnvironmentView from './views/SigningEnvironmentView.vue'

const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/login', component: LoginView, meta: { public: true } },
    {
      path: '/',
      component: OperationsLayout,
      children: [
        { path: '', redirect: '/workflows/business' },
        { path: 'workflows/business', component: PaidAuthorizationsView },
        { path: 'workflows/release', component: ReleaseWorkflowView },
        { path: 'base/customers', component: CustomersView },
        { path: 'base/plans', component: PlanDraftsView },
        { path: 'base/signing', component: SigningEnvironmentView },
        { path: 'system/audit', component: AuditView },
        { path: 'overview', component: OverviewView },
        { path: 'customers', component: CustomersView },
        { path: 'commercial/plans', component: CommercialPlansView },
        { path: 'commercial/plan-drafts', component: PlanDraftsView },
        { path: 'commercial/catalogs', component: PublicCatalogsView },
        { path: 'commercial/publications', component: PublicationsView },
        { path: 'commercial/orders', component: CommercialOrdersView },
        { path: 'commercial/fulfillments', component: PaidAuthorizationsView },
        { path: 'commercial/distributions', component: FreeDistributionsView },
        { path: 'audit-events', component: AuditView },
        { path: 'release-artifacts', component: ReleaseArtifactsView },
        { path: 'release-center', component: ReleaseCenterView },
        { path: 'environment-upgrades', component: EnvironmentUpgradesView },
        { path: 'release-center/:taskID', component: ReleaseCenterView },
        { path: 'change-password', component: ChangePasswordView },
      ],
    },
  ],
})

export async function requireOperationsSession(to: RouteLocationNormalized, sessionLoader = getSession) {
  if (to.meta.public) return true
  try {
    const session = await sessionLoader()
    if (session.operator.password_change_required && to.path !== '/change-password') return '/change-password'
    if (!session.operator.password_change_required && to.path === '/change-password') return '/workflows/business'
    return true
  } catch {
    return { path: '/login', query: { redirect: to.fullPath } }
  }
}

router.beforeEach(to => requireOperationsSession(to))

export default router
