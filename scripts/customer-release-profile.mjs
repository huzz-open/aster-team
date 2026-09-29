import { createPublicKey } from 'node:crypto'
import { readFileSync } from 'node:fs'
import Ajv2020 from 'ajv/dist/2020.js'
import { parse as parseYaml, parseDocument } from 'yaml'

const ajv = new Ajv2020({ strict: true })
for (const name of ['entitlements.v1.schema.yaml', 'license-trust.v1.schema.yaml']) {
  const schema = parseYaml(readFileSync(new URL(`../contracts/schemas/${name}`, import.meta.url), 'utf8'))
  ajv.addSchema(schema, schema.$id.replace(/\.json$/, '.yaml'))
}
const validateLicenseProfile = ajv.compile({
  $ref: 'https://aster-team.local/contracts/license-trust.v1.schema.json#/$defs/issuer_profile',
})

function canonicalEd25519PublicKey(value, name) {
  if (typeof value !== 'string' || !/^[A-Za-z0-9_-]{32,4096}$/.test(value)) {
    throw new Error(`${name} must be a canonical Ed25519 SPKI public key`)
  }
  try {
    const key = createPublicKey({ key: Buffer.from(value, 'base64url'), format: 'der', type: 'spki' })
    const canonical = key.export({ format: 'der', type: 'spki' }).toString('base64url')
    if (key.asymmetricKeyType !== 'ed25519' || canonical !== value) throw new Error('not Ed25519')
    return canonical
  } catch {
    throw new Error(`${name} must be a canonical Ed25519 SPKI public key`)
  }
}

function parseKeyring(raw, name, license = false) {
  let entries
  try {
    if (typeof raw !== 'string' || Buffer.byteLength(raw, 'utf8') > 1 << 20) throw new Error('invalid input')
    entries = JSON.parse(raw || '')
    // JSON.parse discards duplicate fields. YAML's strict key check operates on
    // the original JSON, including escaped and nested field names, before trust
    // can be normalized into the compiled keyring. JSON syntax was checked above.
    if (parseDocument(raw, { uniqueKeys: true }).errors.length) throw new Error('duplicate fields')
  } catch {
    throw new Error(`${name} must be valid JSON with unique fields`)
  }
  if (!Array.isArray(entries) || entries.length < 1 || entries.length > 8) {
    throw new Error(`${name} must contain between one and eight keys`)
  }
  const keyIDs = new Set()
  const publicKeys = new Set()
  return entries.map((entry, index) => {
    if (license ? !validateLicenseProfile(entry) :
        (!entry || typeof entry !== 'object' || Array.isArray(entry) ||
          Object.keys(entry).sort().join(',') !== 'key_id,public_key_spki')) {
      throw new Error(`${name}[${index}] has invalid fields`)
    }
    if ((!license && (typeof entry.key_id !== 'string' || !/^[A-Za-z0-9_.:-]{3,128}$/.test(entry.key_id))) || keyIDs.has(entry.key_id)) {
      throw new Error(`${name}[${index}].key_id is invalid or duplicated`)
    }
    const publicKey = canonicalEd25519PublicKey(entry.public_key_spki, `${name}[${index}].public_key_spki`)
    if (publicKeys.has(publicKey)) throw new Error(`${name} contains a duplicated public key`)
    keyIDs.add(entry.key_id)
    publicKeys.add(publicKey)
    return {
      key_id: entry.key_id,
      public_key_spki: publicKey,
      ...(license && Object.hasOwn(entry, 'policy') ? { policy: entry.policy } : {}),
    }
  })
}

export function readCustomerReleaseProfile(environment = process.env) {
  const licenseTrustedKeys = parseKeyring(environment.ASTER_LICENSE_TRUSTED_KEYS_JSON, 'ASTER_LICENSE_TRUSTED_KEYS_JSON', true)
  const releaseTrustedKeys = parseKeyring(environment.ASTER_RELEASE_TRUSTED_KEYS_JSON, 'ASTER_RELEASE_TRUSTED_KEYS_JSON')
  const licensePublicKeys = new Set(licenseTrustedKeys.map(entry => entry.public_key_spki))
  if (releaseTrustedKeys.some(entry => licensePublicKeys.has(entry.public_key_spki))) {
    throw new Error('license and release signing keys must be cryptographically distinct')
  }
  return {
    licenseTrustedKeys,
    releaseTrustedKeys,
    licenseTrustedKeysJSON: JSON.stringify(licenseTrustedKeys),
    releaseTrustedKeysJSON: JSON.stringify(releaseTrustedKeys),
  }
}
