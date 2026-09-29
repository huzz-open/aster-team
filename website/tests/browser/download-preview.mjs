import { expect } from '@playwright/test'

// Release responses and installer bytes are synthetic browser fixtures.
export async function checkDownloads(browser, origin) {
  const page = await browser.newPage({ viewport: { width: 390, height: 844 } })
  const filename = 'aster-team-2.1.0-linux-amd64.tar.gz'
  const assetURL = `https://github.com/huzz-open/aster-team/releases/download/v2.1.0/${filename}`
  const catalog = { schema: 'aster.website-releases.v1', latest: 'v2.1.0', releases: [
    { tag: 'v2.1.0', version: '2.1.0', filename, url: assetURL, sha256: 'a'.repeat(64), size: 16 },
  ] }
  let status = 200
  let calls = 0
  await page.route('**/api/releases', route => {
    calls++
    return route.fulfill({ status, json: status === 200 ? catalog : { error: 'release_metadata_unavailable' } })
  })
  await page.route(assetURL, route => route.fulfill({ contentType: 'application/gzip', headers: { 'Content-Disposition': `attachment; filename="${filename}"` }, body: 'download fixture' }))
  try {
    await page.goto(origin)
    await expect(page.locator('a.latest-download-button')).toHaveAttribute('href', assetURL)
    await expect(page.getByTestId('install-version')).toContainText('v2.1.0')
    const download = page.waitForEvent('download')
    await page.locator('a.latest-download-button').click()
    expect((await download).suggestedFilename()).toBe(filename)
    expect(calls).toBe(1)
    for (const language of ['Switch to English', '切换为简体中文']) {
      await page.getByRole('button', { name: language, exact: true }).click()
      await expect(page.locator('a.latest-download-button')).toHaveAttribute('href', assetURL)
    }
    expect(calls).toBe(1)
    status = 503
    await page.reload()
    await expect(page.locator('a.latest-download-button')).toHaveAttribute('href',
      'https://github.com/huzz-open/aster-team/releases/download/v2.1.1/aster-team-2.1.1-linux-amd64.tar.gz')
    await expect(page.getByTestId('install-version')).toContainText('v2.1.1')
    await expect(page.locator('.catalog-install [role="status"]')).toHaveCount(0)
  } finally { await page.close() }

  // A LAN HTTP address has no Async Clipboard API; Copy must still work from a click.
  const localURL = new URL(origin)
  const lanOrigin = `http://aster.test${localURL.port ? `:${localURL.port}` : ''}`
  const lanPage = await browser.newPage()
  await lanPage.route(`${lanOrigin}/**`, async route => {
    const url = new URL(route.request().url())
    if (url.pathname === '/api/releases') return route.fulfill({ status: 503, json: { error: 'release_metadata_unavailable' } })
    const response = await route.fetch({ url: new URL(url.pathname + url.search, origin).href })
    return route.fulfill({ response })
  })
  try {
    await lanPage.goto(lanOrigin)
    expect(await lanPage.evaluate(() => isSecureContext || !!navigator.clipboard)).toBe(false)
    const command = await lanPage.locator('.catalog-install code').textContent()
    await lanPage.locator('.catalog-install button').click()
    await lanPage.evaluate(() => {
      const field = document.createElement('textarea')
      field.id = 'clipboard-probe'
      document.body.appendChild(field)
      field.focus()
    })
    await lanPage.keyboard.press('ControlOrMeta+V')
    await expect(lanPage.locator('#clipboard-probe')).toHaveValue(command)
    await expect(lanPage.locator('.catalog-install [role="alert"]')).toHaveCount(0)
  } finally {
    await lanPage.unrouteAll({ behavior: 'wait' })
    await lanPage.close()
  }
}
