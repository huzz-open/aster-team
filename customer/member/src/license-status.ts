import { computed, shallowRef } from 'vue'
import { licenseFeatureAvailable, type PublicLicenseState } from '@aster/sdk'

export const memberLicense = shallowRef<PublicLicenseState | null>(null)
export const memberCanWrite = computed(() => Boolean(memberLicense.value && licenseFeatureAvailable(memberLicense.value, 'member')))
