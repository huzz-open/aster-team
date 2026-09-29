import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'
import { parse } from 'yaml'
import { cacheFamily, manageCaches, planCacheCleanup } from './manage-actions-cache.mjs'
import { npmCachePath } from './npm-cache-path.mjs'

const yaml = path => parse(readFileSync(new URL(`../../${path}`, import.meta.url), 'utf8'), { uniqueKeys: true })
const cache = (id, key = 'v0-rust-build-Linux-x64-compiler-lock', extra = {}) => ({
  id, key, ref: 'refs/heads/main', size_in_bytes: 1_000_000_000,
  created_at: new Date(Date.UTC(2026, 0, id)).toISOString(), ...extra,
})

test('cache families keep release platforms and validation jobs separate', () => {
  assert.notEqual(cacheFamily('v0-rust-build-Linux-x64-a-b'), cacheFamily('v0-rust-customer-Linux-x64-a-b'))
  assert.notEqual(cacheFamily('v0-rust-windows-install-Windows_NT-x64-static-a-b'), cacheFamily('v0-rust-asterctl-windows-Windows_NT-x64-static-a-b'))
  assert.equal(cacheFamily('unknown-user-cache'), null)
  assert.equal(cacheFamily('v0-rust-windows-msvc-static-asterctl-windows-Windows_NT-x64-a-b'), cacheFamily('v0-rust-asterctl-windows-Windows_NT-x64-a-b'))
  assert.equal(cacheFamily(`v0-rust-windows-msvc-static-${'a'.repeat(64)}-windows-install-Windows_NT-x64-a-b`), 'v0-rust-windows-install-windows_nt-x64')
})

test('latest plus one fallback retained; budget pressure removes only fallback', () => {
  const caches = [cache(1), cache(2), cache(3)]
  assert.deepEqual(planCacheCleanup(caches).deletions.map(item => item.id), [1])
  const plan = planCacheCleanup(caches, new Set(), 0)
  assert.deepEqual(plan.deletions.map(item => item.id), [1, 2])
  assert.equal(plan.afterBytes, 1_000_000_000)
  assert.equal(plan.budgetMet, false)
})

test('only confirmed closed PR caches removed; unknown and open refs protected', () => {
  const caches = [
    cache(1, undefined, { ref: 'refs/pull/10/merge' }),
    cache(2, undefined, { ref: 'refs/pull/11/merge' }),
    cache(3, 'unknown', { ref: 'refs/pull/10/merge' }),
    cache(4, undefined, { ref: 'refs/heads/task' }),
  ]
  assert.deepEqual(planCacheCleanup(caches, new Set([10]), 0).deletions.map(item => item.id), [1])
})

test('empty legacy npm removed but populated legacy preserved until replacement exists', () => {
  const old = cache(1, 'node-cache-Linux-x64-npm-lock')
  const empty = cache(2, 'node-cache-Windows-x64-npm-lock', { size_in_bytes: 2448 })
  assert.deepEqual(planCacheCleanup([old, empty]).deletions.map(item => item.id), [2])
  const fresh = cache(3, 'aster-npm-v1-Linux-X64-node22-lock')
  assert.deepEqual(planCacheCleanup([old, fresh]).deletions.map(item => item.id), [1])
})

test('budget cannot evict the latest Linux, Windows, asterctl or runtime cache', () => {
  const caches = [cache(1), cache(2, 'v0-rust-windows-install-Windows_NT-x64-static-a-b'),
    cache(3, 'v0-rust-asterctl-windows-Windows_NT-x64-static-a-b'),
    cache(4, 'aster-windows-build-runtime-Windows-X64-hash')]
  assert.deepEqual(planCacheCleanup(caches, new Set(), 0).deletions, [])
})

test('invalid metadata and duplicate IDs fail closed', () => {
  for (const invalid of [{ id: '../delete' }, { size_in_bytes: -1 }, { created_at: 'invalid' }]) {
    assert.throws(() => planCacheCleanup([cache(1, undefined, invalid)]), /Invalid cache inventory/)
  }
  assert.throws(() => planCacheCleanup([cache(1), cache(1)]), /Invalid cache inventory/)
})

test('API adapter paginates, defaults to read-only and applies exact selected IDs only', () => {
  for (const apply of [false, true]) {
    const calls = []
    const api = args => {
      calls.push(args)
      if (args.includes('--slurp')) return JSON.stringify([
        { actions_caches: [cache(1, undefined, { ref: 'refs/pull/12/merge' })] },
        { actions_caches: [cache(2)] },
      ])
      if (args.at(-1).endsWith('/pulls/12')) return '{"state":"closed"}'
      return ''
    }
    manageCaches({ repository: 'owner/repo', apply, api })
    assert.deepEqual(calls[0], ['api', '--paginate', '--slurp', 'repos/owner/repo/actions/caches?per_page=100'])
    const writes = calls.filter(args => args.includes('DELETE'))
    assert.deepEqual(writes, apply ? [['api', '--method', 'DELETE', 'repos/owner/repo/actions/caches/1']] : [])
  }
})

test('all PR lookups must succeed before any deletion; invalid repo never invokes API', () => {
  const calls = []
  assert.throws(() => manageCaches({ repository: 'owner/repo', apply: true, api: args => {
    calls.push(args)
    if (args.includes('--slurp')) return JSON.stringify([{ actions_caches: [cache(1, undefined, { ref: 'refs/pull/1/merge' })] }])
    throw new Error('network failure')
  } }), /network failure/)
  assert.equal(calls.some(args => args.includes('DELETE')), false)
  assert.throws(() => manageCaches({ repository: '../repos/escape', api: () => assert.fail('must not call API') }), /Invalid GitHub repository/)
})

test('npm cache locator accepts absolute paths with spaces and rejects malformed output', () => {
  const path = resolve('target/cache with spaces')
  assert.equal(npmCachePath(() => ({ status: 0, stdout: `${path}\r\n` })), path)
  for (const stdout of ['relative', `${path}\nother=value`, `${path}\0`]) {
    assert.throws(() => npmCachePath(() => ({ status: 0, stdout })), /invalid cache path/)
  }
  assert.throws(() => npmCachePath(() => ({ status: 1 })), /Cannot locate/)
})

test('every workflow parses and all dependency installations use the shared action', () => {
  for (const file of readdirSync(new URL('../../.github/workflows/', import.meta.url))) {
    if (!file.endsWith('.yml')) continue
    const workflow = yaml(`.github/workflows/${file}`)
    for (const job of Object.values(workflow.jobs)) {
      for (const step of job.steps || []) {
        assert.doesNotMatch(step.run || '', /^npm ci$/m)
        if (step.uses?.startsWith('actions/setup-node@')) {
          assert.equal(step.with['package-manager-cache'], false)
          assert.equal(step.with.cache, undefined)
        }
      }
    }
  }
  const steps = yaml('.github/actions/install-node-dependencies/action.yml').runs.steps
  const install = steps.findIndex(step => step.run === 'npm ci')
  const restore = steps.findIndex(step => step.uses?.startsWith('actions/cache/restore@'))
  const save = steps.findIndex(step => step.uses?.startsWith('actions/cache/save@'))
  assert.ok(restore < install && install < save)
  assert.equal(steps[install].if, undefined, 'cache hit must not skip npm ci')
  assert.match(steps[restore].with.key, /aster-npm-v1-.*hashFiles\('package-lock.json'\)/)
  assert.equal(steps[save].if, "steps.npm-cache.outputs.cache-hit != 'true'")
})

test('release cache changes preserve parallelism and all platform gates', () => {
  const workflow = yaml('.github/workflows/customer-release.yml')
  const preflightSteps = workflow.jobs.preflight.steps
  const dependencyInstall = preflightSteps.findIndex(
    step => step.uses === './.github/actions/install-node-dependencies',
  )
  const releaseTagTest = preflightSteps.findIndex(step => step.run === 'npm run test:release-tag')
  assert.ok(dependencyInstall >= 0 && dependencyInstall < releaseTagTest)
  for (const id of ['preflight', 'asterctl-windows', 'build', 'windows-install']) {
    const step = workflow.jobs[id].steps.find(step => step.uses?.startsWith('Swatinem/rust-cache@'))
    assert.equal(step.with['cache-on-failure'], true)
    assert.equal(step.with['save-if'], "${{ github.ref == 'refs/heads/main' }}")
    for (const restore of workflow.jobs[id].steps.filter(step => step.uses?.startsWith('actions/cache/restore@'))) {
      const save = workflow.jobs[id].steps.find(step => step.with?.key === `\${{ steps.${restore.id}.outputs.cache-primary-key }}`)
      assert.ok(save, `missing save for ${id}/${restore.id}`)
      assert.equal(save.with.path, restore.with.path)
      assert.equal(save.if, `steps.${restore.id}.outputs.cache-hit != 'true'`)
    }
  }
  assert.deepEqual(workflow.jobs.build.needs, ['preflight', 'asterctl-windows'])
  assert.deepEqual(workflow.jobs['windows-install'].needs, ['preflight', 'asterctl-windows'])
  const packageRetention = "${{ github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v') && 7 || 1 }}"
  for (const [jobId, stepName] of [
    ['build', 'Upload verified package'],
    ['windows-install', 'Upload verified Windows package'],
  ]) {
    const upload = workflow.jobs[jobId].steps.find(step => step.name === stepName)
    assert.equal(upload.with['retention-days'], packageRetention)
  }
  assert.deepEqual(workflow.jobs['release-gate'].needs, ['asterctl-windows', 'build', 'linux-primary', 'runner-install', 'customer-install', 'windows-install', 'windows-runner-install'])
  assert.deepEqual(workflow.on.push.tags, ['v*'])
  assert.ok(workflow.on.workflow_dispatch)
  assert.deepEqual(workflow.jobs.publish.needs, ['preflight', 'release-gate'])
  assert.equal(workflow.jobs.publish.if, "${{ always() && github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v') && needs.preflight.result == 'success' && needs['release-gate'].result == 'success' }}")
  assert.equal(workflow.jobs.publish.permissions.contents, 'write')
  assert.equal(workflow.jobs.publish.steps.some(step => step.id === 'public_token'), false)
  const publish = workflow.jobs.publish.steps.find(step => step.run === 'npm run release:publish')
  assert.equal(publish.env.GH_REPO, '${{ github.repository }}')
  assert.equal(publish.env.GH_TOKEN, '${{ github.token }}')
  assert.deepEqual(workflow.jobs['cleanup-published-artifacts'].needs, ['preflight', 'publish'])
  assert.equal(workflow.jobs['cleanup-published-artifacts'].if, "${{ always() && github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v') && needs.preflight.result == 'success' && needs.publish.result == 'success' }}")
  assert.equal(workflow.jobs['cleanup-published-artifacts'].permissions.actions, 'write')
  assert.equal(workflow.permissions.contents, 'read')
  for (const [id, job] of Object.entries(workflow.jobs)) {
    if (id === 'preflight') continue
    for (const step of job.steps || []) {
      if (step.uses?.startsWith('actions/checkout@')) assert.equal(step.with.ref, '${{ needs.preflight.outputs.sha }}')
    }
  }
  const maintenance = yaml('.github/workflows/cache-maintenance.yml')
  assert.equal(maintenance.on.workflow_dispatch.inputs.apply.default, false)
  assert.equal(maintenance.jobs.prune.steps[0].with.ref, 'main')
  assert.equal(maintenance.jobs.prune.steps[0].with['persist-credentials'], false)
  assert.deepEqual(maintenance.on.workflow_run.branches, ['main'])
})

test('Windows lifecycle tests are opt-in while the signed Windows package remains mandatory', () => {
  const release = yaml('.github/workflows/customer-release.yml')
  const verify = yaml('.github/workflows/verify.yml')
  const enabled = "vars.ASTER_ENABLE_WINDOWS_TESTS == 'true'"
  const windows = release.jobs['windows-install'].steps
  for (const name of [
    'Test service launchers under Windows PowerShell 5.1',
    'Build an honest higher-version Windows upgrade fixture',
    'Exercise Windows install, authorization, backup, Blue Green upgrade and cleanup',
  ]) {
    assert.equal(windows.find(step => step.name === name)?.if, enabled)
  }
  assert.equal(release.jobs['windows-runner-install'].if, enabled)
  assert.equal(verify.jobs['asterctl-windows'].steps.find(step => step.name === 'Test service launchers under Windows PowerShell 5.1')?.if, enabled)
  assert.equal(windows.find(step => step.name === 'Build signed Windows package')?.if, undefined)
  assert.equal(windows.find(step => step.name === 'Upload verified Windows package')?.if, undefined)
  const gate = release.jobs['release-gate'].steps.find(step => step.name === 'Require all release jobs to succeed')
  assert.equal(gate.env.WINDOWS_TESTS_ENABLED, '${{ vars.ASTER_ENABLE_WINDOWS_TESTS }}')
  assert.match(gate.run, /\[ "\$WINDOWS_TESTS_ENABLED" = true \]/)
  assert.match(gate.run, /test "\$WINDOWS_RUNNER_RESULT" = skipped/)
  assert.deepEqual(release.jobs.publish.needs, ['preflight', 'release-gate'])
})
