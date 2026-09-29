#!/usr/bin/env node

import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { writeWorkspaceReleaseVersion } from './ci/workspace-release-fixture.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const SEMANTIC_VERSION_PATTERN = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/

function usage() {
  return `Usage:
  npm run release:prepare -- --version VERSION

Updates package.json, package-lock.json, Cargo.toml, and Cargo.lock to one
SemVer release version. The command refuses a dirty worktree or a version
that does not advance the current release.`
}

export function parseReleaseVersionArguments(arguments_) {
  let version = ''
  let help = false
  for (let index = 0; index < arguments_.length; index += 1) {
    const argument = arguments_[index]
    if (argument === '-h' || argument === '--help') help = true
    else if (argument === '--version') version = arguments_[++index] || ''
    else if (argument.startsWith('--version=')) version = argument.slice('--version='.length)
    else throw new Error(`Unknown argument: ${argument}\n${usage()}`)
  }
  if (help) return { help: true, version: '' }
  version = version.replace(/^v/, '')
  if (!SEMANTIC_VERSION_PATTERN.test(version)) throw new Error(`--version is invalid\n${usage()}`)
  return { help: false, version }
}

function parsedSemanticVersion(version) {
  const match = SEMANTIC_VERSION_PATTERN.exec(version)
  if (!match) throw new Error(`Invalid semantic version: ${version}`)
  return {
    core: [BigInt(match[1]), BigInt(match[2]), BigInt(match[3])],
    prerelease: match[4] ? match[4].split('.') : [],
  }
}

export function compareSemanticVersions(left, right) {
  const a = parsedSemanticVersion(left)
  const b = parsedSemanticVersion(right)
  for (let index = 0; index < a.core.length; index += 1) {
    if (a.core[index] !== b.core[index]) return a.core[index] > b.core[index] ? 1 : -1
  }
  if (!a.prerelease.length || !b.prerelease.length) {
    if (a.prerelease.length === b.prerelease.length) return 0
    return a.prerelease.length ? -1 : 1
  }
  const length = Math.max(a.prerelease.length, b.prerelease.length)
  for (let index = 0; index < length; index += 1) {
    if (a.prerelease[index] === undefined) return -1
    if (b.prerelease[index] === undefined) return 1
    if (a.prerelease[index] === b.prerelease[index]) continue
    const aNumeric = /^\d+$/.test(a.prerelease[index])
    const bNumeric = /^\d+$/.test(b.prerelease[index])
    if (aNumeric && bNumeric) return BigInt(a.prerelease[index]) > BigInt(b.prerelease[index]) ? 1 : -1
    if (aNumeric !== bNumeric) return aNumeric ? -1 : 1
    return a.prerelease[index] > b.prerelease[index] ? 1 : -1
  }
  return 0
}

function cargoWorkspaceVersion(rootDirectory) {
  const manifest = readFileSync(resolve(rootDirectory, 'Cargo.toml'), 'utf8')
  const workspace = manifest.match(/\[workspace\.package\]([\s\S]*?)(?:\n\[|$)/)?.[1] || ''
  return workspace.match(/^version\s*=\s*"([^"]+)"/m)?.[1] || ''
}

export function configuredReleaseVersion(rootDirectory) {
  const packageManifest = JSON.parse(readFileSync(resolve(rootDirectory, 'package.json'), 'utf8'))
  const packageLock = JSON.parse(readFileSync(resolve(rootDirectory, 'package-lock.json'), 'utf8'))
  const cargoVersion = cargoWorkspaceVersion(rootDirectory)
  if (packageManifest.version !== cargoVersion
      || packageLock.version !== cargoVersion
      || packageLock.packages?.['']?.version !== cargoVersion) {
    throw new Error('package.json, package-lock.json, and Cargo.toml release versions do not match')
  }
  return cargoVersion
}

function runGit(arguments_, options = {}) {
  const result = spawnSync('git', arguments_, {
    cwd: root,
    encoding: 'utf8',
    stdio: options.capture ? ['ignore', 'pipe', 'inherit'] : 'inherit',
    windowsHide: true,
  })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`git exited with status ${result.status}`)
  return result.stdout?.trim() || ''
}

export function prepareReleaseVersion(rootDirectory, version, cargo = 'cargo') {
  const currentVersion = configuredReleaseVersion(rootDirectory)
  if (compareSemanticVersions(version, currentVersion) <= 0) {
    throw new Error(`Release version ${version} must be greater than current version ${currentVersion}`)
  }
  writeWorkspaceReleaseVersion({
    root: rootDirectory,
    cargo,
    baseVersion: currentVersion,
    candidateVersion: version,
  })
  return { currentVersion, version }
}

function main() {
  const options = parseReleaseVersionArguments(process.argv.slice(2))
  if (options.help) {
    console.log(usage())
    return
  }
  if (runGit(['status', '--porcelain', '--untracked-files=all'], { capture: true })) {
    throw new Error('Refusing to prepare a release version from a dirty worktree')
  }
  const result = prepareReleaseVersion(root, options.version)
  console.log(`Prepared release version ${result.currentVersion} -> ${result.version}`)
  console.log('Updated package.json, package-lock.json, Cargo.toml, and Cargo.lock.')
  console.log('Review and commit these version files before building the release.')
}

const currentFile = fileURLToPath(import.meta.url)
const invokedFile = process.argv[1] ? resolve(process.argv[1]) : ''
if (process.platform === 'win32'
  ? currentFile.toLowerCase() === invokedFile.toLowerCase()
  : currentFile === invokedFile) {
  try {
    main()
  } catch (error) {
    console.error(`error: ${error.message}`)
    process.exitCode = 1
  }
}
