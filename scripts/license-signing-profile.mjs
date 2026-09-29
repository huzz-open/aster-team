import { createPrivateKey, createPublicKey, generateKeyPairSync } from 'node:crypto'
import { isDeepStrictEqual } from 'node:util'
import { parseDocument } from 'yaml'
import { readCustomerReleaseProfile } from './customer-release-profile.mjs'

export function readUniqueJSON(raw, label) {
  try {
    if (typeof raw !== 'string' || Buffer.byteLength(raw) > 1 << 20) throw new Error()
    const value = JSON.parse(raw)
    if (parseDocument(raw, { uniqueKeys: true }).errors.length) throw new Error()
    return value
  } catch { throw new Error(`${label} must be bounded JSON with unique fields`) }
}

// This path reads trusted operator configuration, never a customer document.
// Reuse the public build parser so private and compiled scopes cannot drift.
export function readLicenseSigningProfile(raw, releaseTrustedKeysJSON, expectedPublicJSON) {
  const signers = readUniqueJSON(raw, 'License v2 signers')
  if (!Array.isArray(signers) || signers.length < 1 || signers.length > 8) {
    throw new Error('Local License v2 signers must contain between one and eight keys')
  }
  const publicProfiles = signers.map((entry, index) => {
    const label = `License v2 signer[${index}]`
    if (!entry || typeof entry !== 'object' || Array.isArray(entry) ||
        Object.keys(entry).sort().join(',') !== 'key_id,policy,private_key_pkcs8') {
      throw new Error(`${label} has invalid fields`)
    }
    let publicKey
    try {
      const encoded = entry.private_key_pkcs8
      if (typeof encoded !== 'string' || !/^[A-Za-z0-9_-]+$/.test(encoded)) throw new Error()
      const key = createPrivateKey({ key: Buffer.from(encoded, 'base64url'), format: 'der', type: 'pkcs8' })
      if (key.asymmetricKeyType !== 'ed25519' || key.export({ format: 'der', type: 'pkcs8' }).toString('base64url') !== encoded) throw new Error()
      publicKey = createPublicKey(key).export({ format: 'der', type: 'spki' }).toString('base64url')
    } catch { throw new Error(`${label} must contain a canonical Ed25519 PKCS#8 private key`) }
    return { key_id: entry.key_id, public_key_spki: publicKey, policy: entry.policy }
  })
  const profile = readCustomerReleaseProfile({
    ASTER_LICENSE_TRUSTED_KEYS_JSON: JSON.stringify(publicProfiles),
    ASTER_RELEASE_TRUSTED_KEYS_JSON: releaseTrustedKeysJSON,
  })
  if (expectedPublicJSON !== undefined) {
    const expected = readCustomerReleaseProfile({
      ASTER_LICENSE_TRUSTED_KEYS_JSON: expectedPublicJSON,
      ASTER_RELEASE_TRUSTED_KEYS_JSON: releaseTrustedKeysJSON,
    })
    // Historical public-only issuers may remain, but every active private issuer
    // must match its ID, key material and complete policy before any DB mutation.
    for (const actual of publicProfiles) {
      if (!isDeepStrictEqual(actual, expected.licenseTrustedKeys.find(entry => entry.key_id === actual.key_id))) {
        throw new Error('License v2 signing profile does not match the public keyring and policy')
      }
    }
    return { ...expected, licenseSignersJSON: JSON.stringify(signers) }
  }
  return { ...profile, licenseSignersJSON: JSON.stringify(signers) }
}

export function generateLicenseSigners(policies) {
  if (!Array.isArray(policies) || policies.length < 1 || policies.length > 8) throw new Error('Provide one to eight explicit License issuer policies')
  return policies.map(entry => {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry) || Object.keys(entry).sort().join(',') !== 'key_id,policy') {
      throw new Error('License issuer definition must contain exactly key_id and policy')
    }
    const { privateKey } = generateKeyPairSync('ed25519')
    return { key_id: entry.key_id, private_key_pkcs8: privateKey.export({ format: 'der', type: 'pkcs8' }).toString('base64url'), policy: entry.policy }
  })
}

// Disposable local development defaults. Production generation requires an
// explicit reviewed policy file and never uses these defaults or fixture keys.
export function localLicensePolicies() {
  const rights = seats => ({
    catalog_version: 1, features: ['gateway', 'member', 'runner'],
    ...(seats === 3 ? {} : { feature_sets: ['standard'] }),
    quotas: ['member_seats', 'runners', 'upstream_accounts', 'api_keys_per_member'].map((id, index) => ({
      id, limit: seats === 3 ? { mode: 'limited', value: [3, 1, 1, 1][index] } : index === 0 ? { mode: 'limited', value: seats } : { mode: 'unlimited' },
    })),
  })
  return [
    { key_id: 'license-local-free-v2', policy: { sources: ['free_distribution'], bindings: ['unbound'], expiries: ['none'], entitlement_ceiling: rights(3) } },
    { key_id: 'license-local-paid-v2', policy: { sources: ['commercial_order'], bindings: ['installation'], expiries: ['fixed'], entitlement_ceiling: rights(50) } },
  ]
}
