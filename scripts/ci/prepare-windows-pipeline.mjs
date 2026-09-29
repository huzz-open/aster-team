// Development-only signed fixtures. Never reads production signing configuration,
// publishes artifacts, or represents these packages as a production release.
import { createHash, generateKeyPairSync, sign } from 'node:crypto'
import { closeSync, copyFileSync, existsSync, mkdirSync, openSync, readFileSync, unlinkSync, writeFileSync } from 'node:fs'
import { parse, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { nextPatchFixtureVersion, runCommand } from './workspace-release-fixture.mjs'
import { fileDigest } from '../release-download-cache.mjs'
import { localLicensePolicies } from '../license-signing-profile.mjs'

const root = resolve(fileURLToPath(new URL('../..', import.meta.url)))
const json = path => JSON.parse(readFileSync(path, 'utf8'))
const save = (path, value) => writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`)

export function sourceFingerprint(files, read, inputs) {
  const hash = createHash('sha256').update(JSON.stringify(inputs))
  for (const file of [...new Set(files)].sort()) {
    hash.update(`\0${file}\0`).update(read(file))
  }
  return hash.digest('hex')
}

export function createWindowsLicenseFixture(version, createdAt) {
  if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version) || !Number.isFinite(Date.parse(createdAt))) throw new Error('invalid Windows fixture identity')
  const free = generateKeyPairSync('ed25519')
  const paid = generateKeyPairSync('ed25519')
  const profiles = [free, paid].map((key, index) => ({
    key_id: index === 0 ? 'windows-lab-free-v2' : 'lab-paid-v2',
    public_key_spki: key.publicKey.export({ format: 'der', type: 'spki' }).toString('base64url'),
    policy: localLicensePolicies()[index].policy,
  }))
  const fixture = json(resolve(root, 'contracts/test-vectors/license.v2.json')).cases.find(item => item.name === 'free_no_expiry')
  // Both platform fixtures represent this same immutable plan version.
  // Package version changes must not silently change its minimum software version.
  const claims = structuredClone(fixture.document.claims)
  Object.assign(claims, { key_id: profiles[0].key_id, license_id: 'windows_lab_free_v2', issued_at: createdAt })
  claims.entitlements = structuredClone(profiles[0].policy.entitlement_ceiling)
  claims.validity.not_before = createdAt
  const canonical = value => Array.isArray(value) ? value.map(canonical)
    : value !== null && typeof value === 'object' ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value
  const signature = sign(null, Buffer.from(JSON.stringify(canonical(claims))), free.privateKey).toString('base64url')
  return { licenseKeys: JSON.stringify(profiles), freeLicenseBytes: Buffer.from(`${JSON.stringify({ claims, signature })}\n`), paidSigner: {
    key_id: profiles[1].key_id, private_key_pkcs8: paid.privateKey.export({ format: 'der', type: 'pkcs8' }).toString('base64url'), policy: profiles[1].policy,
  } }
}

async function main() {
  if (process.platform !== 'win32' || process.arch !== 'x64') throw new Error('Windows x64 is required')
  if (process.argv.length > 2) throw new Error('This fixture preparation command accepts no arguments; unchanged builds resume automatically')
  const npmCLI = process.env.npm_execpath
  if (!npmCLI || !existsSync(npmCLI)) throw new Error('Run through npm run test:windows-pipeline:prepare')
  const version = json(resolve(root, 'package.json')).version
  const candidateVersion = nextPatchFixtureVersion(version)
  const capture = (command, args) => runCommand(root, command, args, { capture: true }).trim()
  if (capture('git', ['status', '--porcelain', '--untracked-files=all'])) throw new Error('Windows fixture preparation requires a clean committed source tree')
  const inputs = {
    version, node: process.version, rust: capture('rustc', ['--version']),
    head: capture('git', ['rev-parse', 'HEAD']),
    buildEnvironment: Object.fromEntries(['CARGO_TARGET_DIR', 'RUSTFLAGS', 'CC', 'CXX', 'PERL'].map(name => [name, process.env[name] || ''])),
  }
  const currentFingerprint = () => sourceFingerprint(
    capture('git', ['ls-files', '-z', '--cached', '--others', '--exclude-standard']).split('\0').filter(Boolean),
    file => existsSync(resolve(root, file)) ? readFileSync(resolve(root, file)) : Buffer.from('<deleted>'),
    { ...inputs, head: capture('git', ['rev-parse', 'HEAD']) },
  )
  const fingerprint = currentFingerprint()
  const assertUnchangedSource = () => {
    if (currentFingerprint() !== fingerprint) throw new Error('Source inputs changed during fixture preparation; rerun preparation before using these packages')
  }
  const runRoot = resolve(root, 'target/wp', fingerprint.slice(0, 12))
  mkdirSync(runRoot, { recursive: true })
  const lockPath = resolve(runRoot, 'preparation.lock')
  const lock = openSync(lockPath, 'wx')
  try {
    const receiptPath = resolve(runRoot, 'build-receipt.json')
    let receipt
    if (existsSync(receiptPath)) {
      receipt = json(receiptPath)
      if (receipt.source_clean !== true || receipt.fingerprint !== fingerprint || receipt.schema !== 'aster.windows-pipeline-fixture.v1') throw new Error('Fixture receipt does not match source inputs')
    } else {
      const release = generateKeyPairSync('ed25519')
      const createdAt = new Date().toISOString()
      const licenses = createWindowsLicenseFixture(version, createdAt)
      writeFileSync(resolve(runRoot, 'free-license.json'), licenses.freeLicenseBytes, { flag: 'wx' })
      writeFileSync(resolve(runRoot, 'paid-test-private.pkcs8.base64url'), licenses.paidSigner.private_key_pkcs8, { flag: 'wx', mode: 0o600 })
      writeFileSync(resolve(runRoot, 'release.seed'), Buffer.from(release.privateKey.export({ format: 'jwk' }).d, 'base64url'), { flag: 'wx', mode: 0o600 })
      const keyring = (key, id) => JSON.stringify([{ key_id: id, public_key_spki: key.export({ format: 'der', type: 'spki' }).toString('base64url') }])
      receipt = {
        schema: 'aster.windows-pipeline-fixture.v1', fingerprint, inputs, source_clean: true,
        createdAt, steps: {},
        releaseKeys: keyring(release.publicKey, 'windows-lab-release'),
        licenseKeys: licenses.licenseKeys,
        freeLicenseDigest: await fileDigest(resolve(runRoot, 'free-license.json'), 'sha256'),
        paidSignersDigest: await fileDigest(resolve(runRoot, 'paid-test-private.pkcs8.base64url'), 'sha256'),
        seedDigest: await fileDigest(resolve(runRoot, 'release.seed'), 'sha256'),
      }
      save(receiptPath, receipt)
    }
    if (await fileDigest(resolve(runRoot, 'release.seed'), 'sha256') !== receipt.seedDigest) throw new Error('Fixture signing seed changed')
    if (await fileDigest(resolve(runRoot, 'free-license.json'), 'sha256') !== receipt.freeLicenseDigest || await fileDigest(resolve(runRoot, 'paid-test-private.pkcs8.base64url'), 'sha256') !== receipt.paidSignersDigest) throw new Error('Fixture License inputs changed')
    const outputRoot = resolve(runRoot, 'packages')
    const asterctl = resolve(runRoot, 'asterctl.exe')
    const environment = {
      ...process.env,
      ASTER_RELEASE_OUTPUT_ROOT: outputRoot,
      ASTER_LICENSE_TRUSTED_KEYS_JSON: receipt.licenseKeys,
      ASTER_CUSTOMER_FREE_LICENSE_FILE: resolve(runRoot, 'free-license.json'),
      ASTER_RELEASE_TRUSTED_KEYS_JSON: receipt.releaseKeys,
      ASTER_RELEASE_SIGNING_KEY_FILE: resolve(runRoot, 'release.seed'),
      ASTER_RELEASE_SIGNING_KEY_ID: 'windows-lab-release',
      ASTER_RELEASE_CREATED_AT: receipt.createdAt,
      SOURCE_DATE_EPOCH: String(Math.floor(Date.parse(receipt.createdAt) / 1000)),
      ASTER_CLIENT_ASTERCTL_WINDOWS_X64: asterctl,
      ASTER_OVERWRITE: 'true',
    }
    const run = async (id, args, outputs, finalize = () => {}) => {
      assertUnchangedSource()
      if (receipt.steps[id]) {
        for (const [path, digest] of Object.entries(receipt.steps[id])) {
          if (!existsSync(path) || await fileDigest(path, 'sha256') !== digest) throw new Error(`Completed fixture output changed: ${path}`)
        }
        console.log(`Reusing completed fixture step: ${id}`)
        return
      }
      runCommand(root, process.execPath, [npmCLI, 'run', ...args], { env: environment })
      assertUnchangedSource()
      finalize()
      receipt.steps[id] = Object.fromEntries(await Promise.all(outputs.map(async path => [path, await fileDigest(path, 'sha256')])))
      save(receiptPath, receipt)
    }
    const archive = resolve(outputRoot, 'windows', `aster-team-${version}-windows-amd64.tar.gz`)
    const candidateArchive = resolve(outputRoot, 'windows', `aster-team-${candidateVersion}-windows-amd64.tar.gz`)
    const releaseTool = resolve(runRoot, 'aster-release-tool.exe')
    await run('asterctl', ['build:asterctl:windows', '--', `--output=${asterctl}`], [asterctl])
    await run('base', ['build:windows', '--', `--version=${version}`], [archive, `${archive}.sha256`])
    await run('candidate', ['build:windows-upgrade-fixture', '--', `--base-version=${version}`], [candidateArchive, `${candidateArchive}.sha256`, releaseTool], () => {
      copyFileSync(resolve(root, process.env.CARGO_TARGET_DIR || 'target', 'release/aster-release-tool.exe'), releaseTool)
    })
    assertUnchangedSource()
    const paidLicenseSigner = resolve(runRoot, 'lablicensesigner.exe')
    runCommand(root, process.execPath, [resolve(root, 'scripts/run-go.mjs'), 'build', '-o', paidLicenseSigner, './operations/backend/cmd/lablicensesigner'])
    assertUnchangedSource()
    const configuration = {
      Archive: archive, Version: version, CandidateArchive: candidateArchive, CandidateVersion: candidateVersion,
      ReleaseTool: releaseTool,
      PaidLicenseSigner: paidLicenseSigner, PaidLicensePrivateKey: resolve(runRoot, 'paid-test-private.pkcs8.base64url'),
      ReleaseSigningKey: environment.ASTER_RELEASE_SIGNING_KEY_FILE, ReleaseSigningKeyId: environment.ASTER_RELEASE_SIGNING_KEY_ID,
      WorkRoot: resolve(parse(root).root, 'aster-windows-lab', fingerprint.slice(0, 12)), DiagnosticsRoot: resolve(runRoot, 'diagnostics'),
    }
    const configPath = resolve(runRoot, 'lifecycle.json')
    save(configPath, configuration)
    console.log(`TEST-ONLY packages prepared. Administrator command:\nnpm run test:windows-pipeline -- -Configuration "${configPath}"`)
  } finally {
    closeSync(lock)
    unlinkSync(lockPath)
  }
}

if (resolve(process.argv[1] || '') === fileURLToPath(import.meta.url)) await main()
