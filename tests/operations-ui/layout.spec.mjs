import { readFileSync } from 'node:fs'
import { expect, test } from '@playwright/test'

const definition = JSON.parse(readFileSync(new URL('../../contracts/test-vectors/plan-definition.v1.json', import.meta.url), 'utf8'))
const orderSnapshot = {
  schema: 'aster.order-snapshot.v1', order_id: 'order_visual', customer_id: 'customer_visual',
  plan: { schema: 'aster.plan-snapshot.v1', plan_id: 'plan_visual', version: 2, definition },
  plan_sha256: 'a'.repeat(64), years: 1, discount_basis_points: 10000, amount_minor: 23998,
  currency: 'CNY', tax_mode: 'inclusive', starts_at: '2026-09-16T16:36:46.000Z', ends_at: '2029-09-16T16:36:46.000Z',
}
const order = {
  snapshot: orderSnapshot, sha256: 'b'.repeat(64), operation_id: 'order_visual_op', status: 'fulfilled',
  fulfillment_id: 'fulfillment_visual', fulfillment_status: 'issued', customer_name: '示例客户',
  created_by: 'operator_visual', created_at: '2026-09-16T16:36:46.000Z',
}
const fulfillment = {
  snapshot: {
    schema: 'aster.paid-fulfillment.v1', id: 'fulfillment_visual', environment: 'local',
    payment: { snapshot: { order: orderSnapshot } },
    installation_request: { installation_id: 'installation_visual', request_id: 'request_visual' },
    request_sha256: 'c'.repeat(64), request: { reason: '已核对订单与到账' },
    approved_by: 'operator_visual', approved_at: '2026-09-16T16:50:00.000Z',
  },
  sha256: 'd'.repeat(64), document_sha256: 'e'.repeat(64), status: 'issued',
  claims: { key_id: 'local-paid-test-only' },
}

test('issued workflow fits fixed desktop and 2K viewports without scattered actions', async ({ page }, testInfo) => {
  await page.route('**/api/operations/v1/**', route => {
    const { pathname } = new URL(route.request().url())
    const data = pathname.endsWith('/session')
      ? { operator: { id: 'operator_visual', email: 'operator@example.test', password_change_required: false } }
      : pathname.endsWith('/customers') ? { items: [], next: '' }
      : pathname.endsWith('/commercial/plans') ? { items: [] }
      : pathname.endsWith('/commercial/orders') ? { items: [order], total: 1 }
      : pathname.endsWith('/commercial/orders/order_visual') ? order
      : pathname.endsWith('/commercial/orders/order_visual/fulfillment') ? fulfillment
      : { items: [] }
    return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(data) })
  })

  for (const [width, height] of [[1920, 945], [2560, 1440]]) {
    await page.setViewportSize({ width, height })
    await page.goto('http://127.0.0.1:26388/workflows/business?order=order_visual&step=8')
    await expect(page.locator('.workflow-summary h1')).toHaveText('本次办理')
    await expect(page.locator('.fulfillment-record-head')).toContainText('已签发')
    await expect(page.getByRole('button', { name: '下载授权文件' })).toBeVisible()
    await expect(page.getByRole('button', { name: '补发授权文件' })).toBeVisible()
    const metrics = await page.evaluate(() => {
      const main = document.querySelector('.workflow-main')
      const actions = document.querySelector('.fulfillment-secondary-actions')
      const buttons = [...actions.querySelectorAll('button')]
      return {
        viewport: [innerWidth, innerHeight], document: [document.documentElement.scrollWidth, document.documentElement.scrollHeight],
        main: [main.clientWidth, main.scrollWidth, main.clientHeight, main.scrollHeight],
        actionTops: buttons.map(button => Math.round(button.getBoundingClientRect().top)),
      }
    })
    const screenshot = await page.screenshot({ path: testInfo.outputPath(`issued-${width}x${height}.png`), animations: 'disabled' })
    expect([screenshot.readUInt32BE(16), screenshot.readUInt32BE(20)]).toEqual([width, height])
    expect(metrics.document[0], JSON.stringify(metrics)).toBeLessThanOrEqual(width)
    expect(metrics.document[1], JSON.stringify(metrics)).toBeLessThanOrEqual(height + 1)
    expect(metrics.main[1], JSON.stringify(metrics)).toBeLessThanOrEqual(metrics.main[0] + 1)
    expect(metrics.main[3], JSON.stringify(metrics)).toBeLessThanOrEqual(metrics.main[2] + 1)
    expect(new Set(metrics.actionTops).size, JSON.stringify(metrics)).toBe(1)
  }

  await page.setViewportSize({ width: 1920, height: 945 })
  for (const [step, label] of [[4, '导入申请'], [5, '核对批准'], [6, '选择证书'], [7, '签发授权']]) {
    await page.locator('.workflow-stepper button').nth(step - 1).click()
    await expect(page).toHaveURL(new RegExp(`[?&]step=${step}(?:&|$)`))
    await expect(page.locator('.workflow-stepper li.active')).toContainText(label)
    await expect(page.locator('.fulfillment-node-summary')).toBeVisible()
  }
  await page.reload()
  await expect(page.locator('.workflow-stepper li.active')).toContainText('签发授权')
  await page.locator('.workflow-stepper button').nth(7).click()
  await page.getByRole('button', { name: '补发授权文件' }).click()
  await expect(page.getByRole('dialog', { name: '补发授权文件' })).toBeVisible()
  await expect(page.getByLabel('付费授权补发原因')).toBeVisible()

  for (const [path, action, name] of [
    ['/workflows/business?step=1', '下一步', 'business-new'],
    ['/base/customers?mode=new', '保存客户', 'customer-new'],
    ['/base/plans?mode=new&step=1', '下一步', 'plan-new'],
    ['/workflows/release?step=1', '下一步', 'release-start'],
  ]) {
    await page.goto(`http://127.0.0.1:26388${path}`)
    const button = page.getByRole('button', { name: action, exact: true }).last()
    await expect(button).toBeVisible()
    const bottom = await button.evaluate(element => element.getBoundingClientRect().bottom)
    const documentSize = await page.evaluate(() => [document.documentElement.scrollWidth, document.documentElement.scrollHeight])
    await page.screenshot({ path: testInfo.outputPath(`${name}-1920x945.png`), animations: 'disabled' })
    expect(bottom, path).toBeLessThanOrEqual(945)
    expect(documentSize[0], path).toBeLessThanOrEqual(1920)
    expect(documentSize[1], path).toBeLessThanOrEqual(946)
  }

  for (const step of [2, 3, 4]) {
    await page.goto(`http://127.0.0.1:26388/base/plans?mode=new&step=${step}`)
    await expect(page.locator('.draft-fields')).toBeVisible()
    await expect(page.locator('.editor-actions')).toBeVisible()
    const bounds = await page.evaluate(() => {
      const fieldset = document.querySelector('.draft-fields')
      const actions = document.querySelector('.editor-actions')
      return { contentBottom: fieldset.getBoundingClientRect().bottom, actionTop: actions.getBoundingClientRect().top }
    })
    await page.screenshot({ path: testInfo.outputPath(`plan-step-${step}-1920x945.png`), animations: 'disabled' })
    expect(bounds.contentBottom, `plan step ${step}: ${JSON.stringify(bounds)}`).toBeLessThanOrEqual(bounds.actionTop)
  }
  await page.getByRole('combobox', { name: '报价方式' }).click()
  await page.getByRole('option', { name: '按年订阅' }).click()
  await page.keyboard.press('Escape')
  const annualBounds = await page.evaluate(() => ({
    contentBottom: document.querySelector('.draft-fields').getBoundingClientRect().bottom,
    actionTop: document.querySelector('.editor-actions').getBoundingClientRect().top,
  }))
  await page.screenshot({ path: testInfo.outputPath('plan-annual-1920x945.png'), animations: 'disabled' })
  expect(annualBounds.contentBottom, JSON.stringify(annualBounds)).toBeLessThanOrEqual(annualBounds.actionTop)
})
