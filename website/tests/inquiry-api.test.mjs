import assert from 'node:assert/strict'
import { after, before, test } from 'node:test'
import { buildWebsiteFunctions, createWebsiteRuntime } from './runtime.mjs'

let app
before(async () => { app = await createWebsiteRuntime({ scriptPath: buildWebsiteFunctions(), maximum: 100 }) })
after(async () => { await app?.close() })
function payload() { return { request_id: crypto.randomUUID(), contact: 'local@example.invalid', message: '测试部署咨询', locale: 'zh', website: '', turnstile_token: 'test-token-valid',
  reference: { catalog_revision: `catalog_${'f'.repeat(48)}`, plan_id: 'unverified_test_plan', plan_version: 1, years: 2 } } }
function post(body, overrides = {}) {
  return app.runtime.dispatchFetch(`${app.origin}/api/inquiries`, { method: 'POST', headers: { 'Content-Type': 'application/json', Origin: app.origin }, body: JSON.stringify(body), ...overrides })
}

test('real Pages route and D1 retain only an unverified reference and recover same-ID retries', async () => {
  const body = payload()
  const response = await post(body)
  assert.equal(response.status, 202)
  assert.equal(response.headers.get('Cache-Control'), 'no-store')
  assert.deepEqual(await response.json(), { ok: true, id: body.request_id })
  const saved = await app.database.prepare('SELECT * FROM product_inquiries WHERE id = ?').bind(body.request_id).first()
  assert.deepEqual(JSON.parse(saved.catalog_reference_json), body.reference)
  assert.equal(saved.reference_status, 'unverified')
  assert.equal(saved.notification_status, 'not_configured')
  assert.ok(!JSON.stringify(saved).includes('test-token-valid'))
  const retries = await Promise.all(Array.from({ length: 8 }, () => post({ ...body, turnstile_token: 'test-token-fresh' })))
  assert.ok(retries.every(value => value.status === 202))
  assert.equal(await app.database.prepare('SELECT COUNT(*) AS count FROM product_inquiries WHERE id = ?').bind(body.request_id).first('count'), 1)
  assert.equal((await post({ ...body, message: 'changed content' })).status, 409)
  assert.equal(await app.database.prepare('SELECT COUNT(*) AS count FROM trial_leads').first('count'), 0)
  const general = payload(); delete general.reference
  assert.equal((await post(general)).status, 202)
  assert.equal(await app.database.prepare('SELECT reference_status FROM product_inquiries WHERE id = ?').bind(general.request_id).first('reference_status'), 'none')
})

test('origin, strict payload and production CAPTCHA hostname/action guards precede persistence', async () => {
  assert.equal((await app.runtime.dispatchFetch(`${app.origin}/api/inquiries`)).status, 405)
  assert.equal((await post(payload(), { headers: { 'Content-Type': 'application/json', Origin: 'https://wrong.invalid' } })).status, 403)
  for (const body of [{ ...payload(), price: 1 }, { ...payload(), reference: { ...payload().reference, entitlements: {} } }]) {
    const before = app.outbound.length
    assert.equal((await post(body)).status, 400)
    assert.equal(app.outbound.length, before)
  }
  for (const token of ['bad', 'test-token-bad-host', 'test-token-bad-action']) assert.equal((await post({ ...payload(), turnstile_token: token })).status, 403)
})

test('database write failure reports failure and retry uses the same identity after repair', async () => {
  const body = payload()
  await app.database.exec("CREATE TRIGGER inquiry_write_failure BEFORE INSERT ON product_inquiries BEGIN SELECT RAISE(ABORT, 'isolated failure'); END;")
  try {
    assert.equal((await post(body)).status, 500)
    assert.equal(await app.database.prepare('SELECT COUNT(*) AS count FROM product_inquiries WHERE id = ?').bind(body.request_id).first('count'), 0)
  } finally { await app.database.exec('DROP TRIGGER inquiry_write_failure;') }
  assert.equal((await post(body)).status, 202)
})

test('concurrent first submissions create one record and rate limit still rejects new identities', async () => {
  const body = payload()
  const responses = await Promise.all(Array.from({ length: 12 }, () => post(body)))
  assert.ok(responses.every(response => response.status === 202))
  assert.equal(await app.database.prepare('SELECT COUNT(*) AS count FROM product_inquiries WHERE id = ?').bind(body.request_id).first('count'), 1)
  await app.database.prepare('UPDATE trial_rate_limits SET count = 1000').run()
  assert.equal((await post(payload())).status, 429)
  assert.equal((await post(body)).status, 202)
})

test('the existing trial route still persists its real assessment after shared HTTP and email extraction', async () => {
  await app.database.exec('DELETE FROM trial_rate_limits;')
  const response = await app.runtime.dispatchFetch(`${app.origin}/api/trial`, {
    method: 'POST', headers: { 'Content-Type': 'application/json', Origin: app.origin },
    body: JSON.stringify({ contact: 'trial-regression@example.invalid', company: 'Local test', teamSize: 10, activeUsers: 4,
      evidence: 'time', weeklyTokens: 99999, dailyTime: '4h-8h', usageLevel: 2, recommendedProAccounts: 99,
      locale: 'zh', turnstileToken: 'test-token-valid', website: '' }),
  })
  assert.equal(response.status, 202)
  const result = await response.json()
  const row = await app.database.prepare('SELECT * FROM trial_leads WHERE id = ?').bind(result.id).first()
  assert.equal(row.usage_level, 1)
  assert.equal(row.recommended_pro_accounts, 2)
  assert.equal(row.weekly_tokens_100m, null)
  assert.equal(row.notification_status, 'not_configured')
  assert.equal(await app.database.prepare('SELECT COUNT(*) AS count FROM product_inquiries WHERE id = ?').bind(result.id).first('count'), 0)
})
