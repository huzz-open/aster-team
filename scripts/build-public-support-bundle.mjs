import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import {
  closeSync, constants, existsSync, fstatSync, mkdirSync, openSync, readSync, readdirSync,
  lstatSync, writeFileSync,
} from 'node:fs'
import { basename, dirname, isAbsolute, relative, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const defaultTemplateRoot = resolve(repositoryRoot, 'distribution/public-support')
const allowedTemplates = [
  '.github/ISSUE_TEMPLATE/bug.yml',
  '.github/ISSUE_TEMPLATE/config.yml',
  '.github/ISSUE_TEMPLATE/installation.yml',
  'README.md.template',
  'docs/release-verification.md',
]
const maximumArchiveBytes = 1024 * 1024 * 1024
const versionPattern = /^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$/
const digestPattern = /^[a-f0-9]{64}$/

function expectedPublicFiles(version, platform = 'linux') {
  return [
    '.github/ISSUE_TEMPLATE/bug.yml', '.github/ISSUE_TEMPLATE/config.yml',
    '.github/ISSUE_TEMPLATE/installation.yml', platform === 'windows' ? 'README-WINDOWS.md' : 'README-LINUX.md', 'README.md',
    'docs/release-verification.md', 'docs/user-manual.md',
    `releases/${version}/RELEASE_NOTES.md`, `releases/${version}/SHA256SUMS`,
  ].sort()
}

function archiveEntry(archivePath, bundleName, relativePath, maximumBytes) {
  // GNU tar otherwise treats the colon in a Windows drive path as a remote host.
  const result = spawnSync('tar', ['-xOzf', `./${basename(archivePath)}`, `${bundleName}/${relativePath}`], {
    cwd: dirname(archivePath),
    encoding: null,
    maxBuffer: maximumBytes + 1,
    windowsHide: true,
  })
  if (result.error || result.status !== 0 || !Buffer.isBuffer(result.stdout)
    || result.stdout.length < 1 || result.stdout.length > maximumBytes) throw new Error(`release archive is missing valid ${relativePath}`)
  return result.stdout
}

function bundledLicenseIdentity(archivePath, bundleName, version, platform) {
  const releaseBytes = archiveEntry(archivePath, bundleName, 'RELEASE.json', 4 * 1024 * 1024)
  const licenseBytes = archiveEntry(archivePath, bundleName, 'licenses/free-license.json', 64 * 1024)
  let release
  let license
  try {
    release = JSON.parse(new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(releaseBytes))
    license = JSON.parse(new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(licenseBytes))
  } catch {
    throw new Error('release archive identity documents are invalid')
  }
  if (release.schema !== 'aster.release-manifest.v1' || release.product !== 'aster-team' || release.version !== version
    || release.platform !== platform || release.architecture !== 'amd64' || typeof release.key_id !== 'string'
    || !release.key_id || typeof release.signature !== 'string' || !release.signature || !Array.isArray(release.files)) {
    throw new Error('release archive manifest identity is invalid')
  }
  const entries = release.files.filter(file => file?.path === 'licenses/free-license.json')
  const licenseSHA256 = sha256(licenseBytes)
  if (entries.length !== 1 || entries[0].size !== licenseBytes.length || entries[0].sha256 !== licenseSHA256) {
    throw new Error('bundled free license does not match the release manifest')
  }
  const claims = license?.claims
  if (!claims || claims.schema !== 'aster.license.v2' || claims.product !== 'aster-team'
    || claims.source?.kind !== 'free_distribution' || claims.binding?.mode !== 'unbound'
    || typeof claims.license_id !== 'string' || typeof claims.plan_id !== 'string' || !Number.isSafeInteger(claims.plan_version)
    || typeof claims.edition !== 'string' || typeof claims.minimum_version !== 'string'
    || !Number.isSafeInteger(claims.quota_policy_version) || !claims.entitlements || !claims.validity?.expiry) {
    throw new Error('bundled free license identity is invalid')
  }
  return {
    release_manifest: { path: 'RELEASE.json', sha256: sha256(releaseBytes), key_id: release.key_id },
    bundled_license: {
      path: 'licenses/free-license.json', sha256: licenseSHA256, size_bytes: licenseBytes.length,
      license_id: claims.license_id, plan_id: claims.plan_id, plan_version: claims.plan_version,
      edition: claims.edition, minimum_version: claims.minimum_version,
      entitlements: claims.entitlements, quota_policy_version: claims.quota_policy_version,
      binding: claims.binding.mode, source: claims.source.kind, expiry: claims.validity.expiry,
    },
  }
}

function leafFiles(root, directory = root) {
  const items = []
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = resolve(directory, entry.name)
    if (entry.isSymbolicLink()) throw new Error('public support templates must not contain links')
    if (entry.isDirectory()) items.push(...leafFiles(root, path))
    else if (entry.isFile()) items.push(relative(root, path).split(sep).join('/'))
    else throw new Error('public support templates must contain regular files only')
  }
  return items.sort()
}

function readBoundedRegularFile(path, maximumBytes) {
  const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0) | (constants.O_NONBLOCK ?? 0))
  try {
    const before = fstatSync(descriptor)
    if (!before.isFile() || before.size < 1 || before.size > maximumBytes) throw new Error('input must be a bounded regular file')
    const bytes = Buffer.allocUnsafe(before.size)
    let offset = 0
    while (offset < bytes.length) {
      const count = readSync(descriptor, bytes, offset, bytes.length - offset, null)
      if (count === 0) break
      offset += count
    }
    const after = fstatSync(descriptor)
    if (offset !== bytes.length || before.dev !== after.dev || before.ino !== after.ino || before.size !== after.size || before.mtimeMs !== after.mtimeMs) {
      throw new Error('input changed while it was read')
    }
    return bytes
  } finally {
    closeSync(descriptor)
  }
}

function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex')
}

function hashBoundedRegularFile(path, maximumBytes) {
  const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0) | (constants.O_NONBLOCK ?? 0))
  try {
    const before = fstatSync(descriptor)
    if (!before.isFile() || before.size < 1 || before.size > maximumBytes) throw new Error('input must be a bounded regular file')
    const hash = createHash('sha256')
    const chunk = Buffer.allocUnsafe(1024 * 1024)
    let bytesRead = 0
    while (bytesRead < before.size) {
      const count = readSync(descriptor, chunk, 0, Math.min(chunk.length, before.size - bytesRead), null)
      if (count === 0) break
      hash.update(chunk.subarray(0, count))
      bytesRead += count
    }
    const after = fstatSync(descriptor)
    if (bytesRead !== before.size || before.dev !== after.dev || before.ino !== after.ino || before.size !== after.size || before.mtimeMs !== after.mtimeMs) {
      throw new Error('input changed while it was read')
    }
    return { sha256: hash.digest('hex'), size: before.size }
  } finally {
    closeSync(descriptor)
  }
}

function writeOutput(root, relativePath, bytes) {
  const target = resolve(root, relativePath)
  mkdirSync(dirname(target), { recursive: true })
  writeFileSync(target, bytes, { flag: 'wx', mode: 0o644 })
}

export function buildPublicSupportBundle({
  version,
  archivePath,
  checksumPath,
  outputPath,
  environment = 'local',
  platform = 'linux',
  windowsGuidePath = resolve(repositoryRoot, 'customer/deploy/windows/README.md'),
  templateRoot = defaultTemplateRoot,
  manualPath = resolve(repositoryRoot, 'docs/user-manual.md'),
  linuxGuidePath = resolve(repositoryRoot, 'README-LINUX.md'),
}) {
  if (!versionPattern.test(version) || !['local', 'production'].includes(environment) || !['linux', 'windows'].includes(platform)) throw new Error('version or environment is invalid')
  for (const path of [archivePath, checksumPath, outputPath, templateRoot, manualPath, platform === 'windows' ? windowsGuidePath : linuxGuidePath]) {
    if (!path || !isAbsolute(path)) throw new Error('all public support paths must be absolute')
  }
  if (existsSync(outputPath)) throw new Error('public support output already exists')
  const expectedArchiveName = `aster-team-${version}-${platform}-amd64.tar.gz`
  const bundleName = expectedArchiveName.slice(0, -'.tar.gz'.length)
  if (basename(archivePath) !== expectedArchiveName || basename(checksumPath) !== `${expectedArchiveName}.sha256`) {
    throw new Error('release artifact names do not match the requested version')
  }
  const templates = leafFiles(templateRoot)
  if (JSON.stringify(templates) !== JSON.stringify(allowedTemplates)) throw new Error('public support template whitelist changed')
  const archive = hashBoundedRegularFile(archivePath, maximumArchiveBytes)
  const checksum = readBoundedRegularFile(checksumPath, 256).toString('utf8')
  const archiveSHA256 = archive.sha256
  if (!digestPattern.test(archiveSHA256) || checksum !== `${archiveSHA256}  ${expectedArchiveName}\n`) {
    throw new Error('release checksum does not match the exact artifact')
  }
  const embedded = bundledLicenseIdentity(archivePath, bundleName, version, platform)

  const sources = new Map()
  for (const path of templates) {
    const outputPath = path.endsWith('.template') ? path.slice(0, -'.template'.length) : path
    sources.set(outputPath, readBoundedRegularFile(resolve(templateRoot, path), 1024 * 1024))
  }
  sources.set('docs/user-manual.md', readBoundedRegularFile(manualPath, 4 * 1024 * 1024))
  sources.set(platform === 'windows' ? 'README-WINDOWS.md' : 'README-LINUX.md', readBoundedRegularFile(platform === 'windows' ? windowsGuidePath : linuxGuidePath, 4 * 1024 * 1024))
  sources.set(`releases/${version}/SHA256SUMS`, Buffer.from(checksum, 'utf8'))
  sources.set(`releases/${version}/RELEASE_NOTES.md`, Buffer.from(
    `# Aster Team ${version}\n\n平台：${platform === 'windows' ? 'Windows amd64 实验版\n\n不承诺稳定 缺陷修复可能较慢 正式使用建议选择 Linux' : 'Linux amd64 推荐'}\n\n安装包：\`${expectedArchiveName}\`\n\n请先按照[发行文件校验](../../docs/release-verification.md)核对 SHA-256，再阅读[用户手册](../../docs/user-manual.md)完成安装。\n`,
    'utf8',
  ))
  for (const [path, bytes] of [...sources].sort(([left], [right]) => left.localeCompare(right))) writeOutput(outputPath, path, bytes)
  const files = [...sources].map(([path, bytes]) => ({ path, sha256: sha256(bytes), size_bytes: bytes.length }))
    .sort((left, right) => left.path.localeCompare(right.path))
  const manifest = {
    schema: 'aster.public-support-release.v1',
    environment,
    product: 'aster-team',
    version,
    artifact: { name: expectedArchiveName, sha256: archiveSHA256, size_bytes: archive.size, platform: `${platform}-amd64`, ...(platform === 'windows' ? { channel: 'experimental' } : {}) },
    ...embedded,
    files,
  }
  writeOutput(outputPath, `releases/${version}/manifest.json`, Buffer.from(`${JSON.stringify(manifest)}\n`, 'utf8'))
  verifyPublicSupportBundle({ version, bundlePath: outputPath, expectedEnvironment: environment, platform })
  return manifest
}

export function verifyPublicSupportBundle({ version, bundlePath, expectedEnvironment, platform = 'linux' }) {
  if (!versionPattern.test(version) || !isAbsolute(bundlePath) || !['linux', 'windows'].includes(platform)
    || (expectedEnvironment !== undefined && !['local', 'production'].includes(expectedEnvironment))) {
    throw new Error('public support verification input is invalid')
  }
  const root = lstatSync(bundlePath)
  if (!root.isDirectory() || root.isSymbolicLink()) throw new Error('public support bundle must be a regular directory')
  const manifestPath = `releases/${version}/manifest.json`
  const expectedFiles = expectedPublicFiles(version, platform)
  if (JSON.stringify(leafFiles(bundlePath)) !== JSON.stringify([...expectedFiles, manifestPath].sort())) {
    throw new Error('public support bundle file whitelist changed')
  }
  const manifestBytes = readBoundedRegularFile(resolve(bundlePath, manifestPath), 1024 * 1024)
  let manifest
  try {
    manifest = JSON.parse(new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(manifestBytes))
  } catch {
    throw new Error('public support manifest is invalid')
  }
  if (`${JSON.stringify(manifest)}\n` !== manifestBytes.toString('utf8')
    || manifest.schema !== 'aster.public-support-release.v1' || manifest.product !== 'aster-team'
    || manifest.version !== version || !['local', 'production'].includes(manifest.environment)
    || (expectedEnvironment !== undefined && manifest.environment !== expectedEnvironment)) {
    throw new Error('public support manifest identity is invalid')
  }
  const expectedArchiveName = `aster-team-${version}-${platform}-amd64.tar.gz`
  if (manifest.artifact?.name !== expectedArchiveName || manifest.artifact?.platform !== `${platform}-amd64`
    || (platform === 'windows' && manifest.artifact?.channel !== 'experimental')
    || !digestPattern.test(manifest.artifact?.sha256) || !Number.isSafeInteger(manifest.artifact?.size_bytes)
    || manifest.artifact.size_bytes < 1) throw new Error('public support artifact identity is invalid')
  if (!Array.isArray(manifest.files) || manifest.files.length !== expectedFiles.length
    || JSON.stringify(manifest.files.map(file => file?.path).sort()) !== JSON.stringify(expectedFiles)) {
    throw new Error('public support manifest file whitelist changed')
  }
  for (const file of manifest.files) {
    if (!digestPattern.test(file.sha256) || !Number.isSafeInteger(file.size_bytes) || file.size_bytes < 1) {
      throw new Error(`public support manifest entry is invalid: ${file.path}`)
    }
    const bytes = readBoundedRegularFile(resolve(bundlePath, file.path), 4 * 1024 * 1024)
    if (bytes.length !== file.size_bytes || sha256(bytes) !== file.sha256) {
      throw new Error(`public support file does not match manifest: ${file.path}`)
    }
  }
  const checksum = readBoundedRegularFile(resolve(bundlePath, `releases/${version}/SHA256SUMS`), 256).toString('utf8')
  if (checksum !== `${manifest.artifact.sha256}  ${expectedArchiveName}\n`) {
    throw new Error('public support checksum does not match the artifact identity')
  }
  return manifest
}

function argument(name) {
  const prefix = `--${name}=`
  return process.argv.find(value => value.startsWith(prefix))?.slice(prefix.length)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const version = argument('version') || ''
  const environment = argument('environment') || 'local'
  const platform = argument('platform') || 'linux'
  const verifyArgument = argument('verify')
  if (verifyArgument) {
    const manifest = verifyPublicSupportBundle({ version, bundlePath: resolve(verifyArgument), expectedEnvironment: environment, platform })
    process.stdout.write(`Verified public support bundle: ${resolve(verifyArgument)}\nArtifact SHA-256: ${manifest.artifact.sha256}\n`)
    process.exit(0)
  }
  const archiveArgument = argument('archive')
  const checksumArgument = argument('checksum')
  if (!version || !archiveArgument || !checksumArgument) {
    throw new Error('--version, --archive and --checksum are required')
  }
  const archivePath = resolve(archiveArgument)
  const checksumPath = resolve(checksumArgument)
  const outputPath = argument('output')
    ? resolve(argument('output'))
    : resolve(repositoryRoot, `dist/public-support/aster-team-${version}-${environment}-${platform}`)
  const relativeOutput = relative(resolve(repositoryRoot, 'dist/public-support'), outputPath)
  if (!relativeOutput || relativeOutput.startsWith('..') || isAbsolute(relativeOutput)) throw new Error('output must be below dist/public-support')
  const manifest = buildPublicSupportBundle({ version, archivePath, checksumPath, outputPath, environment, platform })
  process.stdout.write(`Public support bundle: ${outputPath}\nArtifact SHA-256: ${manifest.artifact.sha256}\n`)
}
