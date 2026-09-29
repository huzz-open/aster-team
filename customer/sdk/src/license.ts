import { BUSINESS_OPERATIONS, type BusinessOperationId } from './generated/product-capabilities'

export type PublicLicenseState = {
  state: string
  available: boolean
  features?: readonly string[]
}

/** A projection of the shared operation requirements; never a server authorization grant. */
export function licenseOperationAvailable(license: PublicLicenseState | undefined, operation: BusinessOperationId): boolean {
  const definition = BUSINESS_OPERATIONS.find(entry => entry.id === operation)
  return Boolean(license && Array.isArray(license.features) && definition
    && definition.requires.every(feature => licenseFeatureAvailable(license, feature)))
}

/** UI availability only; every business operation still requires server authorization. */
export function licenseFeatureAvailable(license: PublicLicenseState, feature: string): boolean {
  if (license.state !== 'active' || license.available !== true) return false
  // Older Control versions tied `available` to member. Preserve that one meaning;
  // never infer other capabilities when the response has no feature list.
  if (!Object.hasOwn(license, 'features')) return feature === 'member'
  return Array.isArray(license.features) && license.features.includes(feature)
}

/** Existing data/recovery UI only. Never use for create or execution actions. */
export function licenseFeatureRetained(license: PublicLicenseState, feature: string): boolean {
  const readable = (license.state === 'active' && license.available === true)
    || (license.state === 'expired' && license.available === false)
  return readable && Array.isArray(license.features) && license.features.includes(feature)
}
