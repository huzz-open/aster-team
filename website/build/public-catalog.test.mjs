import assert from 'node:assert/strict'
import test from 'node:test'
import { createHash } from 'node:crypto'
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs'
import { join, resolve, sep } from 'node:path'
import { tmpdir } from 'node:os'
import { fileURLToPath } from 'node:url'
import { canonicalJson, readPublicCatalog, publicCatalogPlugin } from './public-catalog.mjs'

const fixturePath = fileURLToPath(new URL('../../contracts/test-vectors/public-catalog.v1.json', import.meta.url))
const original = readFileSync(fixturePath)
const catalog = JSON.parse(original)
const sha = bytes => createHash('sha256').update(bytes).digest('hex')
const options = (path, bytes = original) => ({ ASTER_WEBSITE_CATALOG_PATH: path, ASTER_WEBSITE_CATALOG_SHA256: sha(bytes), ASTER_WEBSITE_CATALOG_REVISION: catalog.revision, ASTER_WEBSITE_CATALOG_ENVIRONMENT: 'local' })
function removeTestDirectory(directory) {
  const target = resolve(directory)
  assert.ok(target.startsWith(`${resolve(tmpdir())}${sep}aster-public-`))
  rmSync(target, { recursive: true, force: true })
}

test('actual Operations export retains exact bytes, fixed HTML identity and emitted manifest', () => {
  assert.equal(sha(original), '7f12f6bcb6c1666c9fb75e8e330c2f4c03e4dc1e415f8f77601463e406523ab8')
  const snapshot = readPublicCatalog(options(fixturePath))
  assert.deepEqual(Buffer.from(snapshot.source), original)
  assert.equal(snapshot.catalog.plans.length, 3)
  const plugin = publicCatalogPlugin(options(fixturePath))
  const emitted = []
  plugin.generateBundle.call({ emitFile(file) { emitted.push(file) } })
  assert.deepEqual(Buffer.from(emitted[1].source), original)
  assert.equal(JSON.parse(emitted[0].source).catalog.sha256, sha(Buffer.from(emitted[1].source)))
  assert.equal(plugin.transformIndexHtml()[0].attrs.content, catalog.revision)
  assert.match(plugin.load(plugin.resolveId('virtual:aster-public-catalog')), /Object\.freeze/)
})

test('configured invalid inputs fail instead of silently reverting to another catalog', t => {
  const directory = mkdtempSync(join(tmpdir(), 'aster-public-catalog-'))
  t.after(() => removeTestDirectory(directory))
  const path = join(directory, 'plans.json')
  const reject = bytes => { writeFileSync(path, bytes); assert.throws(() => readPublicCatalog(options(path, bytes))) }
  reject(Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), original]))
  reject(Buffer.concat([original, Buffer.from('\n')]))
  reject(Buffer.from(original.toString().replace('"environment":"local"', '"environment":"production"')))
  reject(Buffer.from(original.toString().replace('"schema":"aster.public-plans.v1"', '"schema":"bad","schema":"aster.public-plans.v1"')))
  for (const mutate of [
    c => { c.private_key = 'test-private' },
    c => { c.plans[0].approved_by = 'test-operator' },
    c => { c.plans[0].entitlements.quotas[0].limit.secret = 'internal' },
    c => { c.plans[0].entitlements.quotas.pop() },
    c => { c.plans[1].offer.terms[1].total_amount_minor++ },
    c => { c.plans[1].offer.terms[1].years = 1 },
    c => { c.plans.push(structuredClone(c.plans[0])) },
    c => { c.revision = `catalog_${'a'.repeat(48)}` },
  ]) { const changed = structuredClone(catalog); mutate(changed); reject(Buffer.from(canonicalJson(changed))) }
  writeFileSync(path, original)
  assert.throws(() => readPublicCatalog({ ...options(path), ASTER_WEBSITE_CATALOG_SHA256: '0'.repeat(64) }), /mismatch/)
  assert.throws(() => readPublicCatalog({ ...options(path), ASTER_WEBSITE_CATALOG_PATH: join(directory, 'missing.json') }))
  assert.throws(() => readPublicCatalog({ ASTER_WEBSITE_CATALOG_ENVIRONMENT: 'local' }), /requires/)
})

test('unconfigured and approved empty catalogs remain distinct and no default plans are fabricated', t => {
  assert.equal(readPublicCatalog({}), null)
  const directory = mkdtempSync(join(tmpdir(), 'aster-public-count-'))
  t.after(() => removeTestDirectory(directory))
  const path = join(directory, 'plans.json')
  for (const count of [0, 1, 3, 8]) {
    const changed = structuredClone(catalog)
    changed.plans = Array.from({ length: count }, (_, i) => ({ ...structuredClone(catalog.plans[i % 3]), plan_id: `test_plan_${i}` }))
    const bytes = Buffer.from(canonicalJson(changed))
    writeFileSync(path, bytes)
    assert.equal(readPublicCatalog(options(path, bytes)).catalog.plans.length, count)
  }
})
