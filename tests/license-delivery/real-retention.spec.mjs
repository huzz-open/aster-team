import { expect } from '@playwright/test'
import { test, memberLogin, captureFailures } from './real-fixtures.mjs'

test.use({ expiredLicense: true })

test('expired paid subscription retains pages and password recovery while disabling new usage', async ({ page, context, backend }, testInfo) => {
  const failures = captureFailures(page)
  await page.goto(`${backend.admin_url}/login`)
  await page.getByLabel('管理员邮箱', { exact: true }).fill('owner@example.test')
  await page.getByLabel('密码', { exact: true }).fill('owner-password-strong')
  await page.getByRole('button', { name: '进入管理端', exact: true }).click()
  await expect(page).toHaveURL(/\/overview$/)
  await page.goto(`${backend.admin_url}/users`)
  await expect(page.getByText('retained@example.test', { exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: '新增成员', exact: true })).toBeDisabled()
  await expect(page.getByRole('button', { name: '批量创建', exact: true })).toBeDisabled()
  await expect(page.getByRole('button', { name: '禁用成员', exact: true })).toBeEnabled()
  await expect(page.getByRole('status').filter({ hasText: '订阅已到期' })).toBeVisible()
  await page.screenshot({ path: testInfo.outputPath('expired-admin.png'), fullPage: true })
  await page.goto(`${backend.admin_url}/consumption-logs`)
  await expect(page.getByRole('heading', { name: '全团队消费日志', exact: true })).toBeVisible()
  await page.goto(`${backend.admin_url}/license`)
  await expect(page.getByRole('heading', { name: '已到期', exact: true })).toBeVisible()

  const member = await context.newPage()
  const memberFailures = captureFailures(member)
  await memberLogin(member, backend, 'retained@example.test')
  await member.goto(`${backend.member_url}/keys`)
  await expect(member.getByRole('button', { name: '创建新 Key', exact: true })).toBeDisabled()
  await expect(member.getByRole('status').filter({ hasText: '订阅已到期' })).toBeVisible()
  await member.screenshot({ path: testInfo.outputPath('expired-member.png'), fullPage: true })
  for (const path of ['usage', 'logs', 'quota', 'docs', 'account']) {
    await member.goto(`${backend.member_url}/${path}`)
    await expect(member).toHaveURL(new RegExp(`/${path}$`))
  }
  expect(failures).toEqual([])
  expect(memberFailures).toEqual([])
  const denied = await member.request.post(`${backend.member_url}/api/member/keys`, { data: { name: 'forbidden after expiry' } })
  expect(denied.status()).toBe(403)
  expect(denied.headers()['x-aster-error-number']).toBe('51002')
})
