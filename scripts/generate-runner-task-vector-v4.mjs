import { createPrivateKey, sign } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'

const source = JSON.parse(readFileSync(new URL('../contracts/test-vectors/runner-task.v3.json', import.meta.url), 'utf8'))
const key = createPrivateKey({
  key: Buffer.from(source.private_key_pkcs8, 'base64url'),
  format: 'der',
  type: 'pkcs8',
})
const canonical = (value) => JSON.stringify(value, (_, item) => item && typeof item === 'object' && !Array.isArray(item)
  ? Object.fromEntries(Object.entries(item).sort(([left], [right]) => left.localeCompare(right, 'en')))
  : item)
const update = (entry) => {
  const ticket = entry.ticket
  ticket.schema = 'aster.runner-task.v4'
  ticket.key_id = 'runner-task-vector-v4'
  const claims = { ...ticket }
  delete claims.signature
  entry.canonical_claims = canonical(claims)
  ticket.signature = sign(null, Buffer.from(entry.canonical_claims), key).toString('base64url')
}
source.schema = 'aster.runner-task.test-vector.v4'
update(source)
for (const entry of source.cases) update(entry)
source.cases.push({
  name: 'fetch_asset',
  ticket: {
    schema: 'aster.runner-task.v4', key_id: 'runner-task-vector-v4',
    task_id: 'task-vector-fetch_asset', runner_id: 'runner-vector-123',
    provider_id: 'glm', credential_instance_id: 'credential_00000000000000000000000000000001',
    credential_revision: 7, upstream_host: 'cdn.example.test', command: 'fetch_asset',
    payload_sha256: source.ticket.payload_sha256, issued_at: 1787565600,
    expires_at: 1787565720, nonce: 'nonce-vector-fetch_asset',
    execution_deadline_ms: 1787566200000,
    authorization: {
      kind: 'fetch_asset',
      subject: { identity_id: 'identity-vector-123', api_key_id: 'api-key-vector-123',
        request_id: 'request-vector-123', reservation_id: 'reservation-vector-123',
        reserved_images: 1, reservation_expires_at: 1787565720 },
      resource: { account_id: 'account-vector-123', public_model: 'glm-image',
        upstream_model: 'glm-image' },
      license: { license_id: 'license-vector-123',
        license_sha256: 'ab'.repeat(32), expiry: { kind: 'never' } },
    },
  },
})
update(source.cases.at(-1))
const imageModel = structuredClone(source.cases.at(-1))
imageModel.name = 'image_model'
imageModel.ticket.task_id = 'task-vector-image_model'
imageModel.ticket.nonce = 'nonce-vector-image_model'
imageModel.ticket.command = 'execute'
imageModel.ticket.authorization.kind = 'image_model'
imageModel.ticket.upstream_host = 'api.z.ai'
source.cases.push(imageModel)
update(imageModel)
const output = new URL('../contracts/test-vectors/runner-task.v4.json', import.meta.url)
const serialized = `${JSON.stringify(source, null, 2)}\n`
if (process.argv.includes('--check')) {
  if (readFileSync(output, 'utf8').replace(/\r\n/g, '\n') !== serialized) {
    throw new Error('Runner v4 vector is stale')
  }
} else {
  writeFileSync(output, serialized)
}
