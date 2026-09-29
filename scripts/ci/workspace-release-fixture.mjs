import { readFileSync, writeFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'

export function runCommand(root, command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: options.capture ? 'utf8' : undefined,
    stdio: options.capture ? ['ignore', 'pipe', 'inherit'] : 'inherit',
    env: { ...process.env, ...options.env },
    windowsHide: true,
  })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`${command} exited with status ${result.status}`)
  return result.stdout || ''
}

export function nextPatchFixtureVersion(baseVersion, suffix = 'ci-upgrade') {
  const parsed = /^(\d+)\.(\d+)\.(\d+)(?:[-+][0-9A-Za-z.-]+)?$/.exec(baseVersion)
  if (!parsed) throw new Error('base version must be a semantic version')
  return `${parsed[1]}.${parsed[2]}.${BigInt(parsed[3]) + 1n}-${suffix}`
}

function replaceExactlyOnce(source, pattern, replacement, label) {
  let count = 0
  const result = source.replace(pattern, (...args) => {
    count += 1
    return typeof replacement === 'function' ? replacement(...args) : replacement
  })
  if (count !== 1) throw new Error(`${label} expected one version field, found ${count}`)
  return result
}

function escapeRegularExpression(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

function workspacePackageNames(root, cargo) {
  const encoded = runCommand(root, cargo, ['metadata', '--locked', '--no-deps', '--format-version', '1'], { capture: true })
  const metadata = JSON.parse(encoded)
  const members = new Set(metadata.workspace_members)
  return new Set(metadata.packages.filter(pkg => members.has(pkg.id)).map(pkg => pkg.name))
}

function rewriteCargoLock(source, packageNames, baseVersion, candidateVersion) {
  let changed = 0
  const rewritten = source.replace(/\[\[package\]\][\s\S]*?(?=\r?\n\[\[package\]\]|\s*$)/g, block => {
    const name = /^name = "([^"]+)"$/m.exec(block)?.[1]
    if (!name || !packageNames.has(name)) return block
    const next = block.replace(
      new RegExp(`^version = "${escapeRegularExpression(baseVersion)}"$`, 'm'),
      `version = "${candidateVersion}"`,
    )
    if (next === block) throw new Error(`Cargo.lock workspace package ${name} does not use ${baseVersion}`)
    changed += 1
    return next
  })
  if (changed !== packageNames.size) {
    throw new Error(`Cargo.lock updated ${changed} workspace packages, expected ${packageNames.size}`)
  }
  return rewritten
}

function releaseVersionPaths(root) {
  return {
    cargoManifest: `${root}/Cargo.toml`,
    cargoLock: `${root}/Cargo.lock`,
    packageManifest: `${root}/package.json`,
    packageLock: `${root}/package-lock.json`,
  }
}

export function writeWorkspaceReleaseVersion({ root, cargo, baseVersion, candidateVersion }) {
  const paths = releaseVersionPaths(root)
  const originals = Object.fromEntries(Object.entries(paths).map(([name, path]) => [name, readFileSync(path)]))
  const packageNames = workspacePackageNames(root, cargo)
  try {
    const cargoManifest = originals.cargoManifest.toString('utf8')
    const nextCargoManifest = replaceExactlyOnce(
      cargoManifest,
      /(\[workspace\.package\][\s\S]*?\r?\nversion\s*=\s*")([^"]+)(")/,
      (_match, prefix, version, suffix) => {
        if (version !== baseVersion) throw new Error(`Cargo.toml is ${version}, expected ${baseVersion}`)
        return `${prefix}${candidateVersion}${suffix}`
      },
      'Cargo.toml',
    )
    const packageManifest = JSON.parse(originals.packageManifest.toString('utf8'))
    if (packageManifest.version !== baseVersion) {
      throw new Error(`package.json is ${packageManifest.version}, expected ${baseVersion}`)
    }
    packageManifest.version = candidateVersion
    const packageLock = JSON.parse(originals.packageLock.toString('utf8'))
    if (packageLock.version !== baseVersion || packageLock.packages?.['']?.version !== baseVersion) {
      throw new Error('package-lock.json root version does not match the release version')
    }
    packageLock.version = candidateVersion
    packageLock.packages[''].version = candidateVersion

    writeFileSync(paths.cargoManifest, nextCargoManifest)
    writeFileSync(paths.cargoLock, rewriteCargoLock(originals.cargoLock.toString('utf8'), packageNames, baseVersion, candidateVersion))
    writeFileSync(paths.packageManifest, `${JSON.stringify(packageManifest, null, 2)}\n`)
    writeFileSync(paths.packageLock, `${JSON.stringify(packageLock, null, 2)}\n`)
    runCommand(root, cargo, ['metadata', '--locked', '--no-deps', '--format-version', '1'], { capture: true })
    return paths
  } catch (error) {
    for (const [name, path] of Object.entries(paths)) writeFileSync(path, originals[name])
    throw error
  }
}

export function withWorkspaceReleaseVersion({ root, cargo, baseVersion, candidateVersion }, callback) {
  const paths = releaseVersionPaths(root)
  const originals = Object.fromEntries(Object.entries(paths).map(([name, path]) => [name, readFileSync(path)]))
  try {
    writeWorkspaceReleaseVersion({ root, cargo, baseVersion, candidateVersion })
    return callback(candidateVersion)
  } finally {
    for (const [name, path] of Object.entries(paths)) writeFileSync(path, originals[name])
  }
}
