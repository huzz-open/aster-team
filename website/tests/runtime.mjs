import { execFileSync } from 'node:child_process'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, resolve, extname, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import { Miniflare, convertV4MiniflareOptions } from 'miniflare'
import { unstable_splitSqlQuery as splitSqlQuery } from 'wrangler'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
export function buildWebsiteFunctions() {
  const output = resolve(root, 'dist/website-validation/functions')
  execFileSync(process.execPath, [resolve(root, 'node_modules/wrangler/bin/wrangler.js'), 'pages', 'functions', 'build',
    resolve(root, 'website/functions'), '--outdir', output, '--compatibility-date', '2026-08-29', '--compatibility-flags', 'nodejs_compat'],
  { cwd: root, env: { ...process.env, WRANGLER_SEND_METRICS: 'false' }, windowsHide: true, stdio: 'pipe' })
  return resolve(output, 'index.js')
}

export async function createWebsiteRuntime({ scriptPath, port = 0, origin = 'http://127.0.0.1:26990', staticRoot, maximum = 100 } = {}) {
  const outbound = []
  const runtime = new Miniflare(convertV4MiniflareOptions({
    rootPath: root, modulesRoot: dirname(scriptPath), scriptPath, modules: true,
    compatibilityDate: '2026-08-29', compatibilityFlags: ['nodejs_compat'],
    host: '127.0.0.1', port, cf: false, d1Databases: ['LEADS_DB'],
    bindings: {
      ALLOWED_ORIGINS: origin, TURNSTILE_ACTION: 'trial_request', TURNSTILE_HOSTNAMES: '127.0.0.1', TURNSTILE_TEST_MODE: 'false',
      TURNSTILE_SECRET: 'isolated-test-secret', RATE_LIMIT_SALT: 'isolated-test-salt', SMTP_PASSWORD: '',
      SMTP_HOST: 'smtp.example.invalid', SMTP_PORT: '465', SMTP_USERNAME: 'test@example.invalid',
      LEAD_NOTIFICATION_FROM: 'test@example.invalid', LEAD_NOTIFICATION_TO: 'test@example.invalid',
      RATE_LIMIT_WINDOW_SECONDS: '600', RATE_LIMIT_MAX_SUBMISSIONS: String(maximum),
    },
    serviceBindings: { ASSETS: async request => {
      if (!staticRoot) return new Response('Not found', { status: 404 })
      const pathname = decodeURIComponent(new URL(request.url).pathname)
      const target = resolve(staticRoot, `.${pathname === '/' ? '/index.html' : pathname}`)
      if (!target.startsWith(`${resolve(staticRoot)}${sep}`)) return new Response('Not found', { status: 404 })
      try {
        if (!statSync(target).isFile()) return new Response('Not found', { status: 404 })
        const types = { '.html': 'text/html; charset=utf-8', '.js': 'application/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.png': 'image/png', '.webp': 'image/webp', '.json': 'application/json' }
        return new Response(readFileSync(target), { headers: { 'Content-Type': types[extname(target)] ?? 'application/octet-stream' } })
      } catch { return new Response('Not found', { status: 404 }) }
    } },
    outboundService: async request => {
      outbound.push(request.url)
      if (request.url !== 'https://challenges.cloudflare.com/turnstile/v0/siteverify') return new Response('External traffic disabled in this test', { status: 502 })
      const form = new URLSearchParams(await request.text())
      const token = form.get('response')
      return Response.json({ success: form.get('secret') === 'isolated-test-secret' && Boolean(token?.startsWith('test-token')),
        hostname: token === 'test-token-bad-host' ? 'other.invalid' : '127.0.0.1',
        action: token === 'test-token-bad-action' ? 'wrong_action' : 'trial_request' })
    },
  }))
  try {
    await runtime.ready
    const database = await runtime.getD1Database('LEADS_DB')
    const schemaRoot = resolve(root, 'website/server/schema')
    for (const name of readdirSync(schemaRoot).filter(name => /^\d{4}_.+\.sql$/.test(name)).sort()) {
      const statements = splitSqlQuery(readFileSync(resolve(schemaRoot, name), 'utf8'))
      await database.batch(statements.map(statement => database.prepare(statement)))
    }
    return { runtime, database, outbound, origin, close: () => runtime.dispose() }
  } catch (error) { await runtime.dispose(); throw error }
}
