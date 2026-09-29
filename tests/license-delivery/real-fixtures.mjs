import { expect, test as base } from '@playwright/test'
import { spawn } from 'node:child_process'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve, dirname, basename } from 'node:path'
import { fileURLToPath } from 'node:url'
import { setTimeout as delay } from 'node:timers/promises'
import { prepareRelease } from './release-fixture.mjs'

const root = fileURLToPath(new URL('../../', import.meta.url))
export const test = base.extend({
  capability: ['member', { option: true }],
  releaseFixture: [false, { option: true }],
  seedModels: [false, { option: true }],
  expiredLicense: [false, { option: true }],
  freeSwitch: [false, { option: true }],
  backend: async ({ capability, releaseFixture, seedModels, expiredLicense, freeSwitch }, use, testInfo) => {
    const binary = process.env.ASTER_LICENSE_BROWSER_FIXTURE
    if (!binary) throw new Error('Run npm run test:license-delivery:real to build the real fixture')
    const directory = await mkdtemp(join(tmpdir(), 'aster-license-browser-'))
    let release
    try {
      if (releaseFixture) release = await prepareRelease(root, directory)
    } catch (error) {
      await rm(directory, { recursive: true, force: true })
      throw error
    }
    const environment = { ...process.env }
    delete environment.ASTER_DEV_ASTERCTL_WINDOWS_X64
    delete environment.ASTER_RELEASE_TRUSTED_KEYS_JSON
    if (release) environment.ASTER_RELEASE_TRUSTED_KEYS_JSON = release.trusted_keys
    const readyFile = join(directory, 'ready.json')
    const child = spawn(binary, ['--ready-file', readyFile, '--admin-assets', join(root, 'customer/admin/dist'), '--member-assets', join(root, 'customer/member/dist'), '--capability', capability, ...(seedModels ? ['--seed-models'] : []), ...(expiredLicense ? ['--expired-license'] : []), ...(freeSwitch ? ['--free-switch'] : []), ...(release ? ['--install-root', release.install_root] : [])], {
      cwd: root, env: environment, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true,
    })
    let output = ''
    let failure
    child.stdout.on('data', data => { output += data })
    child.stderr.on('data', data => { output += data })
    child.on('error', error => { failure = error })
    child.stdin.on('error', error => { failure = error; output += `\nfixture stdin: ${error.message}` })
    const exited = new Promise(resolveExit => child.once('close', (code, signal) => resolveExit({ code, signal })))
    try {
      let ready
      const deadline = Date.now() + 45_000
      while (!ready && Date.now() < deadline) {
        if (failure || child.exitCode !== null || child.signalCode !== null) throw new Error(`Fixture startup failed: ${failure || output}`)
        try { ready = JSON.parse(await readFile(readyFile, 'utf8')) }
        catch (error) { if (error.code !== 'ENOENT') throw error }
        if (!ready) await delay(50)
      }
      if (!ready) throw new Error(`Fixture startup timed out: ${output}`)
      expect(ready.features).toEqual(capability === 'none' ? [] : [capability])
      await use({ ...ready, release_fixture: release })
    } finally {
      if (child.pid && child.exitCode === null && child.signalCode === null) child.stdin.end('\n')
      let result = await Promise.race([exited, delay(10_000, null, { ref: false })])
      const forced = !result
      if (!result && child.pid) {
        child.kill()
        result = await Promise.race([exited, delay(10_000, null, { ref: false })])
      }
      await testInfo.attach('real-control-output', { body: output, contentType: 'text/plain' })
      const target = resolve(directory)
      if (dirname(target) !== resolve(tmpdir()) || !basename(target).startsWith('aster-license-browser-')) throw new Error('Unexpected fixture cleanup path')
      await rm(target, { recursive: true, force: true })
      if (failure || forced || !result || result.code !== 0) throw new Error(`Fixture did not shut down cleanly: ${failure || JSON.stringify(result)} ${output}`)
    }
  },
})

async function submitAuthentication(page, path, button) {
  const completed = page.waitForResponse(response => new URL(response.url()).pathname === path && response.request().method() === 'POST')
  await page.getByRole('button', { name: button, exact: true }).click()
  expect((await completed).status()).toBe(200)
}

export async function ownerLogin(page, backend) {
  await page.goto(`${backend.admin_url}/login`)
  await page.getByLabel('管理员邮箱', { exact: true }).fill('owner@example.test')
  await page.getByLabel('密码', { exact: true }).fill('owner-password-strong')
  await submitAuthentication(page, '/api/admin/auth/login', '进入管理端')
  await expect(page).toHaveURL(/\/change-password$/)
  const locked = await page.request.get(`${backend.admin_url}/api/admin/vouchers/recipients`)
  expect(locked.headers()['x-aster-error-number']).toBe('11003')
  await page.getByLabel('当前临时密码', { exact: true }).fill('owner-password-strong')
  await page.getByLabel('新密码', { exact: true }).fill('owner-password-updated')
  await page.getByLabel('确认新密码', { exact: true }).fill('owner-password-updated')
  await submitAuthentication(page, '/api/admin/auth/password', '修改密码并重新登录')
  await expect(page).toHaveURL(/\/login\?/)
  await page.getByLabel('密码', { exact: true }).fill('owner-password-updated')
  await submitAuthentication(page, '/api/admin/auth/login', '进入管理端')
  await expect(page).toHaveURL(/\/overview$/)
}

export function captureFailures(page) {
  const failures = []
  page.on('response', response => {
    if (new URL(response.url()).pathname.startsWith('/api/') && response.status() >= 400) failures.push(`${response.status()} ${response.url()}`)
  })
  page.on('pageerror', error => failures.push(error.message))
  return failures
}

export async function createMember(page, backend, email, name) {
  await page.goto(`${backend.admin_url}/users`)
  await page.getByRole('button', { name: '新增成员', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('显示名称', { exact: true }).fill(name)
  await dialog.getByLabel('邮箱', { exact: true }).fill(email)
  await dialog.getByLabel('初始密码', { exact: true }).fill('member-password-strong')
  const created = page.waitForResponse(response => new URL(response.url()).pathname === '/api/admin/users' && response.request().method() === 'POST')
  await dialog.getByRole('button', { name: '创建成员', exact: true }).click()
  const response = await created
  expect(response.status()).toBe(201)
  await expect(dialog).toBeHidden()
  await expect(page.getByText(email, { exact: true })).toBeVisible()
  return response.json()
}

export async function memberLogin(member, backend, email) {
  await member.goto(`${backend.member_url}/login`)
  await member.getByLabel('成员邮箱', { exact: true }).fill(email)
  await member.getByLabel('密码', { exact: true }).fill('member-password-strong')
  await submitAuthentication(member, '/api/member/auth/login', '进入用户端')
  await expect(member).toHaveURL(/\/change-password$/)
  await member.getByLabel('当前初始密码', { exact: true }).fill('member-password-strong')
  await member.getByLabel('新密码', { exact: true }).fill('member-password-updated')
  await member.getByLabel('确认新密码', { exact: true }).fill('member-password-updated')
  await submitAuthentication(member, '/api/member/auth/password', '修改密码并重新登录')
  await expect(member).toHaveURL(/\/login\?/)
  await member.getByLabel('密码', { exact: true }).fill('member-password-updated')
  await submitAuthentication(member, '/api/member/auth/login', '进入用户端')
  await expect(member).toHaveURL(/\/home$/)
}

export async function createKey(member, name) {
  await member.getByRole('button', { name: '创建新 Key', exact: true }).click()
  await member.getByLabel('用途名称', { exact: true }).fill(name)
  const creation = member.waitForResponse(response => new URL(response.url()).pathname === '/api/member/keys' && response.request().method() === 'POST')
  await member.getByRole('button', { name: '创建 Key', exact: true }).click()
  const response = await creation
  expect(response.status()).toBe(201)
  await expect(member.getByText('完整 Key 只显示这一次，请立即保存。', { exact: true })).toBeVisible()
  await member.getByRole('button', { name: '我已保存，关闭', exact: true }).click()
  await expect(member.getByRole('dialog')).toBeHidden()
  await expect(member.getByText(name, { exact: true })).toBeVisible()
  return response.json()
}

export async function importLicense(page, path) {
  await page.locator('input[type="file"]').setInputFiles(path)
  const submitted = page.waitForResponse(response => new URL(response.url()).pathname === '/api/admin/license' && response.request().method() === 'POST')
  await page.getByRole('button', { name: '校验并更新', exact: true }).click()
  const response = await submitted
  await expect(page.getByRole('heading', { name: '有效', exact: true })).toBeVisible()
  return response
}
