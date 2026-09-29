import { computed, shallowRef, type InjectionKey } from 'vue'
import { licenseFeatureAvailable, request, type CapabilityId, type PublicLicenseState } from '@aster/sdk'

export type AdminProfile = {
  email: string
  password_change_required: boolean
  license_state: string
  license?: PublicLicenseState
}

export const adminProfile = shallowRef<AdminProfile | null>(null)
export const adminProfileLoaded = shallowRef(false)
export function useLicensedFeature(feature: CapabilityId) {
  return computed(() => Boolean(adminProfile.value?.license && licenseFeatureAvailable(adminProfile.value.license, feature)))
}
let profileRequest = 0

export function clearAdminProfile() {
  profileRequest += 1
  adminProfile.value = null
  adminProfileLoaded.value = false
}

export async function loadAdminProfile(): Promise<AdminProfile> {
  const attempt = ++profileRequest
  try {
    const profile = await request<AdminProfile>('/api/admin/me')
    if (attempt === profileRequest) {
      adminProfile.value = profile
      adminProfileLoaded.value = true
    }
    return profile
  } catch (error) {
    if (attempt === profileRequest) {
      adminProfile.value = null
      adminProfileLoaded.value = true
    }
    throw error
  }
}

export const refreshLicenseProfileKey: InjectionKey<() => Promise<void>> = Symbol('refreshLicenseProfile')
