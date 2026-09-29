import { expect, test } from 'vitest'
import { readPaidLicenseRequest } from '../src/commercial/paid-fulfillment'

test('paid request import preserves exact UTF-8 text without parsing away ambiguity', async () => {
  const raw = '{\r\n  "schema": "aster.license-request.v2",\r\n  "request_id": "first",\r\n  "request_id": "second"\r\n}\r\n'
  expect(await readPaidLicenseRequest(new File([raw], 'request.json', { type: 'application/json' }))).toBe(raw)
})

test('paid request import rejects byte-changing or oversized inputs', async () => {
  await expect(readPaidLicenseRequest(new File([], 'empty.json'))).rejects.toThrow('为空')
  await expect(readPaidLicenseRequest(new File([new Uint8Array([0xef, 0xbb, 0xbf]), '{}'], 'bom.json'))).rejects.toThrow('BOM')
  await expect(readPaidLicenseRequest(new File([new Uint8Array([0xc3, 0x28])], 'invalid.json'))).rejects.toThrow('UTF-8')
  await expect(readPaidLicenseRequest(new File(['x'.repeat(16 * 1024 + 1)], 'large.json'))).rejects.toThrow('16 KiB')
})
