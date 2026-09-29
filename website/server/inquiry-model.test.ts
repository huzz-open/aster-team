import assert from 'node:assert/strict'
import test from 'node:test'
import validate from '../shared/generated/inquiry-validator.js'
import { parseInquiry, inquiryDigest, buildInquiryEmailContent } from './inquiry-model'
import { readLimitedJson } from './lead-http'
import { readPendingInquiry, savePendingInquiry, clearPendingInquiry } from '../src/inquiry-journal'

function input() {
  return { request_id: crypto.randomUUID(), contact: 'test@example.invalid', message: '需要了解部署', locale: 'zh' as const, turnstile_token: 'fixture-token', website: '',
    reference: { catalog_revision: `catalog_${'a'.repeat(48)}`, plan_id: 'test_plan', plan_version: 1, years: 2 } }
}

test('inquiry uses a strict untrusted reference and has no price or rights authority', () => {
  const value = input()
  const parsed = parseInquiry(value)
  assert.deepEqual(parsed.content.reference, value.reference)
  assert.notEqual(parsed.content.reference, value.reference)
  assert.ok(!('turnstile_token' in parsed.content))
  for (const changed of [
    { ...value, amount: 1 }, { ...value, team_size: 3 }, { ...value, reference: { ...value.reference, price: 1 } },
    { ...value, reference: { ...value.reference, plan_version: '1' } }, { ...value, reference: { ...value.reference, years: 6 } },
    { ...value, contact: '  ' }, { ...value, website: 'bot' }, { ...value, reference: null },
  ]) assert.throws(() => parseInquiry(changed))
  const { reference: _reference, ...general } = value
  assert.equal(parseInquiry(general).content.reference, undefined)
  assert.match(buildInquiryEmailContent(value.request_id, parsed.content, '2026-09-06T00:00:00.000Z').body, /未经受理核验/)
})

test('generated validator follows Unicode code-point lengths and retry digests ignore fresh CAPTCHA tokens', async () => {
  const value = { ...input(), contact: '😀'.repeat(200) }
  assert.equal(validate(value), true)
  assert.equal(validate({ ...value, contact: '😀'.repeat(201) }), false)
  assert.equal(await inquiryDigest(parseInquiry(value).content), await inquiryDigest(parseInquiry({ ...value, turnstile_token: 'fresh-token' }).content))
})

test('bounded request reading rejects oversized, wrong media type and malformed UTF-8', async () => {
  const request = (body: BodyInit, type = 'application/json') => new Request('http://localhost/api/inquiries', { method: 'POST', headers: { 'Content-Type': type }, body })
  await assert.rejects(readLimitedJson(request(' '.repeat(32 * 1024 + 1)), 32 * 1024), /too large/)
  await assert.rejects(readLimitedJson(request('{}', 'application/jsonp')), /content type/)
  await assert.rejects(readLimitedJson(request(new Uint8Array([0x22, 0xc3, 0x22]))), /valid JSON/)
  await assert.rejects(readLimitedJson(request(new Uint8Array([0xef, 0xbb, 0xbf, 0x7b, 0x7d]))), /valid JSON/)
  assert.deepEqual(await readLimitedJson(request('{"ok":true}')), { ok: true })
})

test('tab recovery keeps the original body and refuses token persistence or clearing a different identity', () => {
  const values = new Map<string, string>()
  const storage = { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value) }, removeItem: (key: string) => { values.delete(key) } }
  const { turnstile_token: _token, ...payload } = input()
  savePendingInquiry(storage, { payload, label: '测试套餐 · 2 年' })
  assert.deepEqual(readPendingInquiry(storage), { payload, label: '测试套餐 · 2 年' })
  assert.ok(![...values.values()][0].includes('fixture-token'))
  assert.throws(() => clearPendingInquiry(storage, crypto.randomUUID()), /identity changed/)
  clearPendingInquiry(storage, payload.request_id)
  assert.equal(readPendingInquiry(storage), null)
  savePendingInquiry(storage, { payload, label: 'test' })
  const key = [...values.keys()][0]
  values.set(key, JSON.stringify({ version: 1, label: 'test', payload: { ...payload, turnstile_token: 'must-not-persist' } }))
  assert.throws(() => readPendingInquiry(storage), /must not store/)
})
