#!/usr/bin/env node

import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  configuredReleaseVersion,
  parseReleaseVersionArguments,
  prepareReleaseVersion,
} from './prepare-release-version.mjs'
import {
  inspectBundledFreeLicense,
  loadLocalReleaseSecurityFiles,
} from './release-security-files.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const workflow = 'customer-release.yml'
const versionFiles = ['package.json', 'package-lock.json', 'Cargo.toml', 'Cargo.lock']

export function usage() {
  return `Usage:
  npm run release:github -- --version VERSION
  npm run release:github -- --version VERSION --dispatch-only

From a clean, synchronized main checkout: prepare the version, commit the four
version files, push main, and dispatch customer-release.yml at that exact SHA.
Requires Git, Cargo and an authenticated GitHub CLI with repository write access.
No local lint, test, build or verification suites are run; existing GitHub Actions
checks remain in effect. Git hooks and remote branch protection remain in effect.
Manual dispatches bundle the signed free-license.json from the configured external
release security directory. Tag builds use the equivalent protected Environment
variables and verify the same SHA-256 before building.

--dispatch-only synchronizes main and uses its committed version without a version
edit or new commit. Existing runs for that version and SHA are returned instead
of dispatching again. GitHub runs do not create Operations release task records.`
}

export function parseArguments(args) {
  if (args.includes('--help') || args.includes('-h')) return { help: true }
  const dispatchOnly = args.includes('--dispatch-only')
  const { version } = parseReleaseVersionArguments(args.filter(arg => arg !== '--dispatch-only'))
  return { help: false, version, dispatchOnly }
}

export function githubRepository(remote) {
  const scp = /^git@([^:]+):(.+)$/.exec(remote)
  let host
  let path
  if (scp) {
    host = scp[1]
    path = scp[2]
  } else {
    let url
    try { url = new URL(remote) } catch { throw new Error('origin must be a GitHub HTTPS or SSH remote') }
    if (!['https:', 'ssh:'].includes(url.protocol) || url.password || url.search || url.hash
      || (url.protocol === 'https:' && url.username)) {
      throw new Error('origin must be a credential-free GitHub HTTPS or SSH remote')
    }
    host = url.host
    path = url.pathname.replace(/^\//, '')
  }
  path = path.replace(/\.git$/, '')
  if (!/^[A-Za-z0-9.-]+$/.test(host) || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(path)) {
    throw new Error('origin must identify one GitHub repository')
  }
  return { host, path, qualified: `${host}/${path}` }
}

export function command(rootDirectory, executable, args) {
  const result = spawnSync(process.platform === 'win32' ? `${executable}.exe` : executable, args, {
    cwd: rootDirectory,
    encoding: 'utf8',
    maxBuffer: 16 * 1024 * 1024,
    stdio: ['ignore', 'pipe', 'inherit'],
    windowsHide: true,
  })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`${executable} ${args[0]} failed (exit ${result.status})`)
  return result.stdout.trim()
}

export function loadWorkflowFreeLicense(rootDirectory) {
  const security = loadLocalReleaseSecurityFiles(rootDirectory)
  const document = readFileSync(security.freeLicenseFile)
  if (document.length < 1 || document.length > 32 * 1024) {
    throw new Error('The workflow free license must contain between 1 byte and 32 KiB')
  }
  const { distributionID } = inspectBundledFreeLicense(document, 32 * 1024)
  return {
    distributionID,
    sha256: createHash('sha256').update(document).digest('hex'),
    base64: document.toString('base64'),
  }
}

export function releaseGithub({
  rootDirectory = root,
  options,
  run = (executable, args) => command(rootDirectory, executable, args),
  prepare = version => prepareReleaseVersion(rootDirectory, version),
  currentVersion = () => configuredReleaseVersion(rootDirectory),
  freeLicense = () => loadWorkflowFreeLicense(rootDirectory),
  log = console.log,
}) {
  const git = (...args) => run('git', args)
  const entry = options.tag ? 'release:tag' : 'release:github'
  const retry = `npm run ${entry} -- --version ${options.version} ${options.tag ? '--tag-only' : '--dispatch-only'}`
  const tag = `v${options.version}`
  const clean = () => {
    if (git('status', '--porcelain', '--untracked-files=all')) {
      throw new Error('A clean worktree is required; commit or preserve existing changes first')
    }
  }
  if (git('branch', '--show-current') !== 'main') throw new Error(`Run ${entry} from main`)
  clean()
  const repository = githubRepository(git('remote', 'get-url', 'origin'))
  const gh = (...args) => run('gh', args)
  // Fail before editing files if GitHub authentication or the workflow is unavailable.
  gh('api', 'user', '--hostname', repository.host, '--jq', '.login')
  const workflowState = gh('api', `repos/${repository.path}/actions/workflows/${workflow}`,
    '--hostname', repository.host, '--jq', '.state')
  if (workflowState !== 'active') throw new Error(`${workflow} is not active`)
  for (const name of ['ASTER_CUSTOMER_FREE_LICENSE_BASE64', 'ASTER_CUSTOMER_FREE_LICENSE_SHA256']) {
    gh('api', `repos/${repository.path}/environments/customer-release/variables/${name}`,
      '--hostname', repository.host, '--jq', '.name')
  }
  const bundledFreeLicense = freeLicense()
  git('fetch', 'origin', '--prune')
  git('merge', '--ff-only', 'origin/main')
  if (git('rev-parse', 'HEAD') !== git('rev-parse', 'origin/main')) {
    throw new Error(`Local main has unpushed commits; push them explicitly before running ${entry}`)
  }
  clean()

  if (options.tag && !options.dispatchOnly) {
    if (git('tag', '--list', tag) || git('ls-remote', 'origin', `refs/tags/${tag}`)) {
      throw new Error(`${tag} already exists; use ${retry} for the committed version, or rerun its existing Actions run`)
    }
  }

  if (options.dispatchOnly) {
    if (currentVersion() !== options.version) throw new Error('--version must match the committed version for recovery')
  } else {
    prepare(options.version)
    const changed = git('diff', '--name-only', '-z').split('\0').filter(Boolean)
    if (changed.length !== versionFiles.length || changed.some(path => !versionFiles.includes(path))) {
      throw new Error('Version preparation changed unexpected files; review the worktree before continuing')
    }
    git('add', '--', ...versionFiles)
    log(git('commit', '-m', `chore(release): prepare ${options.version}`))
    clean()
    try {
      git('push', 'origin', 'main')
    } catch (error) {
      throw new Error(`${error.message}\nThe version commit is retained locally. After resolving the push failure, run git push origin main, then ${retry}`)
    }
  }

  const sha = git('rev-parse', 'HEAD')
  if (!/^[0-9a-f]{40}$/.test(sha)) throw new Error('Could not resolve the release commit')
  // Detect a concurrent push before dispatching; never silently select a newer main.
  const remote = git('ls-remote', 'origin', 'refs/heads/main').split(/\s+/)[0]
  if (remote !== sha) throw new Error('origin/main advanced; synchronize and review the source before dispatching')
  if (currentVersion() !== options.version) throw new Error('The committed version does not match the requested release')
  if (options.tag) {
    const refs = git('ls-remote', 'origin', `refs/tags/${tag}`, `refs/tags/${tag}^{}`)
      .split('\n').filter(Boolean).map(line => line.split(/\s+/))
    const remoteTag = refs.find(([, ref]) => ref.endsWith('^{}'))?.[0] || refs[0]?.[0]
    if (remoteTag) {
      if (remoteTag !== sha) throw new Error(`${tag} already points to a different commit; tags are never moved`)
      log(`${tag} already exists at ${sha}; rerun the existing Actions run if needed`)
      return { sha, existing: true, tag }
    }
    if (git('tag', '--list', tag)) {
      if (git('rev-parse', `${tag}^{commit}`) !== sha) throw new Error(`Local ${tag} points to a different commit; tags are never moved`)
    } else {
      git('tag', '-a', tag, sha, '-m', `Customer ${options.version}`,
        '-m', `Free-License-SHA256: ${bundledFreeLicense.sha256}`,
        '-m', `Free-Distribution-ID: ${bundledFreeLicense.distributionID}`)
    }
    try {
      git('push', 'origin', `refs/tags/${tag}:refs/tags/${tag}`)
    } catch (error) {
      throw new Error(`${error.message}\nThe version and local tag are retained. Inspect the remote first, then recover with ${retry}`)
    }
    log(`Pushed ${tag} at ${sha}; its push triggers Customer build and GitHub Release publication`)
    log(`https://${repository.host}/${repository.path}/actions/workflows/${workflow}`)
    return { sha, existing: false, tag }
  }
  try {
    const runs = JSON.parse(gh('run', 'list', '--repo', repository.qualified, '--workflow', workflow,
      '--branch', 'main', '--commit', sha, '--event', 'workflow_dispatch', '--limit', '100',
      '--json', 'databaseId,displayTitle,headSha,status,conclusion,url'))
    const expectedTitle = `Customer ${options.version} · manual-${options.version}-${sha.slice(0, 12)} · ${sha} · ${bundledFreeLicense.sha256}`
    const existing = runs.find(run => run.headSha === sha && run.displayTitle === expectedTitle)
    if (existing) {
      log(`Existing release run (${existing.conclusion || existing.status}): ${existing.url}`)
      if (['failure', 'cancelled', 'timed_out'].includes(existing.conclusion)) {
        log(`To retry failed jobs, use gh run rerun ${existing.databaseId} --failed --repo ${repository.qualified}`)
      }
      return { sha, existing: true, url: existing.url }
    }
    const output = gh('workflow', 'run', workflow, '--repo', repository.qualified, '--ref', 'main',
      '-f', `version=${options.version}`, '-f', `source_commit_sha=${sha}`,
      '-f', `release_task_id=manual-${options.version}-${sha.slice(0, 12)}`,
      '-f', `free_distribution_id=${bundledFreeLicense.distributionID}`,
      '-f', `free_license_sha256=${bundledFreeLicense.sha256}`,
      '-f', `free_license_base64=${bundledFreeLicense.base64}`)
    log(`Dispatched Customer ${options.version} from main at ${sha}`)
    if (output) log(output)
    return { sha, existing: false, output }
  } catch (error) {
    throw new Error(`${error.message}\nThe version is already on main. Inspect GitHub Actions first because a failed response may still have created a run; retry with ${retry}`)
  }
}

const invoked = process.argv[1] ? resolve(process.argv[1]) : ''
const current = fileURLToPath(import.meta.url)
if (process.platform === 'win32' ? invoked.toLowerCase() === current.toLowerCase() : invoked === current) {
  try {
    const options = parseArguments(process.argv.slice(2))
    if (options.help) console.log(usage())
    else releaseGithub({ options })
  } catch (error) {
    console.error(`error: ${error.message}`)
    process.exitCode = 1
  }
}
