import { createReadStream } from 'node:fs'
import { access, copyFile, mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { dirname, extname, join, relative, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawn } from 'node:child_process'
import { chromium } from 'playwright'

const scriptDirectory = dirname(fileURLToPath(import.meta.url))
const repositoryRoot = resolve(scriptDirectory, '..')
const websiteMediaRoot = join(repositoryRoot, 'website', 'public', 'product-media')
const viteEntry = join(repositoryRoot, 'node_modules', 'vite', 'bin', 'vite.js')
const workerSource = join(repositoryRoot, 'node_modules', 'msw', 'lib', 'mockServiceWorker.js')
const screenshotViewport = { width: 1920, height: 1080 }
const screenshotInstant = '2026-09-03T03:00:00.000Z'
const screenshotTimezone = 'Asia/Shanghai'
const locales = [
  { id: 'zh', appLocale: 'zh-CN' },
  { id: 'en', appLocale: 'en-US' },
]
const scenes = [
  {
    id: 'member-usage', app: 'member', path: '/usage',
    minimumCounts: [['.bar-stack', 7], ['.comparison-line polyline', 1], ['.segment.input', 7], ['.donut-segment', 4], ['.rank-row', 4]],
    fittedSelectors: ['.analytics-page'],
  },
  {
    id: 'member-home', app: 'member', path: '/home',
    minimumCounts: [['.chart-line.current', 1], ['.chart-point.current', 20], ['.model-table tbody tr', 4], ['.recent-table tbody tr', 3]],
    fittedSelectors: ['.dashboard-page'],
  },
  {
    id: 'admin-overview', app: 'admin', path: '/overview',
    minimumCounts: [['.current-line', 1], ['.previous-line', 1], ['.donut-segment', 4], ['.ranking-list .ranking-row', 5], ['.resource-list > div', 4]],
    fittedSelectors: ['.overview-page'],
  },
  {
    id: 'admin-users', app: 'admin', path: '/users',
    minimumCounts: [['.paginated-scroll tbody > tr', 9], ['.row-actions button', 45]],
    fittedSelectors: ['.paginated-scroll'],
  },
  {
    id: 'member-keys', app: 'member', path: '/keys',
    minimumCounts: [['.table-wrap tbody > tr', 7]],
    fittedSelectors: ['.table-wrap'],
  },
]

function run(command, args, options = {}) {
  return new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(command, args, { cwd: repositoryRoot, stdio: 'inherit', ...options })
    child.once('error', rejectPromise)
    child.once('exit', code => code === 0 ? resolvePromise() : rejectPromise(new Error(`${command} exited with code ${code}`)))
  })
}

async function buildDemoApp(app, outputDirectory) {
  const appDirectory = join(repositoryRoot, 'customer', app)
  await run(process.execPath, [viteEntry, 'build', '--mode', 'demo', '--outDir', outputDirectory, '--emptyOutDir'], { cwd: appDirectory })
  await copyFile(workerSource, join(outputDirectory, 'mockServiceWorker.js'))
}

const contentTypes = new Map([
  ['.css', 'text/css; charset=utf-8'], ['.html', 'text/html; charset=utf-8'], ['.js', 'text/javascript; charset=utf-8'],
  ['.json', 'application/json; charset=utf-8'], ['.png', 'image/png'], ['.svg', 'image/svg+xml'], ['.webm', 'video/webm'],
])

async function startStaticServer(rootDirectory) {
  const normalizedRoot = `${resolve(rootDirectory)}${sep}`
  const server = createServer(async (request, response) => {
    try {
      const url = new URL(request.url || '/', 'http://127.0.0.1')
      const requestedPath = decodeURIComponent(url.pathname).replace(/^\/+/, '')
      let filePath = resolve(rootDirectory, requestedPath || 'index.html')
      if (!`${filePath}${sep}`.startsWith(normalizedRoot) && filePath !== resolve(rootDirectory, 'index.html')) {
        response.writeHead(403).end('Forbidden')
        return
      }
      const fileStats = await stat(filePath).catch(() => null)
      if (!fileStats?.isFile()) filePath = resolve(rootDirectory, 'index.html')
      const headers = {
        'Cache-Control': 'no-store',
        'Content-Type': contentTypes.get(extname(filePath)) || 'application/octet-stream',
      }
      if (filePath.endsWith('mockServiceWorker.js')) headers['Service-Worker-Allowed'] = '/'
      response.writeHead(200, headers)
      createReadStream(filePath).pipe(response)
    } catch (error) {
      response.writeHead(500).end(error instanceof Error ? error.message : 'Internal error')
    }
  })
  await new Promise((resolvePromise, rejectPromise) => {
    server.once('error', rejectPromise)
    server.listen(0, '127.0.0.1', resolvePromise)
  })
  const address = server.address()
  if (!address || typeof address === 'string') throw new Error('Could not determine static server port')
  return {
    url: `http://127.0.0.1:${address.port}`,
    close: () => new Promise((resolvePromise, rejectPromise) => server.close(error => error ? rejectPromise(error) : resolvePromise())),
  }
}

async function configureLocale(context, appLocale) {
  await context.addInitScript(value => {
    localStorage.setItem('aster-locale', value)
    localStorage.setItem('aster-theme', 'light')
    localStorage.setItem('aster-sidebar-collapsed', 'false')
  }, appLocale)
}

async function assertSceneContent(page, scene) {
  const documentOverflow = await page.evaluate(() => {
    const root = document.scrollingElement || document.documentElement
    return {
      horizontal: root.scrollWidth - root.clientWidth,
      vertical: root.scrollHeight - root.clientHeight,
    }
  })
  if (documentOverflow.horizontal > 1 || documentOverflow.vertical > 1) {
    throw new Error(`${scene.id} exceeds the screenshot viewport by ${documentOverflow.horizontal}px horizontally and ${documentOverflow.vertical}px vertically`)
  }

  for (const [selector, minimum] of scene.minimumCounts) {
    const count = await page.locator(selector).count()
    if (count < minimum) throw new Error(`${scene.id} expected at least ${minimum} visible data elements for ${selector}, found ${count}`)
  }

  for (const selector of scene.fittedSelectors) {
    const metrics = await page.locator(selector).first().evaluate(element => {
      const bounds = element.getBoundingClientRect()
      return {
        bounds: { left: bounds.left, top: bounds.top, right: bounds.right, bottom: bounds.bottom },
        viewport: { width: window.innerWidth, height: window.innerHeight },
        overflow: { horizontal: element.scrollWidth - element.clientWidth, vertical: element.scrollHeight - element.clientHeight },
      }
    })
    const outsideViewport = metrics.bounds.left < -1 || metrics.bounds.top < -1
      || metrics.bounds.right > metrics.viewport.width + 1 || metrics.bounds.bottom > metrics.viewport.height + 1
    if (outsideViewport || metrics.overflow.horizontal > 1 || metrics.overflow.vertical > 1) {
      throw new Error(`${scene.id} has clipped or scroll-dependent content in ${selector}: ${JSON.stringify(metrics)}`)
    }
  }
}

async function openScene(page, url, scene) {
  await page.goto(url, { waitUntil: 'domcontentloaded' })
  await page.locator('html[data-aster-data-mode="mock"]').waitFor({ state: 'attached' })
  await page.locator('html[data-aster-demo-profile="website-media-synthetic-v1"]').waitFor({ state: 'attached' })
  await page.locator('h1').first().waitFor({ state: 'visible' })
  await page.waitForLoadState('networkidle')
  await page.waitForTimeout(250)

  const visibleText = await page.locator('body').innerText()
  const unsafeEmails = [...visibleText.matchAll(/[\w.+-]+@[\w.-]+\.[A-Za-z]{2,}/g)]
    .map(match => match[0])
    .filter(email => !email.endsWith('@example.invalid'))
  if (unsafeEmails.length) throw new Error(`Website media scene contains non-demo email addresses: ${unsafeEmails.join(', ')}`)
  await assertSceneContent(page, scene)
}

function trackPageFailures(page, failures) {
  page.on('pageerror', error => failures.push(error.message))
  page.on('console', message => {
    if (message.type() === 'error') failures.push(message.text())
  })
}

async function captureScreenshots(browser, appURLs, locale) {
  const context = await browser.newContext({
    viewport: screenshotViewport,
    deviceScaleFactor: 1,
    colorScheme: 'light',
    reducedMotion: 'reduce',
    timezoneId: screenshotTimezone,
  })
  await configureLocale(context, locale.appLocale)
  const page = await context.newPage()
  await page.clock.install({ time: new Date(screenshotInstant) })
  const failures = []
  trackPageFailures(page, failures)
  const localeDirectory = join(websiteMediaRoot, locale.id)
  await mkdir(localeDirectory, { recursive: true })
  for (const scene of scenes) {
    await openScene(page, `${appURLs[scene.app]}${scene.path}`, scene)
    await page.screenshot({ path: join(localeDirectory, `${scene.id}.png`), animations: 'disabled' })
  }
  await context.close()
  if (failures.length) throw new Error(`${locale.id} screenshot browser errors:\n${failures.join('\n')}`)
}

async function verifyStaticBoundary() {
  const forbiddenDirectories = [
    join(repositoryRoot, 'customer', 'admin', 'public', 'mockServiceWorker.js'),
    join(repositoryRoot, 'customer', 'member', 'public', 'mockServiceWorker.js'),
  ]
  for (const path of forbiddenDirectories) {
    try {
      await access(path)
      throw new Error(`Production public directory contains demo worker: ${relative(repositoryRoot, path)}`)
    } catch (error) {
      if (error?.code !== 'ENOENT') throw error
    }
  }
  const manifest = JSON.parse(await readFile(join(websiteMediaRoot, 'manifest.json'), 'utf8'))
  if (manifest.delivery !== 'static-media-only') throw new Error('Unexpected website media delivery mode')
}

async function main() {
  await access(viteEntry)
  await access(workerSource)
  const temporaryRoot = await mkdtemp(join(tmpdir(), 'aster-website-media-'))
  const normalizedTempRoot = `${resolve(tmpdir())}${sep}`
  if (!`${resolve(temporaryRoot)}${sep}`.startsWith(normalizedTempRoot)) throw new Error(`Unsafe temporary directory: ${temporaryRoot}`)
  const npmEntry = process.env.npm_execpath
  const servers = []
  let browser
  try {
    if (npmEntry) {
      await run(process.execPath, [npmEntry, 'run', 'typecheck', '--workspace', '@aster/admin'])
      await run(process.execPath, [npmEntry, 'run', 'typecheck', '--workspace', '@aster/member'])
    } else {
      await run(process.platform === 'win32' ? 'npm.cmd' : 'npm', ['run', 'typecheck', '--workspace', '@aster/admin'], { shell: process.platform === 'win32' })
      await run(process.platform === 'win32' ? 'npm.cmd' : 'npm', ['run', 'typecheck', '--workspace', '@aster/member'], { shell: process.platform === 'win32' })
    }
    const adminOutput = join(temporaryRoot, 'admin')
    const memberOutput = join(temporaryRoot, 'member')
    await buildDemoApp('admin', adminOutput)
    await buildDemoApp('member', memberOutput)
    const adminServer = await startStaticServer(adminOutput)
    const memberServer = await startStaticServer(memberOutput)
    servers.push(adminServer, memberServer)
    const appURLs = { admin: adminServer.url, member: memberServer.url }
    browser = await chromium.launch({ headless: true })
    for (const locale of locales) {
      await rm(join(websiteMediaRoot, locale.id, 'walkthrough.webm'), { force: true })
      await captureScreenshots(browser, appURLs, locale)
    }
    const manifest = {
      version: 1,
      generatedAt: new Date().toISOString(),
      source: 'compiled-admin-and-member-demo-builds',
      delivery: 'static-media-only',
      screenshotViewport,
      screenshotInstant,
      screenshotTimezone,
      dataProfile: 'website-media-synthetic-v1',
      locales: Object.fromEntries(locales.map(locale => [locale.id, {
        images: scenes.map(scene => `${scene.id}.png`),
      }])),
    }
    await writeFile(join(websiteMediaRoot, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`, 'utf8')
    await verifyStaticBoundary()
    console.log(`Generated website media in ${relative(repositoryRoot, websiteMediaRoot)}`)
  } catch (error) {
    if (/Executable doesn't exist|browserType\.launch/.test(String(error))) {
      throw new Error('Playwright Chromium is not installed. Run `npm run setup:website-media` once, then retry.', { cause: error })
    }
    throw error
  } finally {
    if (browser) await browser.close()
    await Promise.allSettled(servers.map(server => server.close()))
    await rm(temporaryRoot, { recursive: true, force: true })
  }
}

await main()
