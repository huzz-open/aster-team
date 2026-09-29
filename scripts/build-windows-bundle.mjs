import {
  copyFileSync, cpSync, existsSync, lstatSync, mkdirSync, readFileSync,
  rmSync, statSync, writeFileSync,
} from 'node:fs'
import { dirname, isAbsolute, relative, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { createTarGz } from './release-archive.mjs'
import { fileDigest, prepareVerifiedDownload } from './release-download-cache.mjs'
import { freezeBundledFreeLicense, stageBundledFreeLicense, verifyStagedBundledFreeLicense } from './bundled-free-license.mjs'
import { readCustomerReleaseProfile } from './customer-release-profile.mjs'
import { officialPluginReleaseProfile, stageOfficialPlugins } from './official-plugin-release.mjs'
import { verifyArtifact, verifySources } from './verify-release-boundaries.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const npm = 'npm.cmd'
const cargo = 'cargo.exe'
const argument = name => process.argv.find(value => value.startsWith(`${name}=`))?.slice(name.length + 1) || ''
const arch = argument('--arch') || 'amd64'
const version = (argument('--version') || process.env.ASTER_RELEASE_VERSION || '').replace(/^v/, '')
const signingKeyFile = resolve(root, process.env.ASTER_RELEASE_SIGNING_KEY_FILE || '')
const signingKeyID = process.env.ASTER_RELEASE_SIGNING_KEY_ID || ''
const createdAt = process.env.ASTER_RELEASE_CREATED_AT || ''
const sourceDateEpoch = Number(process.env.SOURCE_DATE_EPOCH || '')
const profile = readCustomerReleaseProfile()
const pluginProfile = officialPluginReleaseProfile()
const cargoTargetDirectory = resolve(root, process.env.CARGO_TARGET_DIR || 'target')
const releaseOutputRoot = resolve(root, process.env.ASTER_RELEASE_OUTPUT_ROOT || 'dist')
const downloadCache = resolve(root, process.env.ASTER_RELEASE_DOWNLOAD_CACHE || 'target/release-downloads')
const suppliedAsterctl = process.env.ASTER_CLIENT_ASTERCTL_WINDOWS_X64 || ''
const asterctlWindowsX64 = suppliedAsterctl ? resolve(root, suppliedAsterctl) : ''
const bundledFreeLicenseBytes = process.env.ASTER_CUSTOMER_FREE_LICENSE_FILE
  ? freezeBundledFreeLicense(resolve(root, process.env.ASTER_CUSTOMER_FREE_LICENSE_FILE))
  : null

const relativeOutput = relative(root, releaseOutputRoot)
if (!relativeOutput || relativeOutput.startsWith('..') || isAbsolute(relativeOutput)) {
  throw new Error('ASTER_RELEASE_OUTPUT_ROOT must be below the source repository')
}

if (process.platform !== 'win32') throw new Error('customer Windows binaries must be built on Windows')
if (arch !== 'amd64' || process.arch !== 'x64') throw new Error('this builder supports native Windows amd64 only')
if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) throw new Error('--version is invalid')
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
if (!asterctlWindowsX64 || !existsSync(asterctlWindowsX64)) {
  throw new Error('ASTER_CLIENT_ASTERCTL_WINDOWS_X64 must point to the shared Windows x64 asterctl executable')
}
const asterctlInformation = lstatSync(asterctlWindowsX64)
if (!asterctlInformation.isFile() || asterctlInformation.isSymbolicLink()) {
  throw new Error('ASTER_CLIENT_ASTERCTL_WINDOWS_X64 must point to an ordinary file')
}
if (asterctlInformation.size < 64 * 1024 || readFileSync(asterctlWindowsX64).subarray(0, 2).toString('ascii') !== 'MZ') {
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
    shell: /\.(cmd|bat)$/i.test(command),
  })
  if (result.error) throw result.error
  if (result.status !== 0) {
    const termination = result.signal ? `signal ${result.signal}` : `status ${result.status}`
    throw new Error(`${command} failed with ${termination}`)
  }
}

function visualStudioBuildEnvironment() {
  const installerRoot = process.env['ProgramFiles(x86)']
  if (!installerRoot) throw new Error('ProgramFiles(x86) is unavailable')
  const vswhere = resolve(installerRoot, 'Microsoft Visual Studio', 'Installer', 'vswhere.exe')
  if (!existsSync(vswhere) || !statSync(vswhere).isFile()) throw new Error('Visual Studio Build Tools discovery tool is missing')
  const discovery = spawnSync(vswhere, [
    '-latest', '-products', '*', '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64',
    '-property', 'installationPath',
  ], { encoding: 'utf8', windowsHide: true })
  const installation = discovery.stdout?.trim()
  if (discovery.status !== 0 || !installation) {
    throw new Error('Microsoft.VisualStudio.Component.VC.Tools.x86.x64 is required to build the Windows package')
  }
  const developerShell = resolve(installation, 'Common7', 'Tools', 'VsDevCmd.bat')
  const environment = spawnSync('cmd.exe', [
    '/d', '/s', '/c', `""${developerShell}" -arch=amd64 -host_arch=amd64 >nul && set"`,
  ], { encoding: 'utf8', windowsHide: true, windowsVerbatimArguments: true })
  if (environment.status !== 0) throw new Error('Visual Studio developer environment initialization failed')
  return Object.fromEntries(environment.stdout.split(/\r?\n/).flatMap(line => {
    const separator = line.indexOf('=')
    return separator > 0 ? [[line.slice(0, separator), line.slice(separator + 1)]] : []
  }))
}

function requireCompletePerl(environment) {
  const candidateEnvironment = { ...process.env, ...environment }
  const lookup = spawnSync('where.exe', ['perl.exe'], { env: candidateEnvironment, encoding: 'utf8', windowsHide: true })
  for (const candidate of (lookup.stdout || '').split(/\r?\n/).filter(Boolean)) {
    const probe = spawnSync(candidate, ['-MLocale::Maketext::Simple', '-e', '1'], {
      env: candidateEnvironment,
      encoding: 'utf8',
      windowsHide: true,
    })
    if (probe.status === 0) {
      const pathKey = Object.keys(environment).find(key => key.toLowerCase() === 'path') || 'Path'
      return { ...environment, [pathKey]: `${dirname(candidate)};${environment[pathKey] || process.env.Path || ''}` }
    }
  }
  throw new Error('A complete Perl runtime is required for the vendored OpenSSL build; install Strawberry Perl and ensure its bin directory is available in PATH')
}

function verifyStaticCRT(binary, environment) {
  const result = spawnSync('dumpbin.exe', ['/dependents', binary], {
    env: { ...process.env, ...environment },
    encoding: 'utf8',
    windowsHide: true,
  })
  if (result.status !== 0) throw new Error(`dumpbin failed while checking ${binary}`)
  const dependencies = `${result.stdout || ''}\n${result.stderr || ''}`
  if (/\b(?:VCRUNTIME\d*|MSVCP\d*|ucrtbase)\.dll\b/i.test(dependencies)) {
    throw new Error('asterctl.exe must use the static Microsoft C/C++ runtime')
  }
}

async function digest(path, algorithm) {
  return fileDigest(path, algorithm)
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
}
const nativeBuildEnvironment = {
  ...requireCompletePerl(visualStudioBuildEnvironment()),
  RUSTFLAGS: `${process.env.RUSTFLAGS || ''} -C target-feature=+crt-static`.trim(),
}
run(cargo, ['build', '--release', '--locked', '-p', 'aster-control', '--no-default-features', '--features', 'sqlcipher'], { env: { ...nativeBuildEnvironment, ...embeddedKeys } })
run(cargo, ['test', '--release', '--locked', '-p', 'aster-storage', '--no-default-features', '--features', 'sqlcipher', 'initializes_an_encrypted_final_schema'], { env: nativeBuildEnvironment })
run(cargo, ['build', '--release', '--locked', '-p', 'aster-runner'], { env: nativeBuildEnvironment })
run(cargo, ['build', '--release', '--locked', '-p', 'aster-team-cli'], { env: { ...nativeBuildEnvironment, ...embeddedKeys } })
run(cargo, ['build', '--release', '--locked', '-p', 'aster-release-tool'], { env: nativeBuildEnvironment })
run(cargo, ['build', '--release', '--locked', '-p', 'aster-plugin-core', '--features', 'signing', '--bin', 'aster-sign-plugin'], { env: nativeBuildEnvironment })
for (const binary of ['aster-control.exe', 'aster-runner.exe', 'aster-team-cli.exe']) {
  verifyStaticCRT(resolve(cargoTargetDirectory, 'release', binary), nativeBuildEnvironment)
}
verifyStaticCRT(asterctlWindowsX64, nativeBuildEnvironment)

const bundleName = `aster-team-${version}-windows-amd64`
const outputRoot = resolve(releaseOutputRoot, 'windows')
const bundle = resolve(outputRoot, bundleName)
const archive = resolve(outputRoot, `${bundleName}.tar.gz`)
mkdirSync(outputRoot, { recursive: true })
for (const path of [bundle, archive, `${archive}.sha256`]) {
  if (existsSync(path) && process.env.ASTER_OVERWRITE !== 'true') throw new Error(`release output already exists: ${path}`)
}
if (!bundle.toLowerCase().startsWith(`${outputRoot.toLowerCase()}\\`)) throw new Error('unsafe release output path')
rmSync(bundle, { recursive: true, force: true })
if (process.env.ASTER_OVERWRITE === 'true') {
  rmSync(archive, { force: true })
  rmSync(`${archive}.sha256`, { force: true })
}
for (const directory of ['bin', 'client-tools/asterctl/windows-x86_64', 'libexec', 'windows', 'THIRD_PARTY_LICENSES', 'plugins']) {
  mkdirSync(resolve(bundle, directory), { recursive: true })
}
const targetRoot = resolve(cargoTargetDirectory, 'release')
copyFileSync(resolve(root, 'third_party/qrc/LICENSE'), resolve(bundle, 'THIRD_PARTY_LICENSES/qrc-MIT.txt'))
copyFileSync(resolve(targetRoot, 'aster-control.exe'), resolve(bundle, 'bin/aster-control.exe'))
copyFileSync(resolve(targetRoot, 'aster-runner.exe'), resolve(bundle, 'bin/aster-runner.exe'))
copyFileSync(resolve(targetRoot, 'aster-team-cli.exe'), resolve(bundle, 'bin/aster-team-cli.exe'))
copyFileSync(asterctlWindowsX64, resolve(bundle, 'client-tools/asterctl/windows-x86_64/asterctl.exe'))
stageOfficialPlugins({ run: (binary, args) => run(binary, args),
  signer: resolve(targetRoot, 'aster-sign-plugin.exe'), root, bundle, version, profile: pluginProfile })

const caddyRuntime = JSON.parse(readFileSync(resolve(root, 'tools/caddy-runtime.json'), 'utf8'))
if (!/^\d+\.\d+\.\d+$/.test(caddyRuntime.version)
    || !/^https:\/\/github\.com\/caddyserver\/caddy\/releases\/download\//.test(caddyRuntime.windows_amd64_url)
    || !/^[0-9a-f]{128}$/.test(caddyRuntime.windows_amd64_sha512)) {
  throw new Error('tools/caddy-runtime.json Windows runtime is invalid')
}
const caddyStage = resolve(outputRoot, `.caddy-${caddyRuntime.version}-${process.pid}`)
rmSync(caddyStage, { recursive: true, force: true })
mkdirSync(caddyStage, { recursive: true })
try {
  const caddyArchive = await prepareVerifiedDownload({
    cacheDirectory: downloadCache,
    fileName: `caddy-${caddyRuntime.version}-windows-amd64.zip`,
    algorithm: 'sha512',
    expectedDigest: caddyRuntime.windows_amd64_sha512,
    download: path => run('curl.exe', ['--fail', '--location', '--retry', '4', '--output', path, caddyRuntime.windows_amd64_url]),
  })
  run('tar.exe', ['-xf', caddyArchive, '-C', caddyStage, 'caddy.exe', 'LICENSE'])
  copyFileSync(resolve(caddyStage, 'caddy.exe'), resolve(bundle, 'bin/caddy.exe'))
  copyFileSync(resolve(caddyStage, 'LICENSE'), resolve(bundle, 'THIRD_PARTY_LICENSES/Caddy-Apache-2.0.txt'))
} finally {
  rmSync(caddyStage, { recursive: true, force: true })
}

cpSync(resolve(root, 'customer/admin/dist'), resolve(bundle, 'admin'), { recursive: true })
cpSync(resolve(root, 'customer/member/dist'), resolve(bundle, 'member'), { recursive: true })
copyFileSync(resolve(root, 'customer/deploy/windows/init.ps1'), resolve(bundle, 'init.ps1'))
copyFileSync(resolve(root, 'customer/deploy/windows/install.ps1'), resolve(bundle, 'libexec/install.ps1'))
copyFileSync(resolve(root, 'customer/deploy/windows/restore-backup.ps1'), resolve(bundle, 'libexec/restore-backup.ps1'))
copyFileSync(resolve(root, 'customer/deploy/windows/service-launch.ps1'), resolve(bundle, 'windows/service-launch.ps1'))
copyFileSync(resolve(root, 'customer/deploy/windows/README.md'), resolve(bundle, 'README.md'))
writeFileSync(resolve(bundle, 'VERSION'), `${version}\n`, { flag: 'wx' })
run(process.execPath, [
  resolve(root, 'scripts/generate-customer-sbom.mjs'),
  `--output=${resolve(bundle, 'SBOM.cdx.json')}`,
  `--version=${version}`,
  '--platform=windows',
  '--architecture=amd64',
  '--rust-target=x86_64-pc-windows-msvc',
  '--runtime=msvc-static',
])

if (bundledFreeLicenseBytes) {
  const target = stageBundledFreeLicense(bundle, bundledFreeLicenseBytes)
  verifyStagedBundledFreeLicense(bundle, target, (binary, args) => run(binary, args), 'windows')
}

const releaseTool = resolve(targetRoot, 'aster-release-tool.exe')
run(releaseTool, [
  'sign', '--root', bundle, '--private-key', signingKeyFile, '--key-id', signingKeyID,
  '--version', version, '--architecture', arch, '--runtime', 'msvc', '--platform', 'windows', '--created-at', createdAt,
])
run(resolve(bundle, 'bin/aster-team-cli.exe'), ['verify-release', '--root', bundle], { env: embeddedKeys })
const artifactFailures = verifyArtifact('customer', bundle, 'windows')
if (artifactFailures.length) throw new Error(artifactFailures.join('\n'))
createTarGz(bundle, archive, bundleName, sourceDateEpoch)
const archiveHash = await digest(archive, 'sha256')
writeFileSync(`${archive}.sha256`, `${archiveHash}  ${bundleName}.tar.gz\n`, { flag: 'wx' })
console.log(`Customer release: ${archive}`)
console.log(`SHA-256:         ${archiveHash}`)
console.log(`Manifest SHA-256:${await digest(resolve(bundle, 'RELEASE.json'), 'sha256')}`)
