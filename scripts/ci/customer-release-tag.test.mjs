import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { copyFileSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { releaseIdentity, verifyCargoLock } from './customer-release-source.mjs'
import { publishRelease, releaseAssets } from './publish-customer-release.mjs'
import { verifyReleaseRun } from './verify-customer-release-run.mjs'

const sha = 'a'.repeat(40)
test('tag and dispatch resolve one version and source; malformed tags and changed source fail', () => {
  assert.deepEqual(releaseIdentity({ event: 'push', ref: 'refs/tags/v2.0.3-rc.1' }, sha), { version: '2.0.3-rc.1', sha, tag: 'v2.0.3-rc.1' })
  assert.deepEqual(releaseIdentity({ event: 'workflow_dispatch', version: 'v2.0.2', sourceSHA: sha }, sha), { version: '2.0.2', sha, tag: '' })
  for (const ref of ['refs/heads/main', 'refs/tags/operations-v2.0.2', 'refs/tags/vv2.0.2', 'refs/tags/v2.0.2-invalid;command']) {
    assert.throws(() => releaseIdentity({ event: 'push', ref }, sha))
  }
  assert.throws(() => releaseIdentity({ event: 'workflow_dispatch', version: '2.0.2', sourceSHA: 'b'.repeat(40) }, sha), /differs/)
})

test('all workspace Cargo.lock versions must agree, external package versions may differ', () => {
  const metadata = { workspace_members: ['local'], packages: [{ id: 'local', name: 'aster', version: '2.0.2' }] }
  const lock = '[[package]]\nname = "aster"\nversion = "2.0.2"\n\n[[package]]\nname = "external"\nversion = "1.0.0"\n'
  verifyCargoLock(lock, metadata, '2.0.2')
  assert.throws(() => verifyCargoLock(lock.replace('2.0.2', '2.0.1'), metadata, '2.0.2'), /Cargo.lock/)
  assert.throws(() => verifyCargoLock('', metadata, '2.0.2'), /Cargo.lock/)
})

function publication(t, { version = '2.0.2', existing = false, published = false, complete = false, failedUpload = false,
  corruptRemote = false, movedTag = false, existingPublicTag = false, movedPublicTag = false,
  missingPublicTag = false, delayedDraft = false } = {}) {
  const directory = mkdtempSync(join(tmpdir(), 'aster-release-test-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  for (const platform of ['linux', 'windows']) {
    const name = `aster-team-${version}-${platform}-amd64.tar.gz`
    writeFileSync(join(directory, name), platform)
    const hash = createHash('sha256').update(platform).digest('hex')
    writeFileSync(join(directory, `${name}.sha256`), `${hash}  ${name}\n`)
  }
  const calls = []
  const publicTarget = 'c'.repeat(40)
  let publicTag = (existing && !missingPublicTag) || existingPublicTag ? publicTarget : null
  let release = existing ? { id: 1, tag_name: `v${version}`, draft: !published,
    body: `<!-- aster-customer-source:${sha} -->\n<!-- aster-public-target:${publicTarget} -->`,
    assets: [], html_url: 'https://example.test/release' } : null
  if (release && complete) release.assets = readdirSync(directory).map(name => ({ name }))
  let hiddenDraftReads = 0
  const run = (exe, args) => {
    calls.push([exe, ...args])
    if (exe === 'git') return `${movedTag ? 'b'.repeat(40) : sha}\trefs/tags/v${version}^{}`
    if (args[0] === 'api') {
      if (args[1] === '--method') {
        assert.equal(args[2], 'POST')
        assert.equal(args[3], 'repos/huzz-open/aster-team/git/refs')
        assert.ok(release?.draft)
        assert.equal(publicTag, null)
        assert.ok(args.includes(`ref=refs/tags/v${version}`))
        assert.ok(args.includes(`sha=${publicTarget}`))
        publicTag = publicTarget
        return JSON.stringify({ ref: `refs/tags/v${version}`, object: { type: 'commit', sha: publicTag } })
      }
      const path = args[1]
      assert.match(path, /^repos\/huzz-open\/aster-team(?:\/|$)/)
      if (path.includes('/git/matching-refs/tags/')) return JSON.stringify(publicTag ? [{ ref: `refs/tags/v${version}`,
        object: { type: 'commit', sha: movedPublicTag ? 'd'.repeat(40) : publicTag } }] : [])
      if (path === 'repos/huzz-open/aster-team') return JSON.stringify({ default_branch: 'main' })
      if (path.endsWith('/git/ref/heads/main')) return JSON.stringify({ object: { sha: publicTarget } })
      if (path.endsWith('/releases?per_page=100')) {
        if (delayedDraft && release?.draft && hiddenDraftReads++ === 0) return JSON.stringify([[]])
        return JSON.stringify([release ? [release] : []])
      }
      if (path.endsWith('/releases/1')) return JSON.stringify(release)
      throw new Error(`Unexpected API ${path}`)
    }
    if (args[1] === 'create') {
      release = { id: 1, tag_name: `v${version}`, draft: true, body: args[args.indexOf('--notes') + 1], assets: [], html_url: 'https://example.test/release' }
      assert.equal(args[args.indexOf('--target') + 1], publicTarget)
    } else if (args[1] === 'upload') {
      if (failedUpload) throw new Error('upload interrupted')
      release.assets = args.slice(args.indexOf('--repo') + 2, args.indexOf('--clobber')).map(path => ({ name: path.split(/[\\/]/).at(-1), state: 'uploaded',
        digest: `sha256:${corruptRemote ? '0'.repeat(64) : createHash('sha256').update(readFileSync(path)).digest('hex')}` }))
    } else if (args[1] === 'download') {
      const name = args[args.indexOf('--pattern') + 1]
      copyFileSync(join(directory, name), join(args[args.indexOf('--dir') + 1], name))
    } else if (args[1] !== 'edit') throw new Error(`Unexpected command ${args}`)
    return ''
  }
  return { directory, calls, execute: () => publishRelease({ run, log: () => {}, env: { GITHUB_EVENT_NAME: 'push',
    GITHUB_REF: `refs/tags/v${version}`, RELEASE_TAG: `v${version}`, RELEASE_VERSION: version,
    RELEASE_SOURCE_SHA: sha, RELEASE_ASSETS: directory, GH_REPO: 'huzz-open/aster-team' } }) }
}

test('both platform assets are required and archive corruption stops publication', async t => {
  const scenario = publication(t)
  assert.equal((await releaseAssets(scenario.directory, '2.0.2')).length, 4)
  writeFileSync(join(scenario.directory, 'aster-team-2.0.2-linux-amd64.tar.gz'), 'corrupt')
  await assert.rejects(scenario.execute, /Checksum mismatch/)
  assert.equal(scenario.calls.length, 0)
})

test('release is drafted, uploaded and published in order; rc remains prerelease', async t => {
  const scenario = publication(t, { version: '2.0.3-rc.1' })
  await scenario.execute()
  const mutations = scenario.calls.filter(call => call[1] === 'release')
  assert.deepEqual(mutations.map(call => call[2]), ['create', 'upload', 'edit'])
  assert.ok(mutations[0].includes('--draft'))
  assert.ok(mutations[0].includes('--prerelease'))
  assert.ok(mutations[0].includes('--repo'))
  assert.ok(mutations[0].includes('huzz-open/aster-team'))
  assert.equal(mutations[0][mutations[0].indexOf('--target') + 1], 'c'.repeat(40))
  assert.ok(scenario.calls.some(call => call[1] === 'api' && call[2] === '--method' && call[3] === 'POST'))
  assert.ok(mutations[2].includes('--latest=false'))
})

test('public source release reuses its annotated tag and repository token', async t => {
  const fixture = publication(t)
  const calls = []
  let release = null
  const run = (exe, args) => {
    calls.push([exe, ...args])
    if (exe === 'git') return `${sha}\trefs/tags/v2.0.2^{}`
    if (args[0] === 'api' && args[1].endsWith('/releases?per_page=100')) {
      return JSON.stringify([release ? [release] : []])
    }
    if (args[0] === 'api' && args[1].endsWith('/releases/1')) return JSON.stringify(release)
    if (args[0] === 'api') throw new Error(`Unexpected API request: ${args[1]}`)
    if (args[1] === 'create') {
      assert.ok(args.includes('--verify-tag'))
      assert.ok(!args.includes('--target'))
      release = { id: 1, tag_name: 'v2.0.2', draft: true, body: args[args.indexOf('--notes') + 1],
        assets: [], html_url: 'https://example.test/release' }
    } else if (args[1] === 'upload') {
      release.assets = args.slice(args.indexOf('--repo') + 2, args.indexOf('--clobber'))
        .map(path => ({ name: path.split(/[\\/]/).at(-1), state: 'uploaded',
          digest: `sha256:${createHash('sha256').update(readFileSync(path)).digest('hex')}` }))
    } else if (args[1] !== 'edit') throw new Error(`Unexpected command: ${args}`)
    return ''
  }
  await publishRelease({ run, log: () => {}, env: { GITHUB_EVENT_NAME: 'push',
    GITHUB_REPOSITORY: 'huzz-open/aster-team', GITHUB_REF: 'refs/tags/v2.0.2',
    RELEASE_TAG: 'v2.0.2', RELEASE_VERSION: '2.0.2', RELEASE_SOURCE_SHA: sha,
    RELEASE_ASSETS: fixture.directory, GH_REPO: 'huzz-open/aster-team' } })
  assert.deepEqual(calls.filter(call => call[1] === 'release').map(call => call[2]), ['create', 'upload', 'edit'])
  assert.ok(!calls.some(call => call[1] === 'api' && call[2] === '--method'))
})

test('an existing draft without a public tag is pinned before continuing publication', async t => {
  const scenario = publication(t, { existing: true, missingPublicTag: true })
  await scenario.execute()
  assert.ok(!scenario.calls.some(call => call[2] === 'create'))
  assert.ok(scenario.calls.some(call => call[1] === 'api' && call[2] === '--method' && call[3] === 'POST'))
  assert.ok(scenario.calls.some(call => call[2] === 'edit'))
})

test('new draft listing may lag without causing duplicate release creation', async t => {
  const scenario = publication(t, { delayedDraft: true })
  await scenario.execute()
  assert.equal(scenario.calls.filter(call => call[2] === 'create').length, 1)
})

test('partial upload and wrong remote digest leave the release unpublished', async t => {
  for (const options of [{ failedUpload: true }, { corruptRemote: true }]) {
    const scenario = publication(t, options)
    await assert.rejects(scenario.execute)
    assert.ok(!scenario.calls.some(call => call[2] === 'edit'))
  }
})

test('a draft resumes without recreating it; published incomplete assets are never overwritten', async t => {
  const draft = publication(t, { existing: true })
  await draft.execute()
  assert.ok(!draft.calls.some(call => call[2] === 'create'))
  const published = publication(t, { existing: true, published: true })
  await assert.rejects(published.execute, /never overwritten/)
  assert.ok(!published.calls.some(call => ['create', 'upload', 'edit'].includes(call[2])))
})

test('published identical assets return success without writes; moved tags cannot publish', async t => {
  const published = publication(t, { existing: true, published: true, complete: true })
  assert.equal((await published.execute()).existing, true)
  assert.ok(!published.calls.some(call => ['create', 'upload', 'edit'].includes(call[2])))
  const moved = publication(t, { movedTag: true })
  await assert.rejects(moved.execute, /no longer matches/)
  assert.ok(!moved.calls.some(call => call[0] === 'gh'))
})

test('existing public tag without our release and moved public tag stop publication', async t => {
  const occupied = publication(t, { existingPublicTag: true })
  await assert.rejects(occupied.execute, /already exists/)
  assert.ok(!occupied.calls.some(call => call[1] === 'release'))
  const moved = publication(t, { existing: true, movedPublicTag: true })
  await assert.rejects(moved.execute, /Public release tag differs/)
  assert.ok(!moved.calls.some(call => call[1] === 'release'))
})

test('publishing to the private repository is rejected before any GitHub call', async t => {
  const scenario = publication(t)
  await assert.rejects(() => publishRelease({ run: (exe, args) => {
    scenario.calls.push([exe, ...args])
    return ''
  }, env: { GITHUB_EVENT_NAME: 'push', GITHUB_REF: 'refs/tags/v2.0.2', RELEASE_TAG: 'v2.0.2',
    RELEASE_VERSION: '2.0.2', RELEASE_SOURCE_SHA: sha, RELEASE_ASSETS: scenario.directory,
    GH_REPO: 'huzz-max/aster-team' } }), /public release repository/)
  assert.equal(scenario.calls.length, 0)
})

test('verified recovery accepts only the immutable tag run with a passing gate, matrices and both packages', async () => {
  const tag = 'v2.1.1'
  const version = '2.1.1'
  const runId = '35150152762'
  const names = ['Audit release dependencies', 'Build downloadable asterctl (Windows x64)',
    'Build and verify Windows amd64 package', 'Build signed Linux amd64 package',
    'Verify Linux primary (ubuntu-20.04)', 'Require Linux verification and Windows package build',
    ...['rocky-linux-9', 'ubuntu-22.04', 'debian-13', 'debian-12', 'ubuntu-24.04']
      .flatMap(distro => [`Verify customer install (${distro})`, `Verify dedicated Runner install (${distro})`])]
  const source = { id: Number(runId), event: 'push', head_branch: tag, head_sha: sha,
    path: '.github/workflows/customer-release.yml', status: 'completed', conclusion: 'success' }
  const jobs = { total_count: names.length + 1, jobs: [...names.map(name => ({ name, conclusion: 'success' })),
    { name: 'Verify dedicated Runner install (windows-2025)', conclusion: 'skipped' }] }
  const artifacts = { artifacts: ['linux', 'windows'].map(platform => ({ name: `customer-${platform}-amd64-${version}`,
    expired: false, workflow_run: { head_sha: sha } })) }
  const calls = []
  const run = (exe, args) => {
    calls.push([exe, ...args])
    if (exe === 'git') return args[0] === 'ls-remote' ? `${sha}\trefs/tags/${tag}^{}` : ''
    if (args[1].endsWith(`/runs/${runId}`)) return JSON.stringify(source)
    if (args[1].includes('/jobs?')) return JSON.stringify(jobs)
    if (args[1].includes('/artifacts?')) return JSON.stringify(artifacts)
    throw new Error(`Unexpected command: ${args}`)
  }
  const env = { GITHUB_EVENT_NAME: 'workflow_dispatch', GITHUB_REF: 'refs/heads/main',
    GITHUB_REPOSITORY: 'huzz-open/aster-team', RELEASE_TAG: tag, SOURCE_RUN_ID: runId }
  assert.deepEqual(verifyReleaseRun({ env, run }), { tag, version, sha, runId })
  assert.ok(calls.some(call => call[0] === 'git' && call[1] === 'merge-base'))
  assert.throws(() => verifyReleaseRun({ env: { ...env, GITHUB_REF: 'refs/heads/feature' }, run }), /Recovery requires/)
  assert.throws(() => verifyReleaseRun({ env, run: (exe, args) => args[1]?.endsWith(`/runs/${runId}`)
    ? JSON.stringify({ ...source, head_sha: 'b'.repeat(40) }) : run(exe, args) }), /exact immutable tag/)
  assert.throws(() => verifyReleaseRun({ env, run: (exe, args) => args[1]?.includes('/jobs?')
    ? JSON.stringify({ ...jobs, jobs: jobs.jobs.map(job => job.name === 'Require Linux verification and Windows package build'
      ? { ...job, conclusion: 'skipped' } : job) }) : run(exe, args) }), /release gate/)
  assert.throws(() => verifyReleaseRun({ env, run: (exe, args) => args[1]?.includes('/artifacts?')
    ? JSON.stringify({ artifacts: artifacts.artifacts.slice(0, 1) }) : run(exe, args) }), /unavailable/)
})

test('manual publication requires a verified recovery run and never accepts an arbitrary dispatch', async t => {
  const scenario = publication(t, { version: '2.1.1' })
  const base = { GITHUB_EVENT_NAME: 'workflow_dispatch', GITHUB_REF: 'refs/heads/main',
    GITHUB_REPOSITORY: 'huzz-max/aster-team', RELEASE_TAG: 'v2.1.1', RELEASE_VERSION: '2.1.1',
    RELEASE_SOURCE_SHA: sha, RELEASE_ASSETS: scenario.directory, GH_REPO: 'huzz-open/aster-team' }
  await assert.rejects(() => publishRelease({ env: base, run: () => { throw new Error('Unexpected call') } }), /verified release-run recovery/)
  await publishRelease({ env: { ...base, RELEASE_VERIFIED_RUN_ID: '35150152762' }, run: (exe, args) => {
    if (exe === 'git') return `${sha}\trefs/tags/v2.1.1^{}`
    scenario.calls.push([exe, ...args])
    if (args[0] === 'api' && args[1].includes('/releases?')) return JSON.stringify([[]])
    if (args[0] === 'api' && args[1].includes('/matching-refs/')) return '[]'
    if (args[0] === 'api' && args[1] === 'repos/huzz-open/aster-team') return JSON.stringify({ default_branch: 'main' })
    if (args[0] === 'api' && args[1].endsWith('/git/ref/heads/main')) return JSON.stringify({ object: { sha: 'c'.repeat(40) } })
    throw new Error('Stop after verifying the recovery reaches publication')
  } }).then(() => assert.fail('Expected stopped mock'), error => assert.match(error.message, /Stop after verifying/))
  assert.ok(scenario.calls.length > 0)
})

test('optional skipped Windows checks cannot silently skip tag publication and cleanup', () => {
  const workflow = readFileSync(new URL('../../.github/workflows/customer-release.yml', import.meta.url), 'utf8')
  assert.match(workflow, /publish:\s+name: Publish Customer package\s+#.*\s+#.*\s+if: \$\{\{ always\(\).*needs\['release-gate'\]\.result == 'success'/)
  assert.match(workflow, /cleanup-published-artifacts:[\s\S]*?if: \$\{\{ always\(\).*needs\.publish\.result == 'success'/)
  const recovery = readFileSync(new URL('../../.github/workflows/customer-release-publish.yml', import.meta.url), 'utf8')
  assert.match(recovery, /verify-customer-release-run\.mjs/)
  assert.match(recovery, /npm run release:publish/)
})
