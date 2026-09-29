import { describe, expect, it } from 'vitest'
import fixture from '../../../contracts/test-vectors/license-request.v2.json'

import { parseCompactQRPayload } from '../src/license-request-import'

const legacyRequest = {
  schema: 'aster.license-request.v1',
  request_id: 'request_0uTL4YW_I-t6LFy8KTd9J891',
  product: 'aster-team',
  product_version: '2.0.0',
  platform: 'linux',
  architecture: 'amd64',
  installation_id: 'installation_Dmz9pQ4AVs6kzaRgM79_8JDJ',
  machine_fingerprint_sha256: 'UJn2n58Eq5N9_d_VDYJbTKdRpvH9fCQReCSPXs8iBx8',
  machine_factors: [
    { kind: 'dmi_product_uuid', sha256: 'u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7s' },
    { kind: 'machine_id', sha256: 'zMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMw' },
  ],
  generated_at: '2026-08-29T00:00:00.000Z',
} as const

function decodeBase64URL(value: string): Uint8Array {
  const padded = value.replaceAll('-', '+').replaceAll('_', '/') + '='.repeat((4 - value.length % 4) % 4)
  return Uint8Array.from(atob(padded), character => character.charCodeAt(0))
}

function compactQRPayload(): Uint8Array {
  const version = new TextEncoder().encode(legacyRequest.product_version)
  const payload = new Uint8Array(115 + version.length)
  payload.set([0x41, 0x4c, 0x52, 0x02, 1, 1, version.length])
  new DataView(payload.buffer).setBigUint64(7, BigInt(Date.parse(legacyRequest.generated_at)), false)
  payload.set(decodeBase64URL(legacyRequest.request_id.slice('request_'.length)), 15)
  payload.set(decodeBase64URL(legacyRequest.installation_id.slice('installation_'.length)), 33)
  payload.set(decodeBase64URL(legacyRequest.machine_factors[0].sha256), 51)
  payload.set(decodeBase64URL(legacyRequest.machine_factors[1].sha256), 83)
  payload.set(version, 115)
  return payload
}

describe('license request import', () => {
  it('rejects the legacy compact request instead of adapting it', async () => {
    await expect(parseCompactQRPayload(compactQRPayload())).rejects.toThrow('不是当前版本')
  })

  it('decodes every v2 field from the exact Rust compact QR vector', async () => {
    const payload = decodeBase64URL(fixture.compact_qr_v3.payload_base64url)
    await expect(parseCompactQRPayload(payload)).resolves.toEqual(fixture.compact_qr_v3.request)

    const unsupportedCatalog = payload.slice()
    new DataView(unsupportedCatalog.buffer).setUint32(7, 2, false)
    await expect(parseCompactQRPayload(unsupportedCatalog)).rejects.toThrow('协议版本无效')

    const unsupportedQuota = payload.slice()
    new DataView(unsupportedQuota.buffer).setUint32(11, 2, false)
    await expect(parseCompactQRPayload(unsupportedQuota)).rejects.toThrow('协议版本无效')

    await expect(parseCompactQRPayload(payload.subarray(0, payload.length - 1))).rejects.toThrow('长度无效')
  })

  it('does not accept the earlier JSON QR payload', async () => {
    await expect(parseCompactQRPayload(new TextEncoder().encode(JSON.stringify(legacyRequest)))).rejects.toThrow('不是当前版本')
  })

  it('rejects unrelated JSON and QR payloads', async () => {
    await expect(parseCompactQRPayload(new Uint8Array())).rejects.toThrow('不是当前版本')
  })
})
