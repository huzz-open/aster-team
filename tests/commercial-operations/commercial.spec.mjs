import { createHash, createPublicKey, randomUUID, verify } from 'node:crypto'
import { readFile, mkdir, writeFile } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import { test, expect } from '@playwright/test'
import Ajv2020 from 'ajv/dist/2020.js'
import { parse as parseYaml } from 'yaml'
import { publicationSite } from './publication-site.mjs'
import { fulfillLocalDemo, newLocalFulfillment } from '../../scripts/local-commercial-license.mjs'

const origin = 'http://127.0.0.1:26380'
const email = 'commercial-ui@test.invalid'
const bootstrapPassword = 'Aster-UI-Test-Only-2026!'
const password = 'Aster-UI-Test-Changed-2026!'
const fixture = JSON.parse(await readFile(new URL('../../contracts/test-vectors/plan-definition.v1.json', import.meta.url), 'utf8'))
const schemas = new Ajv2020({ strict: true })
for (const name of ['entitlements.v1.schema.yaml', 'license-trust.v1.schema.yaml', 'commercial.v1.schema.yaml', 'license.v2.schema.yaml', 'license-request.v2.schema.yaml', 'free-distribution.v1.schema.yaml', 'public-catalog.v1.schema.yaml', 'catalog-publication.v1.schema.yaml', 'website-release.v1.schema.yaml', 'payment.v1.schema.yaml', 'paid-fulfillment.v1.schema.yaml']) {
  const schema = parseYaml(await readFile(new URL('../../contracts/schemas/' + name, import.meta.url), 'utf8'))
  schemas.addSchema(schema, schema.$id.replace(/\.json$/, '.yaml'))
}
const validateDistribution = schemas.getSchema('https://aster-team.local/contracts/free-distribution.v1.schema.json')
const validateCatalog = schemas.getSchema('https://aster-team.local/contracts/public-catalog.v1.schema.json')
const validateCatalogRecord = schemas.getSchema('https://aster-team.local/contracts/public-catalog.v1.schema.json#/$defs/record')
const validatePublication = schemas.getSchema('https://aster-team.local/contracts/catalog-publication.v1.schema.json')
const validatePublicationFailure = schemas.getSchema('https://aster-team.local/contracts/catalog-publication.v1.schema.json#/$defs/failure')
const validateWebsiteRelease = schemas.getSchema('https://aster-team.local/contracts/website-release.v1.schema.json')
const validateOrder = schemas.getSchema('https://aster-team.local/contracts/commercial.v1.schema.json#/$defs/order')
const validatePayment = schemas.getSchema('https://aster-team.local/contracts/payment.v1.schema.json')
const validatePaymentContext = schemas.getSchema('https://aster-team.local/contracts/payment.v1.schema.json#/$defs/context')
const validatePaidFulfillment = schemas.getSchema('https://aster-team.local/contracts/paid-fulfillment.v1.schema.json')
const validatePaidFulfillmentContext = schemas.getSchema('https://aster-team.local/contracts/paid-fulfillment.v1.schema.json#/$defs/context')
const validateLicenseRequest = schemas.getSchema('https://aster-team.local/contracts/license-request.v2.schema.json')
const validateLicense = schemas.getSchema('https://aster-team.local/contracts/license.v2.schema.json')
let session, customer, savedPlan, actor

async function write(api, path, data, method = 'POST') {
  const token = (await api.storageState()).cookies.find(cookie => cookie.name === 'aster_operations_csrf')?.value
  return api.fetch(`${origin}/api/operations/v1${path}`, { method, data, headers: { Origin: origin, 'X-CSRF-Token': decodeURIComponent(token ?? '') } })
}
test.beforeAll(async ({ playwright }) => {
  if (process.env.ASTER_COMMERCIAL_BROWSER_TEST !== '1') throw new Error('Set ASTER_COMMERCIAL_BROWSER_TEST=1 only with the documented disposable local API')
  actor = await playwright.request.newContext()
  let response = await write(actor, '/session', { email, password })
  if (response.status() === 401) {
    response = await write(actor, '/session', { email, password: bootstrapPassword })
    expect(response.ok()).toBeTruthy()
    const changed = await write(actor, '/session/password', { current_password: bootstrapPassword, new_password: password }, 'PUT')
    expect(changed.status()).toBe(204)
    response = await write(actor, '/session', { email, password })
  }
  expect(response.ok()).toBeTruthy()
  session = await actor.storageState()
  const customerResponse = await write(actor, '/customers', { name: `商业浏览器测试客户 ${randomUUID().slice(0, 8)}`, legal_name: '测试主体', status: 'active', contact_name: '测试', contact_email: 'customer@test.invalid', contact_phone: '', contact_wechat: '', notes: 'isolated browser test' })
  expect(customerResponse.status()).toBe(201); customer = await customerResponse.json()
  const planResponse = await write(actor, '/commercial/plans/versions', { operation_id: `browser_${randomUUID()}`, plan_id: '', expected_version: 0, definition: { ...fixture, code: `browser_${randomUUID()}`, name: '浏览器测试套餐' } })
  expect(planResponse.status()).toBe(201); savedPlan = await planResponse.json()
})
test.afterAll(async () => { await actor?.dispose() })
test.beforeEach(async ({ context }) => { await context.addCookies(session.cookies) })

test('local quick authorization retries one free distribution and paid order after a lost issuance response', async ({}, testInfo) => {
  const overviewBefore = await (await actor.get(`${origin}/api/operations/v1/overview`)).json()
  const request = {
    schema: 'aster.license-request.v2', request_id: `request_${randomUUID()}`, product: 'aster-team',
    product_version: fixture.minimum_version, platform: 'linux', architecture: 'amd64',
    installation_id: `installation_${randomUUID()}`, machine_fingerprint_sha256: 'A'.repeat(43),
    machine_factors: [{ kind: 'dmi_product_uuid', sha256: 'B'.repeat(43) }, { kind: 'machine_id', sha256: 'C'.repeat(43) }],
    generated_at: new Date(Date.now() - 1000).toISOString(), license_schema: 'aster.license.v2',
    capability_catalog_version: 1, quota_policy_version: 1,
  }
  const requestJSON = JSON.stringify(request, null, 2)
  const state = newLocalFulfillment(requestJSON, customer.id)
  const profiles = await actor.get(`${origin}/api/operations/v1/commercial/issuers`)
  expect(profiles.status()).toBe(200)
  const trustedKeys = (await profiles.json()).items
  let dropIssue = true, tamperDownload = false, mutations = 0
  const operations = { async request(path, options = {}) {
    const response = options.method === 'POST' ? await write(actor, path, options.body) : await actor.get(`${origin}/api/operations/v1${path}`)
    expect(response.ok(), `${path}: ${await response.text()}`).toBe(true)
    if (options.method === 'POST') mutations++
    if (path.endsWith('/issue') && dropIssue) { dropIssue = false; throw new Error('simulated lost committed issuance response') }
    if (options.format === 'bytes') return { bytes: tamperDownload ? Buffer.from('{}') : await response.body(), sha256: response.headers()['x-content-sha256'] }
    return response.json()
  } }
  const input = { operations, password, requestJSON, minimumVersion: request.product_version, trustedKeys, state }
  for (const environment of ['production', '', null]) {
    const calls = []
    const blocked = { async request(path, options) {
      calls.push({ path, options })
      if (path !== '/commercial/environment') throw new Error('business state was reached before environment validation')
      return { fulfillment_environment: environment }
    } }
    await expect(fulfillLocalDemo({ ...input, operations: blocked })).rejects.toThrow('只允许履约环境为 local')
    expect(calls).toEqual([{ path: '/commercial/environment', options: undefined }])
  }
  await expect(fulfillLocalDemo(input)).rejects.toThrow('lost committed issuance')
  const result = await fulfillLocalDemo(input)
  const repeated = await fulfillLocalDemo(input)
  expect(repeated.bytes).toEqual(result.bytes)
  expect(repeated.free.bytes).toEqual(result.free.bytes)
  expect(validateDistribution(result.free.issued), JSON.stringify(validateDistribution.errors)).toBe(true)
  expect(result.free.issued.document.claims.source.kind).toBe('free_distribution')
  expect(result.free.issued.document.claims.binding).toEqual({ mode: 'unbound' })
  expect(result.free.issued.document.claims.validity.expiry).toEqual({ mode: 'none' })
  expect(result.free.issued.document.claims.entitlements.quotas.find(entry => entry.id === 'api_keys_per_member').limit).toEqual({ mode: 'limited', value: 1 })
  expect(result.issued.snapshot.request.license_request_json).toBe(requestJSON)
  expect(result.issued.document.claims.binding.installation_id).toBe(request.installation_id)
  expect(result.issued.document.claims.entitlements.quotas.find(entry => entry.id === 'member_seats').limit).toEqual({ mode: 'limited', value: 20 })
  expect(result.issued.document.claims.entitlements.feature_sets).toEqual(['standard'])
  const orders = await actor.get(`${origin}/api/operations/v1/commercial/orders?limit=100`)
  expect((await orders.json()).items.filter(order => order.operation_id === `local_order_${state.id}`)).toHaveLength(1)
  const overviewResponse = await actor.get(`${origin}/api/operations/v1/overview`)
  expect(overviewResponse.status()).toBe(200)
  const overview = await overviewResponse.json()
  expect(overview.plans_total).toBe(overviewBefore.plans_total + 2)
  expect(overview.free_distributions_issued).toBe(overviewBefore.free_distributions_issued + 1)
  expect(overview.paid_licenses_issued).toBe(overviewBefore.paid_licenses_issued + 1)
  expect(overview.orders_pending).toBe(overviewBefore.orders_pending)
  expect(overview.paid_fulfillments_pending).toBe(overviewBefore.paid_fulfillments_pending)
  expect(overview).not.toHaveProperty('licenses_total')
  const exportResponse = await write(actor, '/exports', { current_password: password })
  expect(exportResponse.status()).toBe(200)
  const report = await exportResponse.json()
  expect(report.schema_version).toBe('aster.operations-export.v2')
  expect(report).not.toHaveProperty('license_issuances')
  const fulfillment = report.commercial_records.find(item => item.kind === 'paid_fulfillment' && item.id === result.issued.snapshot.id)
  expect(fulfillment).toEqual({ kind: 'paid_fulfillment', id: result.issued.snapshot.id, status: 'issued', snapshot: result.issued.snapshot, source_snapshot_sha256: result.issued.sha256, document_sha256: result.issued.document_sha256 })
  const freeDistribution = report.commercial_records.find(item => item.kind === 'free_distribution' && item.id === result.free.issued.snapshot.id)
  expect(freeDistribution).toEqual({ kind: 'free_distribution', id: result.free.issued.snapshot.id, status: 'issued', snapshot: result.free.issued.snapshot, source_snapshot_sha256: result.free.issued.sha256, document_sha256: result.free.issued.document_sha256 })
  const exportedOrder = report.commercial_records.find(item => item.kind === 'order' && item.id === result.issued.snapshot.payment.snapshot.order.order_id)
  expect(exportedOrder.status).toBe('fulfilled')
  expect(report.commercial_records.filter(item => item.kind === 'payment' && item.snapshot.order.order_id === exportedOrder.id)).toHaveLength(1)
  expect(report.commercial_records.filter(item => item.kind === 'plan_version' && item.snapshot.plan_id === exportedOrder.snapshot.plan.plan_id)).toHaveLength(1)
  for (const entry of report.commercial_records) {
    expect(entry).not.toHaveProperty('document')
    expect(entry).not.toHaveProperty('claims')
    expect(entry.source_snapshot_sha256).toMatch(/^[0-9a-f]{64}$/)
  }
  const mutationCount = mutations
  await expect(fulfillLocalDemo({ ...input, requestJSON: `${requestJSON} ` })).rejects.toThrow('上下文与当前机器申请不一致')
  expect(mutations).toBe(mutationCount)
  tamperDownload = true
  await expect(fulfillLocalDemo(input)).rejects.toThrow('下载免费授权与分发回执摘要不一致')
  const licensePath = testInfo.outputPath('local-quick.license.json')
  const freeLicensePath = testInfo.outputPath('local-quick.free-license.json')
  const trustPath = testInfo.outputPath('local-quick.trust.json')
  await writeFile(licensePath, result.bytes)
  await writeFile(freeLicensePath, result.free.bytes)
  await writeFile(trustPath, JSON.stringify(trustedKeys))
  if (!process.env.ASTER_COMMERCIAL_RUST_VERIFIER) throw new Error('Use npm run test:commercial:browser')
  const verified = spawnSync(process.env.ASTER_COMMERCIAL_RUST_VERIFIER, [licensePath, trustPath], { encoding: 'utf8', windowsHide: true, timeout: 15_000 })
  expect(verified.status, verified.stderr).toBe(0)
  const verifiedFree = spawnSync(process.env.ASTER_COMMERCIAL_RUST_VERIFIER, [freeLicensePath, trustPath], { encoding: 'utf8', windowsHide: true, timeout: 15_000 })
  expect(verifiedFree.status, verifiedFree.stderr).toBe(0)
  expect(JSON.parse(verifiedFree.stdout)).toEqual(result.free.issued.document.claims)
})

async function publicationFixture() {
  const request = { operation_id: `publication_catalog_${randomUUID()}`, environment: 'local', reason: '公开来源核对测试', plans: [{ plan_id: savedPlan.snapshot.plan_id, version: savedPlan.snapshot.version, expected_sha256: savedPlan.sha256 }] }
  const previewResponse = await write(actor, '/commercial/catalogs/preview', request)
  expect(previewResponse.status()).toBe(200)
  const preview = await previewResponse.json()
  const approveResponse = await write(actor, '/commercial/catalogs', { request, expected_public_sha256: preview.sha256, current_password: password })
  expect(approveResponse.status()).toBe(201)
  const approval = await approveResponse.json()
  const exported = await write(actor, `/commercial/catalogs/${approval.snapshot.id}/export`, { current_password: password })
  expect(exported.status()).toBe(200)
  const file = await actor.get(`${origin}/api/operations/v1/commercial/catalogs/${approval.snapshot.id}/download`)
  expect(file.status()).toBe(200)
  const site = await publicationSite(approval, await file.body())
  return { approval, site }
}

async function observeWebsiteQuote(browser, approval, testInfo) {
  const context = await browser.newContext()
  await context.route('**/*', route => new URL(route.request().url()).origin === 'http://127.0.0.1:26394' ? route.continue() : route.abort('blockedbyclient'))
  try {
    const page = await context.newPage()
    await page.goto('http://127.0.0.1:26394/#section-9')
    const section = page.locator('[data-catalog-revision]')
    const revision = await section.getAttribute('data-catalog-revision')
    expect(revision).toBe(approval.snapshot.id)
    const cards = section.locator('.plan-card')
    await expect(cards).toHaveCount(1)
    await expect(cards.getByRole('heading')).toHaveText(savedPlan.snapshot.definition.name)
    const planID = await cards.getAttribute('data-plan-id')
    const planVersion = Number(await cards.getAttribute('data-plan-version'))
    await cards.getByRole('button', { name: '3 年', exact: true }).click()
    await expect(cards.getByRole('button', { name: '3 年', exact: true })).toHaveAttribute('aria-pressed', 'true')
    const years = Number((await cards.locator('[aria-pressed="true"]').textContent()).trim().split(' ')[0])
    const response = await page.request.get(`http://127.0.0.1:26394/catalog/${revision}/plans.json`)
    expect(response.status()).toBe(200)
    const bytes = await response.body()
    expect(createHash('sha256').update(bytes).digest('hex')).toBe(approval.public.sha256)
    const catalog = JSON.parse(bytes)
    const plan = catalog.plans.find(item => item.plan_id === planID && item.version === planVersion)
    expect(plan).toBeTruthy()
    const term = plan.offer.terms.find(item => item.years === years)
    expect(term).toBeTruthy()
    await expect(cards.locator('.plan-price strong')).toHaveText(new Intl.NumberFormat('zh-CN', { style: 'currency', currency: plan.offer.currency, minimumFractionDigits: 0, maximumFractionDigits: 2 }).format(term.total_amount_minor / 100))
    await cards.screenshot({ path: testInfo.outputPath('website-quoted-plan.png'), animations: 'disabled' })
    return { revision, plan_id: planID, plan_version: planVersion, years, amount_minor: term.total_amount_minor, entitlements: plan.entitlements }
  } finally { await context.close() }
}

async function verifyQuotationFulfillment(order, displayed, testInfo) {
  const orderPath = `/commercial/orders/${order.snapshot.order_id}`
  const paid = await write(actor, `${orderPath}/payment`, {
    operation_id: `quotation_payment_${randomUUID()}`, expected_order_sha256: order.sha256,
    payment_reference: `TEST-ONLY-${randomUUID()}`, received_at: new Date().toISOString(),
    notes: '同一官网报价订单的隔离验收', current_password: password,
  })
  expect(paid.status()).toBe(200)
  const payment = await paid.json()
  expect(payment.snapshot.order).toEqual(order.snapshot)
  const installation = {
    schema: 'aster.license-request.v2', request_id: `request_${randomUUID()}`, product: 'aster-team',
    product_version: order.snapshot.plan.definition.minimum_version, platform: 'linux', architecture: 'amd64',
    installation_id: `installation_${randomUUID()}`, machine_fingerprint_sha256: 'A'.repeat(43),
    machine_factors: [{ kind: 'dmi_product_uuid', sha256: 'B'.repeat(43) }, { kind: 'machine_id', sha256: 'C'.repeat(43) }],
    generated_at: new Date(Date.now() - 1000).toISOString(), license_schema: 'aster.license.v2',
    capability_catalog_version: displayed.entitlements.catalog_version,
    quota_policy_version: order.snapshot.plan.definition.quota_policy_version,
  }
  const approvedResponse = await write(actor, `${orderPath}/fulfillment`, {
    operation_id: `quotation_fulfillment_${randomUUID()}`, expected_order_sha256: order.sha256,
    expected_payment_sha256: payment.sha256, license_request_json: JSON.stringify(installation),
    reason: '核对同一报价来源与安装请求', current_password: password,
  })
  expect(approvedResponse.status()).toBe(200)
  const approved = await approvedResponse.json()
  expect(approved.snapshot.payment.snapshot.order).toEqual(order.snapshot)
  const path = `/commercial/paid-fulfillments/${approved.snapshot.id}`
  const issuedResponse = await write(actor, `${path}/issue`, { key_id: 'local-paid-test-only', current_password: password })
  expect(issuedResponse.status()).toBe(200)
  const issued = await issuedResponse.json()
  const downloaded = await actor.get(`${origin}/api/operations/v1${path}/license`)
  expect(downloaded.status()).toBe(200)
  const bytes = await downloaded.body()
  expect(createHash('sha256').update(bytes).digest('hex')).toBe(issued.document_sha256)
  const profilesResponse = await actor.get(`${origin}/api/operations/v1/commercial/issuers`)
  expect(profilesResponse.status()).toBe(200)
  const profile = (await profilesResponse.json()).items.find(item => item.key_id === 'local-paid-test-only')
  expect(profile).toBeTruthy()
  const licensePath = testInfo.outputPath('website-order.license.json')
  const trustPath = testInfo.outputPath('trusted-public-profiles.json')
  await writeFile(licensePath, bytes)
  await writeFile(trustPath, JSON.stringify([{ key_id: profile.key_id, public_key_spki: profile.public_key_spki, policy: profile.policy }]))
  if (!process.env.ASTER_COMMERCIAL_RUST_VERIFIER) throw new Error('Use npm run test:commercial:browser to build the Rust verification handoff')
  const verifyWithRust = documentPath => spawnSync(process.env.ASTER_COMMERCIAL_RUST_VERIFIER, [documentPath, trustPath], { encoding: 'utf8', windowsHide: true, timeout: 15_000 })
  const verified = verifyWithRust(licensePath)
  expect(verified.error).toBeUndefined()
  expect(verified.status, verified.stderr).toBe(0)
  const claims = JSON.parse(verified.stdout)
  expect(claims).toEqual(issued.document.claims)
  expect(claims.source.order_id).toBe(order.snapshot.order_id)
  expect(claims.plan_id).toBe(displayed.plan_id)
  expect(claims.plan_version).toBe(displayed.plan_version)
  expect(claims.entitlements).toEqual(displayed.entitlements)
  expect(claims.validity).toEqual({ not_before: order.snapshot.starts_at, expiry: { mode: 'fixed', expires_at: order.snapshot.ends_at } })
  expect(claims.binding).toEqual({ mode: 'installation', installation_id: installation.installation_id, machine_fingerprint_sha256: installation.machine_fingerprint_sha256, transfer_sequence: 0 })
  const changed = JSON.parse(bytes)
  changed.claims.plan_version += 1
  const changedPath = testInfo.outputPath('tampered-website-order.license.json')
  await writeFile(changedPath, JSON.stringify(changed))
  const rejected = verifyWithRust(changedPath)
  expect(rejected.error).toBeUndefined()
  expect(rejected.status).toBe(1)
  expect(rejected.stdout).toBe('')
  const currentOrder = await actor.get(`${origin}/api/operations/v1${orderPath}`)
  expect(currentOrder.status()).toBe(200)
  const finalOrder = await currentOrder.json()
  expect(finalOrder.status).toBe('fulfilled')
  expect(finalOrder.snapshot).toEqual(order.snapshot)
  await writeFile(testInfo.outputPath('website-order-rust-evidence.json'), JSON.stringify({
    environment: 'local', trust: 'test-only', displayed, order_sha256: order.sha256,
    order: order.snapshot, payment_sha256: payment.sha256, fulfillment_id: approved.snapshot.id,
    document_sha256: issued.document_sha256, rust_verified_claims: claims, tampered_document_rejected: true,
  }, null, 2))
}

test('quotation order binds an accepted website version and recovers after a lost response', async ({ page, browser }, testInfo) => {
  test.setTimeout(90_000)
  const { approval, site } = await publicationFixture()
  try {
    const head = (await (await actor.get(`${origin}/api/operations/v1/commercial/publication-heads/local`)).json()).active_id
    const preparedResponse = await write(actor, '/commercial/publications', { operation_id: randomUUID(), catalog_revision: approval.snapshot.id, build_sha256: createHash('sha256').update(site.manifest).digest('hex'), expected_active_id: head, accept_until: new Date(Date.now() + 86400_000).toISOString(), reason: '隔离报价订单测试', current_password: password })
    expect(preparedResponse.status()).toBe(201)
    const publication = await preparedResponse.json()
    const acceptance = await write(actor, `/commercial/publications/${publication.snapshot.id}/accept`, { current_password: password })
    expect(acceptance.status()).toBe(200)
    const displayed = await observeWebsiteQuote(browser, approval, testInfo)
    await page.setViewportSize({ width: 1366, height: 648 })
    await page.goto('/commercial/orders')
    await page.getByRole('button', { name: '按官网报价建单', exact: true }).click()
    await page.getByRole('combobox', { name: '报价客户', exact: true }).click()
    await page.getByRole('option').filter({ hasText: customer.name }).click()
    await page.getByLabel('报价来源编号', { exact: true }).fill(displayed.revision)
    await page.getByRole('button', { name: '读取报价来源', exact: true }).click()
    await expect(page.getByRole('dialog', { name: '按官网报价建单', exact: true })).toContainText(publication.snapshot.id)
    await page.getByRole('combobox', { name: '报价套餐版本', exact: true }).click()
    await page.getByRole('option').filter({ hasText: savedPlan.snapshot.definition.code }).click()
    await page.getByRole('combobox', { name: '报价订阅期限', exact: true }).click()
    await page.getByRole('option', { name: '3 年', exact: true }).click()
    await page.getByLabel('报价合同开始时间（UTC）', { exact: true }).fill('2028-02-29T16:30')
    let committed, submitted
    const path = '**/api/operations/v1/commercial/quotation-orders'
    await page.route(path, async route => {
      submitted = route.request().postDataJSON()
      expect(Object.keys(submitted).sort()).toEqual(['operation_id', 'customer_id', 'publication_id', 'catalog_revision', 'plan_id', 'plan_version', 'years', 'starts_at'].sort())
      const response = await route.fetch(); expect(response.status()).toBe(201)
      committed = await response.json()
      await route.abort('failed')
    }, { times: 1 })
    await page.getByRole('button', { name: '保存报价订单', exact: true }).click()
    await expect(page.getByRole('alert')).toContainText('结果尚未确认')
    await page.route(path, route => route.fulfill({ status: 403, contentType: 'application/json', body: JSON.stringify({ error: { code: 'COMMERCIAL_PERMISSION_DENIED', number: 67701, message: '临时权限拒绝' } }) }), { times: 1 })
    await page.getByRole('button', { name: '重试原报价请求', exact: true }).click()
    await expect(page.getByRole('alert')).toContainText('结果尚未确认')
    await page.reload()
    await page.getByRole('button', { name: '继续报价订单', exact: true }).click()
    await expect(page.getByRole('dialog', { name: '按官网报价建单', exact: true })).toContainText(publication.snapshot.id)
    await page.getByRole('button', { name: '重试原报价请求', exact: true }).click()
    const detail = page.getByRole('dialog', { name: '订单权益快照', exact: true })
    await expect(detail).toContainText(publication.snapshot.id)
    await expect(detail).toContainText('本地验证')
    expect(validateOrder(committed.snapshot), JSON.stringify(validateOrder.errors)).toBe(true)
    expect(committed.snapshot.source).toMatchObject({ kind: 'publication', publication_id: publication.snapshot.id, publication_sha256: publication.sha256, catalog_revision: approval.snapshot.id, environment: 'local', ordered_at: new Date(committed.created_at).toISOString() })
    expect(committed.snapshot.plan_sha256).toBe(savedPlan.sha256)
    expect(committed.snapshot.amount_minor).toBe(1529745)
    expect(committed.snapshot.amount_minor).toBe(displayed.amount_minor)
    expect(submitted).toMatchObject({ catalog_revision: displayed.revision, plan_id: displayed.plan_id, plan_version: displayed.plan_version, years: displayed.years })
    const replay = await write(actor, '/commercial/quotation-orders', submitted)
    expect(await replay.json()).toEqual(committed)
    const changed = await write(actor, '/commercial/quotation-orders', { ...submitted, years: 2 })
    expect(changed.status()).toBe(409)
    const forged = await write(actor, '/commercial/quotation-orders', { ...submitted, environment: 'production' })
    expect(forged.status()).toBe(400)
    const list = await actor.get(`${origin}/api/operations/v1/commercial/orders?limit=100`)
    expect((await list.json()).items.filter(r => r.operation_id === submitted.operation_id)).toEqual([committed])
    expect(await page.evaluate(() => Object.keys(sessionStorage).filter(key => key.includes(':quotation-order:')))).toHaveLength(0)
    await mkdir(new URL('../../dist/commercial-validation/screenshots/', import.meta.url), { recursive: true })
    await detail.evaluate(element => { element.scrollTop = element.scrollHeight })
    await page.screenshot({ path: 'dist/commercial-validation/screenshots/quotation-desktop.png', animations: 'disabled' })
    await page.setViewportSize({ width: 390, height: 720 })
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    await page.screenshot({ path: 'dist/commercial-validation/screenshots/quotation-mobile.png', animations: 'disabled' })
    await verifyQuotationFulfillment(committed, displayed, testInfo)
  } finally { await site.close() }
})

test('publication reconciles a real website build and recovers the original acceptance after lost responses', async ({ page }) => {
  test.setTimeout(90_000)
  const { approval, site } = await publicationFixture()
  try {
    expect(validateWebsiteRelease(JSON.parse(site.manifest)), JSON.stringify(validateWebsiteRelease.errors)).toBe(true)
    const buildSHA = createHash('sha256').update(site.manifest).digest('hex')
    const reason = `官网核对 ${randomUUID().slice(0, 8)}`
    await page.setViewportSize({ width: 1366, height: 648 })
    await page.goto('/commercial/publications')
    await page.getByRole('button', { name: '新建核对', exact: true }).click()
    await page.getByLabel('目录版本', { exact: true }).fill(approval.snapshot.id)
    await page.getByRole('button', { name: '读取目录', exact: true }).click()
    await page.getByLabel('官网构建清单', { exact: true }).setInputFiles(site.manifestPath)
    await expect(page.getByRole('dialog')).toContainText(buildSHA)
    // datetime-local uses the browser's local zone; preserve the exact returned
    // deadline rather than recomputing it while recovering the request.
    await page.getByLabel('受理截止时间', { exact: true }).fill(new Date(Date.now() + 86400_000).toISOString().slice(0, 16))
    await page.getByLabel('核对说明', { exact: true }).fill(reason)
    await page.getByRole('checkbox').check()
    await page.getByLabel('当前密码', { exact: true }).fill('wrong-test-only')
    await page.getByRole('button', { name: '保存核对请求', exact: true }).click()
    await expect(page.getByRole('alert')).toContainText('当前密码验证失败')
    await expect(page.getByLabel('当前密码', { exact: true })).toHaveValue('')
    let prepared
    await page.route('**/api/operations/v1/commercial/publications', async route => {
      if (route.request().method() !== 'POST') return route.continue()
      const response = await route.fetch(); expect(response.status()).toBe(201)
      prepared = await response.json(); expect(validatePublication(prepared), JSON.stringify(validatePublication.errors)).toBe(true)
      await route.abort('failed'); await page.unroute('**/api/operations/v1/commercial/publications')
    })
    await page.getByLabel('当前密码', { exact: true }).fill(password)
    await page.getByRole('button', { name: '保存核对请求', exact: true }).click()
    await expect(page.getByRole('alert')).toContainText('重试原请求')
    const pending = await page.evaluate(() => Object.keys(sessionStorage).filter(key => key.includes(':publication:')).map(key => sessionStorage.getItem(key)))
    expect(pending).toHaveLength(1); expect(pending[0]).not.toContain(password); expect(pending[0]).not.toContain('current_password')
    await page.reload()
    await page.getByRole('button', { name: '继续原请求', exact: true }).click()
    await page.getByLabel('当前密码', { exact: true }).fill(password)
    await page.getByRole('button', { name: '重试原请求', exact: true }).click()
    await expect(page.getByRole('dialog', { name: '发布核对详情', exact: true })).toBeVisible()
    expect(await page.evaluate(() => Object.keys(sessionStorage).filter(key => key.includes(':publication:')))).toHaveLength(0)
    site.tamper(true)
    await page.getByLabel('核对验证密码', { exact: true }).fill(password)
    await page.getByRole('button', { name: '核对官网并批准受理', exact: true }).click()
    await expect(page.getByRole('alert')).toContainText('官网内容尚未通过核对')
    const failed = await actor.get(`${origin}/api/operations/v1/commercial/publications/${prepared.snapshot.id}`)
    expect((await failed.json()).status).toBe('prepared')
    const failureResponse = await actor.get(`${origin}/api/operations/v1/commercial/publications/${prepared.snapshot.id}/failures`)
    expect(failureResponse.status()).toBe(200)
    const failureItems = (await failureResponse.json()).items
    expect(failureItems).toHaveLength(1)
    expect(validatePublicationFailure(failureItems[0]), JSON.stringify(validatePublicationFailure.errors)).toBe(true)
    expect(failureItems[0]).toMatchObject({ publication_id: prepared.snapshot.id, publication_sha256: prepared.sha256, stage: 'verification', code: 'content_unverified' })
    await page.reload()
    await page.getByRole('row').filter({ hasText: reason }).getByRole('button', { name: '查看', exact: true }).click()
    await expect(page.getByRole('region', { name: '未完成核对记录' })).toContainText('官网内容未通过核对')
    site.tamper(false)
    let accepted
    const acceptPath = `**/api/operations/v1/commercial/publications/${prepared.snapshot.id}/accept`
    await page.route(acceptPath, async route => {
      const response = await route.fetch(); expect(response.status()).toBe(200)
      accepted = await response.json(); expect(validatePublication(accepted), JSON.stringify(validatePublication.errors)).toBe(true)
      await route.abort('failed')
    }, { times: 1 })
    await page.getByLabel('核对验证密码', { exact: true }).fill(password)
    await page.getByRole('button', { name: '核对官网并批准受理', exact: true }).click()
    await expect(page.getByRole('alert')).toContainText('重新读取原记录')
    await expect(page.getByRole('button', { name: '以此记录准备新核对', exact: true })).toHaveCount(0)
    const readPath = `**/api/operations/v1/commercial/publications/${prepared.snapshot.id}`
    await page.route(readPath, route => route.abort('failed'), { times: 1 })
    await page.getByRole('button', { name: '重新读取原记录', exact: true }).click()
    await expect(page.getByRole('alert')).toBeVisible()
    await expect(page.getByRole('button', { name: '以此记录准备新核对', exact: true })).toHaveCount(0)
    await page.getByRole('button', { name: '重新读取原记录', exact: true }).click()
    await expect(page.getByLabel('核对验证密码', { exact: true })).toHaveCount(0)
    await expect(page.getByRole('button', { name: '以此记录准备新核对', exact: true })).toHaveCount(0)
    await expect(page.getByRole('dialog')).toContainText('http://127.0.0.1:26394')
    expect(accepted.snapshot).toEqual(prepared.snapshot)
    expect(accepted.evidence.build_sha256).toBe(buildSHA)
    const replay = await write(actor, `/commercial/publications/${prepared.snapshot.id}/accept`, { current_password: password })
    expect(await replay.json()).toEqual(accepted)
    const preserved = await actor.get(`${origin}/api/operations/v1/commercial/publications/${prepared.snapshot.id}/failures`)
    expect((await preserved.json()).items).toEqual(failureItems)
    const list = await actor.get(`${origin}/api/operations/v1/commercial/publications?limit=100`)
    expect((await list.json()).items.filter(record => record.snapshot.request.operation_id === prepared.snapshot.request.operation_id)).toHaveLength(1)
    const head = await actor.get(`${origin}/api/operations/v1/commercial/publication-heads/local`)
    expect((await head.json()).active_id).toBe(prepared.snapshot.id)
    const production = await actor.get(`${origin}/api/operations/v1/commercial/publication-heads/production`)
    expect((await production.json()).active_id).toBe('')
    await mkdir(new URL('../../dist/commercial-validation/screenshots/', import.meta.url), { recursive: true })
    await page.getByRole('dialog').evaluate(element => { element.scrollTop = 0 })
    await page.screenshot({ path: 'dist/commercial-validation/screenshots/publication-desktop.png', animations: 'disabled' })
    await page.setViewportSize({ width: 390, height: 720 })
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    await page.screenshot({ path: 'dist/commercial-validation/screenshots/publication-mobile.png', animations: 'disabled' })
  } finally { await site.close() }
})

test('publication preserves inputs after definite absence and creates a new candidate after a saved head conflict', async ({ page }) => {
  test.setTimeout(90_000)
  const { approval, site } = await publicationFixture()
  try {
    const head = (await (await actor.get(`${origin}/api/operations/v1/commercial/publication-heads/local`)).json()).active_id
    const input = { operation_id: `saved_${randomUUID()}`, catalog_revision: approval.snapshot.id, build_sha256: createHash('sha256').update(site.manifest).digest('hex'), expected_active_id: head, accept_until: new Date(Date.now() + 2 * 86400_000).toISOString(), reason: `已保存候选 ${randomUUID()}` }
    const savedResponse = await write(actor, '/commercial/publications', { ...input, current_password: password })
    expect(savedResponse.status()).toBe(201)
    const saved = await savedResponse.json()
    const reason = `未保存候选 ${randomUUID()}`
    const until = new Date(Date.now() + 86400_000).toISOString().slice(0, 16)
    await page.goto('/commercial/publications')
    await page.getByRole('button', { name: '新建核对', exact: true }).click()
    await page.getByLabel('目录版本', { exact: true }).fill(approval.snapshot.id)
    await page.getByRole('button', { name: '读取目录', exact: true }).click()
    await page.getByLabel('官网构建清单', { exact: true }).setInputFiles(site.manifestPath)
    await page.getByLabel('受理截止时间', { exact: true }).fill(until)
    await page.getByLabel('核对说明', { exact: true }).fill(reason)
    await page.getByRole('checkbox').check()
    let original
    await page.route('**/api/operations/v1/commercial/publications', async route => {
      if (route.request().method() !== 'POST') return route.continue()
      original = route.request().postDataJSON()
      await route.abort('failed'); await page.unroute('**/api/operations/v1/commercial/publications')
    })
    await page.getByLabel('当前密码', { exact: true }).fill(password)
    await page.getByRole('button', { name: '保存核对请求', exact: true }).click()
    await expect(page.getByRole('alert')).toContainText('重试原请求')
    const moverResponse = await write(actor, '/commercial/publications', { ...input, operation_id: `mover_${randomUUID()}`, reason: '并发发布', current_password: password })
    expect(moverResponse.status()).toBe(201)
    const mover = await moverResponse.json()
    expect((await write(actor, `/commercial/publications/${mover.snapshot.id}/accept`, { current_password: password })).status()).toBe(200)
    await page.reload()
    await page.getByRole('button', { name: '继续原请求', exact: true }).click()
    await page.getByLabel('当前密码', { exact: true }).fill(password)
    await page.getByRole('button', { name: '重试原请求', exact: true }).click()
    await expect(page.getByRole('alert')).toContainText('原请求未保存')
    await expect(page.getByLabel('目录版本', { exact: true })).toHaveValue(approval.snapshot.id)
    await expect(page.getByLabel('核对说明', { exact: true })).toHaveValue(reason)
    await expect(page.getByLabel('受理截止时间', { exact: true })).toHaveValue(until)
    expect(await page.evaluate(() => Object.keys(sessionStorage).filter(key => key.includes(':publication:')))).toHaveLength(0)
    await page.getByRole('button', { name: '读取目录', exact: true }).click()
    await expect(page.getByRole('dialog')).toContainText(mover.snapshot.id)
    await page.getByLabel('官网构建清单', { exact: true }).setInputFiles(site.manifestPath)
    await expect(page.getByRole('checkbox')).not.toBeChecked()
    await page.getByRole('checkbox').check()
    await page.getByLabel('当前密码', { exact: true }).fill(password)
    const recreatedResponse = page.waitForResponse(response => response.url().endsWith('/commercial/publications') && response.request().method() === 'POST')
    await page.getByRole('button', { name: '保存核对请求', exact: true }).click()
    const recreated = await (await recreatedResponse).json()
    expect(recreated.snapshot.request.operation_id).not.toBe(original.operation_id)
    expect(recreated.snapshot.request.expected_active_id).toBe(mover.snapshot.id)
    const list = (await (await actor.get(`${origin}/api/operations/v1/commercial/publications?limit=100`)).json()).items
    expect(list.some(record => record.snapshot.request.operation_id === original.operation_id)).toBe(false)

    // A different, definitely saved candidate cannot be rewritten after its CAS
    // base changes. Its original row and failed attempt remain available.
    await page.reload()
    await page.getByRole('row').filter({ hasText: input.reason }).getByRole('button', { name: '查看', exact: true }).click()
    await page.getByLabel('核对验证密码', { exact: true }).fill(password)
    await page.getByRole('button', { name: '核对官网并批准受理', exact: true }).click()
    await expect(page.getByRole('alert')).toContainText('重新读取原记录')
    await expect(page.getByRole('button', { name: '以此记录准备新核对', exact: true })).toHaveCount(0)
    await page.getByRole('button', { name: '重新读取原记录', exact: true }).click()
    await expect(page.getByRole('region', { name: '未完成核对记录' })).toContainText('其他发布已更新当前记录')
    await page.getByRole('button', { name: '以此记录准备新核对', exact: true }).click()
    const replacementDialog = page.getByRole('dialog', { name: '准备官网核对', exact: true })
    await expect(page.getByRole('dialog', { name: '发布核对详情', exact: true })).toHaveCount(0)
    await expect(replacementDialog).toContainText('原记录保持不变')
    await expect(replacementDialog).toContainText(saved.snapshot.id)
    await expect(replacementDialog).toContainText(mover.snapshot.id)
    await expect(page.getByLabel('核对说明', { exact: true })).toHaveValue(input.reason)
    await expect(page.getByRole('checkbox')).not.toBeChecked()
    await page.getByRole('checkbox').check()
    await page.getByLabel('当前密码', { exact: true }).fill(password)
    const replacedResponse = page.waitForResponse(response => response.url().endsWith('/commercial/publications') && response.request().method() === 'POST')
    await page.getByRole('button', { name: '保存核对请求', exact: true }).click()
    const replacement = await (await replacedResponse).json()
    expect(replacement.snapshot.id).not.toBe(saved.snapshot.id)
    expect(replacement.snapshot.request.expected_active_id).toBe(mover.snapshot.id)
    const unchanged = await actor.get(`${origin}/api/operations/v1/commercial/publications/${saved.snapshot.id}`)
    expect(await unchanged.json()).toEqual(saved)
    const failures = (await (await actor.get(`${origin}/api/operations/v1/commercial/publications/${saved.snapshot.id}/failures`)).json()).items
    expect(failures).toHaveLength(1); expect(failures[0].code).toBe('head_conflict')
  } finally { await site.close() }
})

test('approved public catalog preserves reviewed versions and recovers local export after lost responses', async ({ page }) => {
  test.setTimeout(60_000)
  await page.setViewportSize({ width: 1366, height: 648 })
  const definitions = [
    { ...fixture, code: `catalog_paid_${randomUUID()}`, name: '目录年度示例', description: '仅为隔离验证 <strong>按文本显示</strong>' },
    { ...fixture, code: `catalog_free_${randomUUID()}`, name: '目录免费示例', offer: { kind: 'free', expiry: { mode: 'none' } } },
    { ...fixture, code: `catalog_contact_${randomUUID()}`, name: '目录联系示例', offer: { kind: 'contact' } },
  ]
  const plans = []
  for (const definition of definitions) {
    const response = await write(actor, '/commercial/plans/versions', { operation_id: `catalog_plan_${randomUUID()}`, plan_id: '', expected_version: 0, definition })
    expect(response.status()).toBe(201); plans.push(await response.json())
  }
  const reason = `本地目录验证 ${randomUUID().slice(0, 8)}`
  await page.goto('/commercial/catalogs')
  await page.getByRole('button', { name: '新建公开目录', exact: true }).click()
  await page.getByLabel('批准说明', { exact: true }).fill(reason)
  for (const definition of definitions) {
    await page.getByRole('combobox', { name: '添加套餐版本', exact: true }).click()
    await page.getByRole('option').filter({ hasText: definition.code }).click()
    await page.getByRole('button', { name: '添加套餐', exact: true }).click()
  }
  await page.getByRole('button', { name: '上移', exact: true }).nth(1).click()
  await page.getByRole('button', { name: '预览公开内容', exact: true }).click()
  await expect(page.getByRole('dialog')).toContainText(definitions[0].description)
  await expect(page.getByRole('dialog').locator('strong')).toHaveCount(0)
  await expect(page.getByRole('heading', { name: '1 · 目录免费示例', exact: true })).toBeVisible()
  await expect.poll(() => page.getByRole('dialog').evaluate(element => element.scrollTop)).toBe(0)
  await mkdir(new URL('../../dist/commercial-validation/screenshots/', import.meta.url), { recursive: true })
  await page.screenshot({ path: 'dist/commercial-validation/screenshots/catalog-preview-desktop.png', fullPage: true, animations: 'disabled' })
  await page.setViewportSize({ width: 390, height: 720 })
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  await expect.poll(() => page.getByRole('dialog').evaluate(element => element.scrollTop)).toBe(0)
  await page.screenshot({ path: 'dist/commercial-validation/screenshots/catalog-preview-mobile.png', fullPage: true, animations: 'disabled' })
  await page.setViewportSize({ width: 1366, height: 648 })
  let approved, approvals = 0
  await page.route('**/api/operations/v1/commercial/catalogs', async route => {
    if (route.request().method() !== 'POST') return route.continue()
    approvals++
    const response = await route.fetch(); expect(response.status()).toBe(201)
    approved = await response.json()
    expect(validateCatalogRecord(approved), JSON.stringify(validateCatalogRecord.errors)).toBe(true)
    return route.abort('failed')
  })
  await page.getByLabel('当前密码', { exact: true }).fill(password)
  await page.getByRole('button', { name: '确认批准', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('保留原请求')
  await expect(page.getByLabel('当前密码', { exact: true })).toHaveValue('')
  const pending = await page.evaluate(() => Object.keys(sessionStorage).filter(key => key.includes(':catalog:')).map(key => sessionStorage.getItem(key)))
  expect(pending).toHaveLength(1); expect(pending[0]).not.toContain(password); expect(pending[0]).not.toContain('current_password')
  await page.reload()
  await page.getByRole('button', { name: '继续批准', exact: true }).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  expect(approvals).toBe(1)
  await page.unroute('**/api/operations/v1/commercial/catalogs')
  expect(await page.evaluate(() => Object.keys(sessionStorage).filter(key => key.includes(':catalog:')))).toHaveLength(0)
  const id = approved.snapshot.id
  expect(id).toMatch(/^catalog_[a-f0-9]{48}$/)
  const path = `/commercial/catalogs/${id}`
  const revised = await write(actor, '/commercial/plans/versions', { operation_id: `catalog_revise_${randomUUID()}`, plan_id: plans[0].snapshot.plan_id, expected_version: 1, definition: { ...definitions[0], name: '批准后修订的目录套餐', offer: { ...fixture.offer, annual_amount_minor: 799900 } } })
  expect(revised.status()).toBe(201)
  expect((await actor.get(`${origin}/api/operations/v1${path}/download`)).status()).toBe(409)
  await page.route(`**/api/operations/v1${path}`, async route => { await route.abort('failed'); await page.unroute(`**/api/operations/v1${path}`) })
  await page.getByRole('row').filter({ hasText: reason }).getByRole('button', { name: '查看', exact: true }).click()
  await expect(page.getByRole('button', { name: '导出到运营主机', exact: true })).toHaveCount(0)
  await page.getByRole('button', { name: '重新读取目录', exact: true }).click()
  await page.getByLabel('导出验证密码', { exact: true }).fill('wrong-test-only')
  await page.getByRole('button', { name: '导出到运营主机', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('当前密码验证失败')
  await expect(page.getByLabel('导出验证密码', { exact: true })).toHaveValue('')
  await expect(page.getByRole('dialog', { name: '登录状态已过期', exact: true })).toHaveCount(0)
  await page.route(`**/api/operations/v1${path}/export`, async route => {
    const response = await route.fetch(); expect(response.status()).toBe(200)
    expect((await response.json()).status).toBe('exported')
    await route.abort('failed'); await page.unroute(`**/api/operations/v1${path}/export`)
  })
  await page.getByLabel('导出验证密码', { exact: true }).fill(password)
  await page.getByRole('button', { name: '导出到运营主机', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('可重试同一目录')
  await expect(page.getByLabel('导出验证密码', { exact: true })).toHaveValue('')
  await page.getByLabel('导出验证密码', { exact: true }).fill(password)
  await page.getByRole('button', { name: '导出到运营主机', exact: true }).click()
  await expect(page.getByRole('button', { name: '核对并重新导出', exact: true })).toBeVisible()
  const downloaded = page.waitForEvent('download')
  await page.getByRole('button', { name: '下载 plans.json', exact: true }).click()
  const file = await downloaded
  expect(file.suggestedFilename()).toBe('plans.json')
  const bytes = await readFile(await file.path())
  const exportedBytes = await readFile(new URL(`../../dist/commercial-validation/public-catalogs/local/${id}/plans.json`, import.meta.url))
  expect(bytes.equals(exportedBytes)).toBe(true)
  expect(createHash('sha256').update(bytes).digest('hex')).toBe(approved.public.sha256)
  const catalog = JSON.parse(bytes.toString('utf8'))
  expect(validateCatalog(catalog), JSON.stringify(validateCatalog.errors)).toBe(true)
  expect(catalog).toEqual(approved.public.catalog)
  expect(catalog.environment).toBe('local')
  expect(catalog.plans.map(plan => plan.plan_id)).toEqual([plans[1].snapshot.plan_id, plans[0].snapshot.plan_id, plans[2].snapshot.plan_id])
  expect(catalog.plans.map(plan => plan.offer.kind)).toEqual(['free', 'fixed_price', 'contact'])
  expect(catalog.plans[1].version).toBe(1)
  expect(catalog.plans[1].offer.terms.find(term => term.years === 3).total_amount_minor).toBe(1529745)
  expect(bytes.toString('utf8')).not.toContain(reason)
  for (const field of ['current_password', 'transfer_limit', 'approved_by', 'expected_sha256']) expect(bytes.toString('utf8')).not.toContain(`"${field}"`)
  const invalidCatalogs = [
    { ...catalog, reason: 'private' },
    { ...catalog, environment: 'unknown' },
    { ...catalog, plans: [{ ...catalog.plans[0], transfer_limit: 2 }] },
    { ...catalog, plans: [{ ...catalog.plans[0], offer: { ...catalog.plans[0].offer, annual_amount_minor: 599900 } }] },
    { ...catalog, plans: [{ ...catalog.plans[0], entitlements: { ...catalog.plans[0].entitlements, private: true } }] },
  ]
  for (const invalid of invalidCatalogs) expect(validateCatalog(invalid)).toBe(false)
  const all = await (await actor.get(`${origin}/api/operations/v1/commercial/catalogs?limit=100`)).json()
  expect(all.items.filter(item => item.snapshot.id === id)).toHaveLength(1)
  await expect(page.getByText('目录版本和文件摘要已核对', { exact: true })).toHaveCount(0)
  await page.screenshot({ path: 'dist/commercial-validation/screenshots/catalog-export-desktop.png', fullPage: true, animations: 'disabled' })

  const emptyRequest = { operation_id: `empty_catalog_${randomUUID()}`, environment: 'local', reason: '隔离空目录验证', plans: [] }
  const emptyPreviewResponse = await write(actor, '/commercial/catalogs/preview', emptyRequest)
  expect(emptyPreviewResponse.status()).toBe(200)
  const emptyPreview = await emptyPreviewResponse.json()
  const mismatch = await write(actor, '/commercial/catalogs', { request: emptyRequest, expected_public_sha256: '0'.repeat(64), current_password: password })
  expect(mismatch.status()).toBe(409); expect((await mismatch.json()).error.number).toBe(67710)
  const emptyApproved = await write(actor, '/commercial/catalogs', { request: emptyRequest, expected_public_sha256: emptyPreview.sha256, current_password: password })
  expect(emptyApproved.status()).toBe(201)
  const emptyID = (await emptyApproved.json()).snapshot.id
  expect((await write(actor, `/commercial/catalogs/${emptyID}/export`, { current_password: password })).status()).toBe(200)
  const emptyDownload = await actor.get(`${origin}/api/operations/v1/commercial/catalogs/${emptyID}/download`)
  expect(emptyDownload.status()).toBe(200)
  expect(emptyDownload.headers()['x-content-sha256']).toBe(emptyPreview.sha256)
  const emptyCatalog = await emptyDownload.json()
  expect(validateCatalog(emptyCatalog), JSON.stringify(validateCatalog.errors)).toBe(true)
  expect(emptyCatalog.plans).toEqual([])
})

test('server drafts preserve concurrent edits and recover the exact frozen revision after a lost response', async ({ page }) => {
  await page.setViewportSize({ width: 1366, height: 648 })
  const definition = { ...fixture, code: `draft_browser_${randomUUID()}`, name: '草稿浏览器验证' }
  const created = await write(actor, '/commercial/plan-drafts', { operation_id: `draft_${randomUUID()}`, draft_id: '', expected_revision: 0, plan_id: '', expected_version: 0, definition })
  expect(created.status()).toBe(201)
  const first = await created.json()
  const id = first.snapshot.draft_id
  const planID = first.snapshot.plan_id
  expect((await actor.get(`${origin}/api/operations/v1/commercial/plans/${planID}`)).status()).toBe(404)
  await page.goto('/commercial/plan-drafts')
  const row = page.getByRole('row').filter({ hasText: definition.code })
  await row.getByRole('button', { name: '编辑草稿', exact: true }).click()
  await page.getByLabel('套餐名称', { exact: true }).fill('保留我的草稿修改')
  const competing = await write(actor, '/commercial/plan-drafts', { operation_id: `draft_${randomUUID()}`, draft_id: id, expected_revision: 1, plan_id: planID, expected_version: 0, definition: { ...definition, name: '其他运营保存的草稿' } })
  expect(competing.status()).toBe(201)
  await page.getByRole('button', { name: '保存草稿', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('草稿已变化')
  await page.getByRole('button', { name: '比较最新修订', exact: true }).click()
  await expect(page.getByRole('heading', { name: '服务器当前配置', exact: true })).toBeVisible()
  await page.getByRole('button', { name: '保留我的配置', exact: true }).click()
  await page.getByRole('button', { name: '保存草稿', exact: true }).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(row).toContainText('r3')
  const old = await actor.get(`${origin}/api/operations/v1/commercial/plan-drafts/${id}?revision=1`)
  expect((await old.json()).sha256).toBe(first.sha256)
  expect((await actor.get(`${origin}/api/operations/v1/commercial/plans/${planID}`)).status()).toBe(404)
  await row.getByRole('button', { name: '生成版本', exact: true }).click()
  await expect(page.getByRole('dialog')).toContainText('保留我的草稿修改')
  let frozen, originalRequest
  await page.route('**/api/operations/v1/commercial/plan-drafts/freeze', async route => {
    originalRequest = route.request().postDataJSON()
    const result = await route.fetch(); expect(result.status()).toBe(201)
    frozen = await result.json()
    await route.abort('failed')
    await page.unroute('**/api/operations/v1/commercial/plan-drafts/freeze')
  })
  await page.getByRole('button', { name: '生成固定版本', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('结果尚未确认')
  const newer = await write(actor, '/commercial/plan-drafts', { operation_id: `draft_${randomUUID()}`, draft_id: id, expected_revision: 3, plan_id: planID, expected_version: 1, definition: { ...definition, name: '生成版本后继续编辑' } })
  expect(newer.status()).toBe(201)
  await page.reload()
  await page.getByRole('button', { name: '继续生成版本', exact: true }).click()
  await expect(page.getByRole('dialog')).toContainText('草稿 r3')
  await expect(page.getByRole('dialog')).toContainText('保留我的草稿修改')
  await page.route('**/api/operations/v1/commercial/plan-drafts/freeze', async route => {
    expect(route.request().postDataJSON()).toEqual(originalRequest)
    const response = await route.fetch(); expect(response.status()).toBe(201)
    expect((await response.json()).sha256).toBe(frozen.sha256)
    await route.fulfill({ response })
  })
  await page.getByRole('button', { name: '重试原请求', exact: true }).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(row).toContainText('r4')
  const current = await (await actor.get(`${origin}/api/operations/v1/commercial/plans/${planID}`)).json()
  expect(current.sha256).toBe(frozen.sha256)
  expect(current.snapshot.version).toBe(1)
  expect(current.snapshot.definition.name).toBe('保留我的草稿修改')
  await expect(page.getByText('套餐版本 1 已生成', { exact: true })).toHaveCount(0)
  await mkdir(new URL('../../dist/commercial-validation/screenshots/', import.meta.url), { recursive: true })
  await page.screenshot({ path: 'dist/commercial-validation/screenshots/drafts-desktop.png', fullPage: true, animations: 'disabled' })
  await page.setViewportSize({ width: 390, height: 720 })
  await row.getByRole('button', { name: '编辑草稿', exact: true }).click()
  await expect(page.getByLabel('套餐名称', { exact: true })).toHaveValue('生成版本后继续编辑')
  await page.screenshot({ path: 'dist/commercial-validation/screenshots/drafts-mobile.png', fullPage: true, animations: 'disabled' })
})

test('approved free source survives lost responses and downloads an unchanged signed license', async ({ page }) => {
  const vector = JSON.parse(await readFile(new URL('../../contracts/test-vectors/license.v2.json', import.meta.url), 'utf8'))
  const freeRights = vector.cases[0].document.claims.entitlements
  const definition = { ...fixture, code: `free_browser_${randomUUID()}`, name: '免费分发浏览器测试', entitlements: freeRights, offer: { kind: 'free', expiry: { mode: 'fixed', expires_at: '2040-01-01T00:00:00.000Z' } } }
  const saved = await write(actor, '/commercial/plans/versions', { operation_id: `free_${randomUUID()}`, plan_id: '', expected_version: 0, definition })
  expect(saved.status()).toBe(201)
  const plan = await saved.json()
  await page.goto('/commercial/distributions')
  await page.getByRole('button', { name: '批准免费分发', exact: true }).click()
  await page.getByRole('combobox', { name: '免费套餐版本', exact: true }).click()
  await page.getByRole('option').filter({ hasText: definition.code }).click()
  await page.getByLabel('生效时间（UTC）', { exact: true }).fill('2026-09-06T00:00')
  await page.getByLabel('批准说明', { exact: true }).fill('仅用于隔离浏览器验证')
  await page.evaluate(() => { window.__commercialExpiredEvents = 0; window.addEventListener('aster:operations-session-expired', () => { window.__commercialExpiredEvents++ }) })
  await page.getByLabel('当前密码', { exact: true }).fill('incorrect-password-test-only')
  await page.getByRole('button', { name: '确认批准', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('当前密码验证失败')
  await expect(page.getByLabel('当前密码', { exact: true })).toHaveValue('')
  await expect(page.getByLabel('批准说明', { exact: true })).toHaveValue('仅用于隔离浏览器验证')
  await expect(page.getByLabel('生效时间（UTC）', { exact: true })).toHaveValue('2026-09-06T00:00')
  expect(await page.evaluate(() => window.__commercialExpiredEvents)).toBe(0)
  await expect(page.getByRole('dialog', { name: '登录状态已过期', exact: true })).toHaveCount(0)
  await page.getByLabel('当前密码', { exact: true }).fill(password)
  let approved, attempts = 0
  await page.route('**/api/operations/v1/commercial/distributions', async route => {
    if (route.request().method() !== 'POST') return route.continue()
    attempts++
    if (attempts === 1) {
      const response = await route.fetch(); expect(response.status()).toBe(201)
      approved = await response.json()
      expect(validateDistribution(approved), JSON.stringify(validateDistribution.errors)).toBe(true)
      expect(JSON.stringify(approved)).not.toContain('current_password')
      return route.abort('failed')
    }
    if (attempts === 2) return route.fulfill({ status: 403, json: { error: { code: 'COMMERCIAL_PERMISSION_DENIED', number: 67701, message: '测试中的临时权限拒绝' } } })
    const response = await route.fetch(); expect(response.status()).toBe(201)
    expect((await response.json()).snapshot.id).toBe(approved.snapshot.id)
    await route.fulfill({ response })
    await page.unroute('**/api/operations/v1/commercial/distributions')
  })
  await page.getByRole('button', { name: '确认批准', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('结果尚未确认')
  await expect(page.getByLabel('当前密码', { exact: true })).toHaveValue('')
  const pending = await page.evaluate(() => Object.keys(sessionStorage).filter(key => key.includes(':distribution:')).map(key => sessionStorage.getItem(key)))
  expect(pending).toHaveLength(1)
  expect(pending[0]).not.toContain(password)
  expect(pending[0]).not.toContain('current_password')
  await page.getByLabel('当前密码', { exact: true }).fill(password)
  await page.getByRole('button', { name: '重试原请求', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('临时权限拒绝')
  await page.reload()
  await page.getByRole('button', { name: '继续批准', exact: true }).click()
  await expect(page.getByLabel('当前密码', { exact: true })).toHaveValue('')
  await page.getByLabel('当前密码', { exact: true }).fill(password)
  await page.getByRole('button', { name: '重试原请求', exact: true }).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  const path = `/commercial/distributions/${approved.snapshot.id}`
  expect((await actor.get(`${origin}/api/operations/v1${path}/download`)).status()).toBe(409)
  const revised = await write(actor, '/commercial/plans/versions', { operation_id: `revise_${randomUUID()}`, plan_id: plan.snapshot.plan_id, expected_version: 1, definition: { ...definition, name: '批准后修改的套餐名称' } })
  expect(revised.status()).toBe(201)
  const row = page.getByRole('row').filter({ hasText: approved.snapshot.id })
  await row.getByRole('button', { name: '查看', exact: true }).click()
  await page.getByRole('combobox', { name: '受限签发密钥', exact: true }).click()
  await page.getByRole('option', { name: 'local-free-test-only', exact: true }).click()
  await page.getByLabel('当前密码', { exact: true }).fill(password)
  let issued
  await page.route(`**/api/operations/v1${path}/issue`, async route => {
    const response = await route.fetch(); expect(response.status()).toBe(200)
    issued = await response.json()
    expect(validateDistribution(issued), JSON.stringify(validateDistribution.errors)).toBe(true)
    await route.abort('failed'); await page.unroute(`**/api/operations/v1${path}/issue`)
  })
  await page.getByRole('button', { name: '签发授权', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('重新读取此记录')
  await expect(page.getByLabel('当前密码', { exact: true })).toHaveValue('')
  await page.getByRole('button', { name: '重新读取', exact: true }).click()
  await expect(page.getByRole('button', { name: '下载授权文件', exact: true })).toBeEnabled()
  const downloaded = page.waitForEvent('download')
  await page.getByRole('button', { name: '下载授权文件', exact: true }).click()
  const file = await downloaded
  const bytes = await readFile(await file.path())
  expect(createHash('sha256').update(bytes).digest('hex')).toBe(issued.document_sha256)
  const document = JSON.parse(bytes.toString('utf8'))
  expect(document.claims).toEqual(issued.claims)
  expect(document.claims.plan_version).toBe(1)
  expect(document.claims.binding).toEqual({ mode: 'unbound' })
  expect(document.claims.entitlements).toEqual(approved.snapshot.plan.definition.entitlements)
  expect(document.claims.validity).toEqual({ not_before: '2026-09-06T00:00:00.000Z', expiry: definition.offer.expiry })
  const publicProfiles = await (await actor.get(`${origin}/api/operations/v1/commercial/issuers`)).json()
  expect(JSON.stringify(publicProfiles)).not.toContain('private_key')
  const publicKey = publicProfiles.items.find(value => value.key_id === document.claims.key_id)
  const canonical = value => Array.isArray(value) ? value.map(canonical) : value && typeof value === 'object' ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value
  expect(verify(null, Buffer.from(JSON.stringify(canonical(document.claims))), createPublicKey({ key: Buffer.from(publicKey.public_key_spki, 'base64url'), format: 'der', type: 'spki' }), Buffer.from(document.signature, 'base64url'))).toBeTruthy()
  const replay = await write(actor, `${path}/issue`, { key_id: document.claims.key_id, current_password: password })
  expect(replay.status()).toBe(200); expect((await replay.json()).document_sha256).toBe(issued.document_sha256)
  const all = await (await actor.get(`${origin}/api/operations/v1/commercial/distributions`)).json()
  expect(all.items.filter(value => value.operation_id === approved.operation_id)).toHaveLength(1)
  await mkdir(new URL('../../dist/commercial-validation/screenshots/', import.meta.url), { recursive: true })
  await page.screenshot({ path: 'dist/commercial-validation/screenshots/free-distribution.png', fullPage: true, animations: 'disabled' })
})

test('real storage conflict recovery preserves edits and the former version', async ({ page }) => {
  await page.goto('/commercial/plans')
  const row = page.getByRole('row').filter({ hasText: savedPlan.snapshot.definition.code })
  await row.getByRole('button', { name: '修订', exact: true }).click()
  await page.getByLabel('套餐名称', { exact: true }).fill('保留浏览器修改')
  const competing = await write(actor, '/commercial/plans/versions', { operation_id: `browser_${randomUUID()}`, plan_id: savedPlan.snapshot.plan_id, expected_version: 1, definition: { ...savedPlan.snapshot.definition, name: '并发保存的版本' } })
  expect(competing.status()).toBe(201)
  await page.route('**/api/operations/v1/commercial/plans/versions', async route => {
    const response = await route.fetch(); expect(response.status()).toBe(409)
    expect((await response.json()).error.code).toBe('COMMERCIAL_PLAN_VERSION_CONFLICT')
    await route.abort('failed')
    await page.unroute('**/api/operations/v1/commercial/plans/versions')
  })
  await page.getByRole('button', { name: '保存版本', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('结果尚未确认')
  await page.getByRole('button', { name: '重试原请求', exact: true }).click()
  await expect(page.getByRole('button', { name: '比较最新版本' })).toBeVisible()
  await page.getByRole('button', { name: '比较最新版本' }).click()
  await expect(page.getByRole('heading', { name: '已保存 v2 · 并发保存的版本' })).toBeVisible()
  await page.getByRole('button', { name: '保留我的配置' }).click()
  await expect(page.getByLabel('套餐名称', { exact: true })).toHaveValue('保留浏览器修改')
  await page.getByRole('button', { name: '保存版本', exact: true }).click()
  await expect(row).toContainText('v3')
  await expect(page.getByRole('dialog')).toHaveCount(0)
  const original = await actor.get(`${origin}/api/operations/v1/commercial/plans/${savedPlan.snapshot.plan_id}/versions/1`)
  expect((await original.json()).sha256).toBe(savedPlan.sha256)
  const current = await actor.get(`${origin}/api/operations/v1/commercial/plans/${savedPlan.snapshot.plan_id}`)
  expect((await current.json()).snapshot.definition.name).toBe('保留浏览器修改')
  await mkdir(new URL('../../dist/commercial-validation/screenshots/', import.meta.url), { recursive: true })
  await page.screenshot({ path: 'dist/commercial-validation/screenshots/plans-desktop.png', fullPage: true, animations: 'disabled' })
})

test('real order survives a lost response with one stored order and fixed rights', async ({ page }) => {
  await page.goto('/commercial/orders')
  await page.getByRole('button', { name: '新增订单', exact: true }).click()
  await page.getByRole('combobox', { name: '客户', exact: true }).click()
  await page.getByRole('option').filter({ hasText: customer.name }).last().click()
  await page.getByRole('combobox', { name: '套餐版本', exact: true }).click()
  await page.getByRole('option').filter({ hasText: savedPlan.snapshot.definition.code }).click()
  await page.getByRole('combobox', { name: '订阅期限', exact: true }).click()
  await page.getByRole('option', { name: '3 年 · 85%' }).click()
  await page.getByLabel('合同开始时间（UTC）', { exact: true }).fill('2028-02-29T16:30')
  let committed, requests = 0
  await page.route('**/api/operations/v1/commercial/orders', async route => {
    if (route.request().method() !== 'POST') return route.continue()
    requests++
    if (requests === 1) {
      const response = await route.fetch(); expect(response.status()).toBe(201)
      committed = await response.json()
      return route.abort('failed')
    }
    // A rejected retry does not prove that the earlier request did not commit.
    if (requests === 2) return route.fulfill({ status: 403, contentType: 'application/json', body: JSON.stringify({ error: { code: 'COMMERCIAL_PERMISSION_DENIED', number: 67701, message: '临时权限拒绝' } }) })
    return route.continue()
  })
  await page.getByRole('button', { name: '创建订单', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('结果尚未确认')
  await page.getByRole('button', { name: '重试原请求', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('临时权限拒绝')
  await page.reload()
  await page.getByRole('button', { name: '继续创建', exact: true }).click()
  await page.getByRole('button', { name: '重试原请求', exact: true }).click()
  await expect(page.getByRole('heading', { name: '订单权益快照', exact: true })).toBeVisible()
  await expect(page.getByRole('dialog', { name: '新增权益订单', exact: true })).toHaveCount(0)
  await expect(page.getByRole('dialog', { name: '订单权益快照', exact: true })).toContainText(committed.snapshot.order_id)
  const records = await actor.get(`${origin}/api/operations/v1/commercial/orders?limit=100`)
  const matches = (await records.json()).items.filter(item => item.operation_id === committed.operation_id)
  expect(matches).toHaveLength(1)
  expect(matches[0].snapshot.amount_minor).toBe(1529745)
  expect(matches[0].snapshot.starts_at).toBe('2028-02-29T16:30:00.000Z')
  expect(matches[0].snapshot.ends_at).toBe('2031-02-28T16:30:00.000Z')
  await page.screenshot({ path: 'dist/commercial-validation/screenshots/order-snapshot.png', fullPage: true, animations: 'disabled' })
  await page.getByRole('dialog', { name: '订单权益快照', exact: true }).getByRole('button', { name: '关闭', exact: true }).click()
  await page.setViewportSize({ width: 390, height: 720 })
  await page.getByRole('button', { name: '新增订单', exact: true }).click()
  await expect(page.getByRole('dialog', { name: '新增权益订单', exact: true })).toBeVisible()
  await expect(page.getByText('正在读取客户和套餐', { exact: true })).toHaveCount(0)
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  await page.screenshot({ path: 'dist/commercial-validation/screenshots/order-mobile.png', fullPage: true, animations: 'disabled' })
})


test('full payment preserves the original receipt across lost responses and reloads', async ({ page }) => {
  const orderResponse = await write(actor, '/commercial/orders', { operation_id: `payment_order_${randomUUID()}`, customer_id: customer.id, plan_id: savedPlan.snapshot.plan_id, plan_version: savedPlan.snapshot.version, years: 1, starts_at: '2028-01-01T00:00:00.000Z' })
  expect(orderResponse.status()).toBe(201)
  const order = await orderResponse.json()
  const path = `/commercial/orders/${order.snapshot.order_id}/payment`
  const projection = await actor.get(`${origin}/api/operations/v1${path}-context`)
  expect(projection.status()).toBe(200)
  const context = await projection.json()
  expect(validatePaymentContext(context), JSON.stringify(validatePaymentContext.errors)).toBe(true)
  expect(context.order_sha256).toBe(order.sha256)
  await page.goto('/commercial/orders')
  await page.getByRole('button', { name: '核对订单到账', exact: true }).click()
  const dialog = page.getByRole('dialog', { name: '核对订单到账', exact: true })
  await dialog.getByLabel('到账订单编号', { exact: true }).fill(order.snapshot.order_id)
  await dialog.getByRole('button', { name: '读取到账订单', exact: true }).click()
  await expect(dialog.getByText(order.sha256, { exact: true })).toBeVisible()
  await dialog.getByLabel('到账凭据', { exact: true }).fill('isolated-bank-receipt')
  await dialog.getByLabel('实际到账时间（UTC）', { exact: true }).fill('2026-09-06T00:00')
  await dialog.getByLabel('到账备注', { exact: true }).fill('完整到账恢复测试')
  await dialog.getByRole('checkbox').check()
  await dialog.getByLabel('到账确认当前密码', { exact: true }).fill('wrong-password')
  await dialog.getByRole('button', { name: '确认全额到账', exact: true }).click()
  await expect(dialog.getByRole('alert')).toContainText('当前密码验证失败')
  let attempts = 0; let committed
  await page.route('**/api/operations/v1/commercial/orders/*/payment', async route => {
    if (route.request().method() !== 'POST') return route.continue()
    attempts++
    if (attempts === 1) {
      const response = await route.fetch()
      expect(response.status()).toBe(200)
      committed = await response.json()
      expect(validatePayment(committed), JSON.stringify(validatePayment.errors)).toBe(true)
      await route.abort('failed')
    } else if (attempts === 2) await route.fulfill({ status: 403, contentType: 'application/json', body: JSON.stringify({ error: { code: 'COMMERCIAL_PERMISSION_DENIED', number: 67701, message: '临时权限拒绝' } }) })
    else await route.continue()
  })
  await dialog.getByLabel('到账确认当前密码', { exact: true }).fill(password)
  await dialog.getByRole('button', { name: '确认全额到账', exact: true }).click()
  await expect(dialog.getByRole('alert')).toContainText('结果尚未确认')
  const pending = await page.evaluate(() => Object.entries(sessionStorage).find(([key]) => key.includes(':payment:'))?.[1])
  expect(pending).toBeTruthy(); expect(pending).not.toContain(password); expect(pending).not.toContain('current_password')
  await dialog.getByLabel('到账确认当前密码', { exact: true }).fill(password)
  await dialog.getByRole('button', { name: '重试原到账确认', exact: true }).click()
  await expect(dialog.getByRole('alert')).toContainText('临时权限拒绝')
  await page.reload()
  await page.getByRole('button', { name: '继续确认到账', exact: true }).click()
  await expect(dialog.getByLabel('到账确认当前密码', { exact: true })).toHaveValue('')
  await dialog.getByLabel('到账确认当前密码', { exact: true }).fill(password)
  await dialog.getByRole('button', { name: '重试原到账确认', exact: true }).click()
  await expect(dialog.getByRole('status')).toHaveText('已记录全额到账')
  const recovered = await actor.get(`${origin}/api/operations/v1${path}`)
  expect(recovered.status()).toBe(200); expect(await recovered.json()).toEqual(committed)
  expect(committed.snapshot.order).toEqual(order.snapshot)
  expect(committed.snapshot.order_sha256).toBe(order.sha256)
  expect(JSON.stringify(committed)).not.toContain('current_password')
  const changed = await write(actor, path, { ...committed.snapshot.request, notes: 'different receipt', current_password: password })
  expect(changed.status()).toBe(409)
  const forged = await write(actor, path, { ...committed.snapshot.request, amount_minor: 1, current_password: password })
  expect(forged.status()).toBe(400)
  const currentOrder = await actor.get(`${origin}/api/operations/v1/commercial/orders/${order.snapshot.order_id}`)
  const current = await currentOrder.json(); expect(current.status).toBe('fulfillment_pending'); expect(current.snapshot).toEqual(order.snapshot)
  expect(await page.evaluate(() => Object.keys(sessionStorage).filter(key => key.includes(':payment:')))).toHaveLength(0)
  await page.setViewportSize({ width: 1366, height: 648 })
  await dialog.evaluate(element => { element.scrollTop = 0 })
  await expect(dialog.getByRole('heading', { name: '核对订单到账', exact: true })).toBeVisible()
  await page.screenshot({ path: 'dist/commercial-validation/screenshots/payment-desktop.png', animations: 'disabled' })
  await page.setViewportSize({ width: 390, height: 720 })
  await dialog.evaluate(element => { element.scrollTop = 0 })
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  await page.screenshot({ path: 'dist/commercial-validation/screenshots/payment-mobile.png', animations: 'disabled' })
})

test('paid fulfillment approves exact installation input and recovers one signed license after lost responses', async ({ page }) => {
  test.setTimeout(90_000)
  const orderResponse = await write(actor, '/commercial/orders', {
    operation_id: `fulfillment_order_${randomUUID()}`, customer_id: customer.id, plan_id: savedPlan.snapshot.plan_id,
    plan_version: savedPlan.snapshot.version, years: 1, starts_at: '2028-01-01T00:00:00.000Z',
  })
  expect(orderResponse.status()).toBe(201)
  const order = await orderResponse.json()
  const paymentResponse = await write(actor, `/commercial/orders/${order.snapshot.order_id}/payment`, {
    operation_id: `fulfillment_payment_${randomUUID()}`, expected_order_sha256: order.sha256,
    payment_reference: `isolated-paid-${randomUUID()}`, received_at: new Date().toISOString(), notes: '付费交付浏览器测试', current_password: password,
  })
  expect(paymentResponse.status()).toBe(200)
  const payment = await paymentResponse.json()
  const contextResponse = await actor.get(`${origin}/api/operations/v1/commercial/orders/${order.snapshot.order_id}/fulfillment-context`)
  expect(contextResponse.status()).toBe(200)
  const fulfillmentContext = await contextResponse.json()
  expect(validatePaidFulfillmentContext(fulfillmentContext), JSON.stringify(validatePaidFulfillmentContext.errors)).toBe(true)
  expect(fulfillmentContext.plan).toEqual(order.snapshot.plan)
  expect(fulfillmentContext.payment_sha256).toBe(payment.sha256)
  expect(fulfillmentContext.environment).toBe('local')

  const request = {
    schema: 'aster.license-request.v2', request_id: `request_${randomUUID()}`, product: 'aster-team',
    product_version: savedPlan.snapshot.definition.minimum_version, platform: 'linux', architecture: 'amd64',
    installation_id: `installation_${randomUUID()}`, machine_fingerprint_sha256: 'A'.repeat(43),
    machine_factors: [{ kind: 'dmi_product_uuid', sha256: 'B'.repeat(43) }, { kind: 'machine_id', sha256: 'C'.repeat(43) }],
    generated_at: new Date(Date.now() - 1000).toISOString(), license_schema: 'aster.license.v2',
    capability_catalog_version: savedPlan.snapshot.definition.entitlements.catalog_version,
    quota_policy_version: savedPlan.snapshot.definition.quota_policy_version,
  }
  expect(validateLicenseRequest(request), JSON.stringify(validateLicenseRequest.errors)).toBe(true)
  const rawRequest = JSON.stringify(request, null, 2).replaceAll('\n', '\r\n') + '\r\n'

  await page.goto('/commercial/orders')
  await page.getByRole('button', { name: '批准与签发', exact: true }).click()
  let dialog = page.getByRole('dialog', { name: '付费授权交付', exact: true })
  await dialog.getByLabel('付费交付订单编号', { exact: true }).fill(order.snapshot.order_id)
  await dialog.getByRole('button', { name: '读取交付状态', exact: true }).click()
  await expect(dialog).toContainText(savedPlan.snapshot.definition.name)
  await expect(dialog).toContainText(fulfillmentContext.order_sha256)
  await dialog.locator('input[type="file"]').setInputFiles({ name: 'paid-request.json', mimeType: 'application/json', buffer: Buffer.from(rawRequest) })
  await dialog.getByLabel('付费交付批准说明', { exact: true }).fill('已核对原订单、到账和目标安装')
  await dialog.getByRole('checkbox').check()
  await dialog.getByLabel('付费批准当前密码', { exact: true }).fill('wrong-password')
  await dialog.getByRole('button', { name: '确认批准交付', exact: true }).click()
  await expect(dialog.getByRole('alert')).toContainText('当前密码验证失败')
  await expect(dialog.getByLabel('付费批准当前密码', { exact: true })).toHaveValue('')

  const approvalPath = `**/api/operations/v1/commercial/orders/${order.snapshot.order_id}/fulfillment`
  let approved
  await page.route(approvalPath, async route => {
    const submitted = route.request().postDataJSON()
    expect(submitted.license_request_json).toBe(rawRequest)
    const response = await route.fetch(); expect(response.status()).toBe(200)
    approved = await response.json()
    expect(validatePaidFulfillment(approved), JSON.stringify(validatePaidFulfillment.errors)).toBe(true)
    await route.abort('failed')
  }, { times: 1 })
  await dialog.getByLabel('付费批准当前密码', { exact: true }).fill(password)
  await dialog.getByRole('button', { name: '确认批准交付', exact: true }).click()
  await expect(dialog.getByRole('alert')).toContainText('结果尚未确认')
  const approvalPending = await page.evaluate(() => Object.entries(sessionStorage).find(([key]) => key.includes(':fulfillment:'))?.[1])
  expect(approvalPending).toBeTruthy(); expect(approvalPending).not.toContain(password); expect(approvalPending).not.toContain('current_password')
  expect(JSON.parse(approvalPending).input.license_request_json).toBe(rawRequest)
  await page.route(approvalPath, route => route.fulfill({ status: 403, contentType: 'application/json', body: JSON.stringify({ error: { code: 'COMMERCIAL_PERMISSION_DENIED', number: 67701, message: '临时权限拒绝' } }) }), { times: 1 })
  await dialog.getByLabel('付费批准当前密码', { exact: true }).fill(password)
  await dialog.getByRole('button', { name: '重试原批准', exact: true }).click()
  await expect(dialog.getByRole('alert')).toContainText('临时权限拒绝')
  await page.reload()
  await page.getByRole('button', { name: '继续批准交付', exact: true }).click()
  dialog = page.getByRole('dialog', { name: '付费授权交付', exact: true })
  await expect(dialog.getByLabel('付费批准当前密码', { exact: true })).toHaveValue('')
  await dialog.getByLabel('付费批准当前密码', { exact: true }).fill(password)
  await dialog.getByRole('button', { name: '重试原批准', exact: true }).click()
  await expect(dialog).toContainText('已批准')
  expect(await page.evaluate(() => Object.keys(sessionStorage).filter(key => key.includes(':fulfillment:')))).toHaveLength(0)
  const approvedRead = await actor.get(`${origin}/api/operations/v1/commercial/orders/${order.snapshot.order_id}/fulfillment`)
  expect(approvedRead.status()).toBe(200); expect(await approvedRead.json()).toEqual(approved)
  expect(approved.snapshot.request.license_request_json).toBe(rawRequest)
  expect(approved.snapshot.installation_request).toEqual(request)

  await dialog.getByRole('button', { name: '读取签发配置', exact: true }).click()
  await dialog.getByRole('combobox', { name: '付费签发密钥', exact: true }).click()
  await page.getByRole('option', { name: 'local-paid-test-only', exact: true }).click()
  await dialog.getByLabel('付费签发当前密码', { exact: true }).fill(password)
  let issued
  const issuePath = `**/api/operations/v1/commercial/paid-fulfillments/${approved.snapshot.id}/issue`
  await page.route(issuePath, async route => {
    expect(route.request().postDataJSON().key_id).toBe('local-paid-test-only')
    const response = await route.fetch(); expect(response.status()).toBe(200)
    issued = await response.json()
    expect(validatePaidFulfillment(issued), JSON.stringify(validatePaidFulfillment.errors)).toBe(true)
    await route.abort('failed')
  }, { times: 1 })
  await dialog.getByRole('button', { name: '签发付费授权', exact: true }).click()
  await expect(dialog.getByRole('alert')).toContainText('结果尚未确认')
  const issuePending = await page.evaluate(() => Object.entries(sessionStorage).find(([key]) => key.includes(':fulfillment-issue:'))?.[1])
  expect(issuePending).toBeTruthy(); expect(issuePending).not.toContain(password); expect(issuePending).not.toContain('current_password')
  expect(JSON.parse(issuePending).input).toMatchObject({ fulfillment_id: approved.snapshot.id, key_id: 'local-paid-test-only' })
  await page.reload()
  await page.getByRole('button', { name: '继续签发交付', exact: true }).click()
  dialog = page.getByRole('dialog', { name: '付费授权交付', exact: true })
  await expect(dialog).toContainText('已签发')
  expect(await page.evaluate(() => Object.keys(sessionStorage).filter(key => key.includes(':fulfillment-issue:')))).toHaveLength(0)

  const downloaded = page.waitForEvent('download')
	await dialog.getByRole('button', { name: '下载授权文件', exact: true }).click()
  const file = await downloaded
  const bytes = await readFile(await file.path())
  expect(createHash('sha256').update(bytes).digest('hex')).toBe(issued.document_sha256)
  const document = JSON.parse(bytes.toString('utf8'))
  expect(validateLicense(document), JSON.stringify(validateLicense.errors)).toBe(true)
  expect(document).toEqual(issued.document)
  expect(document.claims.binding).toEqual({ mode: 'installation', installation_id: request.installation_id, machine_fingerprint_sha256: request.machine_fingerprint_sha256, transfer_sequence: 0 })
  expect(document.claims.entitlements).toEqual(order.snapshot.plan.definition.entitlements)
  expect(document.claims.validity).toEqual({ not_before: order.snapshot.starts_at, expiry: { mode: 'fixed', expires_at: order.snapshot.ends_at } })
  const publicProfiles = await (await actor.get(`${origin}/api/operations/v1/commercial/issuers`)).json()
  const publicKey = publicProfiles.items.find(value => value.key_id === document.claims.key_id)
  expect(publicKey).toBeTruthy(); expect(JSON.stringify(publicProfiles)).not.toContain('private_key')
  const canonical = value => Array.isArray(value) ? value.map(canonical) : value && typeof value === 'object' ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value
  expect(verify(null, Buffer.from(JSON.stringify(canonical(document.claims))), createPublicKey({ key: Buffer.from(publicKey.public_key_spki, 'base64url'), format: 'der', type: 'spki' }), Buffer.from(document.signature, 'base64url'))).toBeTruthy()
  const fulfilledOrder = await actor.get(`${origin}/api/operations/v1/commercial/orders/${order.snapshot.order_id}`)
  expect((await fulfilledOrder.json()).status).toBe('fulfilled')
  const closeToast = page.getByRole('button', { name: '关闭提示', exact: true })
  if (await closeToast.count()) {
    await closeToast.last().click()
    await expect(closeToast).toHaveCount(0)
  }

  await mkdir(new URL('../../dist/commercial-validation/screenshots/', import.meta.url), { recursive: true })
  await page.setViewportSize({ width: 1366, height: 648 })
  await dialog.evaluate(element => { element.scrollTop = 0 })
  expect(await dialog.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true)
  await page.screenshot({ path: 'dist/commercial-validation/screenshots/paid-fulfillment-desktop.png', animations: 'disabled' })
  await page.setViewportSize({ width: 390, height: 720 })
  await dialog.evaluate(element => { element.scrollTop = 0 })
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  expect(await dialog.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true)
  await page.screenshot({ path: 'dist/commercial-validation/screenshots/paid-fulfillment-mobile.png', animations: 'disabled' })
})


test('operations navigation and overview use the current commercial workflow', async ({ page }, testInfo) => {
  const legacyRequests = []
  page.on('request', request => { if (/\/api\/operations\/v1\/(plans|orders|licenses|trials|deliveries)(?:[/?]|$)/.test(request.url())) legacyRequests.push(request.url()) })
  await page.goto('/')
  await expect(page.getByText('已签发付费授权', { exact: true })).toBeVisible()
  await expect(page.locator('a[href="/orders"], a[href="/plans"], a[href="/deliveries"]')).toHaveCount(0)
  for (const [path, title] of [['/commercial/plans', '套餐与权益'], ['/commercial/orders', '订单与收款']]) {
    await page.goto(path)
    await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible()
    await expect(page.locator('a[href="/orders"], a[href="/plans"]')).toHaveCount(0)
  }
  await page.goto('/release-artifacts')
  await expect(page.getByRole('heading', { name: '安装包', exact: true })).toBeVisible()
  await page.getByRole('button', { name: '导入安装包', exact: true }).click()
  await expect(page.getByRole('heading', { name: '从受控 inbox 导入 Release', exact: true })).toBeVisible()
  await page.screenshot({ path: testInfo.outputPath('release-artifacts.png'), animations: 'disabled' })
  expect(legacyRequests).toEqual([])
})
