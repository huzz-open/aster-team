import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { parse } from 'yaml'
import { verifyStaticElf } from './verify-static-elf.mjs'
import { verifyArchive } from './verify-static-archive.mjs'
import { validateLock } from './ci-images.mjs'
import { maintenanceTargets } from './base-images.mjs'
import { npmWorkspacePaths } from './npm-workspace-dependencies.mjs'
const read = file => readFileSync(new URL(`../../${file}`, import.meta.url), 'utf8')

test('npm workspace dependency paths include every declared repository workspace', () => {
  assert.deepEqual(npmWorkspacePaths(JSON.parse(read('package.json'))), [
    'customer/admin', 'customer/demo', 'customer/member', 'operations/console',
    'website', 'customer/sdk', 'packages/ui',
  ])
})

test('npm workspace dependency paths deduplicate static paths without changing their order', () => {
  const manifest = { workspaces: ['packages/ui', 'website', 'packages/ui', 'packages/.shared_UI-2.0'] }
  const original = structuredClone(manifest)
  assert.deepEqual(npmWorkspacePaths(manifest), ['packages/ui', 'website', 'packages/.shared_UI-2.0'])
  assert.deepEqual(manifest, original)
})

test('npm workspace dependency paths reject unsupported manifests and unsafe mount paths', () => {
  for (const manifest of [undefined, null, {}, { workspaces: [] }, { workspaces: 'website' },
    { workspaces: { packages: ['website'] } }]) {
    assert.throws(() => npmWorkspacePaths(manifest), /workspaces must be a non-empty array/)
  }
  for (const workspace of [null, false, 42, {}, [], '', '.', '..', '/absolute', 'C:/outside',
    'C:\\outside', '\\\\host\\share', 'customer\\admin', './website', '../website',
    'customer/./admin', 'customer/../admin', 'customer//admin', 'website/', 'customer/*',
    'packages/**', '[website]', '{admin,member}', '!website', 'website\n', '\twebsite',
    'customer/\0admin', 'customer/\r\nadmin', 'web site', 'customer:admin']) {
    assert.throws(() => npmWorkspacePaths({ workspaces: ['website', workspace] }),
      /Invalid npm workspace path at index 1/)
  }
})

test('npm workspace dependency CLI emits validated paths only and requires an absolute manifest path', () => {
  const directory = mkdtempSync(join(tmpdir(), 'aster npm-workspaces-'))
  const helper = fileURLToPath(new URL('./npm-workspace-dependencies.mjs', import.meta.url))
  const manifestPath = join(directory, 'package.json')
  const run = args => spawnSync(process.execPath, [helper, ...args], {
    cwd: directory, encoding: 'utf8', windowsHide: true,
  })
  try {
    writeFileSync(manifestPath, JSON.stringify({ workspaces: ['customer/admin', 'website', 'website'] }))
    const valid = run([manifestPath])
    assert.equal(valid.status, 0, valid.error?.message || valid.stderr)
    assert.equal(valid.stdout, 'customer/admin\nwebsite\n')
    assert.equal(valid.stderr, '')
    for (const args of [[], ['package.json'], [manifestPath, 'extra']]) {
      const invalid = run(args)
      assert.equal(invalid.status, 1)
      assert.equal(invalid.stdout, '')
      assert.match(invalid.stderr, /Usage:.*absolute/)
    }
    writeFileSync(manifestPath, JSON.stringify({ workspaces: ['website', '../outside'] }))
    const unsafe = run([manifestPath])
    assert.equal(unsafe.status, 1)
    assert.equal(unsafe.stdout, '')
    assert.match(unsafe.stderr, /Invalid npm workspace path/)
    writeFileSync(manifestPath, '{')
    const malformed = run([manifestPath])
    assert.equal(malformed.status, 1)
    assert.equal(malformed.stdout, '')
    assert.notEqual(malformed.stderr, '')
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

function elf(type = 1, tag = 0n) {
  const bytes = Buffer.alloc(160)
  bytes.set([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1])
  bytes.writeUInt16LE(2, 16); bytes.writeUInt16LE(62, 18)
  bytes.writeBigUInt64LE(64n, 32); bytes.writeUInt16LE(56, 54); bytes.writeUInt16LE(1, 56)
  bytes.writeUInt32LE(type, 64); bytes.writeBigUInt64LE(128n, 72); bytes.writeBigUInt64LE(32n, 96)
  bytes.writeBigInt64LE(tag, 128)
  return bytes
}

test('archive inspection supports absolute paths with spaces and leading-dot archive entries', () => {
  const directory = mkdtempSync(join(tmpdir(), 'aster archive-'))
  try {
    mkdirSync(join(directory, 'package/bin'), { recursive: true })
    for (const name of ['aster-team-cli', 'aster-control', 'aster-runner', 'caddy']) {
      writeFileSync(join(directory, 'package/bin', name), elf())
    }
    const tar = spawnSync('tar', ['-czf', './fixture.tar.gz', './package'], { cwd: directory, encoding: 'utf8', windowsHide: true })
    assert.equal(tar.status, 0, tar.error?.message || tar.stderr)
    assert.equal(verifyArchive(join(directory, 'fixture.tar.gz')).length, 4)
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})
test('static executable and static PIE are accepted; runtime linker and library references fail', () => {
  verifyStaticElf(elf())
  const pie = elf(2); pie.writeUInt16LE(3, 16); verifyStaticElf(pie)
  assert.throws(() => verifyStaticElf(elf(3)), /interpreter/)
  for (const tag of [1n, 0x7ffffffdn, 0x7fffffffn]) assert.throws(() => verifyStaticElf(elf(2, tag)), /shared library/)
})
test('malformed executables cannot pass by producing empty inspection output', () => {
  for (const bytes of [Buffer.alloc(0), Buffer.from('#!/bin/sh'), elf().subarray(0, 80)]) assert.throws(() => verifyStaticElf(bytes))
  const badOffset = elf(); badOffset.writeBigUInt64LE(0xffffffffffffffffn, 32)
  assert.throws(() => verifyStaticElf(badOffset), /headers/)
  const badArchitecture = elf(); badArchitecture.writeUInt16LE(183, 18)
  assert.throws(() => verifyStaticElf(badArchitecture), /x86-64/)
  const unterminated = elf(2, 7n); unterminated.writeBigInt64LE(8n, 144)
  assert.throws(() => verifyStaticElf(unterminated), /unterminated/)
})
test('image lock requires every runtime and separate compiler, exact namespace and immutable digests', () => {
  const targets = maintenanceTargets(JSON.parse(read('contracts/release-platforms.json')))
  const lock = { schema_version: 1, platform: 'linux/amd64', images: targets.map(target => ({ target: target.id,
    image: `${target.repository}@sha256:${'a'.repeat(64)}`, recipe: 'b'.repeat(64), image_id: `sha256:${'c'.repeat(64)}` })) }
  assert.equal(validateLock(lock, targets).length, 7)
  const metadata = structuredClone(lock)
  Object.assign(metadata.images[0], { base: `ubuntu@sha256:${'d'.repeat(64)}`, kind: 'builder', family: 'toolchain' })
  const resolved = validateLock(metadata, targets)[0]
  assert.equal(resolved.base, targets[0].base)
  assert.equal(resolved.kind, 'runtime')
  assert.equal(resolved.family, 'apt')
  assert.equal(resolved.upstream_base, metadata.images[0].base)
  for (const mutate of [value => value.images.pop(), value => { value.images[0].image = 'ubuntu:latest' },
    value => { value.images[0] = value.images[1] }, value => { value.platform = 'linux/arm64' }]) {
    const changed = structuredClone(lock); mutate(changed); assert.throws(() => validateLock(changed, targets))
  }
})
test('runtime Dockerfiles contain only the declared baseline and inventory, never compilation packages', () => {
  for (const file of ['apt', 'dnf']) {
    const source = read(`scripts/ci/systemd/${file}.Dockerfile`)
    assert.doesNotMatch(source, /build-essential|musl-tools|pkg-config|libssl-dev|libssl-devel|libsqlite3-dev|nodejs|rustup|COPY --from/)
    assert.match(source, /packages\.tsv/)
  }
  assert.match(read('scripts/ci/linux-lab.Dockerfile'), /build-essential/)
  assert.match(read('scripts/ci/linux-lab.Dockerfile'), /musl-tools/)
  const toolchains = JSON.parse(read('tools/linux-build-runtime.json'))
  const builder = read('scripts/ci/linux-lab.Dockerfile')
  for (const name of ['node', 'go', 'rust']) assert.ok(builder.includes(`ARG ${name.toUpperCase()}_IMAGE=${toolchains[`${name}_image`]}`))
})
test('runtime consumers never build bases or modify OS packages, and verify artifacts first', () => {
  const smoke = read('scripts/ci/exercise-supported-linux-install.sh')
  const lab = read('scripts/ci/linux-lab.sh')
  assert.doesNotMatch(lab, /--env (?:"?GIT_DIR|GIT_WORK_TREE|GIT_COMMON_DIR)=/)
  assert.match(lab, /container\.git.*\/workspace\/\.git:ro/)
  for (const source of [smoke, lab, read('scripts/ci/build-production-linux-in-docker.sh')]) assert.doesNotMatch(source, /docker_cli build/)
  assert.match(smoke, /verify-static-archive\.mjs/)
  assert.ok(smoke.indexOf('verify-static-archive.mjs') < smoke.indexOf('docker_cli run'))
  assert.equal((smoke.match(/check-runtime-baseline\.sh/g) || []).length, 2)
  const compose = parse(read('tests/system-e2e/docker-compose.yml'))
  for (const id of ['customer-json', 'customer-qr', 'customer-terminal']) {
    assert.equal(compose.services[id].build, undefined)
  }
  assert.equal(compose['x-customer'].build, undefined)
  assert.match(compose['x-customer'].image, /ASTER_E2E_RUNTIME_IMAGE/)
  const e2e = read('tests/system-e2e/run.mjs')
  assert.match(e2e, /verifyArchive\(candidateInfo/)
  assert.equal((e2e.match(/checkBaselines\(\)/g) || []).length, 2)
  for (const source of [smoke, e2e, read('scripts/ci/exercise-customer-install.sh'), read('scripts/ci/exercise-runner-install.sh')]) {
    assert.doesNotMatch(source, /(?:apt-get|apt|dnf|yum|apk)\s+(?:[^\n;]*\s)?install\s/)
    assert.doesNotMatch(source, /LD_PRELOAD|LD_LIBRARY_PATH/)
  }
})
test('Actions consume public pinned images without package credentials', () => {
  for (const file of ['customer-release', 'verify', 'system-e2e']) {
    const source = read(`.github/workflows/${file}.yml`)
    const workflow = parse(source)
    assert.equal(workflow.permissions.packages, undefined)
    assert.doesNotMatch(source, /ci:images:(build|publish|update|resume)/)
    assert.match(source, /prepare-ci-images/)
    if (file !== 'system-e2e') {
      assert.match(source, /ci:builder/)
      assert.doesNotMatch(source, /apt-get install --yes musl-tools/)
    }
  }
  const preparation = read('.github/actions/prepare-ci-images/action.yml')
  parse(preparation)
  assert.doesNotMatch(preparation, /docker login|docker\/login-action|ghcr\.io.*token/i)
})
