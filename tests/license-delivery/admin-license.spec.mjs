import { expect, test } from '@playwright/test'

// Frontend HTTP fixtures only. Rust tests separately execute signed imports,
// actual rename failure, persisted history and the expiry retry policy.
const license = {
  schema: 'aster.admin-license-view.v1', protocol_schema: 'aster.license.v2', license_id: 'license_recovery_fixture', serial: 'AT-RECOVERY-FIXTURE', customer_ref: 'customer_fixture', key_id: 'fixture',
  edition: 'enterprise', plan_id: 'team_10', features: ['member'], quotas: { member_seats: 10, runners: 1, upstream_accounts: 1, api_keys_per_member: 2 }, binding: 'installation',
  minimum_version: '0.1.0', transfer_sequence: 1, issued_at: '2026-01-01T00:00:00.000Z',
  not_before: '2026-01-01T00:00:00.000Z', expires_at: '2027-01-01T00:00:00.000Z',
}
const upload = { name: 'license.json', mimeType: 'application/json', buffer: Buffer.from(JSON.stringify({ fixture: 'same-license-retry' })) }

async function responses(page, options = {}) {
  const state = { readFails: true, installFails: false, failReadAfterInstall: false, scheduled: false, state: 'active', previews: 0, posts: 0, ...options }
  await page.route('**/api/**', async route => {
    const request = route.request()
    const path = new URL(request.url()).pathname
    const currentLicense = { ...license, expires_at: state.state === 'unavailable' ? '2026-02-01T00:00:00.000Z' : license.expires_at }
    if (path === '/api/admin/me') {
      const current = state.readFails ? 'unavailable' : state.state
      return route.fulfill({ json: { email: 'owner@example.com', password_change_required: false, license_state: current, license: { state: current, available: current === 'active', features: current === 'active' ? currentLicense.features : [] } } })
    }
    if (path === '/api/admin/license/preview') {
      expect(request.method()).toBe('POST')
      expect(request.postData()).toBe(upload.buffer.toString())
      state.previews += 1
      return route.fulfill({ json: { activation: state.scheduled ? 'scheduled' : 'active', license: currentLicense } })
    }
    expect(path).toBe('/api/admin/license')
    if (request.method() === 'POST') {
      state.posts += 1
      expect(request.postData()).toBe(upload.buffer.toString())
      if (!state.installFails) {
        state.readFails = state.failReadAfterInstall
        if (state.state === 'missing') state.state = 'active'
        return route.fulfill({ json: { ok: true, activation: state.scheduled ? 'scheduled' : 'active', state: state.state, license: currentLicense } })
      }
    } else if (!state.readFails) {
      return route.fulfill({ json: { state: state.state, license: state.state === 'missing' ? null : currentLicense, scheduled_license: state.scheduled ? { ...currentLicense, license_id: 'scheduled_fixture', serial: 'AT-SCHEDULED-FIXTURE', not_before: '2027-01-01T00:00:00.000Z', expires_at: '2028-01-01T00:00:00.000Z' } : null, scheduled_state: state.scheduled ? 'waiting' : 'none', seat_usage: { occupied: 1, licensed: state.state === 'missing' ? null : 10 }, online_runners: 0 } })
    }
    return route.fulfill({ status: 503, json: { error: { code: 'BACKEND_LOCAL_STATE_FAILED', number: 91008, message: '服务端本地状态读取或写入失败，请检查文件与权限' } } })
  })
  return state
}

async function previewAndConfirm(page, label, scheduled = false) {
  await page.locator('input[type="file"]').setInputFiles(upload)
  await page.getByRole('button', { name: label, exact: true }).click()
  const dialog = page.getByRole('dialog', { name: scheduled ? '确认续期授权' : '确认切换授权' })
  await expect(dialog).toBeVisible()
  await dialog.getByRole('button', { name: scheduled ? '保存续期授权' : '立即切换' }).click()
}

for (const viewport of [{ width: 1366, height: 648 }, { width: 390, height: 720 }]) {
  test(`admin can retry license import when status reads fail at ${viewport.width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize(viewport)
    const state = await responses(page, { installFails: true })
    await page.goto('/license')
    await expect(page.getByRole('heading', { name: '暂时无法读取授权状态' })).toBeVisible()
    await expect(page.getByRole('button', { name: '重新读取状态' })).toBeEnabled()
    await page.screenshot({ path: testInfo.outputPath(`admin-recovery-${viewport.width}.png`), fullPage: true })
    await previewAndConfirm(page, '校验许可证')
    await expect.poll(() => state.previews).toBe(1)
    await expect.poll(() => state.posts).toBe(1)
    await expect(page.getByRole('heading', { name: '暂时无法读取授权状态' })).toBeVisible()
    await expect(page.getByRole('dialog', { name: '确认切换授权' }).getByRole('button', { name: '立即切换' })).toBeEnabled()
    state.installFails = false
    await page.getByRole('dialog', { name: '确认切换授权' }).getByRole('button', { name: '立即切换' }).click()
    await expect(page.locator('.current-license-card')).toHaveClass(/is-active/)
    await expect(page.getByRole('heading', { name: '暂时无法读取授权状态' })).toHaveCount(0)
    await expect(page.getByRole('status')).toContainText('已立即切换到新授权')
    await expect(page.getByRole('link', { name: /已授权/ })).toBeVisible()
    expect(state.posts).toBe(2)
  })
}

test('expired license recovery never claims activation', async ({ page }) => {
  await responses(page, { state: 'unavailable' })
  await page.goto('/license')
  await previewAndConfirm(page, '校验许可证')
  await expect(page.getByRole('heading', { name: '不可用', exact: true })).toBeVisible()
  await expect(page.getByRole('status')).toContainText('许可证已保存 请核对当前状态和有效期')
  await expect(page.getByText('许可证已验签、安装并立即生效', { exact: true })).toHaveCount(0)
})

test('failed readback clears old active state and retains recovery controls', async ({ page }) => {
  await responses(page, { readFails: false, failReadAfterInstall: true })
  await page.goto('/license')
  await expect(page.locator('.current-license-card')).toHaveClass(/is-active/)
  await previewAndConfirm(page, '校验并对比')
  await expect(page.getByRole('heading', { name: '暂时无法读取授权状态' })).toBeVisible()
  await expect(page.locator('.current-license-card')).toHaveCount(0)
  await expect(page.getByRole('button', { name: '重新读取状态' })).toBeEnabled()
  await expect(page.getByRole('status')).toContainText('许可证已保存 暂时无法确认当前授权状态')
})

test('first license installation retains the normal activation workflow', async ({ page }) => {
  await responses(page, { readFails: false, state: 'missing' })
  await page.goto('/license')
  await expect(page.getByRole('heading', { name: '导入授权' })).toBeVisible()
  await previewAndConfirm(page, '校验并安装')
  await expect(page.locator('.current-license-card')).toHaveClass(/is-active/)
  await expect(page.getByRole('link', { name: /已授权/ })).toBeVisible()
})

test('future license remains scheduled while the current license stays active', async ({ page }) => {
  await responses(page, { readFails: false, scheduled: true })
  await page.goto('/license')
  await previewAndConfirm(page, '校验并对比', true)
  await expect(page.locator('.current-license-card')).toHaveClass(/is-active/)
  await expect(page.getByRole('heading', { name: '已安排自动生效' })).toBeVisible()
  await expect(page.getByText('scheduled_fixture', { exact: true })).toBeVisible()
  await expect(page.getByRole('status')).toContainText('续期授权已保存 将在签名生效时间自动切换')
})
