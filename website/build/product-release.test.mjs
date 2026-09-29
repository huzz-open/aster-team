import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve, sep } from 'node:path'
import test from 'node:test'
import { productReleasePlugin, readProductRelease } from './product-release.mjs'

const sha256 = bytes => createHash('sha256').update(bytes).digest('hex')

function fixture(environment = 'local', platform = 'linux') {
  const root = mkdtempSync(resolve(tmpdir(), 'aster-website-release-'))
  const version = '2.0.1-test.1'
  const archiveName = `aster-team-${version}-${platform}-amd64.tar.gz`
  const archive = Buffer.from('test-only website release archive')
  const documents = new Map([
    ['docs/user-manual.md', Buffer.from('# User manual\n')],
    [platform === 'windows' ? 'README-WINDOWS.md' : 'README-LINUX.md', Buffer.from('# Installation guide\n')],
    [`releases/${version}/SHA256SUMS`, Buffer.from(`${sha256(archive)}  ${archiveName}\n`)],
  ])
  const freePlan = {
    plan_id: 'test_plan', version: 2, name: 'Free', description: 'Test', edition: 'test', minimum_version: '2.0.0', quota_policy_version: 1,
    offer: { kind: 'free', expiry: { mode: 'none' } }, support_terms_version: 'test-support-v1',
    entitlements: { catalog_version: 1, features: ['gateway'], quotas: [{ id: 'member_seats', limit: { mode: 'limited', value: 3 } }] },
  }
  const files = [...documents].map(([path, bytes]) => ({ path, sha256: sha256(bytes), size_bytes: bytes.length }))
  const manifest = { schema: 'aster.public-support-release.v1', environment, product: 'aster-team', version,
    artifact: { name: archiveName, sha256: sha256(archive), size_bytes: archive.length, platform: `${platform}-amd64`, ...(platform === 'windows' ? { channel: 'experimental' } : {}) }, files }
  manifest.release_manifest = { path: 'RELEASE.json', sha256: '2'.repeat(64), key_id: 'test-release' }
  manifest.bundled_license = {
    path: 'licenses/free-license.json', sha256: '1'.repeat(64), size_bytes: 321, license_id: 'test_free',
    plan_id: freePlan.plan_id, plan_version: freePlan.version, edition: freePlan.edition, minimum_version: freePlan.minimum_version,
    entitlements: freePlan.entitlements, quota_policy_version: freePlan.quota_policy_version,
    binding: 'unbound', source: 'free_distribution', expiry: freePlan.offer.expiry,
  }
  for (const [path, bytes] of documents) {
    const target = resolve(root, path)
    mkdirSync(resolve(target, '..'), { recursive: true })
    writeFileSync(target, bytes)
  }
  const archivePath = resolve(root, archiveName)
  const manifestPath = resolve(root, `releases/${version}/manifest.json`)
  const manifestBytes = Buffer.from(`${JSON.stringify(manifest)}\n`)
  writeFileSync(archivePath, archive)
  writeFileSync(manifestPath, manifestBytes)
  const env = {
    ASTER_WEBSITE_PRODUCT_RELEASE_MANIFEST_PATH: manifestPath,
    ASTER_WEBSITE_PRODUCT_RELEASE_MANIFEST_SHA256: sha256(manifestBytes),
    ASTER_WEBSITE_PRODUCT_RELEASE_ENVIRONMENT: environment,
    ASTER_WEBSITE_PRODUCT_RELEASE_ARCHIVE_PATH: archivePath,
    ...(environment === 'production' ? { ASTER_WEBSITE_PRODUCT_RELEASE_DOWNLOAD_URL: `https://downloads.example.invalid/releases/${version}/${archiveName}` } : {}),
  }
  return { root, archive, archiveName, archivePath, manifestPath, manifest, manifestBytes, env, catalog: { environment, plans: [freePlan] } }
}

function cleanup(root) {
  const target = resolve(root)
  assert.ok(target.startsWith(`${resolve(tmpdir())}${sep}aster-website-release-`))
  rmSync(target, { recursive: true, force: true })
}

test('verified local product release exposes exact metadata, documents and downloadable archive', t => {
  const input = fixture()
  t.after(() => cleanup(input.root))
  const snapshot = readProductRelease(input.env, input.catalog)
  assert.equal(snapshot.release.artifact.sha256, sha256(input.archive))
  assert.equal(snapshot.release.artifact.url, `/downloads/${input.archiveName}`)
  assert.deepEqual(snapshot.documents.manual, Buffer.from('# User manual\n'))

  const plugin = productReleasePlugin(input.env, input.catalog)
  const emitted = []
  plugin.generateBundle.call({ emitFile(file) { emitted.push(file) } })
  assert.deepEqual(emitted.map(file => file.fileName).sort(), [
    'downloads/2.0.1-test.1/SHA256SUMS', 'downloads/README-LINUX.md',
    'downloads/aster-team-user-manual.md', 'product-release.json',
  ])
  const publicManifest = JSON.parse(emitted.find(file => file.fileName === 'product-release.json').source)
  assert.equal(publicManifest.release.support_manifest.sha256, sha256(input.manifestBytes))
  const output = resolve(input.root, 'site')
  plugin.writeBundle({ dir: output })
  assert.deepEqual(readFileSync(resolve(output, `downloads/${input.archiveName}`)), input.archive)
  assert.throws(() => plugin.writeBundle({ dir: output }), /already exists/)
})

test('product release is optional but every configured identity and byte source fails closed', t => {
  assert.equal(readProductRelease({}), null)
  const input = fixture()
  t.after(() => cleanup(input.root))
  assert.throws(() => readProductRelease({ ASTER_WEBSITE_PRODUCT_RELEASE_ENVIRONMENT: 'local' }), /requires/)
  assert.throws(() => readProductRelease({ ...input.env, ASTER_WEBSITE_PRODUCT_RELEASE_MANIFEST_SHA256: '0'.repeat(64) }, input.catalog), /manifest SHA-256/)
  assert.throws(() => readProductRelease(input.env, { environment: 'local', plans: [] }), /public free plan/)
  assert.throws(() => readProductRelease(input.env, { ...input.catalog, environment: 'production' }), /environments do not match/)
  const changedCatalog = structuredClone(input.catalog)
  changedCatalog.plans[0].entitlements.quotas[0].limit.value = 4
  assert.throws(() => readProductRelease(input.env, changedCatalog), /does not match/)
  writeFileSync(input.archivePath, 'changed')
  assert.throws(() => readProductRelease(input.env, input.catalog), /artifact does not match/)
})

test('catalog and product release environments must match in both directions', t => {
  const local = fixture('local')
  const production = fixture('production')
  t.after(() => { cleanup(local.root); cleanup(production.root) })
  assert.throws(() => readProductRelease(local.env, production.catalog), /environments do not match/)
  assert.throws(() => readProductRelease(production.env, local.catalog), /environments do not match/)
})

test('production product release requires an external HTTPS artifact and never copies it into website output', t => {
  const input = fixture('production')
  t.after(() => cleanup(input.root))
  assert.throws(() => readProductRelease({ ...input.env, ASTER_WEBSITE_PRODUCT_RELEASE_DOWNLOAD_URL: '' }, input.catalog), /HTTPS/)
  assert.throws(() => readProductRelease({ ...input.env, ASTER_WEBSITE_PRODUCT_RELEASE_DOWNLOAD_URL: '/downloads/local.tar.gz' }, input.catalog), /HTTPS/)
  assert.throws(() => readProductRelease({ ...input.env, ASTER_WEBSITE_PRODUCT_RELEASE_DOWNLOAD_URL: 'https://downloads.example.invalid/other.tar.gz' }, input.catalog), /download URL/)
  const plugin = productReleasePlugin(input.env, input.catalog)
  const output = resolve(input.root, 'site')
  plugin.writeBundle({ dir: output })
  assert.throws(() => readFileSync(resolve(output, `downloads/${input.archiveName}`)))
})


test('Windows experimental download is separately verified and keeps Linux recommended', t => {
  const linux = fixture()
  const windows = fixture('local', 'windows')
  t.after(() => { cleanup(linux.root); cleanup(windows.root) })
  const windowsEnv = Object.fromEntries(Object.entries(windows.env).map(([key, value]) => [key.replace('PRODUCT_RELEASE', 'WINDOWS_RELEASE'), value]))
  const env = { ...linux.env, ...windowsEnv }
  const plugin = productReleasePlugin(env, linux.catalog)
  const emitted = []
  plugin.generateBundle.call({ emitFile(file) { emitted.push(file) } })
  const manifest = JSON.parse(emitted.find(file => file.fileName === 'product-release.json').source)
  assert.equal(manifest.release.platform, 'linux-amd64')
  assert.equal(manifest.release.windows.artifact.channel, 'experimental')
  assert.equal(manifest.release.windows.platform, 'windows-amd64')
  assert.ok(emitted.some(file => file.fileName === 'downloads/windows/README-WINDOWS.md'))
  assert.ok(emitted.some(file => file.fileName === `downloads/windows/${windows.manifest.version}/SHA256SUMS`))
  const output = resolve(linux.root, 'two-platform-site')
  plugin.writeBundle({ dir: output })
  for (const input of [linux, windows]) assert.deepEqual(readFileSync(resolve(output, `downloads/${input.archiveName}`)), input.archive)
  assert.throws(() => productReleasePlugin(windowsEnv, linux.catalog), /requires the recommended Linux release/)
  const changed = structuredClone(windows.manifest)
  changed.artifact.channel = 'stable'
  const bytes = `${JSON.stringify(changed)}\n`
  writeFileSync(windows.manifestPath, bytes)
  assert.throws(() => productReleasePlugin({ ...env, ASTER_WEBSITE_WINDOWS_RELEASE_MANIFEST_SHA256: sha256(bytes) }, linux.catalog), /artifact identity/)
  assert.throws(() => productReleasePlugin({ ...linux.env, ASTER_WEBSITE_WINDOWS_RELEASE_MANIFEST_PATH: windows.manifestPath }, linux.catalog), /requires absolute/)
})
