import { createHash } from 'node:crypto'

const requestBytes = Buffer.from(Array.from({ length: 18 }, (_, index) => index + 1))
const installationBytes = Buffer.from(Array.from({ length: 18 }, (_, index) => index + 33))
const dmiBytes = Buffer.from(Array.from({ length: 32 }, (_, index) => index + 65))
const machineBytes = Buffer.from(Array.from({ length: 32 }, (_, index) => index + 97))

function base64URL(value) {
  return Buffer.from(value).toString('base64url')
}

const installationID = `installation_${base64URL(installationBytes)}`
const dmiUUID = base64URL(dmiBytes)
const machineID = base64URL(machineBytes)
const fingerprintSource = `aster-team\n${installationID}\ndmi_product_uuid=${dmiUUID}\nmachine_id=${machineID}\n`

export const licenseRequest = Object.freeze({
  schema: 'aster.license-request.v2',
  license_schema: 'aster.license.v2',
  capability_catalog_version: 1,
  quota_policy_version: 1,
  request_id: `request_${base64URL(requestBytes)}`,
  product: 'aster-team',
  product_version: '2.0.0',
  platform: 'linux',
  architecture: 'amd64',
  installation_id: installationID,
  machine_fingerprint_sha256: createHash('sha256').update(fingerprintSource).digest('base64url'),
  machine_factors: [
    { kind: 'dmi_product_uuid', sha256: dmiUUID },
    { kind: 'machine_id', sha256: machineID },
  ],
  generated_at: '2026-08-29T00:00:00.000Z',
})

export function compactLicenseRequest() {
  const version = Buffer.from(licenseRequest.product_version, 'utf8')
  const payload = Buffer.alloc(123 + version.length)
  Buffer.from([0x41, 0x4c, 0x52, 0x03, 1, 1, version.length]).copy(payload)
  payload.writeUInt32BE(1, 7)
  payload.writeUInt32BE(1, 11)
  payload.writeBigUInt64BE(BigInt(Date.parse(licenseRequest.generated_at)), 15)
  requestBytes.copy(payload, 23)
  installationBytes.copy(payload, 41)
  dmiBytes.copy(payload, 59)
  machineBytes.copy(payload, 91)
  version.copy(payload, 123)
  return payload
}

export const licensePolicy = Object.freeze({
  operation_id: 'fulfill_delivery_test',
  license_id: 'license_delivery_test',
  customer_ref: 'customer_delivery_test',
  source_type: 'order',
  source_id: 'order_delivery_test',
  product: 'aster-team',
  edition: 'enterprise',
  features: ['gateway', 'member', 'runner'],
  limits: { member_seats: 25, seat_over_limit_grace_days: 7 },
  minimum_version: '2.0.0',
  valid_until: '2027-08-29T00:00:00.000Z',
  transfer_limit: 2,
  status: 'active',
  transfer_count: 0,
  created_at: '2026-08-29T00:00:00.000Z',
  updated_at: '2026-08-29T00:00:00.000Z',
})

// UI transport fixture only. Real signature checks use the Rust-backed suite.
export const licenseDocument = Object.freeze({
  claims: {
    schema: 'aster.license.v2', key_id: 'license-delivery-test-v2',
    license_id: licensePolicy.license_id, serial: 'AT-DELIVERY-TEST-001',
    product: 'aster-team', edition: licensePolicy.edition, plan_id: 'plan_delivery', plan_version: 1,
    source: { kind: 'commercial_order', order_id: 'order_delivery_test', request_id: licenseRequest.request_id, customer_ref: licensePolicy.customer_ref },
    entitlements: { catalog_version: 1, features: licensePolicy.features, quotas: [
      { id: 'member_seats', limit: { mode: 'limited', value: 25 } },
      ...['runners', 'upstream_accounts', 'api_keys_per_member'].map(id => ({ id, limit: { mode: 'unlimited' } })),
    ] },
    quota_policy_version: 1, minimum_version: licensePolicy.minimum_version,
    binding: { mode: 'installation', installation_id: licenseRequest.installation_id, machine_fingerprint_sha256: licenseRequest.machine_fingerprint_sha256, transfer_sequence: 0 },
    issued_at: '2026-08-29T00:01:00.000Z',
    validity: { not_before: '2026-08-29T00:00:00.000Z', expiry: { mode: 'fixed', expires_at: licensePolicy.valid_until } },
  },
  signature: 'A'.repeat(86),
})

const claims = licenseDocument.claims
export const adminLicenseView = Object.freeze({
  schema: 'aster.admin-license-view.v1', protocol_schema: claims.schema,
  license_id: claims.license_id, serial: claims.serial, customer_ref: claims.source.customer_ref,
  key_id: claims.key_id, edition: claims.edition, plan_id: claims.plan_id, features: claims.entitlements.features,
  quotas: { member_seats: 25, runners: null, upstream_accounts: null, api_keys_per_member: null },
  binding: 'installation', minimum_version: claims.minimum_version, transfer_sequence: claims.binding.transfer_sequence,
  issued_at: claims.issued_at, not_before: claims.validity.not_before, expires_at: claims.validity.expiry.expires_at,
})
