import { expect } from '@playwright/test'
import { test, ownerLogin, memberLogin, createMember, createKey, captureFailures, importLicense } from './real-fixtures.mjs'

test('paid license import updates live capabilities and quotas while invalid imports preserve customer data', async ({ page, browser, backend }, testInfo) => {
  await ownerLogin(page, backend)
  const identity = await createMember(page, backend, 'upgrade@example.test', '升级成员')
  const memberContext = await browser.newContext({ locale: 'zh-CN' })
  try {
    const member = await memberContext.newPage()
    await memberLogin(member, backend, 'upgrade@example.test')
    await member.goto(`${backend.member_url}/keys`)
    const first = await createKey(member, 'preserved-key-one')
    const second = await createKey(member, 'preserved-key-two')
    const full = await member.request.post(`${backend.member_url}/api/member/keys`, { data: { name: 'before-upgrade' } })
    expect(full.headers()['x-aster-error-number']).toBe('22004')
    await page.goto(`${backend.admin_url}/runners`)
    await expect(page.getByRole('heading', { name: '当前授权未包含此功能', exact: true })).toBeVisible()
    await page.getByRole('button', { name: '查看授权状态', exact: true }).click()
    await expect(page).toHaveURL(/\/license$/)
    const original = await (await page.request.get(`${backend.admin_url}/api/admin/license`)).json()
    expect(original.license.binding).toBe('unbound')
    expect(original.license.features).toEqual(['member'])

    const invalid = await importLicense(page, backend.license_files.tampered)
    expect(invalid.ok()).toBe(false)
    expect(invalid.headers()['x-aster-error-number']).toBe('51003')
    const afterInvalid = await (await page.request.get(`${backend.admin_url}/api/admin/license`)).json()
    expect(afterInvalid.license).toEqual(original.license)

    const installed = await importLicense(page, backend.license_files.paid)
    expect(installed.status()).toBe(200)
    expect((await installed.json()).activation).toBe('active')
    const paid = await (await page.request.get(`${backend.admin_url}/api/admin/license`)).json()
    expect(paid.license.license_id).toBe(backend.paid_license_id)
    expect(paid.license.binding).toBe('installation')
    expect(paid.license.features).toContain('runner')
    // Keep the existing app instance and sessions: no reload or new login masks stale state.
    await page.getByRole('complementary').getByRole('link', { name: 'Runner 节点', exact: true }).click()
    await expect(page.getByRole('button', { name: '新增 Runner', exact: true })).toBeVisible()
    await expect(page.getByRole('heading', { name: '当前授权未包含此功能', exact: true })).toHaveCount(0)
    const third = await createKey(member, 'paid-key-three')
    const expectedIds = [first.api_key.id, second.api_key.id, third.api_key.id].sort()
    const assertPreserved = async () => {
      const members = await page.request.get(`${backend.admin_url}/api/admin/users`)
      expect(members.status()).toBe(200)
      expect((await members.json()).items.some(item => item.id === identity.id && item.email === 'upgrade@example.test')).toBe(true)
      const keys = await member.request.get(`${backend.member_url}/api/member/keys`)
      expect(keys.status()).toBe(200)
      const items = (await keys.json()).items
      expect(items.map(item => item.id).sort()).toEqual(expectedIds)
      expect(items.every(item => item.status === 'active')).toBe(true)
    }
    await assertPreserved()
    await page.getByRole('complementary').getByRole('link', { name: '产品授权', exact: true }).click()
    const rollback = await importLicense(page, backend.license_files.free)
    expect(rollback.ok()).toBe(false)
    expect(rollback.headers()['x-aster-error-number']).toBe('51007')
    const invalidAfterUpgrade = await importLicense(page, backend.license_files.tampered)
    expect(invalidAfterUpgrade.headers()['x-aster-error-number']).toBe('51003')
    const current = await (await page.request.get(`${backend.admin_url}/api/admin/license`)).json()
    expect(current.state).toBe('active')
    expect(current.license).toEqual(paid.license)
    await assertPreserved()
    await member.screenshot({ path: testInfo.outputPath('paid-upgrade-preserved-keys.png'), fullPage: true })
  } finally { await memberContext.close() }
})

test('member-only license supports real member creation and assigned voucher selectors', async ({ page, browser, backend }, testInfo) => {
  await ownerLogin(page, backend)
  const failures = captureFailures(page)
  await createMember(page, backend, 'member@example.test', '浏览器成员')
  await page.goto(`${backend.admin_url}/vouchers`)
  await page.getByRole('button', { name: '创建 / 批量投放', exact: true }).click()
  await expect(page.getByRole('checkbox', { name: /浏览器成员/ })).toBeVisible()
  await page.getByText('浏览器成员', { exact: true }).click()
  await expect(page.getByRole('checkbox', { name: /浏览器成员/ })).toBeChecked()
  await page.getByLabel('用途名称', { exact: true }).fill('真实授权体验')
  const issued = page.waitForResponse(response => new URL(response.url()).pathname === '/api/admin/vouchers' && response.request().method() === 'POST')
  await page.getByRole('button', { name: '生成兑换券', exact: true }).click()
  const voucherResponse = await issued
  expect(voucherResponse.status()).toBe(201)
  const voucher = (await voucherResponse.json()).vouchers[0]
  expect(voucher.recipient.email).toBe('member@example.test')
  await page.screenshot({ path: testInfo.outputPath('member-assigned-voucher.png'), fullPage: true })
  await page.goto(`${backend.admin_url}/consumption-logs`)
  await page.getByRole('combobox', { name: '成员', exact: true }).click()
  await expect(page.getByRole('option', { name: /浏览器成员/ })).toBeVisible()
  await page.getByRole('option', { name: /浏览器成员/ }).click()
  await expect.poll(async () => {
    const result = await page.request.get(`${backend.admin_url}/api/admin/vouchers`)
    return (await result.json()).items.length
  }).toBe(1)
  expect(failures).toEqual([])
  const memberContext = await browser.newContext({ locale: 'zh-CN' })
  try {
    const member = await memberContext.newPage()
    await memberLogin(member, backend, 'member@example.test')
    const memberFailures = captureFailures(member)
    await member.goto(`${backend.member_url}/keys`)
    for (const name of ['browser-key-one', 'browser-key-two']) {
      await createKey(member, name)
    }
    expect(memberFailures).toEqual([])
    const overLimit = await member.request.post(`${backend.member_url}/api/member/keys`, { data: { name: 'over-limit' } })
    expect(overLimit.headers()['x-aster-error-number']).toBe('22004')
    expect(overLimit.ok()).toBe(false)
    const unauthorized = await member.request.get(`${backend.admin_url}/api/admin/vouchers/recipients`, { headers: { 'x-aster-business-context': 'member' } })
    expect(unauthorized.status()).toBe(401)
    const cookies = await memberContext.cookies()
    expect(cookies.some(cookie => cookie.name.includes('member'))).toBe(true)
    const forgedCookies = cookies.map(cookie => `${cookie.name.replace('member', 'admin')}=${cookie.value}`).join('; ')
    const impersonated = await member.request.get(`${backend.admin_url}/api/admin/vouchers/recipients`, { headers: { cookie: forgedCookies } })
    expect(impersonated.status()).toBe(401)
    await member.screenshot({ path: testInfo.outputPath('member-real-keys.png'), fullPage: true })
  } finally { await memberContext.close() }
  // A claimed menu context cannot confer a different signed capability.
  const denied = await page.request.get(`${backend.admin_url}/api/admin/runners/connection?context=member&fields=all`, { headers: { 'x-aster-business-context': 'member' } })
  expect(denied.status()).toBe(403)
  expect(denied.headers()['x-aster-error-number']).toBe('51008')
})

test('member-only license supports batch creation and paginated recipient-scoped voucher claims', async ({ page, browser, backend }, testInfo) => {
  await ownerLogin(page, backend)
  const failures = captureFailures(page)
  const emails = ['batch-alice@example.test', 'batch-bob@example.test']
  await page.goto(`${backend.admin_url}/users`)
  await page.getByRole('button', { name: '批量创建', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('成员邮箱（一行一个）', { exact: true }).fill(emails.join('\n'))
  await dialog.getByRole('radio', { name: '固定密码', exact: true }).click()
  await dialog.getByLabel('固定初始密码', { exact: true }).fill('member-password-strong')
  const creation = page.waitForResponse(response => new URL(response.url()).pathname === '/api/admin/users/batch' && response.request().method() === 'POST')
  await dialog.getByRole('button', { name: '批量创建成员', exact: true }).click()
  const created = await creation
  expect(created.status()).toBe(201)
  expect((await created.json()).items.map(item => item.email).sort()).toEqual(emails)
  for (const email of emails) await expect(dialog.getByRole('row').filter({ hasText: email })).toContainText('member-password-strong')
  await dialog.getByRole('button', { name: '关闭', exact: true }).click()
  await expect(dialog).toBeHidden()
  for (const email of emails) await expect(page.getByText(email, { exact: true })).toBeVisible()

  await page.goto(`${backend.admin_url}/vouchers`)
  const issuedIds = new Map(emails.map(email => [email, []]))
  // Nine real UI batches exceed the member page size of eight without changing the signed seat limit.
  for (let index = 0; index < 9; index += 1) {
    await page.getByRole('button', { name: '创建 / 批量投放', exact: true }).click()
    await dialog.getByLabel('用途名称', { exact: true }).fill(`batch-distribution-${index}`)
    await dialog.getByLabel('结算 Token 数量').fill('75')
    await expect(dialog.getByRole('checkbox')).toHaveCount(2)
    await dialog.getByRole('button', { name: '全选', exact: true }).click()
    for (const checkbox of await dialog.getByRole('checkbox').all()) await expect(checkbox).toBeChecked()
    const issuance = page.waitForResponse(response => new URL(response.url()).pathname === '/api/admin/vouchers' && response.request().method() === 'POST')
    await dialog.getByRole('button', { name: '生成兑换券', exact: true }).click()
    const response = await issuance
    expect(response.status()).toBe(201)
    const vouchers = (await response.json()).vouchers
    expect(vouchers.map(voucher => voucher.recipient.email).sort()).toEqual(emails)
    for (const voucher of vouchers) issuedIds.get(voucher.recipient.email).push(voucher.id)
    await dialog.getByRole('button', { name: '我已保存，关闭', exact: true }).click()
    await expect(dialog).toBeHidden()
  }
  expect(failures).toEqual([])

  const contexts = [await browser.newContext({ locale: 'zh-CN' }), await browser.newContext({ locale: 'zh-CN' })]
  try {
    const members = [await contexts[0].newPage(), await contexts[1].newPage()]
    const deliveries = []
    for (const [index, member] of members.entries()) {
      await memberLogin(member, backend, emails[index])
      const response = await member.request.get(`${backend.member_url}/api/member/vouchers?limit=16&offset=0`)
      expect(response.status()).toBe(200)
      const data = await response.json()
      expect(data.total).toBe(9)
      expect(data.available_count).toBe(9)
      expect(data.items.map(item => item.id).sort()).toEqual(issuedIds.get(emails[index]).sort())
      deliveries.push(data.items)
    }
    const alice = members[0]
    const aliceFailures = captureFailures(alice)
    await alice.goto(`${backend.member_url}/quota`)
    const section = alice.locator('.voucher-section')
    await expect(section.locator('.voucher-card')).toHaveCount(8)
    const firstPagePrefixes = await section.locator('.voucher-card code').allTextContents()
    const nextPage = alice.waitForResponse(response => {
      const url = new URL(response.url())
      return url.pathname === '/api/member/vouchers' && url.searchParams.get('offset') === '8'
    })
    await section.getByRole('button', { name: '下一页', exact: true }).click()
    const nextResponse = await nextPage
    expect(nextResponse.status()).toBe(200)
    const last = (await nextResponse.json()).items[0]
    expect(deliveries[0].map(item => item.delivery_id)).toContain(last.delivery_id)
    await expect(section.locator('.voucher-card')).toHaveCount(1)
    await expect(section.locator('.voucher-card code')).toHaveText(`${last.code_prefix}…`)
    expect(firstPagePrefixes).not.toContain(`${last.code_prefix}…`)
    const expectedPrefixes = deliveries[0].map(item => `${item.code_prefix}…`).sort()
    expect([...firstPagePrefixes, `${last.code_prefix}…`].sort()).toEqual(expectedPrefixes)
    await expect(section.getByRole('button', { name: '下一页', exact: true })).toBeDisabled()

    const claimed = alice.waitForResponse(response => new URL(response.url()).pathname === '/api/member/vouchers/redeem' && response.request().method() === 'POST')
    await section.getByRole('button', { name: '领取额度', exact: true }).click()
    expect((await claimed).status()).toBe(200)
    await expect(section.getByRole('button', { name: '已领取', exact: true })).toBeDisabled()
    const profile = await alice.request.get(`${backend.member_url}/api/member/me`)
    expect(profile.status()).toBe(200)
    expect((await profile.json()).balance_tokens).toBe(75)
    const ledger = await alice.request.get(`${backend.member_url}/api/member/ledger`)
    expect(ledger.status()).toBe(200)
    const successfulLedger = (await ledger.json()).items
    expect(successfulLedger).toHaveLength(1)
    expect(successfulLedger[0]).toMatchObject({ kind: 'grant', amount_tokens: 75 })
    await section.getByRole('button', { name: '上一页', exact: true }).click()
    await expect(section.locator('.voucher-card')).toHaveCount(8)
    await section.getByRole('combobox', { name: '每页条数', exact: true }).click()
    await alice.getByRole('option', { name: '16', exact: true }).click()
    await expect(section.locator('.voucher-card')).toHaveCount(9)
    expect((await section.locator('.voucher-card code').allTextContents()).sort()).toEqual(expectedPrefixes)
    expect(aliceFailures).toEqual([])
    await alice.screenshot({ path: testInfo.outputPath('member-batch-voucher-pagination.png'), fullPage: true })

    // A member cannot claim another recipient's delivery, including by adding a claimed identity context.
    const bob = members[1]
    const foreign = await bob.request.post(`${backend.member_url}/api/member/vouchers/redeem`, {
      data: { delivery_id: deliveries[0].find(item => item.delivery_id !== last.delivery_id).delivery_id },
      headers: { 'x-aster-business-context': 'member' },
    })
    expect(foreign.status()).toBe(410)
    expect(foreign.headers()['x-aster-error-number']).toBe('23007')
    const replay = await alice.request.post(`${backend.member_url}/api/member/vouchers/redeem`, { data: { delivery_id: last.delivery_id } })
    expect(replay.status()).toBe(409)
    expect(replay.headers()['x-aster-error-number']).toBe('23008')
    expect((await (await alice.request.get(`${backend.member_url}/api/member/me`)).json()).balance_tokens).toBe(75)
    expect((await (await bob.request.get(`${backend.member_url}/api/member/me`)).json()).balance_tokens).toBe(0)
    const untouched = await bob.request.get(`${backend.member_url}/api/member/vouchers?limit=16&offset=0`)
    expect(untouched.status()).toBe(200)
    expect((await untouched.json()).available_count).toBe(9)
    const aliceVouchers = await alice.request.get(`${backend.member_url}/api/member/vouchers?limit=16&offset=0`)
    expect(aliceVouchers.status()).toBe(200)
    expect((await aliceVouchers.json()).available_count).toBe(8)
    const finalAliceLedger = await alice.request.get(`${backend.member_url}/api/member/ledger`)
    expect(finalAliceLedger.status()).toBe(200)
    expect((await finalAliceLedger.json()).items).toEqual(successfulLedger)
    const bobLedger = await bob.request.get(`${backend.member_url}/api/member/ledger`)
    expect(bobLedger.status()).toBe(200)
    expect((await bobLedger.json()).items).toEqual([])
  } finally { for (const context of contexts) await context.close() }
})

test('member-only quota requests remain operable on short screens through edit withdrawal approval and rejection', async ({ page, browser, backend }, testInfo) => {
  await ownerLogin(page, backend)
  await createMember(page, backend, 'quota@example.test', '额度成员')
  const context = await browser.newContext({ locale: 'zh-CN', viewport: { width: 1366, height: 648 } })
  try {
    const member = await context.newPage()
    await memberLogin(member, backend, 'quota@example.test')
    const memberFailures = captureFailures(member)
    const adminFailures = captureFailures(page)
    await member.goto(`${backend.member_url}/quota`)
    const requests = member.locator('.quota-detail-grid > section').filter({ has: member.getByRole('heading', { name: '我的额度申请', exact: true }) })
    const grants = member.locator('.quota-detail-grid > section').filter({ has: member.getByRole('heading', { name: '额度入账记录', exact: true }) })
    await expect(requests).toBeVisible()
    // The request and grant panels need usable space even when the browser content is only 648px tall.
    for (const panel of [requests, grants]) expect((await panel.boundingBox()).height).toBeGreaterThanOrEqual(300)
    const read = async (surface, path) => {
      const response = await surface.request.get(path)
      expect(response.status()).toBe(200)
      return response.json()
    }
    const submitRequest = async (amount, reason) => {
      await member.getByRole('button', { name: '申请额度', exact: true }).click()
      const dialog = member.getByRole('dialog')
      await dialog.getByLabel('申请 Token 数').fill(String(amount))
      await dialog.getByLabel('申请原因').fill(reason)
      const submitted = member.waitForResponse(response => new URL(response.url()).pathname === '/api/member/quota-requests' && response.request().method() === 'POST')
      await dialog.getByRole('button', { name: '提交申请', exact: true }).click()
      const response = await submitted
      expect(response.status()).toBe(201)
      await expect(dialog).toBeHidden()
      const record = await response.json()
      expect(record).toMatchObject({ amount_tokens: amount, reason, status: 'pending' })
      return record
    }
    const first = await submitRequest(125, 'member-only-initial-request')
    const row = requests.locator('article').filter({ hasText: first.reason })
    const edit = row.getByRole('button', { name: '编辑申请', exact: true })
    await edit.scrollIntoViewIfNeeded()
    await expect(edit).toBeInViewport({ ratio: 1 })
    await edit.click()
    const dialog = member.getByRole('dialog')
    await dialog.getByLabel('申请 Token 数').fill('150')
    await dialog.getByLabel('申请原因').fill('member-only-edited-request')
    const updated = member.waitForResponse(response => new URL(response.url()).pathname === `/api/member/quota-requests/${first.id}` && response.request().method() === 'PATCH')
    await dialog.getByRole('button', { name: '保存修改', exact: true }).click()
    expect((await updated).status()).toBe(200)
    await expect(dialog).toBeHidden()
    const edited = requests.locator('article').filter({ hasText: 'member-only-edited-request' })
    await expect(edited).toContainText('150')
    expect((await read(member, `${backend.member_url}/api/member/quota-requests`)).items).toEqual([
      expect.objectContaining({ id: first.id, amount_tokens: 150, reason: 'member-only-edited-request', status: 'pending' }),
    ])
    await edited.getByRole('button', { name: '撤销申请', exact: true }).click()
    const withdrawn = member.waitForResponse(response => new URL(response.url()).pathname === `/api/member/quota-requests/${first.id}` && response.request().method() === 'DELETE')
    await dialog.getByRole('button', { name: '撤销申请', exact: true }).click()
    expect((await withdrawn).status()).toBe(200)
    await expect(dialog).toBeHidden()
    await expect(edited).toHaveCount(0)
    expect((await read(member, `${backend.member_url}/api/member/quota-requests`)).items).toEqual([])
    expect((await read(page, `${backend.admin_url}/api/admin/quota-requests`)).items).toEqual([])
    expect((await read(member, `${backend.member_url}/api/member/ledger`)).items).toEqual([])

    const review = async (record, approve) => {
      await page.goto(`${backend.admin_url}/quota-requests`)
      const row = page.getByRole('row').filter({ hasText: record.reason })
      await expect(row).toContainText('quota@example.test')
      await row.getByRole('button', { name: approve ? '通过' : '驳回', exact: true }).click()
      await page.getByLabel('审批说明（可选）').fill(approve ? 'member-only-approved' : 'member-only-rejected')
      const reviewed = page.waitForResponse(response => new URL(response.url()).pathname === `/api/admin/quota-requests/${record.id}` && response.request().method() === 'PATCH')
      await page.getByRole('dialog').getByRole('button', { name: approve ? '确认通过并发放' : '确认驳回', exact: true }).click()
      expect((await reviewed).status()).toBe(200)
      await expect(page.getByRole('dialog')).toBeHidden()
      await expect(row).toHaveCount(0)
      await member.reload()
      const reviewedRow = requests.locator('article').filter({ hasText: record.reason })
      await expect(reviewedRow).toContainText(approve ? '已通过' : '已驳回')
      await expect(reviewedRow).toContainText('owner@example.test')
      await expect(reviewedRow.getByRole('button', { name: '编辑申请', exact: true })).toHaveCount(0)
    }
    const approved = await submitRequest(200, 'member-only-approval-request')
    await review(approved, true)
    expect((await read(member, `${backend.member_url}/api/member/me`)).balance_tokens).toBe(200)
    const approvedLedger = (await read(member, `${backend.member_url}/api/member/ledger`)).items
    expect(approvedLedger).toEqual([expect.objectContaining({ kind: 'grant', amount_tokens: 200 })])
    const rejected = await submitRequest(300, 'member-only-rejection-request')
    await review(rejected, false)
    expect((await read(member, `${backend.member_url}/api/member/me`)).balance_tokens).toBe(200)
    expect((await read(member, `${backend.member_url}/api/member/ledger`)).items).toEqual(approvedLedger)
    const finalRequests = (await read(member, `${backend.member_url}/api/member/quota-requests`)).items
    expect(finalRequests).toHaveLength(2)
    expect(finalRequests).toEqual(expect.arrayContaining([
      expect.objectContaining({ id: approved.id, status: 'approved', amount_tokens: 200 }),
      expect.objectContaining({ id: rejected.id, status: 'rejected', amount_tokens: 300 }),
    ]))

    for (const viewport of [{ width: 1366, height: 648 }, { width: 1920, height: 1008 }, { width: 1024, height: 768 }, { width: 901, height: 921 }, { width: 900, height: 744 }, { width: 390, height: 744 }]) {
      await member.setViewportSize(viewport)
      for (const panel of [requests, grants]) {
        expect((await panel.boundingBox()).height).toBeGreaterThanOrEqual(300)
        const filter = panel.getByRole('textbox')
        await filter.scrollIntoViewIfNeeded()
        await expect(filter).toBeInViewport({ ratio: 1 })
        await filter.click()
      }
      const status = requests.getByRole('combobox', { name: '申请状态', exact: true })
      await status.scrollIntoViewIfNeeded()
      await expect(status).toBeInViewport({ ratio: 1 })
      await status.click()
      await member.getByRole('option', { name: '已通过', exact: true }).click()
      await expect(requests.locator('article')).toHaveCount(1)
      await expect(requests.locator('article')).toContainText(approved.reason)
      await requests.getByRole('button', { name: '重置', exact: true }).click()
      await expect(requests.locator('article')).toHaveCount(2)
      const grantsReloaded = member.waitForResponse(response => new URL(response.url()).pathname === '/api/member/ledger')
      await grants.getByRole('button', { name: '重置', exact: true }).click()
      expect((await grantsReloaded).status()).toBe(200)
      const requestedAmount = requests.locator('article').filter({ hasText: approved.reason }).locator('strong.readable-amount')
      await requestedAmount.scrollIntoViewIfNeeded()
      await expect(requestedAmount).toHaveText('200')
      await expect(requestedAmount).toBeInViewport({ ratio: 1 })
      const amount = grants.locator('.grant-list b').filter({ hasText: '+200' })
      await amount.scrollIntoViewIfNeeded()
      await expect(amount).toBeInViewport({ ratio: 1 })
      for (const panel of [requests, grants]) {
        const pagination = panel.getByRole('navigation', { name: '分页导航', exact: true })
        // Scroll the whole footer so fractional text baselines cannot stop at the viewport edge.
        await pagination.scrollIntoViewIfNeeded()
        await expect(pagination).toBeInViewport({ ratio: 1 })
        for (const control of [pagination.locator('.a-pagination-summary'), pagination.getByRole('combobox'), ...await pagination.getByRole('button').all()]) {
          await control.scrollIntoViewIfNeeded()
          await expect(control).toBeInViewport({ ratio: 1 })
        }
        const frame = await panel.boundingBox()
        const pager = await pagination.boundingBox()
        expect(pager.y + pager.height).toBeLessThanOrEqual(frame.y + frame.height)
        const list = await panel.locator('.request-list, .grant-list').boundingBox()
        expect(list.y + list.height).toBeLessThanOrEqual(pager.y + 1)
      }
      expect(await member.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(viewport.width)
      await member.screenshot({ path: testInfo.outputPath(`member-quota-${viewport.width}x${viewport.height}.png`) })
    }
    expect(memberFailures).toEqual([])
    expect(adminFailures).toEqual([])
  } finally { await context.close() }
})

test('member sessions cannot list or revoke another members keys and own revocation releases capacity', async ({ page, browser, backend }, testInfo) => {
  await ownerLogin(page, backend)
  const aliceIdentity = await createMember(page, backend, 'alice@example.test', 'Alice')
  await createMember(page, backend, 'bob@example.test', 'Bob')
  const aliceContext = await browser.newContext({ locale: 'zh-CN' })
  const bobContext = await browser.newContext({ locale: 'zh-CN' })
  try {
    const alice = await aliceContext.newPage()
    const bob = await bobContext.newPage()
    await memberLogin(alice, backend, 'alice@example.test')
    await memberLogin(bob, backend, 'bob@example.test')
    await alice.goto(`${backend.member_url}/keys`)
    const first = await createKey(alice, 'alice-private-one')
    await createKey(alice, 'alice-private-two')
    const aliceId = first.api_key.id
    expect(aliceIdentity.id).toMatch(/^identity_/)
    await bob.goto(`${backend.member_url}/keys`)
    const bobKey = await createKey(bob, 'bob-private')
    const selected = await bob.request.get(`${backend.member_url}/api/member/keys?identity_id=${aliceIdentity.id}&user_id=${aliceIdentity.id}`)
    expect(selected.status()).toBe(200)
    expect((await selected.json()).items.map(item => item.id)).toEqual([bobKey.api_key.id])
    const denial = await bob.request.post(`${backend.member_url}/api/member/keys/${aliceId}/revoke?identity_id=${aliceIdentity.id}`, {
      headers: { 'x-aster-identity-id': aliceIdentity.id, 'x-aster-business-context': 'member' },
    })
    expect(denial.status()).toBe(404)
    const intact = await alice.request.get(`${backend.member_url}/api/member/keys`)
    const intactKeys = (await intact.json()).items
    expect(intactKeys).toHaveLength(2)
    expect(intactKeys.find(item => item.id === aliceId).status).toBe('active')
    await expect(bob.getByText('alice-private-one', { exact: true })).toHaveCount(0)
    const full = await alice.request.post(`${backend.member_url}/api/member/keys`, { data: { name: 'before-revoke' } })
    expect(full.headers()['x-aster-error-number']).toBe('22004')
    const revoked = alice.waitForResponse(response => new URL(response.url()).pathname === `/api/member/keys/${aliceId}/revoke`)
    await alice.getByRole('row').filter({ hasText: 'alice-private-one' }).getByRole('button', { name: '删除', exact: true }).click()
    await alice.getByRole('dialog').getByRole('button', { name: '删除', exact: true }).click()
    expect((await revoked).status()).toBe(200)
    await expect(alice.getByRole('dialog')).toBeHidden()
    await expect(alice.getByText('alice-private-one', { exact: true })).toHaveCount(0)
    await createKey(alice, 'alice-replacement')
    const bobKeys = await bob.request.get(`${backend.member_url}/api/member/keys`)
    expect((await bobKeys.json()).items.map(item => item.id)).toEqual([bobKey.api_key.id])
    await alice.screenshot({ path: testInfo.outputPath('owner-scoped-key-replacement.png'), fullPage: true })
  } finally { await aliceContext.close(); await bobContext.close() }
})

async function createRunnerEnrollment(page, backend, name) {
  await page.goto(`${backend.admin_url}/runners`)
  await page.getByRole('button', { name: '新增 Runner', exact: true }).click()
  await page.getByLabel('节点名称', { exact: true }).fill(name)
  const created = page.waitForResponse(response => new URL(response.url()).pathname === '/api/admin/runners/enrollments' && response.request().method() === 'POST')
  await page.getByRole('button', { name: '生成注册 Token', exact: true }).click()
  const response = await created
  expect(response.status()).toBe(201)
  const enrollment = await response.json()
  await expect(page.getByText(enrollment.token, { exact: true })).toBeVisible()
  await page.getByRole('dialog').getByRole('button', { name: '关闭', exact: true }).click()
  await expect(page.getByRole('dialog')).toBeHidden()
  return enrollment
}

async function exerciseRunnerRegistration(page, request, backend) {
  const enroll = token => request.post(`${backend.admin_url}/api/runner/enroll`, { data: {
    token, version: backend.product_version, protocol_version: backend.runner_protocol,
    platform: 'linux', architecture: 'amd64', max_inflight: 2,
  } })
  const firstToken = await createRunnerEnrollment(page, backend, 'browser-runner-one')
  const first = await enroll(firstToken.token)
  expect(first.status()).toBe(201)
  const registration = await first.json()
  expect(registration.protocol_version).toBe(backend.runner_protocol)
  expect(registration.task_keys.keys).toHaveLength(1)
  await page.reload()
  await expect(page.getByText('browser-runner-one', { exact: true })).toBeVisible()
  const row = page.getByRole('row').filter({ hasText: 'browser-runner-one' })
  const disabled = page.waitForResponse(response => new URL(response.url()).pathname === `/api/admin/runners/${registration.runner_id}` && response.request().method() === 'PATCH')
  await row.getByRole('button', { name: '停用 Runner', exact: true }).click()
  expect((await disabled).status()).toBe(200)
  await expect(row.getByText('已停用', { exact: true })).toBeVisible()
  const secondToken = await createRunnerEnrollment(page, backend, 'browser-runner-two')
  const overLimit = await enroll(secondToken.token)
  expect(overLimit.ok()).toBe(false)
  expect(overLimit.headers()['x-aster-error-number']).toBe('14004')
  const listed = await page.request.get(`${backend.admin_url}/api/admin/runners`)
  expect((await listed.json()).items.map(item => item.id)).toEqual([registration.runner_id])
  const removed = page.waitForResponse(response => new URL(response.url()).pathname === `/api/admin/runners/${registration.runner_id}` && response.request().method() === 'DELETE')
  await page.getByRole('row').filter({ hasText: 'browser-runner-one' }).getByRole('button', { name: '删除 Runner', exact: true }).click()
  await page.getByRole('dialog').getByRole('button', { name: '确认删除', exact: true }).click()
  expect((await removed).status()).toBe(200)
  await expect(page.getByRole('dialog')).toBeHidden()
  // A quota refusal must leave the unused token eligible for a later attempt.
  const replacement = await enroll(secondToken.token)
  expect(replacement.status()).toBe(201)
  const replacementId = (await replacement.json()).runner_id
  const replay = await enroll(secondToken.token)
  expect(replay.ok()).toBe(false)
  expect(replay.headers()['x-aster-error-number']).toBe('14001')
  await page.reload()
  await expect(page.getByText('browser-runner-two', { exact: true })).toBeVisible()
  await expect(page.getByText('browser-runner-one', { exact: true })).toHaveCount(0)
  const afterReplay = await page.request.get(`${backend.admin_url}/api/admin/runners`)
  expect((await afterReplay.json()).items.map(item => item.id)).toEqual([replacementId])
  // Free capacity so quota rejection cannot masquerade as single-use enforcement.
  const replacementRemoved = page.waitForResponse(response => new URL(response.url()).pathname === `/api/admin/runners/${replacementId}` && response.request().method() === 'DELETE')
  await page.getByRole('row').filter({ hasText: 'browser-runner-two' }).getByRole('button', { name: '删除 Runner', exact: true }).click()
  await page.getByRole('dialog').getByRole('button', { name: '确认删除', exact: true }).click()
  expect((await replacementRemoved).status()).toBe(200)
  await expect(page.getByRole('dialog')).toBeHidden()
  const replayWithCapacity = await enroll(secondToken.token)
  expect(replayWithCapacity.ok()).toBe(false)
  expect(replayWithCapacity.headers()['x-aster-error-number']).toBe('14001')
  const afterCapacityReplay = await page.request.get(`${backend.admin_url}/api/admin/runners`)
  expect((await afterCapacityReplay.json()).items).toEqual([])
}

for (const scenario of [
  { capability: 'runner', page: '/runners', endpoints: ['/api/admin/runners', '/api/admin/runners/connection'], denied: '/api/admin/vouchers/recipients' },
  { capability: 'gateway', page: '/models', endpoints: ['/api/admin/models'], denied: '/api/admin/runners/connection' },
  { capability: 'none', page: '/users', endpoints: [], denied: '/api/admin/vouchers/recipients' },
]) {
  test.describe(scenario.capability, () => {
    test.use({ capability: scenario.capability })
    test('real signed minimum capability preserves page dependencies and rejects unrelated operations', async ({ page, request, backend }, testInfo) => {
      await ownerLogin(page, backend)
      const failures = captureFailures(page)
      const responses = scenario.endpoints.map(path => page.waitForResponse(response => new URL(response.url()).pathname === path))
      await page.goto(`${backend.admin_url}${scenario.page}`)
      for (const response of responses) expect((await response).status()).toBe(200)
      const locked = page.getByRole('heading', { name: '当前授权未包含此功能', exact: true })
      if (scenario.capability === 'none') await expect(locked).toBeVisible()
      else await expect(locked).toHaveCount(0)
      if (scenario.capability === 'runner') await exerciseRunnerRegistration(page, request, backend)
      if (scenario.capability === 'gateway') {
        await page.goto(`${backend.admin_url}/upstream-accounts`)
        await expect(page.getByRole('button', { name: '新增 ChatGPT 账号', exact: true })).toBeDisabled()
        await expect(page.getByRole('status').getByText('新增订阅/账号需要以下授权功能', { exact: false })).toBeVisible()
        const premature = await page.request.post(`${backend.admin_url}/api/admin/upstream-providers/openai/enrollments`, { data: {} })
        expect(premature.status()).toBe(403)
        expect(premature.headers()['x-aster-error-number']).toBe('51008')
        const local = await page.request.get(`${backend.admin_url}/api/admin/upstream-accounts`)
        expect(local.status()).toBe(200)
      }
      await page.screenshot({ path: testInfo.outputPath(`${scenario.capability}-page.png`), fullPage: true })
      expect(failures).toEqual([])
      const denied = await page.request.get(`${backend.admin_url}${scenario.denied}?context=${scenario.capability}&fields=all`, { headers: { 'x-aster-business-context': scenario.capability } })
      expect(denied.status()).toBe(403)
      expect(denied.headers()['x-aster-error-number']).toBe('51008')
    })
  })
}
