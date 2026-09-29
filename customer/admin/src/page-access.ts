import { licenseFeatureRetained, type CapabilityId, type PublicLicenseState } from '@aster/sdk'

type PageAccess = { kind: 'support' } | { kind: 'feature'; feature: CapabilityId }

// UI projection of existing backend checks, not a new commercial grant.
// Retained pages do not grant their creation or execution actions.
export const adminPageAccess = {
  '/overview': { kind: 'support' },
  '/consumption-logs': { kind: 'feature', feature: 'member' },
  '/users': { kind: 'feature', feature: 'member' },
  '/billing': { kind: 'feature', feature: 'member' },
  '/quota-requests': { kind: 'feature', feature: 'member' },
  '/models': { kind: 'feature', feature: 'gateway' },
  '/upstream-accounts': { kind: 'feature', feature: 'gateway' },
  '/runners': { kind: 'feature', feature: 'runner' },
  '/settings': { kind: 'support' },
  '/audit-events': { kind: 'support' },
  '/license': { kind: 'support' },
  '/maintenance': { kind: 'support' },
  '/account/password': { kind: 'support' },
} as const satisfies Record<string, PageAccess>

export type AdminPage = keyof typeof adminPageAccess

export function adminPageMeta(page: AdminPage) {
  return { adminPage: page }
}

export function adminPageRequirement(page: unknown): PageAccess | undefined {
  return typeof page === 'string' && Object.hasOwn(adminPageAccess, page)
    ? adminPageAccess[page as AdminPage] : undefined
}

export function adminPageAllowed(page: unknown, license: PublicLicenseState | undefined): boolean {
  const access = adminPageRequirement(page)
  if (!access) return false
  if (access.kind === 'support') return true
  // The new bundled Admin requires the explicit projection. Do not infer
  // capabilities from a legacy active boolean or a malformed feature list.
  return Boolean(license && licenseFeatureRetained(license, access.feature))
}
