import { build } from 'vite'
import { createHash } from 'node:crypto'
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs'
import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { canonicalJson } from '../build/public-catalog.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const source = resolve(root, 'contracts/test-vectors/public-catalog.v1.json')
const bytes = readFileSync(source)
const catalog = JSON.parse(bytes)
const releaseVersion = '2.0.1-test.1'
const releaseRoot = resolve(root, 'dist/website-validation/release-input')
const releaseArchiveName = `aster-team-${releaseVersion}-linux-amd64.tar.gz`
const releaseArchive = Buffer.from('test-only browser downloadable archive')
const releaseArchivePath = resolve(releaseRoot, releaseArchiveName)
const releaseFiles = new Map([
  ['docs/user-manual.md', Buffer.from('# Aster Team browser test manual\n')],
  ['README-LINUX.md', Buffer.from('# Aster Team browser test Linux guide\n')],
  [`releases/${releaseVersion}/SHA256SUMS`, Buffer.from(`${createHash('sha256').update(releaseArchive).digest('hex')}  ${releaseArchiveName}\n`)],
])
for (const [path, content] of releaseFiles) {
  const target = resolve(releaseRoot, path)
  mkdirSync(dirname(target), { recursive: true })
  writeFileSync(target, content)
}
mkdirSync(releaseRoot, { recursive: true })
writeFileSync(releaseArchivePath, releaseArchive)
const releaseManifest = {
  schema: 'aster.public-support-release.v1', environment: 'local', product: 'aster-team', version: releaseVersion,
  artifact: { name: releaseArchiveName, sha256: createHash('sha256').update(releaseArchive).digest('hex'), size_bytes: releaseArchive.length, platform: 'linux-amd64' },
  release_manifest: { path: 'RELEASE.json', sha256: '2'.repeat(64), key_id: 'test-release' },
  bundled_license: {
    path: 'licenses/free-license.json', sha256: '1'.repeat(64), size_bytes: 321, license_id: 'test_free',
    plan_id: catalog.plans[0].plan_id, plan_version: catalog.plans[0].version, edition: catalog.plans[0].edition,
    minimum_version: catalog.plans[0].minimum_version, entitlements: catalog.plans[0].entitlements,
    quota_policy_version: catalog.plans[0].quota_policy_version, binding: 'unbound', source: 'free_distribution', expiry: catalog.plans[0].offer.expiry,
  },
  files: [...releaseFiles].map(([path, content]) => ({ path, sha256: createHash('sha256').update(content).digest('hex'), size_bytes: content.length })),
}
const releaseManifestPath = resolve(releaseRoot, `releases/${releaseVersion}/manifest.json`)
const releaseManifestBytes = Buffer.from(`${JSON.stringify(releaseManifest)}\n`)
writeFileSync(releaseManifestPath, releaseManifestBytes)
const windowsRoot = resolve(releaseRoot, 'windows')
const windowsArchiveName = `aster-team-${releaseVersion}-windows-amd64.tar.gz`
const windowsArchive = Buffer.from('test-only Windows browser downloadable archive')
const windowsArchivePath = resolve(windowsRoot, windowsArchiveName)
const windowsFiles = new Map([
  ['docs/user-manual.md', Buffer.from('# Aster Team browser test manual\n')],
  ['README-WINDOWS.md', Buffer.from('# Windows 实验版\n\n不承诺稳定 缺陷修复可能较慢 建议使用 Linux\n')],
  [`releases/${releaseVersion}/SHA256SUMS`, Buffer.from(`${createHash('sha256').update(windowsArchive).digest('hex')}  ${windowsArchiveName}\n`)],
])
for (const [path, content] of windowsFiles) {
  const target = resolve(windowsRoot, path)
  mkdirSync(dirname(target), { recursive: true })
  writeFileSync(target, content)
}
writeFileSync(windowsArchivePath, windowsArchive)
const windowsManifest = { ...structuredClone(releaseManifest),
  artifact: { name: windowsArchiveName, sha256: createHash('sha256').update(windowsArchive).digest('hex'), size_bytes: windowsArchive.length, platform: 'windows-amd64', channel: 'experimental' },
  files: [...windowsFiles].map(([path, content]) => ({ path, sha256: createHash('sha256').update(content).digest('hex'), size_bytes: content.length })),
}
const windowsManifestPath = resolve(windowsRoot, `releases/${releaseVersion}/manifest.json`)
const windowsManifestBytes = Buffer.from(`${JSON.stringify(windowsManifest)}\n`)
writeFileSync(windowsManifestPath, windowsManifestBytes)
Object.assign(process.env, {
  ASTER_WEBSITE_CATALOG_PATH: source,
  ASTER_WEBSITE_CATALOG_SHA256: createHash('sha256').update(bytes).digest('hex'),
  ASTER_WEBSITE_CATALOG_REVISION: catalog.revision,
  ASTER_WEBSITE_CATALOG_ENVIRONMENT: 'local',
  VITE_ASTER_TURNSTILE_SITE_KEY: 'isolated-browser-sitekey',
  VITE_ASTER_CONTACT_EMAIL: 'test@example.invalid',
  VITE_ASTER_LEAD_ENDPOINT: '/api/trial',
})
const variants = process.argv.includes('--matrix') ? ['actual', 'empty', 'single', 'many', 'unconfigured', 'subscription'] : ['actual']
for (const variant of variants) {
  const releaseEnvironment = {
    ASTER_WEBSITE_WINDOWS_RELEASE_MANIFEST_PATH: windowsManifestPath,
    ASTER_WEBSITE_WINDOWS_RELEASE_MANIFEST_SHA256: createHash('sha256').update(windowsManifestBytes).digest('hex'),
    ASTER_WEBSITE_WINDOWS_RELEASE_ENVIRONMENT: 'local',
    ASTER_WEBSITE_WINDOWS_RELEASE_ARCHIVE_PATH: windowsArchivePath,
    ASTER_WEBSITE_PRODUCT_RELEASE_MANIFEST_PATH: releaseManifestPath,
    ASTER_WEBSITE_PRODUCT_RELEASE_MANIFEST_SHA256: createHash('sha256').update(releaseManifestBytes).digest('hex'),
    ASTER_WEBSITE_PRODUCT_RELEASE_ENVIRONMENT: 'local',
    ASTER_WEBSITE_PRODUCT_RELEASE_ARCHIVE_PATH: releaseArchivePath,
  }
  for (const key of Object.keys(releaseEnvironment)) {
    if (variant === 'actual') process.env[key] = releaseEnvironment[key]
    else delete process.env[key]
  }
  if (variant === 'unconfigured') {
    for (const key of ['PATH', 'SHA256', 'REVISION', 'ENVIRONMENT']) delete process.env[`ASTER_WEBSITE_CATALOG_${key}`]
  } else if (variant !== 'actual') {
    // Synthetic local catalogs exercise cardinality, not real publication approval.
    const changed = structuredClone(catalog)
    const count = { empty: 0, single: 1, many: 8, subscription: 1 }[variant]
    changed.plans = Array.from({ length: count }, (_, i) => ({ ...structuredClone(catalog.plans[i % 3]), plan_id: `test_plan_${i}`, name: `本地数量测试 ${i + 1}` }))
    if (variant === 'subscription') {
      changed.plans = [structuredClone(catalog.plans.find(plan => plan.offer.kind === 'fixed_price'))]
      changed.plans[0].entitlements.features = []
      changed.plans[0].entitlements.feature_sets = ['standard']
    }
    changed.revision = `catalog_${createHash('sha256').update(canonicalJson(changed)).digest('hex').slice(0, 48)}`
    const path = resolve(root, `dist/website-validation/catalog-inputs/${variant}/plans.json`)
    mkdirSync(dirname(path), { recursive: true })
    const content = canonicalJson(changed)
    writeFileSync(path, content)
    Object.assign(process.env, { ASTER_WEBSITE_CATALOG_ENVIRONMENT: 'local', ASTER_WEBSITE_CATALOG_PATH: path, ASTER_WEBSITE_CATALOG_REVISION: changed.revision,
      ASTER_WEBSITE_CATALOG_SHA256: createHash('sha256').update(content).digest('hex') })
  }
  await build({ root: resolve(root, 'website'), configFile: resolve(root, 'website/vite.config.ts'), envDir: false,
    logLevel: 'warn', build: { outDir: resolve(root, `dist/website-validation/${variant === 'actual' ? 'site' : `site-${variant}`}`), emptyOutDir: true } })
}
