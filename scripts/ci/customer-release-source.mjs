import { appendFileSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { configuredReleaseVersion, parseReleaseVersionArguments } from '../prepare-release-version.mjs'
import { command } from '../release-github.mjs'

export function releaseIdentity({ event, ref, version, sourceSHA }, head) {
  const isTag = event === 'push' && ref?.startsWith('refs/tags/v')
  if (!isTag && event !== 'workflow_dispatch') throw new Error('Unsupported Customer release event')
  const requested = isTag ? ref.slice('refs/tags/v'.length) : version
  const normalized = parseReleaseVersionArguments(['--version', requested || '']).version
  if (isTag && requested !== normalized) throw new Error('Tag must be v followed by a SemVer version')
  if (!/^[a-f0-9]{40}$/.test(head)) throw new Error('Invalid release source SHA')
  if (!isTag && sourceSHA && sourceSHA !== head) throw new Error('Checkout differs from requested source SHA')
  return { version: normalized, sha: head, tag: isTag ? `v${normalized}` : '' }
}

export function verifyCargoLock(lock, metadata, version) {
  const ids = new Set(metadata.workspace_members)
  const packages = metadata.packages.filter(pkg => ids.has(pkg.id))
  if (!packages.length) throw new Error('Cargo workspace is empty')
  const blocks = lock.split('[[package]]').slice(1)
  for (const pkg of packages) {
    const blocksForPackage = blocks.filter(block => /^name = "([^"]+)"\r?$/m.exec(block)?.[1] === pkg.name
      && !/^source = /m.test(block))
    if (pkg.version !== version || blocksForPackage.length !== 1
      || /^version = "([^"]+)"\r?$/m.exec(blocksForPackage[0])?.[1] !== version) {
      throw new Error(`Cargo.lock workspace package ${pkg.name} must use ${version}`)
    }
  }
}

export function resolveReleaseSource(root = process.cwd(), env = process.env) {
  const git = (...args) => command(root, 'git', args)
  const identity = releaseIdentity({ event: env.GITHUB_EVENT_NAME, ref: env.GITHUB_REF,
    version: env.DISPATCH_VERSION, sourceSHA: env.RELEASE_SOURCE_SHA }, git('rev-parse', 'HEAD'))
  git('merge-base', '--is-ancestor', identity.sha, 'origin/main')
  if (identity.tag && git('rev-parse', `${identity.tag}^{commit}`) !== identity.sha) {
    throw new Error('Checkout differs from release tag')
  }
  if (configuredReleaseVersion(root) !== identity.version) throw new Error('Release version differs from the committed manifests')
  const metadata = JSON.parse(command(root, 'cargo', ['metadata', '--locked', '--no-deps', '--format-version', '1']))
  verifyCargoLock(readFileSync(resolve(root, 'Cargo.lock'), 'utf8'), metadata, identity.version)
  if (env.GITHUB_OUTPUT) appendFileSync(env.GITHUB_OUTPUT, Object.entries(identity).map(([key, value]) => `${key}=${value}\n`).join(''))
  console.log(`Customer ${identity.version} at ${identity.sha}${identity.tag ? ` (${identity.tag})` : ''}`)
  return identity
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { resolveReleaseSource() } catch (error) { console.error(error.message); process.exitCode = 1 }
}
