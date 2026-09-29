import { expect, test } from '@playwright/test'

// Browser projection tests. The Rust signed-import tests exercise the real
// authenticated /api/admin/me snapshot; these fixtures do not prove V08 E2E.
async function respond(page, initialFeatures) {
  const state = { features: initialFeatures, available: true, businessReads: [], profileReads: 0, memberFilters: [] }
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname
    if (path === '/api/admin/me') {
      state.profileReads += 1
      const status = state.available ? 'active' : 'unavailable'
      return route.fulfill({ json: {
        email: 'owner@example.test', password_change_required: false, license_state: status,
        license: { state: status, available: state.available, features: state.available ? state.features : [] },
      } })
    }
    state.businessReads.push(path)
    if (state.available && state.features.includes('member')) {
      if (path === '/api/admin/vouchers/recipients' || path === '/api/admin/consumption-logs/members') {
        return route.fulfill({ json: { items: [{ id: 'identity_selector', email: 'selector@example.test', display_name: '测试成员' }] } })
      }
      if (path === '/api/admin/vouchers') return route.fulfill({ json: { items: [], total: 0 } })
      if (path === '/api/admin/consumption-logs') {
        state.memberFilters.push(new URL(route.request().url()).searchParams.get('user_id'))
        return route.fulfill({ json: { items: [], total: 0, keys: [], models: [], protocols: [], processing_tiers: [], reasoning_efforts: [] } })
      }
    }
    if (path === '/api/admin/runners' && state.available && state.features.includes('runner')) {
      return route.fulfill({ json: { items: [] } })
    }
    if (path === '/api/admin/runners/connection' && state.available && state.features.includes('runner')) return route.fulfill({ json: { public_api_base_url: 'https://api.example.test' } })
    if (path === '/api/admin/models' && state.available && state.features.includes('gateway')) {
      return route.fulfill({ json: { items: [] } })
    }
    if (path === '/api/admin/license') {
      return route.fulfill({ status: 503, json: { error: { code: 'BACKEND_LOCAL_STATE_FAILED', number: 91008, message: '服务端本地状态读取或写入失败，请检查文件与权限' } } })
    }
    return route.fulfill({ status: 403, json: { error: { code: 'FEATURE_NOT_LICENSED', number: 51008, message: '未授权业务请求' } } })
  })
  return state
}

test('runner-only license locks unrelated pages and still loads runner dependencies', async ({ page }) => {
  const state = await respond(page, ['runner'])
  await page.goto('/users')
  await expect(page.getByRole('heading', { name: '当前授权未包含此功能', exact: true })).toBeVisible()
  expect(state.businessReads).toEqual([])
  await page.goto('/runners')
  await expect.poll(() => state.businessReads.includes('/api/admin/runners')).toBe(true)
  await expect.poll(() => state.businessReads.includes('/api/admin/runners/connection')).toBe(true)
  expect(state.businessReads).not.toContain('/api/admin/settings')
  await expect(page.getByRole('heading', { name: '当前授权未包含此功能', exact: true })).toHaveCount(0)
  await page.screenshot({ path: 'dist/license-recovery/admin-runner-only.png', fullPage: true })
  await page.goto('/models')
  await expect(page.getByRole('heading', { name: '当前授权未包含此功能', exact: true })).toBeVisible()
  expect(state.businessReads).not.toContain('/api/admin/models')
})

test('member selectors operate without the complete member-management endpoint', async ({ page }) => {
  const state = await respond(page, ['member'])
  await page.goto('/vouchers')
  await page.getByRole('button', { name: '创建 / 批量投放', exact: true }).click()
  await expect(page.getByRole('checkbox', { name: /测试成员/ })).toBeVisible()
  await page.getByText('测试成员', { exact: true }).click()
  await expect(page.getByRole('checkbox', { name: /测试成员/ })).toBeChecked()
  await expect(page.getByText('将为 1 个成员分别生成一次性兑换券，互不抢占。', { exact: true })).toBeVisible()
  await page.goto('/consumption-logs')
  await page.getByRole('combobox', { name: '成员', exact: true }).click()
  await page.getByRole('option', { name: /测试成员/ }).click()
  await expect.poll(() => state.memberFilters.includes('identity_selector')).toBe(true)
  expect(state.businessReads).toContain('/api/admin/vouchers/recipients')
  expect(state.businessReads).toContain('/api/admin/consumption-logs/members')
  expect(state.businessReads).not.toContain('/api/admin/users')
  expect(state.businessReads).not.toContain('/api/admin/settings')
})

test('same-tab navigation refreshes features and preserves the recovery page', async ({ page }) => {
  const state = await respond(page, ['gateway'])
  await page.goto('/models')
  await expect.poll(() => state.businessReads.includes('/api/admin/models')).toBe(true)
  state.features = ['runner']
  await page.getByRole('link', { name: 'Runner 节点', exact: true }).first().click()
  await expect(page).toHaveURL(/\/runners$/)
  await expect.poll(() => state.businessReads.includes('/api/admin/runners')).toBe(true)
  const previousModelReads = state.businessReads.filter(path => path === '/api/admin/models').length
  await page.getByRole('link', { name: '可用模型', exact: true }).first().click()
  await expect(page.getByRole('heading', { name: '当前授权未包含此功能', exact: true })).toBeVisible()
  expect(state.businessReads.filter(path => path === '/api/admin/models')).toHaveLength(previousModelReads)
  state.available = false
  await page.getByRole('button', { name: '查看授权状态', exact: true }).click()
  await expect(page).toHaveURL(/\/license$/)
  await expect(page.getByRole('heading', { name: '暂时无法读取授权状态', exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: '重新读取状态', exact: true })).toBeEnabled()
  expect(state.profileReads).toBeGreaterThanOrEqual(4)
})
