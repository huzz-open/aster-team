import { test, expect } from '@playwright/test'
import { checkDemo } from './demo-scenarios.mjs'
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync, mkdirSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { buildWebsiteFunctions, createWebsiteRuntime } from './runtime.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const original = readFileSync(resolve(root, 'contracts/test-vectors/public-catalog.v1.json'))
const catalog = JSON.parse(original)
const journalKey = 'aster:product-inquiry-pending:v1'
const screenshots = resolve(root, 'dist/website-validation/screenshots')
const captchaScript = `window.testChallenges = []; window.turnstile = {
  render(element, options) { const id = String(window.testChallenges.length); window.testChallenges.push(options); element.textContent = 'Local verification fixture'; return id },
  remove() {}, reset() {}
}`
let app

test.beforeAll(async () => {
  test.setTimeout(90_000)
  execFileSync(process.execPath, [resolve(root, 'website/tests/build-site.mjs'), '--matrix'], { cwd: root, stdio: 'pipe', windowsHide: true })
  app = await createWebsiteRuntime({ scriptPath: buildWebsiteFunctions(), port: 26990, staticRoot: resolve(root, 'dist/website-validation/site') })
  mkdirSync(screenshots, { recursive: true })
})
test.afterAll(async () => { await app?.close() })
test.beforeEach(async ({ page }) => {
  await app.database.exec('DELETE FROM trial_rate_limits;')
  await page.route('**/*', route => {
    const url = new URL(route.request().url())
    if (url.origin === app.origin) return route.continue()
    if (url.hostname === 'challenges.cloudflare.com' && url.pathname === '/turnstile/v0/api.js') return route.fulfill({ contentType: 'application/javascript', body: captchaScript })
    return route.abort('blockedbyclient')
  })
})
const inquiry = page => page.locator('.product-inquiry')
async function fillInquiry(page, contact = 'browser@example.invalid') {
  await inquiry(page).locator('input[name="contact"]').fill(contact)
  await inquiry(page).locator('textarea').fill('本地浏览器验证私有部署咨询')
}

async function solve(page, index) {
  await expect.poll(() => page.evaluate(() => window.testChallenges?.length ?? 0)).toBeGreaterThan(index)
  await page.evaluate(i => window.testChallenges[i].callback('test-token-browser'), index)
}
async function saved(id) { return app.database.prepare('SELECT * FROM product_inquiries WHERE id = ?').bind(id).first() }

test('approved bytes, visible terms and saved inquiry reference agree across the production page', async ({ page }) => {
  const errors = []
  page.on('pageerror', error => errors.push(error.message))
  await page.route('**/api/releases', route => route.fulfill({ json: {
    schema: 'aster.website-releases.v1', latest: 'v2.1.1', releases: [{
      tag: 'v2.1.1', version: '2.1.1', filename: 'aster-team-2.1.1-linux-amd64.tar.gz',
      url: 'https://github.com/huzz-open/aster-team/releases/download/v2.1.1/aster-team-2.1.1-linux-amd64.tar.gz',
      sha256: 'a'.repeat(64), size: 12345,
    }],
  } }))
  await page.goto('/')
  const github = page.locator('.header-github')
  await expect(github).toHaveAttribute('href', 'https://github.com/huzz-open/aster-team')
  await expect(github).toHaveAttribute('target', '_blank')
  expect(await github.evaluate(element => getComputedStyle(element).borderTopWidth)).toBe('0px')
  await github.hover()
  expect(await github.evaluate(element => getComputedStyle(element).transform)).toBe('none')
  const headerDownload = page.locator('.header-download')
  await expect(headerDownload).toHaveText('Download free')
  await headerDownload.hover()
  expect(await headerDownload.evaluate(element => getComputedStyle(element).transform)).toBe('none')
  await page.getByRole('button', { name: '切换为简体中文', exact: true }).click()
  await expect(page.locator('.map-flow article').nth(1).locator('small')).toHaveText(/^(数据库|Database)$/)
  await expect(page.locator('.calculator-section,.usage-assessment')).toHaveCount(0)
  await expect(page.locator('.site-header nav button')).toHaveCount(4)
  const manifest = await (await page.request.get('/catalog-manifest.json')).json()
  const bytes = await (await page.request.get(manifest.catalog.path)).body()
  expect(bytes).toEqual(original)
  expect(createHash('sha256').update(bytes).digest('hex')).toBe(manifest.catalog.sha256)
  const productRelease = await (await page.request.get('/product-release.json')).json()
  expect(productRelease.state).toBe('configured')
  expect(productRelease.release.environment).toBe('local')
  const archiveResponse = await page.request.get(productRelease.release.artifact.url)
  const archive = await archiveResponse.body()
  expect(archive.length).toBe(productRelease.release.artifact.size_bytes)
  expect(createHash('sha256').update(archive).digest('hex')).toBe(productRelease.release.artifact.sha256)
  expect(await (await page.request.get(productRelease.release.documents.user_manual)).text()).toBe('# Aster Team browser test manual\n')
  await expect(page.locator('meta[name="aster-catalog-revision"]')).toHaveAttribute('content', catalog.revision)
  await expect(page.locator('meta[name="aster-catalog-sha256"]')).toHaveAttribute('content', manifest.catalog.sha256)
  await page.getByRole('navigation').getByRole('button', { name: '下载与价格' }).click()
  await expect(page.locator('.latest-download-button')).toHaveCount(0)
  await expect(page.locator('.catalog-install')).toContainText(`curl -fsSL ${new URL(page.url()).origin}/install.sh | bash`)
  await expect(page.locator('.command-options')).toHaveAttribute('open', '')
  await page.getByTestId('install-version').getByRole('combobox').click()
  await page.getByRole('option', { name: 'v2.1.1', exact: true }).click()
  await page.getByTestId('install-email').fill('owner@example.com')
  await page.getByTestId('install-protocol').getByRole('combobox').click()
  await page.getByRole('option', { name: 'HTTPS', exact: true }).click()
  await page.getByTestId('install-host').fill('team.example.com')
  await expect(page.locator('.catalog-install code')).toContainText('bash -s -- --version v2.1.1 --email owner@example.com --protocol https --host team.example.com')
  await expect(page.locator('.download-links')).toHaveCount(0)
  await expect(page.locator('.release-card:not(.windows-release-card)')).toHaveAttribute('data-release-version', productRelease.release.version)
  await expect(page.locator('.release-card:not(.windows-release-card)')).toContainText('从免费开始，随团队成长')
  await expect(page.locator('.release-card:not(.windows-release-card)')).toContainText('无需重新部署')
  await expect(page.locator('.release-card:not(.windows-release-card)')).toContainText(productRelease.release.artifact.sha256)
  await expect(page.locator('.release-card:not(.windows-release-card)').getByRole('link', { name: '下载安装包' })).toHaveAttribute('href', productRelease.release.artifact.url)
  await expect(page.locator('.plan-card')).toHaveCount(catalog.plans.length)
  const card = page.locator(`[data-plan-id="${catalog.plans[1].plan_id}"]`)
  await expect(card.locator('.plan-description')).toHaveText(catalog.plans[1].description)
  await expect(card.locator('.plan-description strong')).toHaveCount(0)
  await card.getByRole('button', { name: '2 年', exact: true }).click()
  await expect(card.locator('.plan-price strong')).toHaveText('¥10,798.2')
  await page.locator('.plan-comparison summary').focus()
  await page.keyboard.press('Enter')
  await expect(page.locator('.comparison-scroll')).toBeVisible()
  await card.getByRole('button', { name: '咨询采购' }).click()
  await expect(inquiry(page).locator('input[name="contact"]')).toBeFocused()
  await fillInquiry(page)
  await inquiry(page).getByRole('button', { name: '发送咨询', exact: true }).click()
  const response = page.waitForResponse(response => response.url().endsWith('/api/inquiries'))
  await solve(page, 0)
  const accepted = await (await response).json()
  await expect(inquiry(page).getByRole('heading', { name: '咨询已收到' })).toBeVisible()
  const row = await saved(accepted.id)
  expect(JSON.parse(row.catalog_reference_json)).toEqual({ catalog_revision: catalog.revision, plan_id: catalog.plans[1].plan_id, plan_version: 1, years: 2 })
  expect(row.reference_status).toBe('unverified')
  expect(row.notification_status).toBe('not_configured')
  expect(await page.evaluate(key => sessionStorage.getItem(key), journalKey)).toBeNull()
  expect(errors).toEqual([])
})

test('lost acknowledgement survives language changes and refresh without another inquiry', async ({ page }) => {
  await page.goto('/')
  await page.getByRole('button', { name: '切换为简体中文', exact: true }).click()
  await page.locator(`[data-plan-id="${catalog.plans[1].plan_id}"]`).getByRole('button', { name: '咨询采购' }).click()
  await fillInquiry(page, 'recovery@example.invalid')
  let firstId
  await page.route('**/api/inquiries', async route => {
    firstId = route.request().postDataJSON().request_id
    const response = await route.fetch({ headers: { 'Content-Type': 'application/json', Origin: app.origin }, postData: route.request().postDataJSON(), timeout: 10_000 })
    expect(response.status()).toBe(202)
    await route.abort('failed')
  }, { times: 1 })
  await inquiry(page).getByRole('button', { name: '发送咨询', exact: true }).click()
  await solve(page, 0)
  await expect(inquiry(page).getByRole('alert')).toContainText('结果暂时无法确认')
  await expect(inquiry(page).locator('input[name="contact"]')).toBeDisabled()
  const first = await saved(firstId)
  await page.getByRole('button', { name: 'Switch to English', exact: true }).click()
  await expect(inquiry(page).getByRole('button', { name: 'Check original inquiry' })).toBeEnabled()
  await page.reload()
  await expect(inquiry(page).locator('input[name="contact"]')).toHaveValue('recovery@example.invalid')
  await page.getByRole('button', { name: '切换为简体中文', exact: true }).click()
  await expect(page.locator(`[data-plan-id="${catalog.plans[1].plan_id}"]`).getByRole('button', { name: '咨询采购' })).toBeDisabled()
  await page.getByRole('button', { name: 'Switch to English', exact: true }).click()
  await inquiry(page).getByRole('button', { name: 'Check original inquiry' }).click()
  const response = page.waitForResponse(response => response.url().endsWith('/api/inquiries'))
  await solve(page, 0)
  expect((await (await response).json()).id).toBe(firstId)
  await expect(inquiry(page).getByRole('heading', { name: 'Inquiry received' })).toBeVisible()
  expect(await saved(firstId)).toEqual(first)
  expect(await app.database.prepare('SELECT COUNT(*) AS count FROM product_inquiries WHERE contact = ?').bind('recovery@example.invalid').first('count')).toBe(1)
  expect(await page.evaluate(key => sessionStorage.getItem(key), journalKey)).toBeNull()
})

test('failed script can retry and callbacks from a cancelled widget cannot submit', async ({ page }) => {
  await page.route('https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit', route => route.abort('failed'), { times: 1 })
  await page.goto('/')
  await page.getByRole('button', { name: '切换为简体中文', exact: true }).click()
  await fillInquiry(page, 'captcha@example.invalid')
  await inquiry(page).getByRole('button', { name: '发送咨询', exact: true }).click()
  await expect(inquiry(page).getByRole('alert')).toContainText('安全验证未完成')
  await inquiry(page).getByRole('button', { name: '发送咨询', exact: true }).click()
  await expect.poll(() => page.evaluate(() => window.testChallenges?.length ?? 0)).toBe(1)
  await inquiry(page).getByRole('button', { name: '取消验证' }).click()
  await inquiry(page).getByRole('button', { name: '发送咨询', exact: true }).click()
  await expect.poll(() => page.evaluate(() => window.testChallenges?.length ?? 0)).toBe(2)
  await solve(page, 0)
  await page.evaluate(() => window.testChallenges[0]['error-callback']('late-error'))
  await expect(inquiry(page).getByRole('button', { name: '完成安全验证后提交' })).toBeDisabled()
  expect(await app.database.prepare('SELECT COUNT(*) AS count FROM product_inquiries WHERE contact = ?').bind('captcha@example.invalid').first('count')).toBe(0)
  await solve(page, 1)
  await expect(inquiry(page).getByRole('heading', { name: '咨询已收到' })).toBeVisible()
})

test('existing trial form still completes with the shared verification widget and backend', async ({ page }) => {
  await page.goto('/')
  const form = page.locator('.lead-form')
  await form.locator('input[autocomplete="email tel"]').fill('legacy-browser@example.invalid')
  await form.locator('button[type="submit"]').click()
  const response = page.waitForResponse(response => response.url().endsWith('/api/trial'))
  await solve(page, 0)
  const result = await (await response).json()
  expect(result.ok).toBe(true)
  await expect(page.locator('.lead-success-dialog')).toBeVisible()
  const row = await app.database.prepare('SELECT * FROM trial_leads WHERE id = ?').bind(result.id).first()
  expect(row.contact).toBe('legacy-browser@example.invalid')
  expect(row.notification_status).toBe('not_configured')
})

test('gateway hero explains the product and leads into the tour without excess whitespace', async ({ page }) => {
  await page.goto('/')
  for (const viewport of [{ width: 1920, height: 1080 }, { width: 1366, height: 768 }, { width: 390, height: 844 }]) {
    await page.setViewportSize(viewport)
    for (const language of ['切换为简体中文', 'Switch to English']) {
      await page.getByRole('button', { name: language, exact: true }).click()
      await expect(page.locator('.hero-marquee')).toHaveCount(0)
      await expect(page.locator('.hero-gateway')).toBeVisible()
      await expect(page.locator('.gateway-values > div')).toHaveCount(3)
      await expect(page.locator('.gateway-endpoints li')).toHaveCount(6)
      await expect(page.locator('.hero-download-note')).toHaveCount(0)
      await expect(page.locator('.catalog-heading')).toContainText(language === 'Switch to English' ? 'free license included' : '内置免费授权')
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
      expect(await page.locator('.hero-section').evaluate(element => element.getBoundingClientRect().height >= innerHeight)).toBe(true)
      expect(await page.locator('.gateway-endpoints li').evaluateAll(elements => elements.every(element => element.scrollWidth <= element.clientWidth + 1))).toBe(true)
      // Expanded free-plan details may extend the intro on short viewports, but must remain in the hero.
      expect(await page.locator('.hero-free-start').evaluate(element => element.getBoundingClientRect().bottom <= document.querySelector('.hero-section').getBoundingClientRect().bottom)).toBe(true)
      if (viewport.width >= 981 && viewport.height >= 1000) {
        expect(await page.locator('.hero-free-start').evaluate(element => element.getBoundingClientRect().bottom <= innerHeight)).toBe(true)
      }
      const gap = await page.evaluate(() => document.querySelector('.demo-heading').getBoundingClientRect().top
        - document.querySelector('.hero-section').getBoundingClientRect().bottom)
      expect(gap).toBeGreaterThanOrEqual(31)
      expect(gap).toBeLessThanOrEqual(50)
    }
  }
})

test('gateway light flow can pause and respects reduced motion and offscreen visibility', async ({ page }) => {
  await page.setViewportSize({ width: 1536, height: 960 })
  await page.goto('/')
  const gateway = page.locator('.hero-gateway')
  const current = gateway.locator('.gateway-current').first()
  await expect(gateway).toHaveClass(/is-flowing/)
  const offset = await current.evaluate(element => getComputedStyle(element).strokeDashoffset)
  await expect.poll(() => current.evaluate(element => getComputedStyle(element).strokeDashoffset)).not.toBe(offset)
  await gateway.locator('.gateway-motion-toggle').click()
  await expect(gateway).not.toHaveClass(/is-flowing/)
  expect(await current.evaluate(element => getComputedStyle(element).animationPlayState)).toBe('paused')
  await gateway.locator('.gateway-motion-toggle').click()
  await expect(gateway).toHaveClass(/is-flowing/)
  await page.locator('.site-footer').scrollIntoViewIfNeeded()
  await expect(gateway).not.toHaveClass(/is-flowing/)
  await page.emulateMedia({ reducedMotion: 'reduce' })
  expect(await current.evaluate(element => getComputedStyle(element).animationName)).toBe('none')
  expect(await current.evaluate(element => getComputedStyle(element).display)).toBe('none')
  await expect(gateway.locator('.gateway-motion-toggle')).toBeHidden()
})

test('interactive product tour has complete pages and consistent shared data', async ({ page }) => {
  await page.goto('/')
  await checkDemo(page)
})

test('team assessment form stays readable in both languages and on mobile', async ({ page }) => {
  await page.goto('/')
  for (const width of [1536, 390]) {
    await page.setViewportSize({ width, height: 960 })
    for (const language of ['Switch to English', '切换为简体中文']) {
      await page.getByRole('button', { name: language, exact: true }).click()
      const form = page.locator('.trial-panel')
      for (const [selector, minimum] of [['.lead-field input', 16], ['.lead-field', 14], ['.lead-segmented label', 14], ['.trial-actions button', 15]]) {
        expect(await form.locator(selector).evaluateAll((elements, size) => elements.every(element => Number.parseFloat(getComputedStyle(element).fontSize) >= size), minimum)).toBe(true)
      }
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    }
  }
})

for (const viewport of [{ width: 1920, height: 960 }, { width: 1366, height: 648 }, { width: 390, height: 720 }]) {
  test(`pricing, normal scrolling and keyboard comparison at ${viewport.width}x${viewport.height}`, async ({ page }) => {
    await page.setViewportSize(viewport)
    await page.goto('/')
    await page.getByRole('button', { name: '切换为简体中文', exact: true }).click()
    if (viewport.width < 760) await page.getByRole('button', { name: '打开导航', exact: true }).click()
    await page.getByRole('navigation').getByRole('button', { name: '下载与价格' }).click()
    try { await expect(page.locator('.catalog-heading')).toBeInViewport() }
    catch (error) {
      error.message += `\nScroll state: ${JSON.stringify(await page.evaluate(() => ({
        scroll: scrollY, height: innerHeight, content: document.documentElement.scrollHeight,
        top: document.querySelector('.catalog-heading').getBoundingClientRect().top,
        classes: document.documentElement.className, bodyClasses: document.body.className,
      })))}`
      throw error
    }
    await expect.poll(() => page.locator('.catalog-section').evaluate(element =>
      Math.abs(element.getBoundingClientRect().top - Number.parseFloat(getComputedStyle(element).scrollMarginTop)))).toBeLessThan(2)
    await page.screenshot({ path: resolve(screenshots, `plans-${viewport.width}x${viewport.height}.png`) })
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    await page.locator('.plan-comparison summary').focus()
    await page.keyboard.press('Enter')
    const comparison = page.locator('.comparison-scroll')
    await comparison.focus()
    await expect(comparison).toBeFocused()
    if (viewport.width < 760) {
      await page.keyboard.press('ArrowRight')
      await expect.poll(() => comparison.evaluate(element => element.scrollLeft)).toBeGreaterThan(0)
      expect(await comparison.evaluate(element => element.clientWidth < element.scrollWidth)).toBe(true)
    }
    await page.locator('.catalog-heading').scrollIntoViewIfNeeded()
    const before = await page.evaluate(() => scrollY)
    await page.mouse.move(viewport.width / 2, viewport.height / 2)
    await page.mouse.wheel(0, 160)
    await expect.poll(() => page.evaluate(() => scrollY)).toBeGreaterThan(before)
    expect((await page.evaluate(() => scrollY)) - before).toBeLessThan(viewport.height)
  })
}

for (const viewport of [{ width: 1366, height: 648 }, { width: 390, height: 720 }]) {
  test(`Windows experimental download remains labeled and matches its bytes at ${viewport.width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize(viewport)
    await page.goto('/')
    await page.getByRole('button', { name: '切换为简体中文', exact: true }).click()
    const { release } = await (await page.request.get('/product-release.json')).json()
    const card = page.locator('.windows-release-card')
    await card.scrollIntoViewIfNeeded()
    await expect(card.getByRole('heading', { name: 'Windows 实验版', exact: true })).toBeVisible()
    await expect(card).toContainText('不承诺稳定')
    await expect(card).toContainText('建议选择 Linux')
    await expect(card.getByRole('link', { name: '下载 Windows 实验版', exact: true })).toHaveAttribute('href', release.windows.artifact.url)
    expect(release.windows.artifact.channel).toBe('experimental')
    const response = await page.request.get(release.windows.artifact.url)
    expect(response.status()).toBe(200)
    const bytes = await response.body()
    expect(bytes.length).toBe(release.windows.artifact.size_bytes)
    expect(createHash('sha256').update(bytes).digest('hex')).toBe(release.windows.artifact.sha256)
    expect(await (await page.request.get(release.windows.documents.windows_guide)).text()).toContain('Windows 实验版')
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    await card.screenshot({ path: testInfo.outputPath(`windows-download-${viewport.width}.png`), animations: 'disabled' })
  })
}


for (const [variant, count] of [['empty', 0], ['single', 1], ['many', 8], ['unconfigured', 0]]) {
  test(`production ${variant} catalog has ${count} plans and keeps consultation usable`, async ({ page }) => {
    await app.close()
    app = await createWebsiteRuntime({ scriptPath: resolve(root, 'dist/website-validation/functions/index.js'), port: 26990,
      staticRoot: resolve(root, `dist/website-validation/site-${variant}`) })
    await page.setViewportSize({ width: 390, height: 720 })
    const errors = []
    page.on('pageerror', error => errors.push(error.message))
    await page.goto('/')
    await page.getByRole('button', { name: '切换为简体中文', exact: true }).click()
    await expect(page.locator('.plan-card')).toHaveCount(count)
    if (!count) {
      await expect(page.locator('.plan-empty')).toContainText('企业授权与部署支持')
      await expect(page.locator('.plan-empty')).toContainText('当前未发布固定价格表')
      await expect(page.locator('.plan-empty button')).toBeEnabled()
    }
    const manifest = await (await page.request.get('/catalog-manifest.json')).json()
    expect(manifest.state).toBe(variant === 'unconfigured' ? 'unconfigured' : 'configured')
    if (manifest.catalog) {
      const payload = await (await page.request.get(manifest.catalog.path)).json()
      expect(payload.plans.length).toBe(count)
      await expect(page.locator('.catalog-section')).toHaveAttribute('data-catalog-revision', payload.revision)
    }
    await fillInquiry(page, `${variant}@example.invalid`)
    await inquiry(page).getByRole('button', { name: '发送咨询', exact: true }).click()
    const response = page.waitForResponse(response => response.url().endsWith('/api/inquiries'))
    await solve(page, 0)
    const result = await (await response).json()
    await expect(inquiry(page).getByRole('heading', { name: '咨询已收到' })).toBeVisible()
    expect((await saved(result.id)).reference_status).toBe('none')
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    expect(errors).toEqual([])
  })
}

test('subscription plans explain automatic standard updates and expand the same rights in comparison', async ({ page }) => {
  await app.close()
  app = await createWebsiteRuntime({ scriptPath: buildWebsiteFunctions(), port: 26990,
    staticRoot: resolve(root, 'dist/website-validation/site-subscription') })
  await page.goto(app.origin)
  await page.getByRole('button', { name: '切换为简体中文', exact: true }).click()
  const card = page.locator('.plan-card')
  await expect(card).toHaveCount(1)
  await expect(card).toContainText('订阅期间包含标准功能更新与升级')
  await page.locator('.plan-comparison summary').click()
  for (const label of ['模型接入', '成员协作', 'Runner']) {
    await expect(page.locator('.plan-comparison tbody tr').filter({ hasText: label }).first().locator('td')).toHaveText('包含')
  }
  await page.setViewportSize({ width: 1366, height: 648 })
  await card.screenshot({ path: resolve(screenshots, 'standard-subscription-desktop.png') })
  await page.setViewportSize({ width: 390, height: 720 })
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  await card.screenshot({ path: resolve(screenshots, 'standard-subscription-mobile.png') })
})
