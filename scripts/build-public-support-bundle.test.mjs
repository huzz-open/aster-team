import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { relative, resolve, sep } from 'node:path'
import test from 'node:test'
import { parse } from 'yaml'
import { buildPublicSupportBundle, verifyPublicSupportBundle } from './build-public-support-bundle.mjs'

const sha256 = bytes => createHash('sha256').update(bytes).digest('hex')

function leafFiles(root, directory = root) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = resolve(directory, entry.name)
    return entry.isDirectory() ? leafFiles(root, path) : [relative(root, path).split(sep).join('/')]
  }).sort()
}

function fixture(version = '2.0.1-test.1', platform = 'linux') {
  const root = mkdtempSync(resolve(tmpdir(), 'aster-public-support-'))
  const name = `aster-team-${version}-${platform}-amd64.tar.gz`
  const archivePath = resolve(root, name)
  const checksumPath = `${archivePath}.sha256`
  const bundleName = name.slice(0, -'.tar.gz'.length)
  const bundle = resolve(root, bundleName)
  const license = Buffer.from(JSON.stringify({ claims: {
    schema: 'aster.license.v2', product: 'aster-team', license_id: 'test_free', plan_id: 'test_plan', plan_version: 2,
    edition: 'test', minimum_version: '2.0.0', quota_policy_version: 1,
    source: { kind: 'free_distribution' }, binding: { mode: 'unbound' }, validity: { expiry: { mode: 'none' } },
    entitlements: { catalog_version: 1, features: ['gateway'], quotas: [{ id: 'member_seats', limit: { mode: 'limited', value: 3 } }] },
  }, signature: 'test' }))
  mkdirSync(resolve(bundle, 'licenses'), { recursive: true })
  writeFileSync(resolve(bundle, 'licenses/free-license.json'), license)
  writeFileSync(resolve(bundle, 'RELEASE.json'), JSON.stringify({
    schema: 'aster.release-manifest.v1', key_id: 'test-release', product: 'aster-team', version,
    platform, architecture: 'amd64', signature: 'test-signature',
    files: [{ path: 'licenses/free-license.json', size: license.length, sha256: sha256(license) }],
  }))
  execFileSync('tar', ['-czf', archivePath, '-C', root, bundleName], { windowsHide: true })
  const bytes = readFileSync(archivePath)
  writeFileSync(checksumPath, `${sha256(bytes)}  ${name}\n`)
  return { root, version, platform, name, archivePath, checksumPath, bytes, license }
}

test('public support bundle contains only reviewed docs, issue templates and exact release identity', () => {
  const input = fixture()
  const outputPath = resolve(input.root, 'output')
  const manifest = buildPublicSupportBundle({ ...input, outputPath })
  assert.equal(manifest.environment, 'local')
  assert.deepEqual(manifest.artifact, {
    name: input.name,
    sha256: sha256(input.bytes),
    size_bytes: input.bytes.length,
    platform: 'linux-amd64',
  })
  assert.equal(manifest.release_manifest.key_id, 'test-release')
  assert.deepEqual(manifest.bundled_license, {
    path: 'licenses/free-license.json', sha256: sha256(input.license), size_bytes: input.license.length,
    license_id: 'test_free', plan_id: 'test_plan', plan_version: 2, edition: 'test', minimum_version: '2.0.0',
    entitlements: { catalog_version: 1, features: ['gateway'], quotas: [{ id: 'member_seats', limit: { mode: 'limited', value: 3 } }] },
    quota_policy_version: 1, binding: 'unbound', source: 'free_distribution', expiry: { mode: 'none' },
  })
  assert.deepEqual(readFileSync(resolve(outputPath, 'docs/user-manual.md')), readFileSync(new URL('../docs/user-manual.md', import.meta.url)))
  assert.deepEqual(readFileSync(resolve(outputPath, 'README-LINUX.md')), readFileSync(new URL('../README-LINUX.md', import.meta.url)))
  assert.match(readFileSync(resolve(outputPath, 'README.md'), 'utf8'), /不包含 Aster Team 产品源代码/)
  assert.equal(readFileSync(resolve(outputPath, `releases/${input.version}/SHA256SUMS`), 'utf8'), `${sha256(input.bytes)}  ${input.name}\n`)
  const persisted = JSON.parse(readFileSync(resolve(outputPath, `releases/${input.version}/manifest.json`), 'utf8'))
  assert.deepEqual(persisted, manifest)
  assert.deepEqual(leafFiles(outputPath), [
    '.github/ISSUE_TEMPLATE/bug.yml',
    '.github/ISSUE_TEMPLATE/config.yml',
    '.github/ISSUE_TEMPLATE/installation.yml',
    'README-LINUX.md',
    'README.md',
    'docs/release-verification.md',
    'docs/user-manual.md',
    `releases/${input.version}/manifest.json`,
    `releases/${input.version}/RELEASE_NOTES.md`,
    `releases/${input.version}/SHA256SUMS`,
  ].sort())
  for (const path of ['bug.yml', 'config.yml', 'installation.yml']) {
    assert.doesNotThrow(() => parse(readFileSync(resolve(outputPath, '.github/ISSUE_TEMPLATE', path), 'utf8')))
  }
  assert.ok(manifest.files.every(file => !file.path.includes('internal') && !file.path.includes('src/')))
  assert.deepEqual(verifyPublicSupportBundle({ version: input.version, bundlePath: outputPath, expectedEnvironment: 'local' }), manifest)
})

test('public support bundle rejects mismatched artifacts and immutable output reuse', () => {
  const input = fixture()
  writeFileSync(input.checksumPath, `${'0'.repeat(64)}  ${input.name}\n`)
  assert.throws(() => buildPublicSupportBundle({ ...input, outputPath: resolve(input.root, 'bad') }), /checksum/)
  writeFileSync(input.checksumPath, `${sha256(input.bytes)}  ${input.name}\n`)
  const outputPath = resolve(input.root, 'good')
  buildPublicSupportBundle({ ...input, outputPath })
  assert.throws(() => buildPublicSupportBundle({ ...input, outputPath }), /already exists/)
})

test('public support verification rejects files added, removed or changed after export', () => {
  const input = fixture()
  const outputPath = resolve(input.root, 'output')
  buildPublicSupportBundle({ ...input, outputPath })
  writeFileSync(resolve(outputPath, 'unexpected-source.rs'), 'fn main() {}\n')
  assert.throws(() => verifyPublicSupportBundle({ version: input.version, bundlePath: outputPath }), /file whitelist/)
  rmSync(resolve(outputPath, 'unexpected-source.rs'))
  writeFileSync(resolve(outputPath, 'README.md'), 'changed after review\n')
  assert.throws(() => verifyPublicSupportBundle({ version: input.version, bundlePath: outputPath }), /does not match manifest/)
  rmSync(resolve(outputPath, 'README.md'))
  assert.throws(() => verifyPublicSupportBundle({ version: input.version, bundlePath: outputPath }), /file whitelist/)
})


test('Windows support export is explicitly experimental and rejects a Linux identity', t => {
  const input = fixture('2.0.1-test.1', 'windows')
  t.after(() => rmSync(input.root, { recursive: true, force: true }))
  const outputPath = resolve(input.root, 'windows-output')
  const manifest = buildPublicSupportBundle({ ...input, outputPath })
  assert.equal(manifest.artifact.platform, 'windows-amd64')
  assert.equal(manifest.artifact.channel, 'experimental')
  assert.match(readFileSync(resolve(outputPath, 'README-WINDOWS.md'), 'utf8'), /不承诺稳定/)
  assert.match(readFileSync(resolve(outputPath, `releases/${input.version}/RELEASE_NOTES.md`), 'utf8'), /Windows amd64 实验版/)
  assert.deepEqual(verifyPublicSupportBundle({ version: input.version, bundlePath: outputPath, platform: 'windows' }), manifest)
  assert.throws(() => verifyPublicSupportBundle({ version: input.version, bundlePath: outputPath }), /whitelist|identity/)
  const path = resolve(outputPath, `releases/${input.version}/manifest.json`)
  manifest.artifact.channel = 'stable'
  writeFileSync(path, `${JSON.stringify(manifest)}\n`)
  assert.throws(() => verifyPublicSupportBundle({ version: input.version, bundlePath: outputPath, platform: 'windows' }), /artifact identity/)
})
