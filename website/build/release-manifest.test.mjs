import test from 'node:test'
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { createReleaseManifest } from './release-manifest.mjs'

test('release manifest describes exact emitted runtime and excludes internal build metadata', () => {
  const identity = { revision: `catalog_${'a'.repeat(48)}`, sha256: 'b'.repeat(64), environment: 'local', path: `/catalog/catalog_${'a'.repeat(48)}/plans.json` }
  const bundle = Object.fromEntries(['index.html', 'catalog-manifest.json', identity.path.slice(1), 'assets/app.css'].map(path => [path, { type: 'asset', source: `bytes for ${path}` }]))
  bundle['assets/app.js'] = { type: 'chunk', code: 'globalThis.version=1', moduleIds: ['/private/checkout.js'] }
  const raw = createReleaseManifest(bundle, identity)
  const manifest = JSON.parse(raw)
  assert.deepEqual(manifest.catalog, identity)
  assert.equal(manifest.files.length, 5)
  assert.ok(!raw.includes('/private/'))
  const script = manifest.files.find(file => file.path.endsWith('.js'))
  assert.equal(script.sha256, createHash('sha256').update(bundle['assets/app.js'].code).digest('hex'))
  assert.equal(createReleaseManifest(Object.fromEntries(Object.entries(bundle).reverse()), identity), raw)
  const changed = structuredClone(bundle); changed['assets/app.js'].code += ';'
  assert.notEqual(createReleaseManifest(changed, identity), raw)
  assert.equal(createReleaseManifest(bundle, null), null)
  for (const required of ['index.html', 'catalog-manifest.json', identity.path.slice(1)]) {
    const incomplete = { ...bundle }; delete incomplete[required]
    assert.throws(() => createReleaseManifest(incomplete, identity), /missing/)
  }
  for (const path of ['../escape', '//host/path', 'assets/a?b', 'assets/a%2Fb']) {
    assert.throws(() => createReleaseManifest({ ...bundle, [path]: { type: 'asset', source: '' } }, identity), /path/)
  }
})
