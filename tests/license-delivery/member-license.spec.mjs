import { expect, test } from '@playwright/test'

// HTTP fixtures isolate frontend routing. Rust HTTP tests separately exercise
// actual signed imports and business rejection; these fixtures grant no rights.
async function controlResponses(page, initial) {
  const state = { license: initial, publicRequests: 0, memberRequests: 0 }
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname
    if (path === '/api/public/license-state') {
      state.publicRequests += 1
      // Fail loudly if a regression causes a navigation loop.
      expect(state.publicRequests).toBeLessThan(20)
      return route.fulfill({ json: state.license })
    }
    state.memberRequests += 1
    return route.fulfill({ status: 401, json: { error: { code: 'UNAUTHORIZED', message: 'Sign in required' } } })
  })
  return state
}

for (const viewport of [{ width: 1366, height: 648 }, { width: 390, height: 720 }]) {
  test(`member license without member capability stays on a recoverable page at ${viewport.width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize(viewport)
    const state = await controlResponses(page, { state: 'active', available: true, features: ['runner'] })
    await page.goto('/home')
    await expect(page).toHaveURL(/\/license-required\?reason=feature$/)
    await expect(page.getByRole('heading', { name: '当前授权未包含成员功能' })).toBeVisible()
    await page.getByRole('button', { name: '重新检查授权' }).click()
    await expect(page.getByRole('status')).toContainText('当前许可证未包含成员功能')
    expect(state.memberRequests).toBe(0)
    await page.screenshot({ path: testInfo.outputPath(`member-unlicensed-${viewport.width}.png`), fullPage: true })

    state.license = { state: 'unavailable', available: false, features: [] }
    await page.getByRole('button', { name: '重新检查授权' }).click()
    await expect(page).toHaveURL(/\/license-required$/)
    await expect(page.getByRole('heading', { name: '系统尚未获得有效授权' })).toBeVisible()

    state.license = { state: 'active', available: true, features: ['member', 'runner'] }
    await page.getByRole('button', { name: '重新检查授权' }).click()
    await expect(page).toHaveURL(url => url.pathname === '/login' && url.searchParams.get('redirect') === '/home')
    expect(state.memberRequests).toBe(1)
  })
}

test('member license response without feature grants fails closed without a loop', async ({ page }) => {
  const state = await controlResponses(page, { state: 'active', available: true })
  await page.goto('/home')
  await expect(page).toHaveURL(/\/license-required\?reason=feature$/)
  await expect(page.getByRole('heading', { name: '当前授权未包含成员功能' })).toBeVisible()
  expect(state.memberRequests).toBe(0)
  expect(state.publicRequests).toBeGreaterThan(0)
  expect(state.publicRequests).toBeLessThan(20)
})

test('member license recheck shows network errors without claiming activation', async ({ page }) => {
  await controlResponses(page, { state: 'missing', available: false, features: [] })
  await page.goto('/home')
  await expect(page).toHaveURL(/\/license-required$/)
  await page.route('**/api/public/license-state', route => route.abort('failed'))
  await page.getByRole('button', { name: '重新检查授权' }).click()
  await expect(page.getByRole('heading', { name: '系统尚未获得有效授权' })).toBeVisible()
  await expect(page.getByRole('button', { name: '重新检查授权' })).toBeEnabled()
  await expect(page.getByRole('alert')).toBeVisible()
})
