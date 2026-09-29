import { test, expect } from '@playwright/test'
import { createHash, randomBytes } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { mkdirSync, readFileSync } from 'node:fs'
import { dirname, join, resolve, basename, toNamespacedPath } from 'node:path'
import { fileURLToPath } from 'node:url'
import { buildWebsiteFunctions, createWebsiteRuntime } from './runtime.mjs'
import { createSystemCommandRunner } from '../../tests/system-e2e/command-runner.mjs'
import { resolveCiImage } from '../../scripts/ci/ci-images.mjs'
import { verifyArchive } from '../../scripts/ci/verify-static-archive.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const inputs = JSON.parse(readFileSync(process.env.ASTER_WEBSITE_RELEASE_TEST_INPUTS, 'utf8'))
const hash = bytes => createHash('sha256').update(bytes).digest('hex')
let app
test.beforeAll(async () => {
  app = await createWebsiteRuntime({ scriptPath: buildWebsiteFunctions(), port: 26991,
    origin: 'http://127.0.0.1:26991', staticRoot: inputs.site_root })
})
test.afterAll(async () => { await app?.close() })

function verifyDownloadedPackage(path) {
  expect(verifyArchive(path)).toHaveLength(4)
  const bundle = inputs.artifact.name.slice(0, -'.tar.gz'.length)
  const extraction = spawnSync('tar', ['-xzOf', `./${basename(path)}`, `${bundle}/client-tools/asterctl/windows-x86_64/asterctl.exe`],
    { cwd: dirname(path), windowsHide: true, maxBuffer: 64 * 1024 * 1024 })
  expect(extraction.status).toBe(0)
  expect(hash(extraction.stdout)).toBe(inputs.tool_sha256)
  const { executable, environment } = createSystemCommandRunner(root)
  const docker = executable('docker')
  const name = `aster-website-verify-${randomBytes(6).toString('hex')}`
  const args = ['run', '--rm', '--name', name, '--network', 'none', '--read-only',
    '--mount', `type=bind,source=${dirname(path)},target=/input,readonly`, '--tmpfs', '/verify:rw,exec,size=512m',
    resolveCiImage('ubuntu-22.04', 'runtime').image, 'sh', '-ec',
    'tar -xzf "$1" -C /verify; "/verify/$2/bin/aster-team-cli" verify-release --root "/verify/$2"',
    'website-package-check', `/input/${basename(path)}`, bundle]
  const result = spawnSync(docker, args, { cwd: root, env: environment, windowsHide: true, encoding: 'utf8', timeout: 90_000 })
  if (result.error) {
    // A timed-out Docker client may leave its owned container alive.
    const cleanup = spawnSync(docker, ['rm', '-f', name], { cwd: root, env: environment, windowsHide: true, encoding: 'utf8', timeout: 15_000 })
    throw new Error(`Package verification failed: ${result.error.message}; cleanup exit ${cleanup.status}${cleanup.error ? `: ${cleanup.error.message}` : ''}`)
  }
  expect(result.status, result.stderr).toBe(0)
  expect(result.stdout).toContain(`Verified Aster Team ${inputs.version} (amd64) signed by ${inputs.release_manifest.key_id}.`)
}

test('local production website delivers the actual installed package and canonical manuals', async ({ page }, testInfo) => {
  const errors = []
  page.on('pageerror', error => errors.push(error.message))
  await page.route('**/*', route => new URL(route.request().url()).origin === app.origin ? route.continue() : route.abort('blockedbyclient'))
  await page.goto('/')
  const releaseResponse = await page.request.get('/product-release.json')
  expect(releaseResponse.status()).toBe(200)
  const { state, release } = await releaseResponse.json()
  expect(state).toBe('configured')
  expect(release.environment).toBe('local')
  expect(release.version).toBe(inputs.version)
  expect(release.artifact).toMatchObject(inputs.artifact)
  expect(release.free_plan).toEqual({ plan_id: inputs.bundled_license.plan_id, plan_version: inputs.bundled_license.plan_version,
    license_id: inputs.bundled_license.license_id, license_sha256: inputs.bundled_license.sha256 })
  const manifest = await (await page.request.get('/catalog-manifest.json')).json()
  const catalogResponse = await page.request.get(manifest.catalog.path)
  expect(await catalogResponse.body()).toEqual(readFileSync(inputs.catalog_path))
  const catalog = JSON.parse(readFileSync(inputs.catalog_path, 'utf8'))
  expect(catalog.plans[0].entitlements).toEqual({ ...inputs.bundled_license.entitlements, quotas: [...inputs.bundled_license.entitlements.quotas].sort((a, b) => a.id.localeCompare(b.id)) })

  for (const viewport of [{ width: 1366, height: 648 }, { width: 390, height: 744 }]) {
    await page.setViewportSize(viewport)
    await page.goto('/')
    if (viewport.width < 760) await page.getByRole('button', { name: '打开导航', exact: true }).click()
    await page.getByRole('navigation').getByRole('button', { name: '下载与价格', exact: true }).click()
    await expect(page.locator('.plan-card')).toHaveCount(4)
    for (const [id, amount, total] of [['local_review_20', '5,999', '20,996.5'], ['local_review_50', '9,999', '34,996.5']]) {
      const card = page.locator(`[data-plan-id="${id}"]`)
      await expect(card).toContainText('订阅期间包含标准功能更新与升级')
      await expect(card.locator('.plan-price')).toContainText(amount)
      await card.getByRole('button', { name: '5 年', exact: true }).click()
      await expect(card.locator('.plan-price')).toContainText(total)
      await card.getByRole('button', { name: '1 年', exact: true }).click()
    }
    await expect(page.locator('[data-plan-id="local_review_custom"] .plan-quotas dd')).toHaveText(['按报价确定', '不限量', '不限量', '不限量'])
    const releaseCard = page.locator('.release-card:not(.windows-release-card)')
    await expect(releaseCard).toHaveAttribute('data-release-version', inputs.version)
    await expect(releaseCard).toContainText('从免费开始，随团队成长')
    await expect(releaseCard).toContainText('无需重新部署')
    await expect(releaseCard).toContainText(inputs.artifact.sha256)
    const link = viewport.width > 760
      ? releaseCard.getByRole('link', { name: '下载安装包', exact: true })
      : page.locator('.plan-card.is-free').getByRole('link', { name: '下载安装包', exact: true })
    await link.scrollIntoViewIfNeeded()
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    await page.screenshot({ path: testInfo.outputPath(`website-release-${viewport.width}x${viewport.height}.png`) })
    const downloaded = page.waitForEvent('download')
    await link.click()
    const download = await downloaded
    expect(await download.failure()).toBeNull()
    expect(download.suggestedFilename()).toBe(inputs.artifact.name)
    const saved = testInfo.outputPath(String(viewport.width), inputs.artifact.name)
    await download.saveAs(saved)
    const bytes = readFileSync(saved)
    expect(bytes.length).toBe(inputs.artifact.size_bytes)
    expect(hash(bytes)).toBe(inputs.artifact.sha256)
    verifyDownloadedPackage(saved)
    // Read the actual public files through the same links users receive.
    for (const [label, relativePath] of [
      ['查看用户手册', 'docs/user-manual.md'], ['Linux 安装说明', 'README-LINUX.md'],
      ['校验文件', `releases/${inputs.version}/SHA256SUMS`],
    ]) {
      const url = await releaseCard.getByRole('link', { name: label, exact: true }).getAttribute('href')
      expect(url).toBeTruthy()
      const response = await page.request.get(url)
      expect(response.status()).toBe(200)
      expect(await response.body()).toEqual(readFileSync(join(inputs.support_root, relativePath)))
      if (!relativePath.startsWith('releases/')) expect(await response.body()).toEqual(readFileSync(join(root, relativePath)))
    }
  }
  if (inputs.windows) {
    const windows = inputs.windows
    expect(release.windows.artifact).toMatchObject(windows.artifact)
    expect(release.windows.free_plan).toMatchObject({ plan_id: inputs.bundled_license.plan_id, plan_version: inputs.bundled_license.plan_version })
    const card = page.locator('.windows-release-card')
    await expect(card).toContainText('不承诺稳定')
    await expect(card).toContainText(windows.artifact.sha256)
    const downloading = page.waitForEvent('download')
    await card.getByRole('link', { name: '下载 Windows 实验版', exact: true }).click()
    const download = await downloading
    expect(await download.failure()).toBeNull()
    expect(download.suggestedFilename()).toBe(windows.artifact.name)
    const saved = testInfo.outputPath('windows', windows.artifact.name)
    await download.saveAs(saved)
    const bytes = readFileSync(saved)
    expect(bytes.length).toBe(windows.artifact.size_bytes)
    expect(hash(bytes)).toBe(windows.artifact.sha256)
    const extract = testInfo.outputPath('windows', 'extracted')
    mkdirSync(extract, { recursive: true })
    const unpacked = spawnSync('tar', ['-xzf', `./${basename(saved)}`, '-C', extract], { cwd: dirname(saved), encoding: 'utf8', windowsHide: true, timeout: 60_000 })
    expect(unpacked.status, unpacked.stderr).toBe(0)
    const bundle = join(extract, windows.artifact.name.slice(0, -'.tar.gz'.length))
    const verified = spawnSync(toNamespacedPath(join(bundle, 'bin/aster-team-cli.exe')), ['verify-release', '--root', bundle],
      { encoding: 'utf8', windowsHide: true, timeout: 90_000 })
    expect(verified.error).toBeUndefined()
    expect(verified.status, verified.stderr).toBe(0)
    expect(verified.stdout).toContain(`signed by ${windows.release_manifest.key_id}`)
    expect(hash(readFileSync(join(bundle, 'licenses/free-license.json')))).toBe(windows.bundled_license.sha256)
    for (const [label, relativePath] of [['Windows 安装说明', 'README-WINDOWS.md'], ['校验文件', `releases/${inputs.version}/SHA256SUMS`]]) {
      const url = await card.getByRole('link', { name: label, exact: true }).getAttribute('href')
      expect(url).toBeTruthy()
      const response = await page.request.get(url)
      expect(response.status()).toBe(200)
      expect(await response.body()).toEqual(readFileSync(join(windows.support_root, relativePath)))
    }
  }
  expect(errors).toEqual([])
  expect(app.outbound).toEqual([])
})
