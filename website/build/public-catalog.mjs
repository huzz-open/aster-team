import { createHash } from 'node:crypto'
import { openSync, closeSync, fstatSync, readSync, readFileSync, writeFileSync } from 'node:fs'
import { isAbsolute, resolve } from 'node:path'
import Ajv2020 from 'ajv/dist/2020.js'
import addFormats from 'ajv-formats'
import { parse } from 'yaml'
import { createReleaseManifest } from './release-manifest.mjs'

const maximumCatalogBytes = 4 * 1024 * 1024
const ajv = new Ajv2020({ strict: true, allErrors: true })
addFormats(ajv, ['date-time'])
const validate = ajv.compile(JSON.parse(readFileSync(new URL('../shared/generated/public-catalog.schema.json', import.meta.url), 'utf8')))
const capabilities = parse(readFileSync(new URL('../../contracts/catalogs/product-capabilities.yaml', import.meta.url), 'utf8'))

export function canonicalJson(value) {
  if (typeof value === 'string' && !value.isWellFormed()) throw new Error('catalog contains invalid Unicode')
  // Match the existing Operations wire encoding, including Go's HTML and line
  // separator escaping. Do not silently re-encode approved download bytes.
  if (typeof value === 'string') return JSON.stringify(value).replace(/[<>&\u2028\u2029]/g, character => `\\u${character.charCodeAt(0).toString(16).padStart(4, '0')}`)
  if (value === null || typeof value !== 'object') return JSON.stringify(value)
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`
  return `{${Object.keys(value).sort().map(key => `${canonicalJson(key)}:${canonicalJson(value[key])}`).join(',')}}`
}

function validateSemantics(catalog) {
  const selected = new Set()
  for (const plan of catalog.plans) {
    if (selected.has(plan.plan_id)) throw new Error('catalog repeats a plan')
    selected.add(plan.plan_id)
    if (!plan.name.trim() || plan.name !== plan.name.trim()) throw new Error('catalog has an invalid plan name')
    if (plan.offer.kind === 'free' && (plan.entitlements.feature_sets ?? []).length) throw new Error('free catalog plans cannot grant feature sets')
    const features = new Set(plan.entitlements.features)
    for (const entry of capabilities.capabilities) {
      if ((plan.entitlements.feature_sets ?? []).includes(entry.feature_set)) features.add(entry.id)
    }
    for (const capability of capabilities.capabilities) {
      if (features.has(capability.id) && capability.requires.some(id => !features.has(id))) throw new Error('catalog capability dependency is missing')
    }
    if (plan.offer.kind !== 'fixed_price') continue
    const years = new Set()
    for (const term of plan.offer.terms) {
      if (years.has(term.years)) throw new Error('catalog repeats a contract term')
      years.add(term.years)
      const numerator = BigInt(plan.offer.annual_amount_minor) * BigInt(term.years) * BigInt(term.discount_basis_points)
      if ((numerator + 5000n) / 10000n !== BigInt(term.total_amount_minor)) throw new Error('catalog term amount does not match its approved price rule')
    }
  }
}

export function readPublicCatalog(env = process.env) {
  const path = env.ASTER_WEBSITE_CATALOG_PATH
  const sha256 = env.ASTER_WEBSITE_CATALOG_SHA256
  const revision = env.ASTER_WEBSITE_CATALOG_REVISION
  const environment = env.ASTER_WEBSITE_CATALOG_ENVIRONMENT
  if ([path, sha256, revision, environment].every(value => value === undefined || value === '')) return null
  if (!path || !isAbsolute(path) || !/^[a-f0-9]{64}$/.test(sha256 || '')
    || !/^catalog_[a-f0-9]{48}$/.test(revision || '') || !['local', 'production'].includes(environment)) {
    throw new Error('catalog input requires an absolute path, SHA-256, revision and environment')
  }
  const descriptor = openSync(path, 'r')
  let bytes
  try {
    const info = fstatSync(descriptor)
    if (!info.isFile() || info.size > maximumCatalogBytes) throw new Error('catalog input must be a bounded regular file')
    const buffer = Buffer.alloc(maximumCatalogBytes + 1)
    let length = 0
    while (length < buffer.length) {
      const count = readSync(descriptor, buffer, length, buffer.length - length, null)
      if (!count) break
      length += count
    }
    if (length > maximumCatalogBytes) throw new Error('catalog input is too large')
    bytes = buffer.subarray(0, length)
  } finally {
    closeSync(descriptor)
  }
  if (createHash('sha256').update(bytes).digest('hex') !== sha256) throw new Error('catalog SHA-256 mismatch')
  const source = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes)
  const catalog = JSON.parse(source)
  if (!validate(catalog)) throw new Error(`invalid public catalog: ${ajv.errorsText(validate.errors)}`)
  if (canonicalJson(catalog) !== source) throw new Error('catalog input is not canonical JSON')
  if (catalog.revision !== revision || catalog.environment !== environment) throw new Error('catalog identity mismatch')
  validateSemantics(catalog)
  return { catalog, source, identity: { revision, sha256, environment, path: `/catalog/${revision}/plans.json` } }
}

export function publicCatalogPlugin(env = process.env) {
  const snapshot = readPublicCatalog(env)
  const moduleId = 'virtual:aster-public-catalog'
  const resolvedId = `\0${moduleId}`
  const manifest = JSON.stringify({ schema: 'aster.website-catalog.v1', state: snapshot ? 'configured' : 'unconfigured', catalog: snapshot?.identity ?? null })
  return {
    name: 'aster-public-catalog',
    resolveId(id) { if (id === moduleId) return resolvedId },
    load(id) {
      if (id !== resolvedId) return
      return `function freeze(value) { if (value && typeof value === 'object') { Object.values(value).forEach(freeze); Object.freeze(value) } return value }\nexport const identity = freeze(${JSON.stringify(snapshot?.identity ?? null)});\nexport default freeze(${JSON.stringify(snapshot?.catalog ?? null)});`
    },
    transformIndexHtml() {
      return snapshot ? [
        { tag: 'meta', attrs: { name: 'aster-catalog-revision', content: snapshot.identity.revision }, injectTo: 'head' },
        { tag: 'meta', attrs: { name: 'aster-catalog-sha256', content: snapshot.identity.sha256 }, injectTo: 'head' },
      ] : []
    },
    generateBundle() {
      this.emitFile({ type: 'asset', fileName: 'catalog-manifest.json', source: manifest })
      if (snapshot) this.emitFile({ type: 'asset', fileName: snapshot.identity.path.slice(1), source: snapshot.source })
    },
    writeBundle(options, bundle) {
      const release = createReleaseManifest(bundle, snapshot?.identity)
      if (release === null) return
      if (!options.dir) throw new Error('website release requires a directory output')
      writeFileSync(resolve(options.dir, 'website-release.json'), release)
    },
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const path = request.url?.split('?')[0]
        const content = path === '/catalog-manifest.json' ? manifest : snapshot && path === snapshot.identity.path ? snapshot.source : null
        if (content === null) return next()
        response.setHeader('Content-Type', 'application/json; charset=utf-8')
        response.setHeader('Cache-Control', 'no-store')
        response.setHeader('X-Content-Type-Options', 'nosniff')
        response.end(content)
      })
    },
  }
}
