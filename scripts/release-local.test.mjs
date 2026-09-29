import assert from 'node:assert/strict'
import { createPrivateKey, createPublicKey, generateKeyPairSync, randomBytes } from 'node:crypto'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { parse } from 'yaml'

import {
  assertReleaseOutputsAbsent,
  loadLocalReleaseSecurity,
  parseReleaseArguments,
  publishStagedArtifacts,
  releaseVerificationCommands,
  releaseOutputPaths,
  requireWindowsSystemPerl,
  smokeWindowsArchive,
  windowsReleaseTestsEnabled,
} from './release-local.mjs'

const licensePolicy = JSON.parse(readFileSync(new URL('../contracts/test-vectors/license-trust.v1.json', import.meta.url), 'utf8')).keyring[0].policy

function keyringEntry(keyID, publicKey) {
  return {
    key_id: keyID,
    public_key_spki: publicKey.export({ format: 'der', type: 'spki' }).toString('base64url'),
  }
}

function releasePrivateKey(seed) {
  const prefix = Buffer.from('302e020100300506032b657004220420', 'hex')
  return createPrivateKey({ key: Buffer.concat([prefix, seed]), format: 'der', type: 'pkcs8' })
}

function fixture() {
  const temporary = mkdtempSync(join(tmpdir(), 'aster-release-local-test-'))
  const repository = join(temporary, 'repository')
  const security = join(temporary, 'security')
  mkdirSync(repository)
  mkdirSync(security)
  const license = generateKeyPairSync('ed25519')
  const releaseSeed = randomBytes(32)
  const releasePrivate = releasePrivateKey(releaseSeed)
  const pluginSeed = randomBytes(32)
  const pluginPrivate = releasePrivateKey(pluginSeed)
  const pluginPublic = createPublicKey(pluginPrivate).export({ format: 'der', type: 'spki' }).subarray(-32).toString('base64')
  writeFileSync(
    join(security, 'license-v2.public-keyring.json'),
    JSON.stringify([{ ...keyringEntry('license-v2', license.publicKey), policy: licensePolicy }]),
  )
  writeFileSync(
    join(security, 'release-v1.public-keyring.json'),
    JSON.stringify([keyringEntry('release-v1', createPublicKey(releasePrivate))]),
  )
  writeFileSync(join(security, 'release-v1.seed'), releaseSeed)
  writeFileSync(join(security, 'plugin-v1.public-keyring.json'), JSON.stringify(
    ['openai', 'deepseek', 'glm'].map(provider => ({
      key_id: `plugin-${provider}-v1`, bundle_id: `aster.${provider}`, public_key_base64: pluginPublic,
    })),
  ))
  writeFileSync(join(security, 'plugin-v1.seed'), pluginSeed)
  writeFileSync(join(security, 'free-license.json'), JSON.stringify({
    claims: {
      source: { kind: 'free_distribution', distribution_id: 'dist_free_1' },
      binding: { mode: 'unbound' },
      validity: { expiry: { mode: 'none' } },
    },
    signature: 'free-license-fixture',
  }))
  writeFileSync(join(repository, '.env'), `ASTER_LOCAL_SECURITY_CONFIG_DIR='${security}'\n`)
  return { repository, security }
}

test('release arguments require a platform and accept an optional version assertion', () => {
  assert.deepEqual(
    parseReleaseArguments(['--platform=linux']),
    { help: false, version: '', platform: 'linux', buildID: '', verify: 'full', checks: [] },
  )
  assert.deepEqual(
    parseReleaseArguments(['--version=2.0.0', '--platform', 'all']),
    { help: false, version: '2.0.0', platform: 'all', buildID: '', verify: 'full', checks: [] },
  )
  assert.deepEqual(
    parseReleaseArguments(['--version=2.0.0', '--platform=all', '--build-id=50eec16']),
    { help: false, version: '2.0.0', platform: 'all', buildID: '50eec16', verify: 'full', checks: [] },
  )
  assert.throws(() => parseReleaseArguments(['--version=2.0.0']), /--platform is invalid/)
  assert.throws(
    () => parseReleaseArguments(['--version=2.0.0', '--platform=macos']),
    /--platform is invalid/,
  )
  assert.throws(
    () => parseReleaseArguments(['--version=2.0.0', '--platform=all', '--build-id=../../escape']),
    /--build-id is invalid/,
  )
  assert.deepEqual(
    parseReleaseArguments(['--version=2.0.0', '--platform=linux', '--verify=none']),
    { help: false, version: '2.0.0', platform: 'linux', buildID: '', verify: 'none', checks: [] },
  )
  assert.deepEqual(
    parseReleaseArguments(['--version=2.0.0', '--platform=linux', '--checks=e2e,lint,e2e']),
    { help: false, version: '2.0.0', platform: 'linux', buildID: '', verify: 'full', checks: ['lint', 'e2e'] },
  )
  assert.throws(
    () => parseReleaseArguments(['--version=2.0.0', '--platform=linux', '--verify=changed', '--checks=unit']),
    /--verify and --checks cannot be combined/,
  )
  assert.throws(
    () => parseReleaseArguments(['--version=2.0.0', '--platform=linux', '--checks=fast']),
    /Unknown --checks value: fast/,
  )
  assert.throws(
    () => parseReleaseArguments(['--version=2.0.0', '--platform=linux', '--checks=']),
    /--checks requires at least one value/,
  )
})

test('local release maps verification levels and named check groups to fixed commands', () => {
  assert.deepEqual(releaseVerificationCommands({ verify: 'full', checks: [] }), [['run', 'verify']])
  assert.deepEqual(releaseVerificationCommands({ verify: 'changed', checks: [] }), [['run', 'verify:changed']])
  assert.deepEqual(releaseVerificationCommands({ verify: 'none', checks: [] }), [])
  assert.deepEqual(releaseVerificationCommands({ verify: 'full', checks: ['lint', 'unit', 'e2e'] }), [
    ['run', 'check:rust'],
    ['run', 'typecheck'],
    ['run', 'verify:table-layout'],
    ['run', 'verify:typography'],
    ['run', 'test'],
    ['run', 'test:system-e2e'],
  ])
})

test('local release validates scoped public License trust without a License private key', () => {
  const { repository, security } = fixture()
  const configuration = loadLocalReleaseSecurity(repository)
  assert.equal(configuration.directory, realpathSync(security))
  assert.equal(configuration.releaseSigningKeyID, 'release-v1')
  assert.equal(configuration.freeLicenseFile, join(realpathSync(security), 'free-license.json'))
  assert.equal(JSON.parse(configuration.licenseTrustedKeysJSON)[0].key_id, 'license-v2')
  assert.equal(JSON.parse(configuration.releaseTrustedKeysJSON)[0].key_id, 'release-v1')
  assert.equal(JSON.parse(configuration.pluginTrustedKeysJSON)[0].key_id, 'plugin-openai-v1')
})

test('local release requires the signed free license as an external release input', () => {
  const { repository, security } = fixture()
  const freeLicense = join(security, 'free-license.json')
  writeFileSync(freeLicense, '')
  assert.throws(() => loadLocalReleaseSecurity(repository), /between 1 byte and 64 KiB/)
})

test('local release rejects a bound, expiring, or unsigned bundled free license', () => {
  for (const document of [
    { claims: { source: { kind: 'free_distribution', distribution_id: 'dist_free_1' }, binding: { mode: 'machine' }, validity: { expiry: { mode: 'none' } } }, signature: 'signed' },
    { claims: { source: { kind: 'free_distribution', distribution_id: 'dist_free_1' }, binding: { mode: 'unbound' }, validity: { expiry: { mode: 'fixed' } } }, signature: 'signed' },
    { claims: { source: { kind: 'free_distribution', distribution_id: 'dist_free_1' }, binding: { mode: 'unbound' }, validity: { expiry: { mode: 'none' } } }, signature: '' },
  ]) {
    const { repository, security } = fixture()
    writeFileSync(join(security, 'free-license.json'), JSON.stringify(document))
    assert.throws(() => loadLocalReleaseSecurity(repository), /signed, unbound, non-expiring/)
  }
})

test('local release security preserves scoped trust and rejects missing policy', () => {
  const { repository, security } = fixture()
  const path = join(security, 'license-v2.public-keyring.json')
  const legacy = JSON.parse(readFileSync(path, 'utf8'))[0]
  const scoped = JSON.parse(readFileSync(new URL('../contracts/test-vectors/license-trust.v1.json', import.meta.url), 'utf8')).keyring[1]
  writeFileSync(path, JSON.stringify([legacy, scoped]))
  assert.deepEqual(JSON.parse(loadLocalReleaseSecurity(repository).licenseTrustedKeysJSON), [legacy, scoped])
  const { policy: _policy, ...unscoped } = legacy
  writeFileSync(path, JSON.stringify([unscoped]))
  assert.throws(() => loadLocalReleaseSecurity(repository), /invalid fields/)
  writeFileSync(path, JSON.stringify([legacy, { ...scoped, policy: null }]))
  assert.throws(() => loadLocalReleaseSecurity(repository), /invalid fields/)
})

test('local release security rejects a release seed that does not match the keyring', () => {
  const { repository, security } = fixture()
  writeFileSync(join(security, 'release-v1.seed'), randomBytes(32))
  assert.throws(
    () => loadLocalReleaseSecurity(repository),
    /Release signing seed does not match the Release keyring/,
  )
})

test('local release rejects repository directories whose names start with two dots', () => {
  const { repository } = fixture()
  const directory = join(repository, '..security')
  mkdirSync(directory)
  writeFileSync(join(repository, '.env'), `ASTER_LOCAL_SECURITY_CONFIG_DIR='${directory}'\n`)
  assert.throws(() => loadLocalReleaseSecurity(repository), /must be outside the source repository/)
})

test('local release refuses any existing selected-platform output', () => {
  const { repository } = fixture()
  mkdirSync(join(repository, 'dist', 'linux'), { recursive: true })
  writeFileSync(join(repository, 'dist', 'linux', 'aster-team-2.0.0-linux-amd64.tar.gz'), 'old')
  assert.throws(
    () => assertReleaseOutputsAbsent(repository, '2.0.0', 'all'),
    /Refusing to overwrite existing release output/,
  )
  assert.doesNotThrow(() => assertReleaseOutputsAbsent(repository, '2.0.0', 'windows'))
})

test('Windows local releases require a complete system Perl without installing one', () => {
  const calls = []
  const execute = (command, arguments_) => {
    calls.push([command, arguments_])
    if (command === 'where.exe') {
      return {
        status: 0,
        stdout: 'C:\\Program Files\\Git\\usr\\bin\\perl.exe\r\nC:\\Strawberry\\perl\\bin\\perl.exe\r\n',
      }
    }
    return { status: command.startsWith('C:\\Strawberry') ? 0 : 1 }
  }
  assert.equal(
    requireWindowsSystemPerl({ Path: 'C:\\Windows\\System32' }, execute),
    'C:\\Strawberry\\perl\\bin\\perl.exe',
  )
  assert.deepEqual(calls[1][1], ['-MLocale::Maketext::Simple', '-e', '1'])
  assert.throws(
    () => requireWindowsSystemPerl({}, () => ({ status: 1, stdout: '' })),
    /complete system Perl was not found[\s\S]*do not download or install Perl/,
  )
})

test('local Windows release runs the same PowerShell 7 Control and Runner entry points as CI', () => {
  const { repository, security } = fixture()
  const calls = []
  smokeWindowsArchive('2.0.1-rc.1', '2.0.2-ci-upgrade', join(repository, 'staging'), {
    releaseSeedFile: join(security, 'release-v1.seed'), releaseSigningKeyID: 'release-v1',
  }, join(repository, 'runs with spaces'), (...call) => calls.push(call), 'npm-cli.js')
  assert.ok(calls.every(([program, args]) => program === process.execPath && args[0] === 'npm-cli.js'))
  assert.deepEqual(calls.map(([, args]) => args.slice(1, 4)), [
    ['run', 'test:windows-release', '--'], ['run', 'test:windows-runner-install', '--'],
  ])
  const option = (args, name) => args[args.indexOf(name) + 1]
  assert.equal(option(calls[0][1], '-Archive'), option(calls[1][1], '-Archive'))
  assert.notEqual(option(calls[0][1], '-WorkRoot'), option(calls[1][1], '-WorkRoot'))
  const scripts = JSON.parse(readFileSync(new URL('../package.json', import.meta.url))).scripts
  const workflow = parse(readFileSync(new URL('../.github/workflows/customer-release.yml', import.meta.url), 'utf8'))
  for (const [name, script] of [
    ['test:windows-release', 'exercise-windows-release.ps1'],
    ['test:windows-runner-install', 'exercise-windows-runner-install.ps1'],
  ]) {
    assert.equal(scripts[name], `pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File ./scripts/ci/${script}`)
    const steps = Object.values(workflow.jobs).flatMap(job => job.steps || [])
    assert.equal(steps.filter(step => step.run?.includes(`npm run ${name} --`)).length, 1)
  }
  const upgrade = readFileSync(new URL('./ci/exercise-windows-upgrade.ps1', import.meta.url), 'utf8')
  assert.doesNotMatch(upgrade, /\$env:RUNNER_TEMP/)
})

test('local Windows release tests remain disabled unless the switch is explicitly true', () => {
  for (const value of [undefined, '', 'false', 'TRUE']) {
    assert.equal(windowsReleaseTestsEnabled({ ASTER_ENABLE_WINDOWS_TESTS: value }), false)
  }
  assert.equal(windowsReleaseTestsEnabled({ ASTER_ENABLE_WINDOWS_TESTS: 'true' }), true)
})

test('a failed Windows Control lifecycle prevents Runner checks and publication', () => {
  const calls = []
  assert.throws(() => smokeWindowsArchive('2.0.1', '2.0.2-ci-upgrade', 'staging', {
    releaseSeedFile: 'test.seed', releaseSigningKeyID: 'test-key',
  }, 'runs', (...call) => { calls.push(call); throw new Error('Control smoke failed') }, 'npm-cli.js'), /Control smoke failed/)
  assert.equal(calls.length, 1)
})

function writeStagedArtifact(repository, staging, version, platform) {
  const paths = releaseOutputPaths(repository, version, platform, staging)
  mkdirSync(paths.bundle, { recursive: true })
  writeFileSync(paths.archive, 'archive')
  writeFileSync(paths.checksum, 'checksum')
  return paths
}

test('local publication moves all staged artifacts only after every platform is present', () => {
  const { repository } = fixture()
  const staging = join(repository, 'target', 'release-local', 'staging', 'build')
  const windows = writeStagedArtifact(repository, staging, '2.0.0', 'windows')
  const linux = writeStagedArtifact(repository, staging, '2.0.0', 'linux')
  publishStagedArtifacts(repository, '2.0.0', 'all', staging)
  assert.equal(existsSync(windows.bundle), false)
  assert.equal(existsSync(linux.archive), false)
  assert.equal(existsSync(releaseOutputPaths(repository, '2.0.0', 'windows').bundle), true)
  assert.equal(existsSync(releaseOutputPaths(repository, '2.0.0', 'linux').archive), true)
})

test('local publication rolls back the first platform when a later staged output is incomplete', () => {
  const { repository } = fixture()
  const staging = join(repository, 'target', 'release-local', 'staging', 'build')
  const windows = writeStagedArtifact(repository, staging, '2.0.0', 'windows')
  assert.throws(
    () => publishStagedArtifacts(repository, '2.0.0', 'all', staging),
    /Staged release output is missing/,
  )
  assert.equal(existsSync(windows.bundle), true)
  assert.equal(existsSync(windows.archive), true)
  assert.equal(existsSync(releaseOutputPaths(repository, '2.0.0', 'windows').bundle), false)
})
