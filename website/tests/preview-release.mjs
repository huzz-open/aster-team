import { createHash } from 'node:crypto'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium } from '@playwright/test'
import { buildWebsiteFunctions, createWebsiteRuntime } from './runtime.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const args = process.argv.slice(2)
const runID = args[0]
const capture = args[1] === '--capture'
if (!runID || !/^\d{17}-[a-f0-9]{8}$/.test(runID) || args.length > 2 || (args[1] && !capture)) {
  throw new Error('Usage: npm run preview:website:release -- <run-id> [--capture]')
}
const runRoot = join(root, 'dist/website-release-validation', runID)
const json = path => JSON.parse(readFileSync(path, 'utf8'))
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex')
const result = json(join(runRoot, 'result.json'))
const inputBytes = readFileSync(join(runRoot, 'inputs.json'))
const inputs = JSON.parse(inputBytes)
if (result.schema !== 'aster.website-release-acceptance.v1' || result.status !== 'passed'
  || result.trust !== 'test-build' || result.run_id !== runID
  || sha256(inputBytes) !== result.inputs_sha256 || inputs.commit !== result.commit) {
  throw new Error('A completed local acceptance with matching input evidence is required')
}
const siteRoot = join(runRoot, 'site')
if (resolve(inputs.site_root) !== siteRoot) throw new Error('Acceptance site path does not match its run directory')
function sitePath(path) {
  if (typeof path !== 'string' || !path.startsWith('/') || path.includes('\\')) throw new Error('Invalid site asset path')
  const target = resolve(siteRoot, `.${path}`)
  if (!target.startsWith(`${siteRoot}${sep}`)) throw new Error('Site asset escapes the preview directory')
  return target
}
const release = json(join(siteRoot, 'website-release.json'))
if (release.catalog?.environment !== 'local') throw new Error('Only local acceptance artifacts can be previewed')
for (const entry of release.files) {
  const bytes = readFileSync(sitePath(entry.path))
  if (bytes.length !== entry.size_bytes || sha256(bytes) !== entry.sha256) throw new Error(`Changed site asset: ${entry.path}`)
}
const product = json(join(siteRoot, 'product-release.json'))
if (product.state !== 'configured' || product.release.environment !== 'local') throw new Error('Missing local package')
const archive = readFileSync(sitePath(product.release.artifact.url))
if (archive.length !== result.artifact.size_bytes || sha256(archive) !== result.artifact.sha256) throw new Error('Changed downloadable package')

if (inputs.windows) {
  if (!product.release.windows || product.release.windows.environment !== 'local') throw new Error('Missing local Windows package')
  const windows = readFileSync(sitePath(product.release.windows.artifact.url))
  if (windows.length !== inputs.windows.artifact.size_bytes || sha256(windows) !== inputs.windows.artifact.sha256) throw new Error('Changed downloadable Windows package')
}

const port = 26992
const origin = `http://127.0.0.1:${port}`
const app = await createWebsiteRuntime({ scriptPath: buildWebsiteFunctions(), port, origin, staticRoot: siteRoot })
let browser
let browserStart
let closing
const close = () => closing ??= (async () => {
  try { await (await browserStart)?.close() }
  finally { await app.close() }
})()
for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => { void close().catch(error => { console.error(error); process.exitCode = 1 }) })
console.log(`Local website review: ${origin}\nFrontend source: ${result.commit}\nPackage source: ${inputs.package_commit ?? result.commit}\nTest catalog and test-signed package only. No CF deployment or email delivery.\nThe frontend is the preserved build; local Functions use the current checkout.\nStop with Ctrl+C. Forms without a configured CAPTCHA do not submit; use test:website:browser for isolated form acceptance.`)

if (capture) {
  try {
    const output = join(runRoot, 'visual-review', new Date().toISOString().replace(/[-:.TZ]/g, ''))
    mkdirSync(output, { recursive: true })
    browserStart = chromium.launch({ headless: true })
    browser = await browserStart
    if (closing) throw new Error('Visual capture cancelled during browser startup')
    const evidence = { schema: 'aster.website-visual-review.v1', source_commit: result.commit,
      acceptance_run: runID, captured_at: new Date().toISOString(), captures: [], page_errors: [], blocked_external_requests: [] }
    for (const viewport of [{ width: 1920, height: 960 }, { width: 1366, height: 648 }, { width: 390, height: 720 }]) {
      const page = await browser.newPage({ viewport, deviceScaleFactor: 1, reducedMotion: 'reduce' })
      page.on('pageerror', error => evidence.page_errors.push(error.message))
      await page.route('**/*', route => {
        if (new URL(route.request().url()).origin === origin) return route.continue()
        evidence.blocked_external_requests.push(route.request().url())
        return route.abort('blockedbyclient')
      })
      await page.goto(origin, { waitUntil: 'networkidle' })
      await page.evaluate(() => document.fonts.ready)
      const dimensions = await page.evaluate(() => ({ width: document.documentElement.scrollWidth, height: document.documentElement.scrollHeight }))
      if (dimensions.width > viewport.width) throw new Error(`Horizontal overflow at ${viewport.width}`)
      const prefix = `${viewport.width}x${viewport.height}`
      await page.screenshot({ path: join(output, `${prefix}-full.png`), fullPage: true })
      evidence.captures.push({ file: `${prefix}-full.png`, viewport, kind: 'full-page', height: dimensions.height })
      // Overlap preserves context around the fixed navigation between successive screen-sized images.
      const stride = viewport.height - 100
      const positions = []
      for (let top = 0; top < dimensions.height - viewport.height; top += stride) positions.push(top)
      positions.push(Math.max(0, dimensions.height - viewport.height))
      for (const [index, top] of positions.entries()) {
        await page.evaluate(y => window.scrollTo({ top: y, behavior: 'instant' }), top)
        await page.evaluate(() => new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done))))
        const actualTop = await page.evaluate(() => scrollY)
        if (Math.abs(actualTop - top) > 2) throw new Error(`Unexpected scroll position: wanted ${top}, received ${actualTop}`)
        const file = `${prefix}-${String(index + 1).padStart(2, '0')}.png`
        await page.screenshot({ path: join(output, file) })
        evidence.captures.push({ file, viewport, kind: 'viewport', scroll_y: actualTop })
      }
      await page.close()
    }
    if (evidence.page_errors.length || app.outbound.length) throw new Error('Page errors or unexpected backend traffic during visual review')
    writeFileSync(join(output, 'screenshots.json'), `${JSON.stringify(evidence, null, 2)}\n`)
    const links = evidence.captures.map(item => `<li><a href="${item.file}">${item.file}</a> ${item.kind === 'viewport' ? `scroll ${item.scroll_y}` : 'full page'}</li>`).join('\n')
    writeFileSync(join(output, 'index.html'), `<!doctype html><html lang="zh-CN"><meta charset="utf-8"><title>Aster 本地视觉审查</title><style>body{font:16px system-ui;max-width:960px;margin:40px auto;padding:0 24px;line-height:1.7}a{color:#334a96}</style><h1>Aster 本地视觉审查</h1><p>源提交 ${result.commit}</p><p>测试套餐与测试签名包 不代表正式发行或视觉已获批准</p><p>文件名为浏览器内容视口尺寸 不包含浏览器工具栏 分屏保留 100 像素上下文</p><ul>${links}</ul></html>`)
    console.log(`Visual evidence: ${output}`)
  } finally { await close() }
}
