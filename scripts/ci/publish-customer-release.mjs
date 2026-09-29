import { createHash } from 'node:crypto'
import { appendFileSync, createReadStream, mkdtempSync, readFileSync, readdirSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseReleaseVersionArguments } from '../prepare-release-version.mjs'
import { command } from '../release-github.mjs'

export const PUBLIC_RELEASE_REPOSITORY = 'huzz-open/aster-team'

export async function digest(path) {
  const hash = createHash('sha256')
  for await (const chunk of createReadStream(path)) hash.update(chunk)
  return hash.digest('hex')
}

export async function releaseAssets(directory, version) {
  const archives = ['linux', 'windows'].map(platform => `aster-team-${version}-${platform}-amd64.tar.gz`)
  const expected = archives.flatMap(name => [name, `${name}.sha256`]).sort()
  if (JSON.stringify(readdirSync(directory).sort()) !== JSON.stringify(expected)) {
    throw new Error('Publication requires exactly the Linux and Windows packages and their checksums')
  }
  const assets = await Promise.all(expected.map(async name => ({ name, path: resolve(directory, name),
    digest: await digest(resolve(directory, name)) })))
  for (const name of archives) {
    const checksum = readFileSync(resolve(directory, `${name}.sha256`), 'utf8').trim()
    const expectedDigest = assets.find(asset => asset.name === name).digest
    if (checksum !== `${expectedDigest}  ${name}`) throw new Error(`Checksum mismatch: ${name}`)
  }
  return assets
}

export async function publishRelease({ env = process.env, run = (exe, args) => command(process.cwd(), exe, args),
  log = console.log } = {}) {
  const version = parseReleaseVersionArguments(['--version', env.RELEASE_VERSION || '']).version
  const tag = env.RELEASE_TAG
  const sha = env.RELEASE_SOURCE_SHA
  const sameRepository = env.GITHUB_REPOSITORY === PUBLIC_RELEASE_REPOSITORY
  const tagPush = env.GITHUB_EVENT_NAME === 'push' && env.GITHUB_REF === `refs/tags/${tag}`
  const verifiedRecovery = env.GITHUB_EVENT_NAME === 'workflow_dispatch' && env.GITHUB_REF === 'refs/heads/main'
    && ['huzz-max/aster-team', PUBLIC_RELEASE_REPOSITORY].includes(env.GITHUB_REPOSITORY)
    && /^[1-9][0-9]*$/.test(env.RELEASE_VERIFIED_RUN_ID || '')
  if ((!tagPush && !verifiedRecovery) || tag !== `v${version}` || !/^[a-f0-9]{40}$/.test(sha || '')) {
    throw new Error('Only an exact version tag or a verified release-run recovery may publish a Customer release')
  }
  if (env.GH_REPO !== PUBLIC_RELEASE_REPOSITORY) throw new Error('Customer assets must publish to the public release repository')
  const assets = await releaseAssets(resolve(env.RELEASE_ASSETS || 'release-assets'), version)
  const gh = (...args) => run('gh', args)
  const verifySourceTag = () => {
    const refs = run('git', ['ls-remote', 'origin', `refs/tags/${tag}`, `refs/tags/${tag}^{}`])
      .split('\n').filter(Boolean).map(line => line.split(/\s+/))
    const current = refs.find(([, ref]) => ref.endsWith('^{}'))?.[0] || refs[0]?.[0]
    if (current !== sha) throw new Error('Remote release tag no longer matches the verified commit')
  }
  verifySourceTag()
  const repository = PUBLIC_RELEASE_REPOSITORY
  const api = path => JSON.parse(gh('api', `repos/${repository}${path ? `/${path}` : ''}`))
  const publicTags = () => api(`git/matching-refs/tags/${tag}`)
    .filter(ref => ref.ref === `refs/tags/${tag}`)
  const verifyPublicTag = target => {
    const refs = publicTags()
    if (refs.length !== 1 || refs[0].object.type !== 'commit' || refs[0].object.sha !== target) {
      throw new Error('Public release tag differs from its pinned public source commit')
    }
  }
  const marker = `<!-- aster-customer-source:${sha} -->`
  // List drafts too; a failed request is never interpreted as a missing release.
  const findRelease = () => JSON.parse(gh('api', `repos/${repository}/releases?per_page=100`, '--paginate', '--slurp'))
    .flat().find(item => item.tag_name === tag)
  let release = findRelease()
  if (release && !release.body?.includes(marker)) throw new Error('Existing release was not created for this verified source; refusing to modify it')
  let publicTarget = release?.body?.match(/<!-- aster-public-target:([a-f0-9]{40}) -->/)?.[1]
  if (release && !sameRepository && !publicTarget) throw new Error('Existing release lacks its public source commit; refusing to modify it')
  if (!release) {
    if (sameRepository) {
      gh('release', 'create', tag, '--repo', repository, '--verify-tag', '--draft', '--title', `Customer ${version}`,
        '--notes', `Source commit: ${sha}\n\n${marker}`,
        ...(version.split('+')[0].includes('-') ? ['--prerelease'] : []))
    } else {
      if (publicTags().length) throw new Error('Public release tag already exists without a matching release')
      const defaultBranch = api('').default_branch
      if (!/^[A-Za-z0-9._/-]+$/.test(defaultBranch || '')) throw new Error('Public repository has no valid default branch')
      publicTarget = api(`git/ref/heads/${defaultBranch}`).object.sha
      if (!/^[a-f0-9]{40}$/.test(publicTarget || '')) throw new Error('Public default branch has no valid commit')
      const targetMarker = `<!-- aster-public-target:${publicTarget} -->`
      gh('release', 'create', tag, '--repo', repository, '--target', publicTarget, '--draft', '--title', `Customer ${version}`,
        '--notes', `Private source commit: ${sha}\n\n${marker}\n${targetMarker}`,
        ...(version.split('+')[0].includes('-') ? ['--prerelease'] : []))
    }
    // A newly created draft may not appear in the list API immediately.
    for (let attempt = 0; attempt < 5 && !release; attempt++) {
      release = findRelease()
      if (!release && attempt < 4) await new Promise(resolve => setTimeout(resolve, 1000))
    }
    if (!release?.draft || !release.body?.includes(marker)
      || (!sameRepository && !release.body?.includes(`<!-- aster-public-target:${publicTarget} -->`))) {
      throw new Error('Could not confirm the release draft')
    }
  }
  // GitHub does not materialize the tag when creating an unpublished draft.
  // Pin the public tag to the recorded public source commit before uploading.
  if (!sameRepository && release.draft && publicTags().length === 0) {
    gh('api', '--method', 'POST', `repos/${repository}/git/refs`,
      '-f', `ref=refs/tags/${tag}`, '-f', `sha=${publicTarget}`)
  }
  if (!sameRepository) verifyPublicTag(publicTarget)
  if (!release.draft) {
    const names = (release.assets || []).map(asset => asset.name).sort()
    if (JSON.stringify(names) !== JSON.stringify(assets.map(asset => asset.name))) throw new Error('Published asset set differs; published releases are never overwritten')
    const directory = mkdtempSync(join(tmpdir(), 'aster-published-release-'))
    try {
      for (const asset of assets) {
        gh('release', 'download', tag, '--repo', repository, '--pattern', asset.name, '--dir', directory)
        if (await digest(join(directory, asset.name)) !== asset.digest) {
          throw new Error(`Published asset differs: ${asset.name}; published releases are never overwritten`)
        }
      }
    } finally {
      rmSync(directory, { recursive: true, force: true })
    }
    log(`Release already published with identical assets: ${release.html_url}`)
    return { existing: true, url: release.html_url }
  }
  if ((release.assets || []).some(asset => !assets.some(local => local.name === asset.name))) {
    throw new Error('Draft contains unexpected assets; review it before retrying')
  }
  // Only our unpublished draft may replace partial assets after a failed upload.
  gh('release', 'upload', tag, '--repo', repository, ...assets.map(asset => asset.path), '--clobber')
  const uploaded = api(`releases/${release.id}`)
  if (!uploaded.draft || uploaded.assets.length !== assets.length
    || assets.some(asset => !uploaded.assets.some(remote => remote.name === asset.name
      && remote.state === 'uploaded' && remote.digest === `sha256:${asset.digest}`))) {
    throw new Error('Draft upload is incomplete or its remote checksums differ; leaving it unpublished')
  }
  verifySourceTag()
  if (!sameRepository) verifyPublicTag(publicTarget)
  gh('release', 'edit', tag, '--repo', repository, '--draft=false', '--verify-tag',
    ...(version.split('+')[0].includes('-') ? ['--prerelease', '--latest=false'] : []))
  log(`Published Customer ${version}: ${uploaded.html_url}`)
  if (env.GITHUB_STEP_SUMMARY) appendFileSync(env.GITHUB_STEP_SUMMARY,
    `\nCustomer ${version}: ${uploaded.html_url}\n\nSource: ${sha}\n`)
  return { existing: false, url: uploaded.html_url }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { await publishRelease() } catch (error) { console.error(error.message); process.exitCode = 1 }
}
