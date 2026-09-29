import { createHash } from 'node:crypto'
import {
  chmodSync, copyFileSync, cpSync, createReadStream, existsSync, mkdirSync,
  readFileSync, rmSync, statSync, writeFileSync,
} from 'node:fs'
import { dirname, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { createTarGz } from './release-archive.mjs'
import { readCustomerReleaseProfile } from './customer-release-profile.mjs'
import { officialPluginReleaseProfile, stageOfficialPlugins } from './official-plugin-release.mjs'
import { verifyArtifact, verifySources } from './verify-release-boundaries.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const argument = name => process.argv.find(value => value.startsWith(`${name}=`))?.slice(name.length + 1) || ''
const nativeArch = process.arch === 'x64' ? 'amd64' : process.arch === 'arm64' ? 'arm64' : ''
const arch = argument('--arch') || nativeArch
const version = (argument('--version') || process.env.ASTER_RELEASE_VERSION || '').replace(/^v/, '')
const signingKeyFile = resolve(root, process.env.ASTER_RELEASE_SIGNING_KEY_FILE || '')
const signingKeyID = process.env.ASTER_RELEASE_SIGNING_KEY_ID || ''
const createdAt = process.env.ASTER_RELEASE_CREATED_AT || ''
const sourceDateEpoch = Number(process.env.SOURCE_DATE_EPOCH || '')
const profile = readCustomerReleaseProfile()
const pluginProfile = officialPluginReleaseProfile()
const cargoTargetDirectory = resolve(root, process.env.CARGO_TARGET_DIR || 'target')

if (process.platform !== 'darwin') throw new Error('customer macOS binaries must be built on macOS')
if (!nativeArch || arch !== nativeArch) throw new Error('macOS packages must be built natively for amd64 or arm64')
if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) throw new Error('--version is invalid')
if (!/^[A-Za-z0-9_.:-]{3,128}$/.test(signingKeyID)) throw new Error('ASTER_RELEASE_SIGNING_KEY_ID is invalid')
if (!profile.releaseTrustedKeys.some(entry => entry.key_id === signingKeyID)) throw new Error('release signing key is not trusted by this build')
if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/.test(createdAt) || Number.isNaN(Date.parse(createdAt))) throw new Error('ASTER_RELEASE_CREATED_AT is invalid')
if (!Number.isSafeInteger(sourceDateEpoch) || sourceDateEpoch < 0) throw new Error('SOURCE_DATE_EPOCH is required')
if (!statSync(signingKeyFile).isFile() || statSync(signingKeyFile).size !== 32) throw new Error('release signing seed must contain exactly 32 bytes')

const cargoVersion = /\[workspace\.package\][\s\S]*?\nversion\s*=\s*"([^"]+)"/.exec(readFileSync(resolve(root, 'Cargo.toml'), 'utf8'))?.[1]
const packageVersion = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf8')).version
if (version !== cargoVersion || version !== packageVersion) throw new Error(`release version ${version} does not match workspace versions`)

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: options.cwd || root, env: { ...process.env, ...options.env }, stdio: 'inherit' })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`${command} failed with ${result.signal || result.status}`)
}

async function digest(path, algorithm) {
  const value = createHash(algorithm)
  for await (const chunk of createReadStream(path)) value.update(chunk)
  return value.digest('hex')
}

const sourceFailures = verifySources('customer')
if (sourceFailures.length) throw new Error(sourceFailures.join('\n'))
run(process.execPath, [resolve(root, 'scripts/verify-customer-logging.mjs')])
run('npm', ['run', 'build', '--workspace', '@aster/admin'])
run('npm', ['run', 'build', '--workspace', '@aster/member'])
run(process.execPath, [resolve(root, 'scripts/verify-customer-production-assets.mjs')])
const embeddedKeys = {
  ASTER_LICENSE_TRUSTED_KEYS_JSON: profile.licenseTrustedKeysJSON,
  ASTER_RELEASE_TRUSTED_KEYS_JSON: profile.releaseTrustedKeysJSON,
  ASTER_PLUGIN_TRUSTED_KEYS_JSON: pluginProfile.keyringJSON,
}
run('cargo', ['build', '--release', '--locked', '-p', 'aster-control', '--no-default-features', '--features', 'sqlcipher'], { env: embeddedKeys })
run('cargo', ['build', '--release', '--locked', '-p', 'aster-runner'])
run('cargo', ['build', '--release', '--locked', '-p', 'aster-team-cli'], { env: embeddedKeys })
run('cargo', ['build', '--release', '--locked', '-p', 'aster-release-tool'])
run('cargo', ['build', '--release', '--locked', '-p', 'aster-plugin-core', '--features', 'signing', '--bin', 'aster-sign-plugin'])

const bundleName = `aster-team-${version}-macos-${arch}`
const outputRoot = resolve(root, 'dist', 'macos')
const bundle = resolve(outputRoot, bundleName)
const archive = resolve(outputRoot, `${bundleName}.tar.gz`)
mkdirSync(outputRoot, { recursive: true })
for (const path of [bundle, archive, `${archive}.sha256`]) {
  if (existsSync(path) && process.env.ASTER_OVERWRITE !== 'true') throw new Error(`release output already exists: ${path}`)
}
if (!bundle.startsWith(`${outputRoot}/`)) throw new Error('unsafe release output path')
rmSync(bundle, { recursive: true, force: true })
if (process.env.ASTER_OVERWRITE === 'true') {
  rmSync(archive, { force: true })
  rmSync(`${archive}.sha256`, { force: true })
}
for (const directory of ['bin', 'libexec', 'macos', 'launchd', 'THIRD_PARTY_LICENSES', 'plugins']) mkdirSync(resolve(bundle, directory), { recursive: true })
const targetRoot = resolve(cargoTargetDirectory, 'release')
copyFileSync(resolve(root, 'third_party/qrc/LICENSE'), resolve(bundle, 'THIRD_PARTY_LICENSES/qrc-MIT.txt'))
for (const binary of ['aster-control', 'aster-runner', 'aster-team-cli']) copyFileSync(resolve(targetRoot, binary), resolve(bundle, `bin/${binary}`))
stageOfficialPlugins({ run: (binary, args) => run(binary, args),
  signer: resolve(targetRoot, 'aster-sign-plugin'), root, bundle, version, profile: pluginProfile })

const caddyRuntime = JSON.parse(readFileSync(resolve(root, 'tools/caddy-runtime.json'), 'utf8'))
const caddyUrl = caddyRuntime[`macos_${arch}_url`]
const caddySha512 = caddyRuntime[`macos_${arch}_sha512`]
if (!/^https:\/\/github\.com\/caddyserver\/caddy\/releases\/download\//.test(caddyUrl || '') || !/^[0-9a-f]{128}$/.test(caddySha512 || '')) throw new Error('macOS Caddy runtime lock is invalid')
const caddyStage = resolve(outputRoot, `.caddy-${caddyRuntime.version}-${process.pid}`)
const caddyArchive = resolve(caddyStage, 'caddy.tar.gz')
rmSync(caddyStage, { recursive: true, force: true })
mkdirSync(caddyStage, { recursive: true })
try {
  run('curl', ['--fail', '--location', '--retry', '4', '--output', caddyArchive, caddyUrl])
  const actual = await digest(caddyArchive, 'sha512')
  if (actual !== caddySha512) throw new Error(`Caddy archive SHA-512 mismatch: ${actual}`)
  run('tar', ['-xzf', caddyArchive, '-C', caddyStage, 'caddy', 'LICENSE'])
  copyFileSync(resolve(caddyStage, 'caddy'), resolve(bundle, 'bin/caddy'))
  copyFileSync(resolve(caddyStage, 'LICENSE'), resolve(bundle, 'THIRD_PARTY_LICENSES/Caddy-Apache-2.0.txt'))
} finally {
  rmSync(caddyStage, { recursive: true, force: true })
}

cpSync(resolve(root, 'customer/admin/dist'), resolve(bundle, 'admin'), { recursive: true })
cpSync(resolve(root, 'customer/member/dist'), resolve(bundle, 'member'), { recursive: true })
cpSync(resolve(root, 'customer/deploy/launchd'), resolve(bundle, 'launchd'), { recursive: true })
copyFileSync(resolve(root, 'customer/deploy/macos/init-macos.sh'), resolve(bundle, 'init-macos.sh'))
copyFileSync(resolve(root, 'customer/deploy/macos/install-macos.sh'), resolve(bundle, 'libexec/install-macos.sh'))
copyFileSync(resolve(root, 'customer/deploy/macos/restore-backup-macos.sh'), resolve(bundle, 'libexec/restore-backup-macos.sh'))
copyFileSync(resolve(root, 'customer/deploy/macos/service-launch.sh'), resolve(bundle, 'macos/service-launch.sh'))
copyFileSync(resolve(root, 'customer/deploy/macos/README.md'), resolve(bundle, 'README.md'))
writeFileSync(resolve(bundle, 'VERSION'), `${version}\n`, { flag: 'wx' })
const rustTarget = arch === 'amd64' ? 'x86_64-apple-darwin' : 'aarch64-apple-darwin'
run(process.execPath, [
  resolve(root, 'scripts/generate-customer-sbom.mjs'),
  `--output=${resolve(bundle, 'SBOM.cdx.json')}`,
  `--version=${version}`,
  '--platform=macos',
  `--architecture=${arch}`,
  `--rust-target=${rustTarget}`,
  '--runtime=native',
])
for (const executable of ['bin/aster-control', 'bin/aster-runner', 'bin/aster-team-cli', 'bin/caddy', 'init-macos.sh', 'libexec/install-macos.sh', 'libexec/restore-backup-macos.sh', 'macos/service-launch.sh']) chmodSync(resolve(bundle, executable), 0o755)
run(resolve(targetRoot, 'aster-release-tool'), [
  'sign', '--root', bundle, '--private-key', signingKeyFile, '--key-id', signingKeyID,
  '--version', version, '--architecture', arch, '--platform', 'macos', '--runtime', 'native', '--created-at', createdAt,
])
run(resolve(bundle, 'bin/aster-team-cli'), ['verify-release', '--root', bundle], { env: embeddedKeys })
const artifactFailures = verifyArtifact('customer', bundle, 'macos')
if (artifactFailures.length) throw new Error(artifactFailures.join('\n'))
createTarGz(bundle, archive, bundleName, sourceDateEpoch)
const archiveHash = await digest(archive, 'sha256')
writeFileSync(`${archive}.sha256`, `${archiveHash}  ${bundleName}.tar.gz\n`, { flag: 'wx' })
console.log(`Customer release: ${archive}`)
console.log(`SHA-256:         ${archiveHash}`)
console.log(`Manifest SHA-256:${await digest(resolve(bundle, 'RELEASE.json'), 'sha256')}`)
