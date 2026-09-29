import {
  chmodSync, closeSync, copyFileSync, cpSync, existsSync, lstatSync, mkdirSync,
  openSync, readFileSync, readSync, readdirSync, rmSync, statSync, writeFileSync,
} from 'node:fs'
import { dirname, isAbsolute, relative, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { createTarGz } from './release-archive.mjs'
import {
  freezeBundledFreeLicense, stageBundledFreeLicense, verifyStagedBundledFreeLicense,
} from './bundled-free-license.mjs'
import { fileDigest, prepareVerifiedDownload } from './release-download-cache.mjs'
import { readCustomerReleaseProfile } from './customer-release-profile.mjs'
import { officialPluginReleaseProfile, stageOfficialPlugins } from './official-plugin-release.mjs'
import { verifyArtifact, verifySources } from './verify-release-boundaries.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm'
const cargo = process.platform === 'win32' ? 'cargo.exe' : 'cargo'
const argument = name => process.argv.find(value => value.startsWith(`${name}=`))?.slice(name.length + 1) || ''
const arch = argument('--arch') || 'amd64'
const version = (argument('--version') || process.env.ASTER_RELEASE_VERSION || '').replace(/^v/, '')
const signingKeyFile = resolve(root, process.env.ASTER_RELEASE_SIGNING_KEY_FILE || '')
const signingKeyID = process.env.ASTER_RELEASE_SIGNING_KEY_ID || ''
const createdAt = process.env.ASTER_RELEASE_CREATED_AT || ''
const sourceDateEpoch = Number(process.env.SOURCE_DATE_EPOCH || '')
const profile = readCustomerReleaseProfile()
const pluginProfile = officialPluginReleaseProfile()
const rustTarget = 'x86_64-unknown-linux-musl'
const cargoTargetDirectory = resolve(root, process.env.CARGO_TARGET_DIR || 'target')
const releaseOutputRoot = resolve(root, process.env.ASTER_RELEASE_OUTPUT_ROOT || 'dist')
const downloadCache = resolve(root, process.env.ASTER_RELEASE_DOWNLOAD_CACHE || 'target/release-downloads')
const asterctlWindowsX64 = resolve(root, process.env.ASTER_CLIENT_ASTERCTL_WINDOWS_X64 || '')
const bundledFreeLicenseFile = process.env.ASTER_CUSTOMER_FREE_LICENSE_FILE
  ? resolve(root, process.env.ASTER_CUSTOMER_FREE_LICENSE_FILE)
  : null
const bundledFreeLicenseBytes = bundledFreeLicenseFile
  ? freezeBundledFreeLicense(bundledFreeLicenseFile)
  : null

const relativeOutput = relative(root, releaseOutputRoot)
if (!relativeOutput || relativeOutput.startsWith('..') || isAbsolute(relativeOutput)) {
  throw new Error('ASTER_RELEASE_OUTPUT_ROOT must be below the source repository')
}

if (process.platform !== 'linux') throw new Error('customer Linux binaries must be built inside the pinned Linux build environment')
if (arch !== 'amd64' || process.arch !== 'x64') throw new Error('this production builder currently supports native Linux amd64 only')
if (!/^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) throw new Error('--version is invalid')
if (!/^[A-Za-z0-9_.:-]{3,128}$/.test(signingKeyID)) throw new Error('ASTER_RELEASE_SIGNING_KEY_ID is invalid')
if (!profile.releaseTrustedKeys.some(entry => entry.key_id === signingKeyID)) {
  throw new Error('ASTER_RELEASE_SIGNING_KEY_ID is not present in ASTER_RELEASE_TRUSTED_KEYS_JSON')
}
if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/.test(createdAt) || Number.isNaN(Date.parse(createdAt))) {
  throw new Error('ASTER_RELEASE_CREATED_AT must use YYYY-MM-DDTHH:MM:SS.mmmZ')
}
if (!Number.isSafeInteger(sourceDateEpoch) || sourceDateEpoch < 0) throw new Error('SOURCE_DATE_EPOCH is required')
if (!statSync(signingKeyFile).isFile() || statSync(signingKeyFile).size !== 32) {
  throw new Error('ASTER_RELEASE_SIGNING_KEY_FILE must point to a 32-byte raw Ed25519 seed')
}
if (!process.env.ASTER_CLIENT_ASTERCTL_WINDOWS_X64 || !existsSync(asterctlWindowsX64)) {
  throw new Error('ASTER_CLIENT_ASTERCTL_WINDOWS_X64 must point to the Windows x64 asterctl executable')
}
const asterctlInformation = lstatSync(asterctlWindowsX64)
if (!asterctlInformation.isFile() || asterctlInformation.isSymbolicLink()) {
  throw new Error('ASTER_CLIENT_ASTERCTL_WINDOWS_X64 must be an ordinary file')
}
const asterctlDescriptor = openSync(asterctlWindowsX64, 'r')
const asterctlMagic = Buffer.alloc(2)
try {
  readSync(asterctlDescriptor, asterctlMagic, 0, asterctlMagic.length, 0)
} finally {
  closeSync(asterctlDescriptor)
}
const asterctlHeader = asterctlMagic.toString('ascii')
const asterctlSize = asterctlInformation.size
if (asterctlHeader !== 'MZ' || asterctlSize === 0 || asterctlSize > 64 * 1024 * 1024) {
  throw new Error('ASTER_CLIENT_ASTERCTL_WINDOWS_X64 is not a valid Windows executable')
}
const cargoManifest = readFileSync(resolve(root, 'Cargo.toml'), 'utf8')
const cargoVersion = /\[workspace\.package\][\s\S]*?\nversion\s*=\s*"([^"]+)"/.exec(cargoManifest)?.[1]
const packageVersion = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf8')).version
if (version !== cargoVersion || version !== packageVersion) {
  throw new Error(`release version ${version} must match Cargo.toml (${cargoVersion}) and package.json (${packageVersion})`)
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd || root,
    env: { ...process.env, ...options.env },
    stdio: 'inherit',
    shell: process.platform === 'win32' && /\.(cmd|bat)$/i.test(command),
  })
  if (result.error) throw result.error
  if (result.status !== 0) {
    const termination = result.signal ? `signal ${result.signal}` : `status ${result.status}`
    throw new Error(`${command} failed with ${termination}`)
  }
}

async function sha256(path) {
  return fileDigest(path, 'sha256')
}

function normalizeReleaseModes(directory) {
  chmodSync(directory, 0o755)
  for (const entry of readdirSync(directory)) {
    const path = resolve(directory, entry)
    const stats = statSync(path)
    if (stats.isDirectory()) normalizeReleaseModes(path)
    else if (stats.isFile()) chmodSync(path, 0o644)
    else throw new Error(`unsupported release entry: ${path}`)
  }
}

const sourceFailures = verifySources('customer')
if (sourceFailures.length) throw new Error(sourceFailures.join('\n'))
run(process.execPath, [resolve(root, 'scripts/verify-customer-logging.mjs')])

run(npm, ['run', 'build', '--workspace', '@aster/admin'])
run(npm, ['run', 'build', '--workspace', '@aster/member'])
run(process.execPath, [resolve(root, 'scripts/verify-customer-production-assets.mjs')])
const embeddedKeys = {
  ASTER_LICENSE_TRUSTED_KEYS_JSON: profile.licenseTrustedKeysJSON,
  ASTER_RELEASE_TRUSTED_KEYS_JSON: profile.releaseTrustedKeysJSON,
  ASTER_PLUGIN_TRUSTED_KEYS_JSON: pluginProfile.keyringJSON,
  CC_x86_64_unknown_linux_musl: 'musl-gcc',
  OPENSSL_STATIC: '1',
}
run(cargo, ['build', '--release', '--locked', '--target', rustTarget, '-p', 'aster-control', '--no-default-features', '--features', 'sqlcipher,mariadb'], { env: embeddedKeys })
run(cargo, ['build', '--release', '--locked', '--target', rustTarget, '-p', 'aster-runner'])
run(cargo, ['build', '--release', '--locked', '--target', rustTarget, '-p', 'aster-team-cli'], { env: embeddedKeys })
run(cargo, ['build', '--release', '--locked', '--target', rustTarget, '-p', 'aster-release-tool'])
run(cargo, ['build', '--release', '--locked', '--target', rustTarget, '-p', 'aster-plugin-core', '--features', 'signing', '--bin', 'aster-sign-plugin'])

const bundleName = `aster-team-${version}-linux-amd64`
const outputRoot = resolve(releaseOutputRoot, 'linux')
const bundle = resolve(outputRoot, bundleName)
const archive = resolve(outputRoot, `${bundleName}.tar.gz`)
for (const path of [bundle, archive, `${archive}.sha256`]) {
  if (existsSync(path) && process.env.ASTER_OVERWRITE !== 'true') {
    throw new Error(`release output already exists: ${path}`)
  }
}
if (!bundle.startsWith(`${outputRoot}/`) && !bundle.startsWith(`${outputRoot}\\`)) throw new Error('unsafe release output path')
rmSync(bundle, { recursive: true, force: true })
if (process.env.ASTER_OVERWRITE === 'true') {
  rmSync(archive, { force: true })
  rmSync(`${archive}.sha256`, { force: true })
}
mkdirSync(resolve(bundle, 'bin'), { recursive: true })
mkdirSync(resolve(bundle, 'client-tools/asterctl/windows-x86_64'), { recursive: true })
mkdirSync(resolve(bundle, 'libexec'), { recursive: true })
mkdirSync(resolve(bundle, 'THIRD_PARTY_LICENSES'), { recursive: true })
mkdirSync(resolve(bundle, 'plugins'), { recursive: true })
const customerTargetRoot = resolve(cargoTargetDirectory, rustTarget, 'release')
copyFileSync(resolve(root, 'third_party/qrc/LICENSE'), resolve(bundle, 'THIRD_PARTY_LICENSES/qrc-MIT.txt'))
copyFileSync(resolve(customerTargetRoot, 'aster-control'), resolve(bundle, 'bin/aster-control'))
copyFileSync(resolve(customerTargetRoot, 'aster-runner'), resolve(bundle, 'bin/aster-runner'))
copyFileSync(resolve(customerTargetRoot, 'aster-team-cli'), resolve(bundle, 'bin/aster-team-cli'))
copyFileSync(asterctlWindowsX64, resolve(bundle, 'client-tools/asterctl/windows-x86_64/asterctl.exe'))
stageOfficialPlugins({ run: (binary, args) => run(binary, args),
  signer: resolve(customerTargetRoot, 'aster-sign-plugin'), root, bundle, version, profile: pluginProfile })
if (bundledFreeLicenseBytes) {
  stageBundledFreeLicense(bundle, bundledFreeLicenseBytes)
}
const caddyRuntime = JSON.parse(readFileSync(resolve(root, 'tools/caddy-runtime.json'), 'utf8'))
if (!/^\d+\.\d+\.\d+$/.test(caddyRuntime.version)
    || !/^https:\/\/github\.com\/caddyserver\/caddy\/releases\/download\//.test(caddyRuntime.linux_amd64_url)
    || !/^[0-9a-f]{128}$/.test(caddyRuntime.linux_amd64_sha512)) {
  throw new Error('tools/caddy-runtime.json is invalid')
}
const caddyStage = resolve(outputRoot, `.caddy-${caddyRuntime.version}-${process.pid}`)
rmSync(caddyStage, { recursive: true, force: true })
mkdirSync(caddyStage, { recursive: true })
try {
  const caddyArchive = await prepareVerifiedDownload({
    cacheDirectory: downloadCache,
    fileName: `caddy-${caddyRuntime.version}-linux-amd64.tar.gz`,
    algorithm: 'sha512',
    expectedDigest: caddyRuntime.linux_amd64_sha512,
    download: path => run('curl', ['--fail', '--location', '--retry', '4', '--output', path, caddyRuntime.linux_amd64_url]),
  })
  run('tar', ['-xzf', caddyArchive, '-C', caddyStage, 'caddy', 'LICENSE'])
  copyFileSync(resolve(caddyStage, 'caddy'), resolve(bundle, 'bin/caddy'))
  copyFileSync(resolve(caddyStage, 'LICENSE'), resolve(bundle, 'THIRD_PARTY_LICENSES/Caddy-Apache-2.0.txt'))
} finally {
  rmSync(caddyStage, { recursive: true, force: true })
}
cpSync(resolve(root, 'customer/admin/dist'), resolve(bundle, 'admin'), { recursive: true })
cpSync(resolve(root, 'customer/member/dist'), resolve(bundle, 'member'), { recursive: true })
cpSync(resolve(root, 'customer/deploy/systemd'), resolve(bundle, 'systemd'), { recursive: true })
copyFileSync(resolve(root, 'customer/deploy/init.sh'), resolve(bundle, 'init.sh'))
copyFileSync(resolve(root, 'customer/deploy/install.sh'), resolve(bundle, 'libexec/install.sh'))
copyFileSync(resolve(root, 'customer/deploy/restore-backup.sh'), resolve(bundle, 'libexec/restore-backup.sh'))
copyFileSync(resolve(root, 'customer/deploy/service-health.sh'), resolve(bundle, 'libexec/service-health.sh'))
copyFileSync(resolve(root, 'customer/deploy/README.md'), resolve(bundle, 'README.md'))
writeFileSync(resolve(bundle, 'VERSION'), `${version}\n`, { flag: 'wx' })
run(process.execPath, [
  resolve(root, 'scripts/generate-customer-sbom.mjs'),
  `--output=${resolve(bundle, 'SBOM.cdx.json')}`,
  `--version=${version}`,
  '--platform=linux',
  '--architecture=amd64',
  `--rust-target=${rustTarget}`,
  '--runtime=musl-static',
])
normalizeReleaseModes(bundle)
chmodSync(resolve(bundle, 'bin/aster-control'), 0o755)
chmodSync(resolve(bundle, 'bin/aster-runner'), 0o755)
chmodSync(resolve(bundle, 'bin/aster-team-cli'), 0o755)
chmodSync(resolve(bundle, 'bin/caddy'), 0o755)
chmodSync(resolve(bundle, 'init.sh'), 0o755)
chmodSync(resolve(bundle, 'libexec/install.sh'), 0o755)
chmodSync(resolve(bundle, 'libexec/restore-backup.sh'), 0o755)

if (bundledFreeLicenseBytes) {
  verifyStagedBundledFreeLicense(
    bundle,
    resolve(bundle, 'licenses/free-license.json'),
    (binary, args) => run(binary, args),
  )
}

run(resolve(customerTargetRoot, 'aster-release-tool'), [
  'sign', '--root', bundle, '--private-key', signingKeyFile, '--key-id', signingKeyID,
  '--version', version, '--architecture', arch, '--runtime', 'musl-static', '--created-at', createdAt,
])
run(resolve(bundle, 'bin/aster-team-cli'), ['verify-release', '--root', bundle], { env: embeddedKeys })

const artifactFailures = verifyArtifact('customer', bundle, 'linux')
if (artifactFailures.length) throw new Error(artifactFailures.join('\n'))
createTarGz(bundle, archive, bundleName, sourceDateEpoch)
const archiveHash = await sha256(archive)
writeFileSync(`${archive}.sha256`, `${archiveHash}  ${bundleName}.tar.gz\n`, { flag: 'wx' })

console.log(`Customer release: ${archive}`)
console.log(`SHA-256:         ${archiveHash}`)
console.log(`Manifest SHA-256:${await sha256(resolve(bundle, 'RELEASE.json'))}`)
