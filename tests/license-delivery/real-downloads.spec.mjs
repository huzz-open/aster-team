import { expect } from '@playwright/test'
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFile, writeFile } from 'node:fs/promises'
import { test, ownerLogin, memberLogin, createMember, captureFailures, importLicense } from './real-fixtures.mjs'

test.use({ releaseFixture: true })

test('member-only documentation downloads the signed installed executable and rejects invalid artifacts and lost entitlement', async ({ page, browser, request, backend }, testInfo) => {
  const release = backend.release_fixture
  const artifactsPath = '/api/member/asterctl/artifacts'
  const downloadPath = `${artifactsPath}/${release.id}/download`
  const context = await browser.newContext()
  const member = await context.newPage()
  try {
    await ownerLogin(page, backend)
    await createMember(page, backend, 'downloads@example.test', 'Download Member')
    await memberLogin(member, backend, 'downloads@example.test')
    const failures = captureFailures(member)
    const paths = ['/api/member/docs?locale=zh-CN', artifactsPath, downloadPath]
    for (const path of paths) {
      expect((await request.get(`${backend.member_url}${path}`)).status()).toBe(401)
      expect((await page.request.get(`${backend.member_url}${path}`)).status()).toBe(401)
    }
    const metadata = await member.request.get(`${backend.member_url}${artifactsPath}`)
    expect(metadata.status()).toBe(200)
    const { artifacts } = await metadata.json()
    expect(artifacts).toHaveLength(1)
    expect(artifacts[0]).toMatchObject({
      id: release.id, version: release.version, file_name: 'asterctl.exe',
      sha256: release.sha256, size_bytes: release.size_bytes, download_url: downloadPath,
    })
    expect((await member.request.get(`${backend.member_url}${artifactsPath}/unknown/download`)).status()).toBe(404)
    const documentation = await member.request.get(`${backend.member_url}/api/member/docs?locale=zh-CN`)
    expect(documentation.status()).toBe(200)
    const docs = await documentation.json()
    expect(docs.models).toEqual([{ id: 'gpt-5.6-sol', display_name: 'Browser Docs Model' }])
    const settingsPath = `/api/member/claude-cli/settings?version=${encodeURIComponent(docs.claude.compatibility.minimum_version)}`
    paths.push(settingsPath)
    const settingsResponse = await member.request.get(`${backend.member_url}${settingsPath}`)
    expect(settingsResponse.status()).toBe(200)
    const settings = await settingsResponse.json()
    expect(settings.supported).toBe(true)
    expect(settings.settings).toContain('gpt-5.6-sol')
    expect(settings.settings).toContain(docs.base_urls.anthropic)
    await member.goto(`${backend.member_url}/docs`)
    await member.getByRole('button', { name: /^客户端接入：/ }).click()

    async function downloadAndVerify(button, label) {
      const downloaded = member.waitForEvent('download')
      const responded = member.waitForResponse(response => new URL(response.url()).pathname === downloadPath)
      await button.click()
      const response = await responded
      expect(response.status()).toBe(200)
      expect(response.headers()).toMatchObject({
        'content-type': 'application/octet-stream',
        'content-disposition': 'attachment; filename="asterctl.exe"',
        'content-length': String(release.size_bytes),
        etag: `"sha256:${release.sha256}"`, 'cache-control': 'private, no-cache',
      })
      const download = await downloaded
      expect(await download.failure()).toBeNull()
      expect(download.suggestedFilename()).toBe('asterctl.exe')
      const saved = testInfo.outputPath(`${label}-asterctl.exe`)
      await download.saveAs(saved)
      const bytes = await readFile(saved)
      expect(bytes.length).toBe(release.size_bytes)
      expect(createHash('sha256').update(bytes).digest('hex')).toBe(release.sha256)
      const result = spawnSync(saved, ['--version'], { encoding: 'utf8', windowsHide: true, timeout: 10_000 })
      expect(result.error).toBeUndefined()
      expect(result.status).toBe(0)
      expect(result.stdout.trim()).toBe(`asterctl ${release.version}`)
    }
    await downloadAndVerify(member.getByRole('button', { name: '下载 Windows x64 版 asterctl', exact: true }), 'primary')
    await member.locator('summary').filter({ hasText: '全部平台与版本' }).click()
    const table = member.getByRole('table')
    await expect(table.getByText(release.sha256, { exact: true })).toBeVisible()
    await expect(table.getByText(release.version, { exact: true })).toBeVisible()
    await downloadAndVerify(table.getByRole('button', { name: '下载', exact: true }), 'table')
    for (const client of ['Codex 客户端', 'Claude CLI']) {
      await member.getByRole('radio', { name: client, exact: true }).click()
      await expect(member.getByText('复制并运行初始化命令', { exact: true })).toBeVisible()
      await member.locator('summary').filter({ hasText: '手动配置' }).click()
      await expect(member.locator('.advanced-content')).toBeVisible()
      await expect(member.locator('.advanced-content .a-copy-code').first()).toBeVisible()
      await expect(member.locator('.advanced-content .a-copy-code button').first()).toBeEnabled()
    }
    expect(failures).toEqual([])
    await member.screenshot({ path: testInfo.outputPath('member-download-documentation.png'), fullPage: true })

    const originalArtifact = await readFile(release.artifact_path)
    const originalManifest = await readFile(release.manifest_path)
    const tampered = Buffer.from(originalArtifact)
    tampered[tampered.length - 1] ^= 1
    async function expectIntegrityRejection() {
      for (const path of [artifactsPath, downloadPath]) {
        const response = await member.request.get(`${backend.member_url}${path}`)
        expect(response.status()).toBe(503)
        expect(response.headers()['x-aster-error-number']).toBe('91002')
      }
    }
    try {
      await writeFile(release.artifact_path, tampered)
      await expectIntegrityRejection()
      await writeFile(release.artifact_path, originalArtifact)
      const changedManifest = JSON.parse(originalManifest)
      changedManifest.version = '99.0.0'
      await writeFile(release.manifest_path, JSON.stringify(changedManifest))
      await expectIntegrityRejection()
    } finally {
      await writeFile(release.artifact_path, originalArtifact)
      await writeFile(release.manifest_path, originalManifest)
    }
    const restored = await member.request.get(`${backend.member_url}${downloadPath}`)
    expect(restored.status()).toBe(200)
    expect(createHash('sha256').update(await restored.body()).digest('hex')).toBe(release.sha256)
    await page.goto(`${backend.admin_url}/license`)
    expect((await importLicense(page, backend.license_files.without_member)).status()).toBe(200)
    for (const path of paths) {
      const response = await member.request.get(`${backend.member_url}${path}`)
      expect(response.status()).toBe(403)
      expect(response.headers()['x-aster-error-number']).toBe('51008')
    }
    await member.goto(`${backend.member_url}/docs`)
    await expect(member).toHaveURL(/\/license-required\?reason=feature$/)
    await expect(member.getByRole('button', { name: '下载 Windows x64 版 asterctl', exact: true })).toHaveCount(0)
    await testInfo.attach('verified-download', { body: JSON.stringify({ version: release.version, sha256: release.sha256, size_bytes: release.size_bytes }), contentType: 'application/json' })
  } finally {
    await context.close()
  }
})
