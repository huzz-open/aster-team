import { expect, request as playwrightRequest, test } from '@playwright/test'
import { readFile, writeFile } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import { dirname, join } from 'node:path'

const runtimePath = process.env.ASTER_SYSTEM_E2E_RUNTIME
if (!runtimePath) throw new Error('ASTER_SYSTEM_E2E_RUNTIME is required')
const runtime = JSON.parse(await readFile(runtimePath, 'utf8'))
const changedOperationsPassword = 'Operations-E2E-Changed-2026!'
const changedOwnerPassword = 'Owner-E2E-Changed-2026!'
const memberEmail = `member-e2e-${runtime.run_id}@example.test`
const initialMemberPassword = 'Member-E2E-Initial-2026!'
const memberPassword = 'Member-E2E-Changed-2026!'
const testPNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z7V8AAAAASUVORK5CYII=', 'base64')

function classifyLiveFailure(message) {
  if (/LIVE_BLOCKED|login|MFA|risk.?control|required capability|at least one live .* model/i.test(message)) return 'BLOCKED'
  if (/LIVE_INCONCLUSIVE|timeout|ECONN|ENOTFOUND|network|socket|browser has been closed|upstream.*5\d\d/i.test(message)) return 'INCONCLUSIVE'
  return 'FAILED'
}

test.afterEach(async ({}, testInfo) => {
  if (runtime.upstream_mode !== 'live' || testInfo.status === testInfo.expectedStatus) return
  const status = classifyLiveFailure(testInfo.error?.message || '')
  await writeFile(join(dirname(runtimePath), 'live-test-status.json'), `${JSON.stringify({ status })}\n`)
})

async function loginOperations(page) {
  await page.goto(`${runtime.operations.url}/login`)
  await page.getByLabel('操作员邮箱').fill(runtime.operations.email)
  await page.getByLabel('密码', { exact: true }).fill(runtime.operations.password)
  await page.getByRole('button', { name: '进入运营系统' }).click()
  try {
    await page.waitForURL(/\/(overview|change-password)$/, { timeout: 30_000 })
  } catch {
    await page.getByLabel('密码', { exact: true }).fill(changedOperationsPassword)
    await page.getByRole('button', { name: '进入运营系统' }).click()
    await page.waitForURL(/\/(overview|change-password)$/)
  }
  if (new URL(page.url()).pathname === '/change-password') {
    await page.getByLabel('当前密码').fill(runtime.operations.password)
    await page.getByLabel('新密码', { exact: true }).fill(changedOperationsPassword)
    await page.getByLabel('确认新密码').fill(changedOperationsPassword)
    await page.getByRole('button', { name: '修改密码' }).click()
    await page.waitForURL(/\/overview$/)
  }
}

async function operationsAPI(page, path, method = 'GET', body) {
  return page.evaluate(async ({ path, method, body }) => {
    const prefix = 'aster_operations_csrf='
    const csrf = document.cookie.split(';').map(value => value.trim()).find(value => value.startsWith(prefix))?.slice(prefix.length)
    const response = await fetch(`/api/operations/v1${path}`, {
      method,
      credentials: 'include',
      headers: {
        ...(body === undefined ? {} : { 'content-type': 'application/json' }),
        ...(csrf ? { 'x-csrf-token': decodeURIComponent(csrf) } : {}),
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    })
    const text = await response.text()
    const decoded = text ? JSON.parse(text) : null
    if (!response.ok) throw new Error(`${method} ${path} failed: ${response.status} ${text}`)
    return decoded
  }, { path, method, body })
}

async function createLicensePolicy(page) {
  const suffix = runtime.run_id
  const customer = await operationsAPI(page, '/customers', 'POST', {
    name: `E2E Customer ${suffix}`, legal_name: `E2E Customer ${suffix}`, status: 'active',
    contact_name: 'E2E Owner', contact_email: runtime.owner.email, contact_phone: '', contact_wechat: '', notes: 'System E2E',
  })
  const fixture = JSON.parse(await readFile(new URL('../../contracts/test-vectors/plan-definition.v1.json', import.meta.url), 'utf8'))
  const minimumFeatures = { json: ['member', 'runner'], 'qr-png': ['gateway'], 'terminal-screenshot': ['gateway', 'runner'] }
  const plans = new Map()
  for (const installation of runtime.customers) {
    const definition = structuredClone(fixture)
    definition.code = `e2e_${suffix}_${installation.id.replaceAll('-', '_')}`
    definition.name = `System E2E ${installation.id}`
    definition.minimum_version = '2.0.0'
    definition.entitlements.features = runtime.upstream_mode === 'fake' ? minimumFeatures[installation.id] : ['gateway', 'member', 'runner']
    const plan = await operationsAPI(page, '/commercial/plans/versions', 'POST', {
      operation_id: `plan-${suffix}-${installation.id}`, plan_id: '', expected_version: 0, definition,
    })
    plans.set(installation.id, plan)
  }
  return { customer, plans }
}

async function terminalScreenshot(browser, source, output) {
  const text = await readFile(source, 'utf8')
  const page = await browser.newPage()
  const dataURL = await page.evaluate(terminal => {
    const black = '\u001b[0;30;40m'
    const white = '\u001b[0;30;47m'
    const reset = '\u001b[0m'
    const start = terminal.indexOf(white)
    if (start < 0) throw new Error('terminal QR start sequence is missing')
    const input = terminal.slice(start)
    let colour = '#ffffff'
    let row = []
    const rows = []
    for (let index = 0; index < input.length;) {
      if (input.startsWith(black, index)) { colour = '#000000'; index += black.length; continue }
      if (input.startsWith(white, index)) { colour = '#ffffff'; index += white.length; continue }
      if (input.startsWith(reset, index)) { index += reset.length; continue }
      const character = input[index]
      index += 1
      if (character === '\r') continue
      if (character === '\n') {
        if (row.length) rows.push(row)
        row = []
        if (rows.length && !input.slice(index).includes(white)) break
        continue
      }
      if (character === ' ') row.push(colour)
    }
    const width = Math.max(...rows.map(value => value.length))
    // A terminal cell is roughly twice as tall as it is wide. Keep each
    // two-space QR module at a realistic 16×16 px instead of reducing the
    // captured image to a borderline 8×8 px per module.
    const cellWidth = 8
    const cellHeight = 16
    const canvas = document.createElement('canvas')
    canvas.width = width * cellWidth
    canvas.height = rows.length * cellHeight
    const context = canvas.getContext('2d')
    context.fillStyle = '#000000'
    context.fillRect(0, 0, canvas.width, canvas.height)
    rows.forEach((cells, y) => cells.forEach((cell, x) => {
      context.fillStyle = cell
      context.fillRect(x * cellWidth, y * cellHeight, cellWidth, cellHeight)
    }))
    return canvas.toDataURL('image/png')
  }, text)
  await page.close()
  await writeFile(output, Buffer.from(dataURL.slice(dataURL.indexOf(',') + 1), 'base64'))
}

async function routeCustomerOrigin(context, customer) {
  const ports = new Map([
    [customer.admin, 11082],
    [customer.member, 11081],
    [customer.api, 11080],
  ])
  for (const [hostPort, servicePort] of ports) {
    await context.route(`http://127.0.0.1:${hostPort}/**`, async route => {
      const response = await route.fetch({
        headers: { ...route.request().headers(), host: `${customer.access_host}:${servicePort}` },
      })
      await route.fulfill({ response })
    })
  }
}

async function loginCustomerAdmin(page, customer) {
  await page.goto(`${customer.admin_url}/login`)
  await page.getByLabel('管理员邮箱').fill(runtime.owner.email)
  await page.getByLabel('密码', { exact: true }).fill(runtime.owner.password)
  await page.getByRole('button', { name: '进入管理端' }).click()
  try {
    await page.waitForURL(/\/(overview|change-password)$/, { timeout: 5_000 })
  } catch {
    await page.getByLabel('密码', { exact: true }).fill(changedOwnerPassword)
    await page.getByRole('button', { name: '进入管理端' }).click()
    await page.waitForURL(/\/(overview|change-password)$/)
  }
  if (new URL(page.url()).pathname === '/change-password') {
    await page.getByLabel('当前临时密码').fill(runtime.owner.password)
    await page.getByLabel(/^新密码/).fill(changedOwnerPassword)
    await page.getByLabel('确认新密码', { exact: true }).fill(changedOwnerPassword)
    await page.getByRole('button', { name: '修改密码并重新登录' }).click()
    await page.waitForURL(/\/login/)
    await page.getByLabel('管理员邮箱').fill(runtime.owner.email)
    await page.getByLabel('密码', { exact: true }).fill(changedOwnerPassword)
    await page.getByRole('button', { name: '进入管理端' }).click()
    await page.waitForURL(/\/overview$/)
  }
}

async function issueAndInstall(browser, operationsPage, policy, customer, testInfo) {
  // Manually created contexts do not inherit Playwright's `use` TLS setting.
  // Only the isolated fake upstream has a short-lived test CA.
  const context = await browser.newContext({ ignoreHTTPSErrors: runtime.upstream_mode === 'fake' })
  await routeCustomerOrigin(context, customer)
  const page = await context.newPage()
  await loginCustomerAdmin(page, customer)
  const before = await page.evaluate(async () => (await fetch('/api/admin/license', { credentials: 'include' })).json())
  expect(before.state).toBe('active')
  expect(before.license.protocol_schema).toBe('aster.license.v2')
  expect(before.license.binding).toBe('unbound')
  expect(new Set(before.license.features)).toEqual(new Set(['gateway', 'member', 'runner']))
  // Prepare real existing resources while the bundled free license is active.
  // These remain usable after removing gateway or Runner from the paid grants.
  if (runtime.upstream_mode === 'fake' && ['json', 'qr-png'].includes(customer.id)) {
    await connectUpstream(page, context, customer)
  }
  const plan = policy.plans.get(customer.id)
  const order = await operationsAPI(operationsPage, '/commercial/orders', 'POST', {
    operation_id: `order-${runtime.run_id}-${customer.id}`, customer_id: policy.customer.id,
    plan_id: plan.snapshot.plan_id, plan_version: plan.snapshot.version, years: 1,
    starts_at: new Date(Date.now() - 10_000).toISOString(),
  })
  const orderID = order.snapshot.order_id
  const payment = await operationsAPI(operationsPage, `/commercial/orders/${orderID}/payment`, 'POST', {
    operation_id: `payment-${runtime.run_id}-${customer.id}`, expected_order_sha256: order.sha256,
    payment_reference: `TEST-ONLY-${runtime.run_id}-${customer.id}`, received_at: new Date().toISOString(),
    notes: 'Disposable system E2E payment record', current_password: changedOperationsPassword,
  })
  let requestFile = customer.request_json
  if (customer.id === 'qr-png') requestFile = customer.request_qr_png
  if (customer.id === 'terminal-screenshot') {
    requestFile = testInfo.outputPath('actual-terminal-request.png')
    await terminalScreenshot(browser, customer.terminal_output, requestFile)
  }
  await operationsPage.goto(`${runtime.operations.url}/workflows/business?order=${encodeURIComponent(orderID)}&step=4`)
  const workflow = operationsPage.locator('.workflow-main')
  await workflow.getByLabel('付费履约类型', { exact: true }).waitFor()
  await workflow.locator('input[type="file"]').setInputFiles(requestFile)
  const expectedRequest = JSON.parse(await readFile(customer.request_json, 'utf8'))
  expect(expectedRequest.schema).toBe('aster.license-request.v2')
  try {
    await expect(operationsPage).toHaveURL(/step=5/)
  } catch (error) {
    const alerts = await workflow.locator('[role="alert"]').allTextContents()
    const approvalVisible = await workflow.getByLabel('付费交付批准说明', { exact: true }).isVisible().catch(() => false)
    const pickerCount = await workflow.locator('input[type="file"]').count()
    console.error(`Request import did not advance for ${customer.id}: step=${new URL(operationsPage.url()).searchParams.get('step')}, approvalVisible=${approvalVisible}, pickerCount=${pickerCount}, alerts=${JSON.stringify(alerts.map(value => value.slice(0, 200)))}`)
    throw error
  }
  await workflow.getByLabel('付费交付批准说明', { exact: true }).fill('Verified test order, test payment and actual installation')
  await workflow.getByRole('checkbox').check()
  await workflow.getByRole('button', { name: '确认批准交付', exact: true }).click()
  const approvalDialog = operationsPage.getByRole('dialog', { name: '确认批准交付', exact: true })
  await approvalDialog.getByLabel('当前操作员密码').fill(changedOperationsPassword)
  const approvalResponse = operationsPage.waitForResponse(response => response.request().method() === 'POST' && new URL(response.url()).pathname.endsWith(`/orders/${orderID}/fulfillment`))
  await approvalDialog.getByRole('button', { name: '确认执行' }).click()
  const approvedHTTP = await approvalResponse
  expect(approvedHTTP.status()).toBe(200)
  const approved = await approvedHTTP.json()
  expect(approved.snapshot.installation_request).toEqual(expectedRequest)
  expect(approved.snapshot.payment.sha256).toBe(payment.sha256)
  await workflow.getByRole('combobox', { name: '付费签发密钥', exact: true }).click()
  await operationsPage.getByRole('option', { name: runtime.paid_signer_id, exact: true }).click()
  await workflow.getByRole('button', { name: '下一步：签发授权' }).click()
  await workflow.getByRole('button', { name: '签发付费授权', exact: true }).click()
  const issueDialog = operationsPage.getByRole('dialog', { name: '确认签发授权', exact: true })
  await issueDialog.getByLabel('当前操作员密码').fill(changedOperationsPassword)
  const issueResponse = operationsPage.waitForResponse(response => response.request().method() === 'POST' && new URL(response.url()).pathname.endsWith(`/paid-fulfillments/${approved.snapshot.id}/issue`))
  await issueDialog.getByRole('button', { name: '确认执行' }).click()
  const issuedHTTP = await issueResponse
  expect(issuedHTTP.status()).toBe(200)
  const issued = await issuedHTTP.json()
  const downloadPromise = operationsPage.waitForEvent('download')
  await workflow.getByRole('button', { name: '下载授权文件', exact: true }).click()
  const download = await downloadPromise
  const licensePath = testInfo.outputPath(`${customer.id}.license.json`)
  await download.saveAs(licensePath)
  const bytes = await readFile(licensePath)
  expect(createHash('sha256').update(bytes).digest('hex')).toBe(issued.document_sha256)
  const document = JSON.parse(bytes)
  expect(document).toEqual(issued.document)
  expect(document.claims.source.order_id).toBe(orderID)
  expect(document.claims.entitlements).toEqual(plan.snapshot.definition.entitlements)
  expect(document.claims.binding.installation_id).toBe(expectedRequest.installation_id)
  expect(document.claims.binding.machine_fingerprint_sha256).toBe(expectedRequest.machine_fingerprint_sha256)
  await page.goto(`${customer.admin_url}/license`)
  await page.locator('input[type="file"]').setInputFiles(licensePath)
  await page.getByRole('button', { name: '校验并对比', exact: true }).click()
  const licenseDialog = page.getByRole('dialog', { name: '确认切换授权', exact: true })
  await expect(licenseDialog).toBeVisible()
  const installationResponse = page.waitForResponse(response => response.request().method() === 'POST' && new URL(response.url()).pathname === '/api/admin/license')
  await licenseDialog.getByRole('button', { name: '立即切换', exact: true }).click()
  const installedHTTP = await installationResponse
  expect(installedHTTP.status()).toBe(200)
  expect((await installedHTTP.json()).activation).toBe('active')
  await expect.poll(async () => page.evaluate(async () => (await fetch('/api/admin/license', { credentials: 'include' })).json()))
    .toMatchObject({ state: 'active', license: {
      protocol_schema: 'aster.license.v2', license_id: document.claims.license_id, key_id: document.claims.key_id,
      binding: 'installation', plan_id: document.claims.plan_id, features: document.claims.entitlements.features,
      quotas: Object.fromEntries(document.claims.entitlements.quotas.map(quota => [quota.id, quota.limit.mode === 'unlimited' ? null : quota.limit.value ?? 0])),
      not_before: document.claims.validity.not_before, expires_at: document.claims.validity.expiry.expires_at,
    } })
  return { context, page, document }
}

async function connectUpstream(adminPage, adminContext, customer = runtime.customers[0]) {
  let redirectedToFakeUpstream = false
  if (runtime.upstream_mode === 'fake') {
    await adminContext.route('https://auth.openai.com/oauth/authorize**', route => {
      redirectedToFakeUpstream = true
      const source = new URL(route.request().url())
      const target = new URL('/oauth/authorize', runtime.upstream.url)
      target.search = source.search
      return route.fulfill({ status: 302, headers: { location: target.toString() } })
    })
  }
  await adminPage.goto(`${customer.admin_url}/upstream-accounts`)
  const popupPromise = adminPage.waitForEvent('popup')
  await adminPage.getByRole('button', { name: '新增 ChatGPT 账号' }).click()
  const popup = await popupPromise
  const callbackRequestPromise = popup.waitForRequest(
    request => /^http:\/\/localhost:1455\/auth\/callback\?/.test(request.url()),
    { timeout: 15 * 60_000 },
  )
  if (runtime.upstream_mode === 'fake') {
    try {
      await popup.getByLabel('Email').fill(runtime.upstream.email)
    } catch (error) {
      const current = new URL(popup.url())
      const body = await popup.locator('body').innerText({ timeout: 1000 }).catch(() => '')
      let contractError = ''
      try {
        const parsed = JSON.parse(body)
        if (parsed.error?.code === 'strict_mock_contract_violation') contractError = parsed.error.message
      } catch {
        // The page may be a browser error or a normal HTML document.
      }
      console.error(`Fake upstream login unavailable: redirected=${redirectedToFakeUpstream}, page=${current.origin}${current.pathname}, title=${JSON.stringify(await popup.title().catch(() => ''))}, contractError=${JSON.stringify(contractError)}`)
      throw error
    }
    await popup.getByLabel('Password').fill(runtime.upstream.password)
    await popup.getByRole('button', { name: 'Sign in' }).click({ noWaitAfter: true })
  } else {
    console.log('Complete the upstream account login in the opened browser window; the fixed test will resume automatically.')
  }
  let callback
  try {
    callback = (await callbackRequestPromise).url()
  } catch (error) {
    if (runtime.upstream_mode === 'live') {
      throw new Error('LIVE_BLOCKED: upstream login, MFA, or risk-control did not complete before the acceptance deadline', { cause: error })
    }
    throw error
  }
  await popup.close().catch(() => {})
  await adminPage.getByLabel('完整回调地址').fill(callback)
  const completionResponse = adminPage.waitForResponse(response => (
    response.request().method() === 'POST'
      && /\/api\/admin\/upstream-enrollments\/[^/]+\/actions$/.test(new URL(response.url()).pathname)
  ))
  const syncResponse = adminPage.waitForResponse(response => response.request().method() === 'POST' && /\/upstream-accounts\/[^/]+\/models\/sync$/.test(new URL(response.url()).pathname))
  await adminPage.getByRole('button', { name: '完成接入' }).click()
  const completion = await completionResponse
  expect(completion.status(), `OAuth completion failed: ${await completion.text()}`).toBe(201)
  const synced = await syncResponse
  expect(synced.status()).toBe(200)
  expect((await synced.json()).synced).toBeGreaterThan(0)
  await expect.poll(async () => adminPage.evaluate(async () => {
    const response = await fetch('/api/admin/upstream-accounts', { credentials: 'include' })
    if (!response.ok) return 0
    return (await response.json()).items.length
  }), { timeout: 60_000 }).toBeGreaterThan(0)
}

async function adminAPI(page, path, method = 'GET', body) {
  return page.evaluate(async ({ path, method, body }) => {
    const response = await fetch(path, { method, credentials: 'include',
      headers: body === undefined ? {} : { 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body) })
    return { status: response.status, number: response.headers.get('x-aster-error-number'), body: await response.json() }
  }, { path, method, body })
}

async function assertMinimalMaintenance(sessions) {
  if (runtime.upstream_mode !== 'fake') return
  const gateway = sessions[1].page
  const listed = await adminAPI(gateway, '/api/admin/upstream-accounts')
  expect(listed.status).toBe(200)
  expect(listed.body.items).toHaveLength(1)
  const account = listed.body.items[0]
  const models = await adminAPI(gateway, '/api/admin/models')
  expect(models.status).toBe(200)
  expect(models.body.items.length).toBeGreaterThan(0)
  const model = models.body.items[0]
  for (const enabled of [false, true]) {
    await gateway.goto(`${runtime.customers[1].admin_url}/upstream-accounts`)
    const accountRow = gateway.getByRole('row').filter({ has: gateway.getByText(account.email, { exact: true }) })
    await expect(accountRow.getByRole('button', { name: '通过任一健康 Runner 测试并同步模型', exact: true })).toBeDisabled()
    const changedAccount = customerResponse(gateway, `/api/admin/upstream-accounts/${account.id}`, 'PATCH')
    await accountRow.getByRole('button', { name: enabled ? '启用订阅/账号' : '停用订阅/账号', exact: true }).click()
    expect((await changedAccount).status()).toBe(200)
    await expect(accountRow.getByText(enabled ? '可路由' : '已停用', { exact: true })).toBeVisible()
    expect((await adminAPI(gateway, '/api/admin/upstream-accounts')).body.items[0].status).toBe(enabled ? 'active' : 'disabled')
    await gateway.goto(`${runtime.customers[1].admin_url}/models`)
    const modelRow = gateway.getByRole('row').filter({ has: gateway.getByText(model.public_name, { exact: true }) })
    const changedModel = customerResponse(gateway, `/api/admin/models/${model.id}`, 'PATCH')
    await modelRow.getByRole('button', { name: enabled ? '向成员开放' : '停止向成员开放', exact: true }).click()
    expect((await changedModel).status()).toBe(200)
    await expect(modelRow.getByText(enabled ? '已开放' : '已关闭', { exact: true })).toBeVisible()
    expect((await adminAPI(gateway, '/api/admin/models')).body.items.find(item => item.id === model.id).enabled).toBe(enabled)
  }
  const credentialsPath = `/api/admin/upstream-accounts/${account.id}/credentials`
  const original = await adminAPI(gateway, credentialsPath)
  expect(original.status).toBe(200)
  expect(original.body.items).toHaveLength(1)
  for (const path of [`/api/admin/upstream-accounts/${account.id}/models/sync`, `${credentialsPath}/${original.body.items[0].id}/refresh`]) {
    const rejected = await adminAPI(gateway, path, 'POST')
    expect(rejected.status).toBe(403)
    expect(rejected.number).toBe('51008')
  }
  expect((await adminAPI(gateway, credentialsPath)).body).toEqual(original.body)
  await gateway.goto(`${runtime.customers[1].admin_url}/upstream-accounts`)
  await gateway.getByRole('row').filter({ has: gateway.getByText(account.email, { exact: true }) })
    .getByRole('button', { name: '删除订阅/账号', exact: true }).click()
  const removed = customerResponse(gateway, `/api/admin/upstream-accounts/${account.id}`, 'DELETE')
  await gateway.getByRole('dialog').getByRole('button', { name: '确认删除', exact: true }).click()
  expect((await removed).status()).toBe(200)
  await expect(gateway.getByRole('dialog')).toBeHidden()
  await expect(gateway.getByRole('row').filter({ has: gateway.getByText(account.email, { exact: true }) })).toHaveCount(0)
  expect((await adminAPI(gateway, '/api/admin/upstream-accounts')).body.items).toEqual([])

  const maintenance = sessions[2]
  await connectUpstream(maintenance.page, maintenance.context, runtime.customers[2])
  const connected = await adminAPI(maintenance.page, '/api/admin/upstream-accounts')
  expect(connected.status).toBe(200)
  expect(connected.body.items).toHaveLength(1)
  const path = `/api/admin/upstream-accounts/${connected.body.items[0].id}/credentials`
  const before = await adminAPI(maintenance.page, path)
  expect(before.status).toBe(200)
  expect(before.body.items).toHaveLength(1)
  const credential = before.body.items[0]
  const refreshed = await adminAPI(maintenance.page, `${path}/${credential.id}/refresh`, 'POST')
  expect(refreshed.status).toBe(200)
  expect((await adminAPI(maintenance.page, path)).body.items[0].credential_revision).toBe(credential.credential_revision + 1)
  const memberDenied = await adminAPI(maintenance.page, '/api/admin/users')
  expect(memberDenied.status).toBe(403)
  expect(memberDenied.number).toBe('51008')
  const gatewayDenied = await adminAPI(sessions[0].page, '/api/admin/upstream-accounts')
  expect(gatewayDenied.status).toBe(403)
  expect(gatewayDenied.number).toBe('51008')
}

function customerResponse(page, path, method = 'GET', matchesQuery = () => true) {
  return page.waitForResponse(response => {
    const url = new URL(response.url())
    return url.pathname === path && response.request().method() === method && matchesQuery(url.searchParams)
  })
}

async function assertInstalledMaintenance(page, customer, testInfo) {
  const loaded = customerResponse(page, '/api/admin/maintenance')
  await page.goto(`${customer.admin_url}/maintenance`)
  const response = await loaded
  expect(response.status()).toBe(200)
  expect(await response.json()).toMatchObject({
    current_version: runtime.candidate.version,
    versions: [{ version: runtime.candidate.version, current: true }],
    jobs: [],
    platform: 'linux',
    busy: false,
    upgrade_capabilities: { supported_modes: ['maintenance'] },
  })
  await expect(page.locator('.current-version')).toHaveText(`当前版本 · ${runtime.candidate.version}`)
  await expect(page.locator('.version-row')).toHaveCount(1)
  await expect(page.locator('.version-row strong')).toHaveText(runtime.candidate.version)
  await expect(page.locator('.version-row').getByText('当前运行', { exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: '删除版本', exact: true })).toHaveCount(0)
  await expect(page.getByRole('button', { name: '校验并维护升级', exact: true })).toBeDisabled()
  await expect(page.getByText('还没有升级或清理记录。', { exact: true })).toBeVisible()
  await page.screenshot({ path: testInfo.outputPath(`maintenance-${customer.id}.png`), fullPage: true })
}

async function createMember(adminPage) {
  await adminPage.goto(`${runtime.customers[0].admin_url}/users`)
  await adminPage.getByRole('button', { name: '新增成员' }).click()
  await adminPage.getByLabel('显示名称').fill('System E2E Member')
  await adminPage.getByLabel('邮箱', { exact: true }).fill(memberEmail)
  await adminPage.getByLabel('初始密码').fill(initialMemberPassword)
  await adminPage.getByRole('button', { name: '创建成员' }).click()
  await expect(adminPage.getByText(memberEmail, { exact: true })).toBeVisible()
}

async function loginMember(page) {
  const customer = runtime.customers[0]
  await page.goto(`${customer.member_url}/login`)
  await page.getByLabel('成员邮箱').fill(memberEmail)
  await page.getByLabel('密码', { exact: true }).fill(initialMemberPassword)
  await page.getByRole('button', { name: '进入用户端' }).click()
  await page.waitForURL(/\/(home|change-password)$/)
  if (new URL(page.url()).pathname === '/change-password') {
    await page.getByLabel('当前初始密码').fill(initialMemberPassword)
    await page.getByLabel(/^新密码/).fill(memberPassword)
    await page.getByLabel('确认新密码', { exact: true }).fill(memberPassword)
    await page.getByRole('button', { name: '修改密码并重新登录' }).click()
    await page.waitForURL(/\/login/)
    await page.getByLabel('成员邮箱').fill(memberEmail)
    await page.getByLabel('密码', { exact: true }).fill(memberPassword)
    await page.getByRole('button', { name: '进入用户端' }).click()
    await page.waitForURL(/\/home$/)
  }
}

async function assertMemberClaudeSettings(page) {
  const supported = await page.evaluate(async () => {
    const response = await fetch('/api/member/claude-cli/settings?version=2.1.255%20(Claude%20Code)')
    return { status: response.status, body: await response.json() }
  })
  expect(supported.status).toBe(200)
  expect(supported.body.supported).toBe(true)
  expect(supported.body.minimum_version).toBe('2.1.255')
  const settings = JSON.parse(supported.body.settings)
  expect(settings.model).toBe('opus')
  expect(settings.env.ANTHROPIC_DEFAULT_FABLE_MODEL).toBe('claude-fable-5-1')
  expect(settings.env.ANTHROPIC_DEFAULT_OPUS_MODEL).toBe('claude-opus-5')
  expect(settings.env.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe('claude-sonnet-5')
  expect(settings.env.ANTHROPIC_DEFAULT_HAIKU_MODEL).toBe('claude-haiku-4-5-20251001')
  expect(Object.keys(settings.modelOverrides).sort()).toEqual([
    'claude-fable-5-1',
    'claude-haiku-4-5-20251001',
    'claude-opus-5',
    'claude-sonnet-5',
  ])
  expect(Object.values(settings.modelOverrides).every(value => typeof value === 'string' && value.length > 0)).toBe(true)

  const unsupported = await page.evaluate(async () => {
    const response = await fetch('/api/member/claude-cli/settings?version=2.1.254')
    return { status: response.status, body: await response.json() }
  })
  expect(unsupported.status).toBe(200)
  expect(unsupported.body.supported).toBe(false)
  expect(unsupported.body.settings).toBeNull()
}

async function requestAndApproveQuota(memberPage, adminPage) {
  await memberPage.goto(`${runtime.customers[0].member_url}/quota`)
  await memberPage.getByRole('button', { name: '申请额度' }).click()
  await memberPage.locator('input[type="number"]').fill('1000000')
  await memberPage.getByLabel('申请原因').fill('System E2E protocol matrix execution')
  await memberPage.getByRole('button', { name: '提交申请' }).click()
  await expect(memberPage.getByText('额度申请已提交，请等待管理员审批。')).toBeVisible()

  await adminPage.goto(`${runtime.customers[0].admin_url}/quota-requests`)
  await expect(adminPage.getByText(memberEmail, { exact: true })).toBeVisible()
  await adminPage.getByRole('button', { name: '通过' }).click()
  await adminPage.getByLabel('审批说明（可选）').fill('Approved by deterministic system E2E')
  await adminPage.getByRole('button', { name: '确认通过并发放' }).click()
  await expect(adminPage.getByText('申请已通过，额度已写入成员账本。', { exact: true })).toBeVisible()
  await expect(adminPage.getByText(memberEmail, { exact: true })).toHaveCount(0)
}

async function createAPIKey(memberPage) {
  await memberPage.goto(`${runtime.customers[0].member_url}/keys`)
  await memberPage.getByRole('button', { name: '创建新 Key' }).click()
  await memberPage.getByLabel('用途名称').fill('System E2E')
  await memberPage.getByRole('button', { name: '创建 Key' }).click()
  const value = await memberPage.locator('.notice.code').textContent()
  expect(value).toMatch(/^ask_/)
  return value.trim()
}

async function assertJSON(response, label) {
  const body = await response.text()
  expect(response.status(), `${label}: ${body}`).toBe(200)
  return JSON.parse(body)
}

async function installMockCase(mockRequest, caseID, scenario) {
  const response = await mockRequest.post(`${runtime.upstream.url}/__e2e/cases/${caseID}`, {
    headers: { authorization: `Bearer ${runtime.upstream.control_token}`, 'content-type': 'application/json' },
    data: scenario,
  })
  expect(response.status(), `install mock case ${caseID}: ${await response.text()}`).toBe(201)
}

async function runStrictFailureMatrix(request, apiURL, headers) {
  if (runtime.upstream_mode !== 'fake') return []
  const mockRequest = await playwrightRequest.newContext({ ignoreHTTPSErrors: true })
  const readRequests = async () => {
    const response = await mockRequest.get(`${runtime.upstream.url}/__e2e/requests`, {
      headers: { authorization: `Bearer ${runtime.upstream.control_token}` },
    })
    expect(response.status()).toBe(200)
    return (await response.json()).items
  }
  const cases = [
    ['refresh401', { mode: 'status', status: 401 }, 502, 35002],
    ['rate429', { mode: 'status', status: 429 }, 502, 35002],
    ['server503', { mode: 'status', status: 503 }, 502, 35002],
    ['badjson', { mode: 'invalid_json' }, 502, 35004],
    ['badsse', { mode: 'invalid_sse' }, 502, 35004],
    ['interrupted', { mode: 'interrupt' }, 409, 33004],
    ['timeout', { mode: 'timeout', delay_ms: 1500 }, 409, 33004],
    ['request400', { mode: 'status', status: 400 }, 502, 35002],
    ['missing404', { mode: 'status', status: 404 }, 502, 35002],
    ['conflict409', { mode: 'status', status: 409 }, 502, 35002],
    ['forbidden403', { mode: 'status', status: 403 }, 502, 35002],
  ]
  const tokenRequestsBefore = (await readRequests()).filter(item => item.path === '/oauth/token').length
  const failures = []
  for (const [caseID, scenario, status, number] of cases) {
    await installMockCase(mockRequest, caseID, scenario)
    const response = await request.post(`${apiURL}/v1/responses`, {
      headers,
      data: { model: 'gpt-5.6-e2e', input: `__aster_e2e_case:${caseID}`, stream: false },
      timeout: 30_000,
    })
    expect(response.status(), `${caseID} must remain a visible failure`).toBe(status)
    const body = await response.json()
    expect(body.error.number, caseID).toBe(number)
    const requestID = response.headers()['x-aster-request-id']
    expect(requestID, caseID).toBeTruthy()
    expect(body.request_id, caseID).toBe(requestID)
    failures.push({ caseID, requestID })
  }
  const requests = await readRequests()
  for (const [caseID] of cases) {
    const attempts = requests.filter(item => item.case_id === caseID)
    expect(attempts, caseID).toHaveLength(1)
    expect(attempts[0].client_request_id, caseID).toBeTruthy()
  }
  expect(requests.filter(item => item.path === '/oauth/token')).toHaveLength(tokenRequestsBefore)
  expect(requests.every(item => !('authorization' in item))).toBeTruthy()
  await mockRequest.dispose()
  return failures
}

async function runExecutionOptionsMatrix(request, apiURL, key, baseModel) {
  if (runtime.upstream_mode !== 'fake') return
  const openAIHeaders = { authorization: `Bearer ${key}`, 'content-type': 'application/json' }
  const anthropicHeaders = { 'content-type': 'application/json', 'anthropic-version': '2023-06-01', 'x-api-key': key }
  const successCases = [
    {
      id: 'execution-responses-variant',
      label: 'Responses model variant',
      path: '/v1/responses',
      headers: openAIHeaders,
      data: { model: `${baseModel}-fast-high`, input: '__aster_e2e_case:execution-responses-variant', stream: false },
      expected: { responseModel: `${baseModel}-fast-high`, serviceTier: 'fast', reasoningEffort: 'high' },
    },
    {
      id: 'execution-chat-variant',
      label: 'Chat reasoning-only model variant',
      path: '/v1/chat/completions',
      headers: openAIHeaders,
      data: { model: `${baseModel}-high`, messages: [{ role: 'user', content: '__aster_e2e_case:execution-chat-variant' }], stream: false },
      expected: { responseModel: `${baseModel}-high`, serviceTier: undefined, reasoningEffort: 'high' },
    },
    {
      id: 'execution-anthropic-variant',
      label: 'Anthropic speed-only model variant',
      path: '/v1/messages',
      headers: anthropicHeaders,
      data: { model: `${baseModel}-fast`, max_tokens: 128, messages: [{ role: 'user', content: '__aster_e2e_case:execution-anthropic-variant' }], stream: false },
      expected: { responseModel: `${baseModel}-fast`, serviceTier: 'fast', reasoningEffort: undefined },
    },
    {
      id: 'execution-responses-explicit',
      label: 'Responses native execution fields',
      path: '/v1/responses',
      headers: openAIHeaders,
      data: { model: baseModel, input: '__aster_e2e_case:execution-responses-explicit', service_tier: 'priority', reasoning: { effort: 'medium' }, stream: false },
      expected: { responseModel: baseModel, serviceTier: 'fast', reasoningEffort: 'medium' },
    },
    {
      id: 'execution-chat-explicit',
      label: 'Chat native execution fields',
      path: '/v1/chat/completions',
      headers: openAIHeaders,
      data: { model: baseModel, messages: [{ role: 'user', content: '__aster_e2e_case:execution-chat-explicit' }], service_tier: 'fast', reasoning_effort: 'medium', stream: false },
      expected: { responseModel: baseModel, serviceTier: 'fast', reasoningEffort: 'medium' },
    },
    {
      id: 'execution-anthropic-explicit',
      label: 'Anthropic native execution fields',
      path: '/v1/messages',
      headers: anthropicHeaders,
      data: { model: baseModel, max_tokens: 128, messages: [{ role: 'user', content: '__aster_e2e_case:execution-anthropic-explicit' }], speed: 'fast', output_config: { effort: 'medium' }, stream: false },
      expected: { responseModel: baseModel, serviceTier: 'fast', reasoningEffort: 'medium' },
    },
  ]
  for (const item of successCases) {
    const decoded = await assertJSON(await request.post(`${apiURL}${item.path}`, {
      headers: item.headers,
      data: item.data,
    }), item.label)
    expect(decoded.model, `${item.label} must preserve the caller-visible model name`).toBe(item.expected.responseModel)
    expect(JSON.stringify(decoded), `${item.label} returned no content`).toContain('Aster E2E response')
  }

  const conflictCases = [
    {
      id: 'execution-responses-conflict',
      label: 'Responses conflicting execution fields',
      path: '/v1/responses',
      headers: openAIHeaders,
      data: { model: `${baseModel}-fast-high`, input: '__aster_e2e_case:execution-responses-conflict', reasoning: { effort: 'low' }, stream: false },
    },
    {
      id: 'execution-chat-conflict',
      label: 'Chat conflicting execution fields',
      path: '/v1/chat/completions',
      headers: openAIHeaders,
      data: { model: `${baseModel}-fast-high`, messages: [{ role: 'user', content: '__aster_e2e_case:execution-chat-conflict' }], reasoning_effort: 'low', stream: false },
    },
    {
      id: 'execution-anthropic-conflict',
      label: 'Anthropic conflicting execution fields',
      path: '/v1/messages',
      headers: anthropicHeaders,
      data: { model: `${baseModel}-fast-high`, max_tokens: 128, messages: [{ role: 'user', content: '__aster_e2e_case:execution-anthropic-conflict' }], output_config: { effort: 'low' }, stream: false },
    },
  ]
  for (const item of conflictCases) {
    const response = await request.post(`${apiURL}${item.path}`, { headers: item.headers, data: item.data })
    const body = await response.text()
    expect(response.status(), `${item.label}: ${body}`).toBe(400)
    expect(JSON.parse(body).error?.number, `${item.label} must use the gateway invalid-request error`).toBe(32001)
  }

  const mockRequest = await playwrightRequest.newContext({ ignoreHTTPSErrors: true })
  const logResponse = await mockRequest.get(`${runtime.upstream.url}/__e2e/requests`, {
    headers: { authorization: `Bearer ${runtime.upstream.control_token}` },
  })
  expect(logResponse.status()).toBe(200)
  const log = await logResponse.json()
  for (const item of successCases) {
    const recorded = log.items.find(entry => entry.case_id === item.id)
    expect(recorded, `${item.label} did not reach the upstream`).toBeTruthy()
    expect(recorded.model, `${item.label} did not route through the base model`).toBe(baseModel)
    // The public canonical tier is `fast`; the Codex upstream transport
    // requires the same tier to be spelled `priority`.
    const upstreamTier = item.expected.serviceTier === 'fast' ? 'priority' : item.expected.serviceTier
    expect(recorded.service_tier, `${item.label} processing tier was not mapped for the upstream`).toBe(upstreamTier)
    expect(recorded.reasoning_effort, `${item.label} reasoning effort was not normalized`).toBe(item.expected.reasoningEffort)
  }
  for (const item of conflictCases) {
    expect(log.items.some(entry => entry.case_id === item.id), `${item.label} must be rejected before the upstream`).toBeFalsy()
  }
  await mockRequest.dispose()
}

async function runProtocolMatrix(request, apiURL, key) {
  const headers = { authorization: `Bearer ${key}`, 'content-type': 'application/json' }
  for (const [label, requestHeaders, data] of [
    ['missing API key', { 'content-type': 'application/json' }, { model: 'gpt-5.6-e2e', input: 'unauthorized' }],
    ['invalid API key', { authorization: 'Bearer ask_invalid_e2e_key', 'content-type': 'application/json' }, { model: 'gpt-5.6-e2e', input: 'unauthorized' }],
    ['invalid Responses body', headers, { model: 'gpt-5.6-e2e' }],
  ]) {
    const response = await request.post(`${apiURL}/v1/responses`, { headers: requestHeaders, data })
    expect(response.status(), label).toBeGreaterThanOrEqual(400)
  }
  const modelList = await assertJSON(await request.get(`${apiURL}/v1/models`, { headers }), 'OpenAI model list')
  const textModel = runtime.upstream_mode === 'fake' ? 'gpt-5.6-e2e' : modelList.data[0]?.id
  expect(textModel, 'at least one live text model is required').toBeTruthy()
  expect(modelList.data.some(item => item.id === textModel)).toBeTruthy()
  const anthropicModels = await assertJSON(await request.get(`${apiURL}/v1/models`, { headers: { ...headers, 'anthropic-version': '2023-06-01' } }), 'Anthropic model list')
  expect(anthropicModels.data.length).toBeGreaterThan(0)
  if (runtime.upstream_mode === 'fake') {
    const claudeSettings = await assertJSON(await request.get(`${apiURL}/v1/claude-cli/settings?version=2.1.255`, {
      headers: { 'x-api-key': key },
    }), 'Claude CLI automatic model settings')
    expect(claudeSettings.minimum_version).toBe('2.1.255')
    expect(Object.values(claudeSettings.model_mappings)).toEqual([textModel, textModel, textModel, textModel])

    const unsupportedClaudeSettings = await request.get(`${apiURL}/v1/claude-cli/settings?version=2.1.254`, {
      headers: { 'x-api-key': key },
    })
    expect(unsupportedClaudeSettings.status(), 'unsupported Claude Code version').toBe(400)
  }

  const cases = [
    ['Responses string', '/v1/responses', { model: textModel, input: '你好', stream: false }],
    ['Responses message array', '/v1/responses', { model: textModel, input: [{ role: 'user', content: [{ type: 'input_text', text: 'hello' }] }], stream: false }],
    ['Chat Completions', '/v1/chat/completions', { model: textModel, messages: [{ role: 'user', content: 'hello' }], stream: false }],
  ]
  for (const [label, path, data] of cases) {
    const decoded = await assertJSON(await request.post(`${apiURL}${path}`, { headers, data }), label)
    if (runtime.upstream_mode === 'fake') expect(JSON.stringify(decoded)).toContain('Aster E2E response')
    else expect(decoded.output?.length, `${label} returned no output`).toBeGreaterThan(0)
  }
  const anthropic = await assertJSON(await request.post(`${apiURL}/v1/messages`, {
    headers: { ...headers, 'anthropic-version': '2023-06-01', 'x-api-key': key },
    data: { model: textModel, max_tokens: 128, messages: [{ role: 'user', content: 'hello' }], stream: false },
  }), 'Anthropic Messages')
  if (runtime.upstream_mode === 'fake') expect(JSON.stringify(anthropic)).toContain('Aster E2E response')
  else expect(anthropic.content?.length, 'Anthropic Messages returned no content').toBeGreaterThan(0)

  for (const [label, path, data, extraHeaders = {}] of [
    ['Responses stream', '/v1/responses', { model: textModel, input: 'stream', stream: true }],
    ['Chat stream', '/v1/chat/completions', { model: textModel, messages: [{ role: 'user', content: 'stream' }], stream: true }],
    ['Anthropic stream', '/v1/messages', { model: textModel, max_tokens: 128, messages: [{ role: 'user', content: 'stream' }], stream: true }, { 'anthropic-version': '2023-06-01', 'x-api-key': key }],
  ]) {
    const response = await request.post(`${apiURL}${path}`, { headers: { ...headers, ...extraHeaders }, data })
    expect(response.status(), label).toBe(200)
    expect((await response.text()).length, label).toBeGreaterThan(20)
  }

  const outputFormats = runtime.upstream_mode === 'fake' ? ['png', 'jpeg', 'webp'] : ['png']
  for (const outputFormat of outputFormats) {
    const generated = await assertJSON(await request.post(`${apiURL}/v1/images/generations`, {
      headers,
      data: { model: 'gpt-image-2.5-flare', prompt: `A test ${outputFormat} image`, size: '1024x1024', response_format: 'b64_json', output_format: outputFormat },
    }), `Image generation ${outputFormat}`)
    expect(Buffer.from(generated.data[0].b64_json, 'base64').length).toBeGreaterThan(10)
  }
  if (runtime.upstream_mode === 'fake') {
    const generatedMany = await assertJSON(await request.post(`${apiURL}/v1/images/generations`, {
      headers,
      data: { model: 'gpt-image-2.5-flare', prompt: 'A test image batch', size: '1024x1024', response_format: 'b64_json', n: 3 },
    }), 'Image generation n > 1')
    expect(generatedMany.data).toHaveLength(3)
  }
  const editForm = new FormData()
  editForm.append('model', 'gpt-image-2.5-sunburst')
  editForm.append('prompt', runtime.upstream_mode === 'fake' ? 'Edit the test images with a mask' : 'Edit the test image')
  editForm.append('output_format', 'png')
  editForm.append('image[]', new Blob([testPNG], { type: 'image/png' }), 'input-1.png')
  if (runtime.upstream_mode === 'fake') {
    editForm.append('image[]', new Blob([testPNG], { type: 'image/png' }), 'input-2.png')
    editForm.append('mask', new Blob([testPNG], { type: 'image/png' }), 'mask.png')
  }
  const edited = await assertJSON(await request.post(`${apiURL}/v1/images/edits`, {
    headers: { authorization: `Bearer ${key}` },
    multipart: editForm,
  }), 'Multipart image edit with mask and multiple image[] fields')
  expect(Buffer.from(edited.data[0].b64_json, 'base64').length).toBeGreaterThan(10)
  if (runtime.upstream_mode === 'fake') {
    const dataImage = `data:image/png;base64,${testPNG.toString('base64')}`
    const editedMany = await assertJSON(await request.post(`${apiURL}/v1/images/edits`, {
      headers,
      data: { model: 'gpt-image-2.5-sunburst', prompt: 'Edit multiple test images', output_format: 'webp', images: [{ image_url: dataImage }, { image_url: dataImage }] },
    }), 'Image edit with multiple image inputs')
    expect(Buffer.from(editedMany.data[0].b64_json, 'base64').length).toBeGreaterThan(10)
  }

  await runExecutionOptionsMatrix(request, apiURL, key, textModel)
  const failures = await runStrictFailureMatrix(request, apiURL, headers)
  return { textModel, failures }
}

async function assertAccountingOracle(adminPage, testedModel, failures) {
  const result = await adminPage.evaluate(async memberEmailValue => {
    const usageResponse = await fetch(`/api/admin/consumption-logs?limit=1000&offset=0&keyword=${encodeURIComponent(memberEmailValue)}`, { credentials: 'include' })
    const auditResponse = await fetch('/api/admin/audit-events?limit=1000&offset=0', { credentials: 'include' })
    return {
      usageStatus: usageResponse.status,
      usage: await usageResponse.json(),
      auditStatus: auditResponse.status,
      audit: await auditResponse.json(),
    }
  }, memberEmail)
  expect(result.usageStatus).toBe(200)
  if (runtime.upstream_mode === 'fake') {
    expect(result.usage.total).toBe(30)
    expect(result.usage.items).toHaveLength(30)
    const successful = result.usage.items.filter(item => item.kind === 'usage')
    const failed = result.usage.items.filter(item => item.kind === 'usage_failed')
    expect(successful).toHaveLength(19)
    expect(failed).toHaveLength(11)
    expect(successful.filter(item => item.raw_tokens === 17 && item.billed_tokens === 17)).toHaveLength(18)
    expect(successful.filter(item => item.raw_tokens === 51 && item.billed_tokens === 51)).toHaveLength(1)
    expect(failed.every(item => item.raw_tokens === 0 && item.billed_tokens === 0)).toBeTruthy()
    expect(result.usage.items.every(item => item.attempt_count === 1)).toBeTruthy()
    expect(result.usage.items.reduce((sum, item) => sum + item.raw_tokens, 0)).toBe(357)
    expect(result.usage.items.reduce((sum, item) => sum + item.billed_tokens, 0)).toBe(357)
    expect(result.usage.summary).toMatchObject({ request_count: 19, raw_tokens: 357, billed_tokens: 357 })
    expect(result.usage.items.every(item => item.requested_model)).toBeTruthy()
    const variantUsage = result.usage.items.find(item => item.requested_model === `${testedModel}-fast-high`)
    expect(variantUsage?.model).toBe(testedModel)
    expect(variantUsage?.processing_tier).toBe('fast')
    expect(variantUsage?.reasoning_effort).toBe('high')
    expect(result.usage.processing_tiers).toEqual(expect.arrayContaining(['fast', 'model_default']))
    expect(result.usage.reasoning_efforts).toEqual(expect.arrayContaining(['high', 'medium', 'model_default']))
  } else {
    expect(result.usage.total).toBeGreaterThanOrEqual(9)
    expect(result.usage.items.reduce((sum, item) => sum + item.raw_tokens, 0)).toBeGreaterThan(0)
  }
  expect(new Set(result.usage.items.map(item => item.protocol))).toEqual(new Set([
    'openai_responses', 'openai_chat', 'anthropic_messages', 'openai_images', 'openai_image_edits',
  ]))
  expect(result.auditStatus).toBe(200)
  if (runtime.upstream_mode === 'fake') {
    const unknown = result.audit.items.filter(item => item.action === 'gateway.request.unbilled_upstream_unknown' && item.outcome === 'failed')
    const unknownCases = new Set(['server503', 'badjson', 'badsse', 'interrupted', 'timeout'])
    const expectedIDs = failures.filter(item => unknownCases.has(item.caseID)).map(item => item.requestID)
    expect(unknown).toHaveLength(5)
    expect(new Set(unknown.map(item => item.target_id))).toEqual(new Set(expectedIDs))
  }
}

async function assertMemberUsageFilters(page, testedModel, testInfo) {
  if (runtime.upstream_mode !== 'fake') return
  const today = await page.evaluate(() => {
    const now = new Date()
    return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`
  })
  const initial = customerResponse(page, '/api/member/usage-summary', 'GET', query => query.get('to') === today)
  await page.goto(`${runtime.customers[0].member_url}/usage`)
  expect((await initial).status()).toBe(200)
  const keys = await adminAPI(page, '/api/member/keys')
  expect(keys.status).toBe(200)
  expect(keys.body.items).toHaveLength(1)
  const keyID = keys.body.items[0].id
  const selectedKey = customerResponse(page, '/api/member/usage-summary', 'GET', query => query.get('to') === today && query.get('api_key_id') === keyID)
  await page.getByRole('combobox', { name: '全部来源', exact: true }).click()
  await page.getByRole('option', { name: /^System E2E\s/ }).click()
  const keyResponse = await selectedKey
  expect(keyResponse.status()).toBe(200)
  expect((await keyResponse.json()).summary).toMatchObject({ request_count: 19, raw_tokens: 357, billed_tokens: 357 })

  const selectedPeriod = customerResponse(page, '/api/member/usage-summary', 'GET', query =>
    query.get('to') === today && query.get('api_key_id') === keyID
      && Date.parse(query.get('to')) - Date.parse(query.get('from')) === 29 * 86_400_000)
  await page.getByRole('radio', { name: '30 天', exact: true }).click()
  const periodResponse = await selectedPeriod
  expect(periodResponse.status()).toBe(200)
  const usage = await periodResponse.json()
  expect(usage.summary).toMatchObject({ request_count: 19, raw_tokens: 357, billed_tokens: 357 })
  await expect(page.locator('.metric-card--blue strong')).toHaveText('357')
  await expect(page.locator('.metric-card--violet strong')).toHaveText('19')
  const from = new URL(periodResponse.url()).searchParams.get('from')
  const chart = page.getByRole('list', { name: '用量趋势', exact: true })
  await expect(chart.getByRole('listitem')).toHaveCount(30)
  // Switch between two models backed by actual settled Runner requests, then inspect
  // the displayed day totals. Selecting an option alone would not prove chart filtering.
  const imageModel = usage.model_trend.find(item => item.model.startsWith('gpt-image-') && item.raw_tokens > 0)?.model
  expect(imageModel, 'the image requests must produce a distinct settled model trend').toBeTruthy()
  for (const model of [testedModel, imageModel]) {
    const point = usage.model_trend.find(item => item.model === model && item.raw_tokens > 0)
    expect(point, `missing actual settled trend for ${model}`).toBeTruthy()
    expect(point.raw_tokens).toBeLessThan(usage.summary.raw_tokens)
    await page.getByRole('combobox', { name: '模型', exact: true }).click()
    await page.getByRole('option', { name: model, exact: true }).click()
    await expect(page.getByRole('combobox', { name: '模型', exact: true })).toContainText(model)
    const dayIndex = (Date.parse(point.date) - Date.parse(from)) / 86_400_000
    await chart.getByRole('listitem').nth(dayIndex).hover()
    const tooltip = page.locator('.chart-tooltip')
    await expect(tooltip.locator('strong')).toHaveText(point.date)
    await expect(tooltip.locator('span').filter({ hasText: /^原始 Token/ }).locator('b')).toHaveText(String(point.raw_tokens))
    await expect(tooltip.locator('span').filter({ hasText: /^请求数/ }).locator('b')).toHaveText(String(point.request_count))
  }
  await page.getByRole('combobox', { name: '模型', exact: true }).click()
  await page.getByRole('option', { name: '全部模型', exact: true }).click()
  const day = usage.trend.find(item => item.raw_tokens > 0)
  await chart.getByRole('listitem').nth((Date.parse(day.date) - Date.parse(from)) / 86_400_000).hover()
  await expect(page.locator('.chart-tooltip span').filter({ hasText: /^原始 Token/ }).locator('b')).toHaveText(String(day.raw_tokens))
  await page.screenshot({ path: testInfo.outputPath('member-real-usage-filters.png'), fullPage: true })
}

test('repeatable production delivery, account, quota, and protocol workflow', async ({ browser, page }, testInfo) => {
  await loginOperations(page)
  const policy = await createLicensePolicy(page)
  const customerSessions = []
  for (const customer of runtime.customers) {
    customerSessions.push(await issueAndInstall(browser, page, policy, customer, testInfo))
  }

  const primary = customerSessions[0]
  await assertMinimalMaintenance(customerSessions)
  for (let index = 0; index < customerSessions.length; index += 1) {
    await assertInstalledMaintenance(customerSessions[index].page, runtime.customers[index], testInfo)
  }
  if (runtime.upstream_mode === 'live') await connectUpstream(primary.page, primary.context)
  await createMember(primary.page)
  const memberContext = await browser.newContext()
  await routeCustomerOrigin(memberContext, runtime.customers[0])
  const memberPage = await memberContext.newPage()
  await loginMember(memberPage)
  await assertMemberClaudeSettings(memberPage)
  await requestAndApproveQuota(memberPage, primary.page)
  const key = await createAPIKey(memberPage)
  const apiRequest = await playwrightRequest.newContext({
    extraHTTPHeaders: { host: `${runtime.customers[0].access_host}:11080` },
  })
  const { textModel: testedModel, failures } = await runProtocolMatrix(apiRequest, runtime.customers[0].api_url, key)
  await apiRequest.dispose()
  await assertAccountingOracle(primary.page, testedModel, failures)
  await assertMemberUsageFilters(memberPage, testedModel, testInfo)

  await memberPage.goto(`${runtime.customers[0].member_url}/logs`)
  await expect(memberPage.getByText(testedModel).first()).toBeVisible()
  await primary.page.goto(`${runtime.customers[0].admin_url}/consumption-logs`)
  await expect(primary.page.getByText(memberEmail, { exact: true }).first()).toBeVisible()

  await memberContext.close()
  for (const session of customerSessions) await session.context.close()
})
