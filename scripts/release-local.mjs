#!/usr/bin/env node

import { createHash, createPrivateKey, createPublicKey } from 'node:crypto'
import {
  createReadStream,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, isAbsolute, join, relative, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { nextPatchFixtureVersion } from './ci/workspace-release-fixture.mjs'
import { configuredReleaseVersion } from './prepare-release-version.mjs'
import { resolveWindowsDockerRuntime } from './windows-docker-runtime.mjs'
import { readCustomerReleaseProfile } from './customer-release-profile.mjs'
import { officialPluginReleaseProfile } from './official-plugin-release.mjs'
import {
  inspectBundledFreeLicense,
  loadLocalReleaseSecurityFiles,
} from './release-security-files.mjs'

export { inspectBundledFreeLicense } from './release-security-files.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const VERSION_PATTERN = /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/
const PLATFORMS = new Set(['windows', 'linux', 'all'])
const BUILD_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/
const VERIFICATION_LEVELS = new Set(['none', 'changed', 'full'])
const CHECK_ORDER = ['lint', 'contracts', 'docs', 'unit', 'e2e', 'build', 'assets']
const CHECK_COMMANDS = {
  lint: [
    ['run', 'check:rust'],
    ['run', 'typecheck'],
    ['run', 'verify:table-layout'],
    ['run', 'verify:typography'],
  ],
  contracts: [['run', 'verify:contracts']],
  docs: [['run', 'verify:docs']],
  unit: [['run', 'test']],
  e2e: [['run', 'test:system-e2e']],
  build: [['run', 'build']],
  assets: [['run', 'verify:customer-assets']],
}

function usage() {
  return `Usage:
  npm run release:local -- --platform windows|linux|all [options]

Options:
  --version VERSION                     Assert the configured version (defaults to manifests).
  --build-id ID                         Identify logs and staging (defaults to HEAD).
  --verify none|changed|full             Select source verification (default: full).
  --checks lint,unit,e2e,build,...       Run only named source checks.

Builds production-signed Customer packages from the external security directory
configured by ASTER_LOCAL_SECURITY_CONFIG_DIR in the repository .env file.
The build ID defaults to the 12-character Git commit and may be supplied as
--build-id=$(git rev-parse --short=12 HEAD). Existing outputs are never
overwritten and artifacts are not uploaded.

Named checks: ${CHECK_ORDER.join(', ')}. --verify and --checks are mutually exclusive.
Package signing, SBOM, archive integrity, and platform smoke checks always run.`
}

export function parseReleaseArguments(arguments_) {
  let version = ''
  let platform = ''
  let buildID = ''
  let verify = 'full'
  let verifySpecified = false
  let checks = []
  let checksSpecified = false
  let help = false
  for (let index = 0; index < arguments_.length; index += 1) {
    const argument = arguments_[index]
    if (argument === '-h' || argument === '--help') {
      help = true
    } else if (argument === '--version') {
      version = arguments_[++index] || ''
    } else if (argument.startsWith('--version=')) {
      version = argument.slice('--version='.length)
    } else if (argument === '--platform') {
      platform = arguments_[++index] || ''
    } else if (argument.startsWith('--platform=')) {
      platform = argument.slice('--platform='.length)
    } else if (argument === '--build-id') {
      buildID = arguments_[++index] || ''
    } else if (argument.startsWith('--build-id=')) {
      buildID = argument.slice('--build-id='.length)
    } else if (argument === '--verify') {
      verify = arguments_[++index] || ''
      verifySpecified = true
    } else if (argument.startsWith('--verify=')) {
      verify = argument.slice('--verify='.length)
      verifySpecified = true
    } else if (argument === '--checks') {
      checksSpecified = true
      checks = (arguments_[++index] || '').split(',').map(value => value.trim()).filter(Boolean)
    } else if (argument.startsWith('--checks=')) {
      checksSpecified = true
      checks = argument.slice('--checks='.length).split(',').map(value => value.trim()).filter(Boolean)
    } else {
      throw new Error(`Unknown argument: ${argument}\n${usage()}`)
    }
  }
  if (help) return { help: true, version: '', platform: '', buildID: '', verify: 'full', checks: [] }
  version = version.replace(/^v/, '')
  if (version && !VERSION_PATTERN.test(version)) throw new Error(`--version is invalid\n${usage()}`)
  if (!PLATFORMS.has(platform)) throw new Error(`--platform is invalid\n${usage()}`)
  if (buildID && !BUILD_ID_PATTERN.test(buildID)) throw new Error(`--build-id is invalid\n${usage()}`)
  if (!VERIFICATION_LEVELS.has(verify)) throw new Error(`--verify is invalid\n${usage()}`)
  if (checksSpecified && !checks.length) throw new Error(`--checks requires at least one value\n${usage()}`)
  if (verifySpecified && checks.length) throw new Error(`--verify and --checks cannot be combined\n${usage()}`)
  const unknownChecks = checks.filter(check => !CHECK_COMMANDS[check])
  if (unknownChecks.length) throw new Error(`Unknown --checks value: ${unknownChecks.join(', ')}\n${usage()}`)
  checks = CHECK_ORDER.filter(check => checks.includes(check))
  return { help: false, version, platform, buildID, verify, checks }
}

export function releaseVerificationCommands(options) {
  if (options.checks.length) return options.checks.flatMap(check => CHECK_COMMANDS[check])
  if (options.verify === 'none') return []
  return [['run', options.verify === 'changed' ? 'verify:changed' : 'verify']]
}

function publicFromReleaseSeed(path) {
  const seed = readFileSync(path)
  if (seed.length !== 32) throw new Error('Release signing seed must contain exactly 32 raw bytes')
  const prefix = Buffer.from('302e020100300506032b657004220420', 'hex')
  const privateKey = createPrivateKey({
    key: Buffer.concat([prefix, seed]),
    format: 'der',
    type: 'pkcs8',
  })
  return createPublicKey(privateKey).export({ format: 'der', type: 'spki' }).toString('base64url')
}

export function loadLocalReleaseSecurity(rootDirectory, envPath = resolve(rootDirectory, '.env')) {
  const {
    directory,
    licenseKeyringFile,
    releaseKeyringFile,
    releaseSeedFile,
    pluginKeyringFile,
    pluginSeedFile,
    freeLicenseFile,
  } = loadLocalReleaseSecurityFiles(rootDirectory, envPath)
  const profile = readCustomerReleaseProfile({
    ASTER_LICENSE_TRUSTED_KEYS_JSON: readFileSync(licenseKeyringFile, 'utf8'),
    ASTER_RELEASE_TRUSTED_KEYS_JSON: readFileSync(releaseKeyringFile, 'utf8'),
  })
  const licenseKeys = profile.licenseTrustedKeys
  const releaseKeys = profile.releaseTrustedKeys
  const releasePublic = publicFromReleaseSeed(releaseSeedFile)
  const releaseSigner = releaseKeys.find(entry => entry.public_key_spki === releasePublic)
  if (!releaseSigner) throw new Error('Release signing seed does not match the Release keyring')
  if (licenseKeys.some(entry => entry.public_key_spki === releasePublic)) {
    throw new Error('License and Release signing keys must be cryptographically distinct')
  }
  const plugin = officialPluginReleaseProfile({
    ASTER_PLUGIN_TRUSTED_KEYS_JSON: readFileSync(pluginKeyringFile, 'utf8'),
    ASTER_PLUGIN_SIGNING_KEY_FILE: pluginSeedFile,
    ASTER_PLUGIN_SIGNING_KEY_ID: JSON.parse(readFileSync(pluginKeyringFile, 'utf8'))[0]?.key_id,
  })
  return {
    directory,
    licenseKeyringFile,
    releaseKeyringFile,
    releaseSeedFile,
    pluginKeyringFile,
    pluginSeedFile,
    pluginSigningKeyID: plugin.signingKeyID,
    pluginTrustedKeysJSON: plugin.keyringJSON,
    freeLicenseFile,
    releaseSigningKeyID: releaseSigner.key_id,
    licenseTrustedKeysJSON: profile.licenseTrustedKeysJSON,
    releaseTrustedKeysJSON: profile.releaseTrustedKeysJSON,
  }
}

function run(command, arguments_, options = {}) {
  const result = spawnSync(command, arguments_, {
    cwd: options.cwd || root,
    env: { ...process.env, ...options.env },
    stdio: options.capture ? ['ignore', 'pipe', 'pipe'] : 'inherit',
    encoding: options.capture ? 'utf8' : undefined,
    shell: /\.(cmd|bat)$/i.test(command),
    windowsHide: true,
  })
  if (result.error) throw result.error
  if (result.status !== 0) {
    const detail = options.capture ? `: ${(result.stderr || result.stdout || '').trim()}` : ''
    throw new Error(`${command} failed with status ${result.status}${detail}`)
  }
  return options.capture ? (result.stdout || '').trim() : ''
}

function git(arguments_) {
  return run('git', arguments_, { capture: true })
}

function bashCommand() {
  if (process.platform !== 'win32') return 'bash'
  const gitExecutables = run('where.exe', ['git.exe'], { capture: true }).split(/\r?\n/)
  for (const gitExecutable of gitExecutables) {
    const candidate = resolve(dirname(dirname(gitExecutable)), 'bin', 'bash.exe')
    if (existsSync(candidate) && statSync(candidate).isFile()) return candidate
  }
  throw new Error('Git for Windows Bash was not found; run npm run setup:linux-lab')
}

export function requireWindowsSystemPerl(environment = process.env, execute = spawnSync) {
  const lookup = execute('where.exe', ['perl.exe'], {
    env: environment,
    encoding: 'utf8',
    windowsHide: true,
  })
  const candidates = lookup.status === 0
    ? (lookup.stdout || '').split(/\r?\n/).map(value => value.trim()).filter(Boolean)
    : []
  for (const candidate of candidates) {
    const probe = execute(candidate, ['-MLocale::Maketext::Simple', '-e', '1'], {
      env: environment,
      encoding: 'utf8',
      windowsHide: true,
    })
    if (!probe.error && probe.status === 0) return candidate
  }
  throw new Error(
    'A complete system Perl was not found on PATH. Install Strawberry Perl, restart the terminal, '
      + 'and verify `perl -MLocale::Maketext::Simple -e 1`; local releases do not download or install Perl.',
  )
}

export function releaseOutputPaths(rootDirectory, version, platform, releaseRoot = resolve(rootDirectory, 'dist')) {
  const platformID = platform === 'windows' ? 'windows-amd64' : 'linux-amd64'
  const outputRoot = resolve(releaseRoot, platform)
  const bundleName = `aster-team-${version}-${platformID}`
  const bundle = resolve(outputRoot, bundleName)
  const archive = `${bundle}.tar.gz`
  return { platform, bundleName, outputRoot, bundle, archive, checksum: `${archive}.sha256` }
}

function selectedPlatforms(platform) {
  return platform === 'all' ? ['windows', 'linux'] : [platform]
}

export function assertReleaseOutputsAbsent(rootDirectory, version, platform) {
  const collisions = selectedPlatforms(platform)
    .flatMap(value => {
      const paths = releaseOutputPaths(rootDirectory, version, value)
      return [paths.bundle, paths.archive, paths.checksum]
    })
    .filter(existsSync)
  if (collisions.length > 0) {
    throw new Error(`Refusing to overwrite existing release output: ${collisions[0]}`)
  }
}

function preflightSource(version, platform, requestedBuildID = '') {
  if (selectedPlatforms(platform).includes('windows')
      && (process.platform !== 'win32' || process.arch !== 'x64')) {
    throw new Error('Windows amd64 releases must be built on a native Windows x64 host')
  }
  const configuredVersion = configuredReleaseVersion(root)
  if (version !== configuredVersion) {
    throw new Error(`Release version ${version} must match the configured version ${configuredVersion}`)
  }
  const status = git(['status', '--porcelain', '--untracked-files=all'])
  if (status) throw new Error('Refusing a production release from a dirty worktree')
  const branch = git(['branch', '--show-current'])
  if (branch !== 'main') {
    throw new Error(`Production releases require main; current branch is ${branch || '<detached>'}`)
  }
  run('git', ['show-ref', '--verify', '--quiet', 'refs/remotes/origin/main'])
  const commit = git(['rev-parse', 'HEAD'])
  const originMain = git(['rev-parse', 'refs/remotes/origin/main'])
  if (commit !== originMain) throw new Error('Local main must exactly match origin/main')
  assertReleaseOutputsAbsent(root, version, platform)
  const sourceDateEpoch = git(['show', '-s', '--format=%ct', 'HEAD'])
  const createdAt = new Date(Number(sourceDateEpoch) * 1000).toISOString()
  const shortCommit = git(['rev-parse', '--short=12', 'HEAD'])
  const buildID = requestedBuildID || shortCommit
  if (!BUILD_ID_PATTERN.test(buildID)) throw new Error('Resolved build ID is invalid')
  return { commit, shortCommit, buildID, sourceDateEpoch, createdAt }
}

async function digest(path) {
  const hash = createHash('sha256')
  for await (const chunk of createReadStream(path)) hash.update(chunk)
  return hash.digest('hex')
}

async function verifyChecksum(paths) {
  if (!existsSync(paths.archive) || !existsSync(paths.checksum) || !existsSync(paths.bundle)) {
    throw new Error(`Release output is incomplete for ${paths.platform}`)
  }
  const expectedLine = readFileSync(paths.checksum, 'utf8').trim()
  const match = expectedLine.match(/^([0-9a-f]{64})\s+([^\s]+)$/)
  if (!match || match[2] !== `${paths.bundleName}.tar.gz`) {
    throw new Error(`Release checksum file is invalid: ${paths.checksum}`)
  }
  const actual = await digest(paths.archive)
  if (match[1] !== actual) throw new Error(`Release archive checksum mismatch: ${paths.archive}`)
  return actual
}

async function verifyWindowsArchive(paths, security) {
  const temporaryRoot = mkdtempSync(join(tmpdir(), 'aster-local-release-'))
  try {
    run('tar.exe', ['-xzf', paths.archive, '-C', temporaryRoot])
    const extracted = resolve(temporaryRoot, paths.bundleName)
    if (!existsSync(extracted) || !statSync(extracted).isDirectory()) {
      throw new Error('Windows release archive did not contain the expected bundle root')
    }
    const environment = {
      ASTER_LICENSE_TRUSTED_KEYS_JSON: security.licenseTrustedKeysJSON,
      ASTER_RELEASE_TRUSTED_KEYS_JSON: security.releaseTrustedKeysJSON,
      ASTER_PLUGIN_TRUSTED_KEYS_JSON: security.pluginTrustedKeysJSON,
    }
    run(resolve(extracted, 'bin', 'aster-team-cli.exe'), ['verify-release', '--root', extracted], {
      env: environment,
    })
    run(resolve(extracted, 'client-tools', 'asterctl', 'windows-x86_64', 'asterctl.exe'), ['version'])
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true })
  }
}

function repositoryRelativeDirectory(path) {
  const value = relative(root, path)
  if (!value || value.startsWith('..') || isAbsolute(value)) {
    throw new Error(`Release working directory must be below the repository: ${path}`)
  }
  return value.replaceAll('\\', '/')
}

function requireWindowsAdministrator() {
  run('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command',
    '$p=[Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent()); if(-not $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)){Write-Error "Administrator PowerShell is required";exit 1}; $names=@("Control Blue","Control Green","Runner","Caddy","Maintenance"); foreach($name in $names){if(Get-ScheduledTask -TaskName $name -TaskPath "\\Aster Team\\" -ErrorAction SilentlyContinue){Write-Error "An Aster Team scheduled task already exists on this release host";exit 2}}',
  ], { capture: true })
}

function requireWindowsBuildTools() {
  const installerRoot = process.env['ProgramFiles(x86)'] || 'C:\\Program Files (x86)'
  const vswhere = resolve(installerRoot, 'Microsoft Visual Studio', 'Installer', 'vswhere.exe')
  if (!existsSync(vswhere) || !statSync(vswhere).isFile()) {
    throw new Error('Visual Studio Build Tools discovery tool is missing')
  }
  const installation = run(vswhere, [
    '-latest', '-products', '*',
    '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64',
    '-requires', 'Microsoft.VisualStudio.Component.Windows11SDK.26100',
    '-property', 'installationPath',
  ], { capture: true })
  if (!installation) throw new Error('Required Visual Studio C++ Build Tools and Windows SDK are missing')
}

function preflightBuildTools(version, platform, security) {
  let environment = process.env
  let bash = ''
  if (selectedPlatforms(platform).includes('windows')) {
    run('pwsh', ['-NoProfile', '-NonInteractive', '-Command',
      'if ($PSVersionTable.PSVersion.Major -lt 7) { throw "Windows lifecycle verification requires PowerShell 7" }',
    ], { capture: true })
    requireWindowsAdministrator()
    requireWindowsSystemPerl()
    requireWindowsBuildTools()
    run('where.exe', ['cargo.exe'], { capture: true })
    run('where.exe', ['rustup.exe'], { capture: true })
    run('where.exe', ['tar.exe'], { capture: true })
  }
  if (selectedPlatforms(platform).includes('linux')) {
    if (process.platform === 'win32') {
      requireWindowsBuildTools()
      run('where.exe', ['cargo.exe'], { capture: true })
      run('where.exe', ['rustup.exe'], { capture: true })
    }
    bash = bashCommand()
    if (process.platform === 'win32') {
      environment = resolveWindowsDockerRuntime().environment
    }
    run(bash, [
      './scripts/ci/build-production-linux-in-docker.sh',
      '--version', version,
      '--security-directory', security.directory,
      '--release-signing-key-id', security.releaseSigningKeyID,
      '--preflight-only',
    ], { env: environment })
  }
  return { bash, environment }
}

function verifyReleaseSource(version, platform, buildID, verification) {
  const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm'
  const commands = releaseVerificationCommands(verification)
  if (!commands.length) console.warn('warning: source verification skipped by --verify=none')
  for (const arguments_ of commands) run(npm, arguments_)
  return preflightSource(version, platform, buildID)
}

function buildWindows(version, source, security, stagingRoot, asterctlWindowsX64) {
  const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm'
  run(npm, ['run', 'build:windows', '--', `--version=${version}`], {
    env: {
      ASTER_LICENSE_TRUSTED_KEYS_JSON: security.licenseTrustedKeysJSON,
      ASTER_RELEASE_TRUSTED_KEYS_JSON: security.releaseTrustedKeysJSON,
      ASTER_PLUGIN_TRUSTED_KEYS_JSON: security.pluginTrustedKeysJSON,
      ASTER_PLUGIN_SIGNING_KEY_FILE: security.pluginSeedFile,
      ASTER_PLUGIN_SIGNING_KEY_ID: security.pluginSigningKeyID,
      ASTER_RELEASE_SIGNING_KEY_ID: security.releaseSigningKeyID,
      ASTER_RELEASE_SIGNING_KEY_FILE: security.releaseSeedFile,
      ASTER_CUSTOMER_FREE_LICENSE_FILE: security.freeLicenseFile,
      ASTER_RELEASE_CREATED_AT: source.createdAt,
      SOURCE_DATE_EPOCH: source.sourceDateEpoch,
      ASTER_OVERWRITE: 'false',
      ASTER_RELEASE_OUTPUT_ROOT: stagingRoot,
      ASTER_RELEASE_DOWNLOAD_CACHE: resolve(root, 'target', 'release-downloads'),
      ASTER_CLIENT_ASTERCTL_WINDOWS_X64: asterctlWindowsX64,
    },
  })
}

function buildWindowsUpgradeFixture(version, source, security, stagingRoot, asterctlWindowsX64) {
  run(process.execPath, ['./scripts/ci/build-windows-upgrade-fixture.mjs', `--base-version=${version}`], {
    env: {
      ASTER_LICENSE_TRUSTED_KEYS_JSON: security.licenseTrustedKeysJSON,
      ASTER_RELEASE_TRUSTED_KEYS_JSON: security.releaseTrustedKeysJSON,
      ASTER_PLUGIN_TRUSTED_KEYS_JSON: security.pluginTrustedKeysJSON,
      ASTER_PLUGIN_SIGNING_KEY_FILE: security.pluginSeedFile,
      ASTER_PLUGIN_SIGNING_KEY_ID: security.pluginSigningKeyID,
      ASTER_RELEASE_SIGNING_KEY_ID: security.releaseSigningKeyID,
      ASTER_RELEASE_SIGNING_KEY_FILE: security.releaseSeedFile,
      ASTER_RELEASE_CREATED_AT: source.createdAt,
      SOURCE_DATE_EPOCH: source.sourceDateEpoch,
      ASTER_OVERWRITE: 'false',
      ASTER_RELEASE_OUTPUT_ROOT: stagingRoot,
      ASTER_RELEASE_DOWNLOAD_CACHE: resolve(root, 'target', 'release-downloads'),
      ASTER_CLIENT_ASTERCTL_WINDOWS_X64: asterctlWindowsX64,
    },
  })
  return nextPatchFixtureVersion(version)
}

function prepareAsterctlWindowsX64(stagingRoot) {
  if (process.platform !== 'win32' || process.arch !== 'x64') {
    const supplied = process.env.ASTER_CLIENT_ASTERCTL_WINDOWS_X64 || ''
    if (!supplied) {
      throw new Error('ASTER_CLIENT_ASTERCTL_WINDOWS_X64 must identify a native Windows x64 asterctl.exe on non-Windows release hosts')
    }
    return realpathSync(supplied)
  }
  const output = resolve(stagingRoot, 'client-tools', 'asterctl-windows-x86_64.exe')
  const npm = 'npm.cmd'
  run(npm, ['run', 'build:asterctl:windows', '--', `--output=${output}`])
  return output
}

function buildLinux(version, security, stagingRoot, tools, asterctlWindowsX64) {
  run(tools.bash, [
    './scripts/ci/build-production-linux-in-docker.sh',
    '--version', version,
    '--security-directory', security.directory,
    '--release-signing-key-id', security.releaseSigningKeyID,
    '--asterctl-windows-x64', asterctlWindowsX64,
    '--output-root', repositoryRelativeDirectory(stagingRoot),
  ], { env: tools.environment })
}

function smokeLinuxArchive(version, stagingRoot, tools, runID) {
  const common = [
    './scripts/ci/exercise-supported-linux-install.sh',
    '--target', 'ubuntu-20.04',
    '--mode', 'container',
    '--package-dir', `./${repositoryRelativeDirectory(stagingRoot)}/linux`,
    '--version', version,
    '--image', 'ubuntu:20.04',
    '--package-family', 'apt',
  ]
  for (const role of ['customer', 'runner']) {
    run(tools.bash, [...common, '--role', role], {
      env: {
        ...tools.environment,
        GITHUB_RUN_ID: runID,
        GITHUB_JOB: role,
        GITHUB_RUN_ATTEMPT: '1',
      },
    })
  }
}

export function smokeWindowsArchive(version, candidateVersion, stagingRoot, security, runRoot, execute = run, npmCLI = process.env.npm_execpath) {
  if (!npmCLI) throw new Error('Invoke the Windows release lifecycle through npm run release:local')
  const paths = releaseOutputPaths(root, version, 'windows', stagingRoot)
  const candidate = releaseOutputPaths(root, candidateVersion, 'windows', stagingRoot)
  // Run npm through Node, avoiding cmd.exe splitting paths containing spaces.
  execute(process.execPath, [npmCLI, 'run', 'test:windows-release', '--',
    '-Archive', paths.archive,
    '-Version', version,
    '-CandidateVersion', candidateVersion,
    '-CandidateArchive', candidate.archive,
    '-ReleaseTool', resolve(root, 'target', 'release', 'aster-release-tool.exe'),
    '-ReleaseSigningKey', security.releaseSeedFile,
    '-ReleaseSigningKeyId', security.releaseSigningKeyID,
    '-WorkRoot', resolve(runRoot, 'windows-smoke'),
    '-DiagnosticsRoot', resolve(runRoot, 'windows-diagnostics'),
  ])
  execute(process.execPath, [npmCLI, 'run', 'test:windows-runner-install', '--',
    '-Archive', paths.archive,
    '-Version', version,
    '-WorkRoot', resolve(runRoot, 'windows-runner-smoke'),
    '-DiagnosticsRoot', resolve(runRoot, 'windows-runner-diagnostics'),
  ])
}

export const windowsReleaseTestsEnabled = (environment = process.env) => environment.ASTER_ENABLE_WINDOWS_TESTS === 'true'

export function publishStagedArtifacts(rootDirectory, version, platform, stagingRoot) {
  const moved = []
  try {
    for (const value of selectedPlatforms(platform)) {
      const staged = releaseOutputPaths(rootDirectory, version, value, stagingRoot)
      const destination = releaseOutputPaths(rootDirectory, version, value)
      mkdirSync(destination.outputRoot, { recursive: true })
      for (const key of ['bundle', 'archive', 'checksum']) {
        if (!existsSync(staged[key])) throw new Error(`Staged release output is missing: ${staged[key]}`)
        renameSync(staged[key], destination[key])
        moved.push({ source: staged[key], destination: destination[key] })
      }
    }
  } catch (error) {
    for (const move of moved.reverse()) {
      try {
        if (existsSync(move.destination) && !existsSync(move.source)) {
          renameSync(move.destination, move.source)
        }
      } catch {
        // Preserve the original publication failure; the run record identifies the failed transaction.
      }
    }
    throw error
  }
}

function writeRunState(runRoot, state) {
  mkdirSync(runRoot, { recursive: true })
  writeFileSync(
    resolve(runRoot, 'result.json'),
    `${JSON.stringify({ schema: 'aster.local-release-run.v1', ...state }, null, 2)}\n`,
  )
}

async function artifactReport(version, source, platform, releaseRoot) {
  const paths = releaseOutputPaths(root, version, platform, releaseRoot)
  const archiveSHA256 = await verifyChecksum(paths)
  const manifestSHA256 = await digest(resolve(paths.bundle, 'RELEASE.json'))
  const bytes = statSync(paths.archive).size
  return { platform, version, source, archiveSHA256, manifestSHA256, bytes }
}

function printArtifactReport(report) {
  const paths = releaseOutputPaths(root, report.version, report.platform)
  console.log(`\n${report.platform} release complete`)
  console.log(`  Commit:               ${report.source.commit}`)
  console.log(`  Build ID:             ${report.source.buildID}`)
  console.log(`  Version:              ${report.version}`)
  console.log(`  Archive:              ${paths.archive}`)
  console.log(`  Bytes:                ${report.bytes}`)
  console.log(`  Archive SHA-256:       ${report.archiveSHA256}`)
  console.log(`  RELEASE.json SHA-256: ${report.manifestSHA256}`)
  console.log('  Runtime:              fully offline')
}

async function main() {
  const options = parseReleaseArguments(process.argv.slice(2))
  if (options.help) {
    console.log(usage())
    return
  }
  options.version ||= configuredReleaseVersion(root)
  const security = loadLocalReleaseSecurity(root)
  let source = preflightSource(options.version, options.platform, options.buildID)
  const timestamp = new Date().toISOString().replaceAll(/[-:.TZ]/g, '')
  const runID = `${source.buildID}-${timestamp}-${process.pid}`
  const runRoot = resolve(root, 'target', 'release-local', 'runs', runID)
  const stagingRoot = resolve(root, 'target', 'release-local', 'staging', runID)
  writeRunState(runRoot, {
    status: 'running', build_id: source.buildID, commit: source.commit,
    version: options.version, platform: options.platform,
    verification: options.checks.length ? { mode: 'custom', checks: options.checks } : { mode: options.verify },
  })

  try {
    const tools = preflightBuildTools(options.version, options.platform, security)
    source = verifyReleaseSource(options.version, options.platform, source.buildID, options)
    rmSync(stagingRoot, { recursive: true, force: true })
    mkdirSync(stagingRoot, { recursive: true })
    console.log('Local Customer production release')
    console.log(`  Commit:   ${source.commit}`)
    console.log(`  Build ID: ${source.buildID}`)
    console.log(`  Version:  ${options.version}`)
    console.log(`  Platform: ${options.platform}`)
    console.log(`  Signer:   ${security.releaseSigningKeyID}`)
    console.log(`  Verify:   ${options.checks.length ? options.checks.join(',') : options.verify}`)

    const platforms = selectedPlatforms(options.platform)
    const asterctlWindowsX64 = platforms.some(platform => platform === 'windows' || platform === 'linux')
      ? prepareAsterctlWindowsX64(stagingRoot)
      : ''
    if (platforms.includes('windows')) {
      buildWindows(options.version, source, security, stagingRoot, asterctlWindowsX64)
      const paths = releaseOutputPaths(root, options.version, 'windows', stagingRoot)
      await verifyChecksum(paths)
      await verifyWindowsArchive(paths, security)
      if (windowsReleaseTestsEnabled()) {
        const candidateVersion = buildWindowsUpgradeFixture(options.version, source, security, stagingRoot, asterctlWindowsX64)
        smokeWindowsArchive(options.version, candidateVersion, stagingRoot, security, runRoot)
      }
    }
    if (platforms.includes('linux')) {
      buildLinux(options.version, security, stagingRoot, tools, asterctlWindowsX64)
      await verifyChecksum(releaseOutputPaths(root, options.version, 'linux', stagingRoot))
      smokeLinuxArchive(options.version, stagingRoot, tools, runID)
    }
    const reports = []
    for (const platform of selectedPlatforms(options.platform)) {
      reports.push(await artifactReport(options.version, source, platform, stagingRoot))
    }
    publishStagedArtifacts(root, options.version, options.platform, stagingRoot)
    for (const report of reports) printArtifactReport(report)
    try {
      writeRunState(runRoot, {
        status: 'succeeded', build_id: source.buildID, commit: source.commit,
        version: options.version, platform: options.platform,
        verification: options.checks.length ? { mode: 'custom', checks: options.checks } : { mode: options.verify },
        artifacts: reports.map(report => ({
          platform: report.platform,
          archive: releaseOutputPaths(root, report.version, report.platform).archive,
          bytes: report.bytes,
          archive_sha256: report.archiveSHA256,
          manifest_sha256: report.manifestSHA256,
        })),
      })
    } catch (error) {
      console.warn(`warning: release succeeded, but the run record could not be updated: ${error.message}`)
    }
  } catch (error) {
    try {
      writeRunState(runRoot, {
        status: 'failed', build_id: source.buildID, commit: source.commit,
        version: options.version, platform: options.platform, error: error.message,
        verification: options.checks.length ? { mode: 'custom', checks: options.checks } : { mode: options.verify },
      })
    } catch (recordError) {
      console.warn(`warning: failed release run record could not be updated: ${recordError.message}`)
    }
    throw error
  } finally {
    try {
      rmSync(stagingRoot, { recursive: true, force: true })
    } catch (error) {
      console.warn(`warning: release staging cleanup failed: ${error.message}`)
    }
  }
  console.log('\nArtifacts were retained locally and were not uploaded.')
}

const currentFile = fileURLToPath(import.meta.url)
const invokedFile = process.argv[1] ? resolve(process.argv[1]) : ''
if (process.platform === 'win32'
  ? currentFile.toLowerCase() === invokedFile.toLowerCase()
  : currentFile === invokedFile) {
  main().catch(error => {
    console.error(`error: ${error.message}`)
    process.exitCode = 1
  })
}
