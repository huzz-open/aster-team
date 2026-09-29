import { createPrivateKey, createPublicKey, sign, verify } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

// This key is already a published test fixture. Never read operational signing
// configuration here; reproducibility must not require a production secret.
const previous = JSON.parse(readFileSync(new URL('../../contracts/test-vectors/runner-task.v2.json', import.meta.url), 'utf8'))
const privateKey = createPrivateKey({ key: Buffer.from(previous.private_key_pkcs8, 'base64url'), format: 'der', type: 'pkcs8' })
const publicKey = createPublicKey({ key: Buffer.from(previous.public_key_spki, 'base64url'), format: 'der', type: 'spki' })
const { signature: _previousSignature, ...claims } = previous.ticket
claims.schema = 'aster.runner-task.v3'
claims.key_id = 'runner-task-vector-v3'
claims.execution_deadline_ms = claims.issued_at * 1000 + 600_000
claims.authorization = {
  kind: 'model',
  subject: {
    identity_id: 'identity-vector-123',
    api_key_id: 'api-key-vector-123',
    request_id: 'request-vector-123',
    reservation_id: 'reservation-vector-123',
    reserved_tokens: 100,
    reservation_expires_at: claims.expires_at,
  },
  resource: { account_id: 'account-vector-123', public_model: 'gpt-test', upstream_model: 'gpt-test' },
  license: { license_id: 'license-vector-123', license_sha256: 'ab'.repeat(32), expiry: { kind: 'never' } },
}
// Fixture values contain only ASCII keys and safe integers, so sorting object
// keys and JSON encoding gives the same canonical bytes as Rust license-core.
function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical)
  if (value !== null && typeof value === 'object') return Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])]))
  return value
}
const canonicalBytes = Buffer.from(JSON.stringify(canonical(claims)))
const signature = sign(null, canonicalBytes, privateKey)
if (!verify(null, canonicalBytes, publicKey, signature)) throw new Error('Runner fixture signature verification failed')
const fixture = {
  schema: 'aster.runner-task.test-vector.v3',
  public_key_spki: previous.public_key_spki,
  private_key_pkcs8: previous.private_key_pkcs8,
  payload_json: previous.payload_json,
  canonical_claims: canonicalBytes.toString(),
  ticket: { ...claims, signature: signature.toString('base64url') },
}
fixture.cases = [
  {
    kind: 'discover_models',
    actor: { kind: 'admin', identity_id: 'admin-vector-123' },
    account_id: 'account-vector-123',
    license: { ...claims.authorization.license, expiry: { kind: 'fixed', expires_at: claims.issued_at + 90 } },
  },
  {
    kind: 'refresh_credential',
    actor: { kind: 'service', service: 'credential_broker' },
    account_id: 'account-vector-123',
    lease_sha256: 'cd'.repeat(32),
    lease_expires_at: claims.issued_at + 45,
    license: claims.authorization.license,
  },
  {
    kind: 'authorize_credential',
    actor: { identity_id: 'admin-vector-123' },
    enrollment_id: 'enrollment-vector-123',
    session_expires_at: claims.issued_at + 30,
    license: claims.authorization.license,
  },
].map(authorization => {
  const enrollment = authorization.kind === 'authorize_credential'
  const ticket = {
    ...claims,
    command: authorization.kind,
    authorization,
    task_id: `task-vector-${authorization.kind}`,
    nonce: `nonce-vector-${authorization.kind}`,
    credential_instance_id: enrollment ? null : claims.credential_instance_id,
    credential_revision: enrollment ? null : claims.credential_revision,
    expires_at: Math.min(claims.expires_at, authorization.license.expiry.expires_at ?? Infinity, authorization.lease_expires_at ?? Infinity, authorization.session_expires_at ?? Infinity),
  }
  const bytes = Buffer.from(JSON.stringify(canonical(ticket)))
  const signature = sign(null, bytes, privateKey)
  if (!verify(null, bytes, publicKey, signature)) throw new Error(`Runner ${authorization.kind} signature verification failed`)
  return { name: authorization.kind, canonical_claims: bytes.toString(), ticket: { ...ticket, signature: signature.toString('base64url') } }
})
const expected = `${JSON.stringify(fixture, null, 2)}\n`
const path = fileURLToPath(new URL('../../contracts/test-vectors/runner-task.v3.json', import.meta.url))
if (process.argv.includes('--check')) {
  if (readFileSync(path, 'utf8').replaceAll('\r\n', '\n') !== expected) throw new Error('Runner v3 vector differs; run node tools/codegen/generate-runner-task-vector.mjs')
} else {
  writeFileSync(path, expected)
}
