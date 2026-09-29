import { createServer } from 'node:http'
import { readFile, writeFile, mkdir } from 'node:fs/promises'
import { resolve, relative, extname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { build } from 'vite'
import vue from '@vitejs/plugin-vue'
import { publicCatalogPlugin } from '../../website/build/public-catalog.mjs'
import { productReleasePlugin } from '../../website/build/product-release.mjs'

// Actual production build of the catalog approved by the local Operations API.
// This fixture serves only loopback and has no deployment or notification code.
export async function publicationSite(record, bytes) {
  if (record.snapshot.request.environment !== 'local' || !/^catalog_[a-f0-9]{48}$/.test(record.snapshot.id)) throw new Error('Publication browser fixture requires a local catalog')
  const root = fileURLToPath(new URL('../../', import.meta.url))
  const directory = resolve(root, 'dist/commercial-validation/publication-sites', record.snapshot.id)
  const output = resolve(directory, 'site')
  await mkdir(directory, { recursive: true })
  const input = resolve(directory, 'plans.json')
  await writeFile(input, bytes)
  const environment = { ASTER_WEBSITE_CATALOG_PATH: input, ASTER_WEBSITE_CATALOG_SHA256: record.public.sha256,
    ASTER_WEBSITE_CATALOG_REVISION: record.snapshot.id, ASTER_WEBSITE_CATALOG_ENVIRONMENT: 'local' }
  await build({ root: resolve(root, 'website'), configFile: false, envDir: false, logLevel: 'warn',
    plugins: [vue(), publicCatalogPlugin(environment), productReleasePlugin(environment)],
    define: { 'import.meta.env.VITE_ASTER_CONTACT_EMAIL': JSON.stringify('test@example.invalid'), 'import.meta.env.VITE_ASTER_TURNSTILE_SITE_KEY': JSON.stringify('isolated-browser-sitekey'), 'import.meta.env.VITE_ASTER_LEAD_ENDPOINT': JSON.stringify('/api/trial') },
    build: { outDir: output, emptyOutDir: true, sourcemap: false } })
  const manifestPath = resolve(output, 'website-release.json')
  const manifest = await readFile(manifestPath)
  let tampered = false
  const server = createServer(async (request, response) => {
    try {
      if (request.method !== 'GET') { response.writeHead(405).end(); return }
      const pathname = decodeURIComponent(new URL(request.url, 'http://127.0.0.1:26394').pathname)
      const path = resolve(output, `.${pathname === '/' ? '/index.html' : pathname}`)
      const local = relative(output, path)
      if (!local || local.startsWith('..') || local.includes(':')) { response.writeHead(404).end(); return }
      let content = await readFile(path)
      if (tampered && extname(path) === '.js') { content = Buffer.from(content); content[0] ^= 1 }
      response.setHeader('Cache-Control', 'no-store')
      response.setHeader('Content-Type', ({ '.html': 'text/html', '.json': 'application/json', '.js': 'text/javascript', '.css': 'text/css' })[extname(path)] || 'application/octet-stream')
      response.end(content)
    } catch { response.writeHead(404).end() }
  })
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(26394, '127.0.0.1', resolve) })
  return { manifestPath, manifest, tamper(value) { tampered = value }, close() { return new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve())) } }
}
