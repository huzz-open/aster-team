import assert from 'node:assert/strict'
import test from 'node:test'
import { githubRepository, parseArguments, releaseGithub } from './release-github.mjs'
import { parseTagArguments } from './release-tag.mjs'

const baseSHA = 'a'.repeat(40)
const releaseSHA = 'b'.repeat(40)
const files = ['package.json', 'package-lock.json', 'Cargo.toml', 'Cargo.lock']

function fixture(overrides = {}) {
  let sha = overrides.dispatchOnly ? releaseSHA : baseSHA
  let version = overrides.dispatchOnly ? '2.0.2' : '2.0.1'
  let remoteSHA = sha
  const calls = []
  const messages = []
  const run = (executable, args) => {
    calls.push([executable, ...args])
    if (overrides.fail?.(executable, args)) throw new Error('injected failure')
    if (executable === 'git') {
      switch (args[0]) {
        case 'branch': return overrides.branch || 'main'
        case 'status': return overrides.dirty ? ' M unrelated.txt' : ''
        case 'remote': return 'git@github.com:owner/project.git'
        case 'fetch': case 'merge': case 'add': return ''
        case 'rev-parse': return args[1].endsWith('^{commit}') ? (overrides.localTagSHA || sha) : args[1] === 'HEAD' ? sha : (overrides.ahead ? 'c'.repeat(40) : remoteSHA)
        case 'diff': return (overrides.changedFiles || files).join('\0') + '\0'
        case 'commit': sha = releaseSHA; return 'version committed'
        case 'push': remoteSHA = sha; return ''
        case 'tag': return args[1] === '--list' && overrides.localTagSHA ? 'v2.0.2' : ''
        case 'ls-remote': return args[2].startsWith('refs/tags/')
          ? (overrides.remoteTagSHA ? `${overrides.remoteTagSHA}\trefs/tags/v2.0.2^{}` : '')
          : `${overrides.remoteAdvanced ? 'c'.repeat(40) : remoteSHA}\trefs/heads/main`
        default: throw new Error(`Unexpected git call: ${args}`)
      }
    }
    if (executable === 'gh') {
      if (args[0] === 'api') return args[1] === 'user' ? 'operator' : (overrides.workflowState || 'active')
      if (args[0] === 'run') return JSON.stringify(overrides.runs || [])
      if (args[0] === 'workflow') return 'https://github.com/owner/project/actions/runs/1'
    }
    throw new Error(`Unexpected command: ${executable} ${args}`)
  }
  return {
    calls, messages,
    execute: () => releaseGithub({
      options: { version: '2.0.2', dispatchOnly: Boolean(overrides.dispatchOnly), tag: overrides.tag },
      run,
      prepare: requested => { calls.push(['prepare', requested]); version = requested },
      currentVersion: () => overrides.currentVersion || version,
      freeLicense: () => ({ distributionID: 'dist_free_1', sha256: 'd'.repeat(64), base64: 'e30=' }),
      log: message => messages.push(message),
    }),
  }
}

test('accepts explicit versions and a dispatch-only recovery mode', () => {
  assert.deepEqual(parseArguments(['--version', 'v2.0.2']), { help: false, version: '2.0.2', dispatchOnly: false })
  assert.deepEqual(parseArguments(['--dispatch-only', '--version=2.0.2']), { help: false, version: '2.0.2', dispatchOnly: true })
  assert.deepEqual(parseArguments(['--help']), { help: true })
  assert.throws(() => parseArguments([]), /--version/)
  assert.throws(() => parseArguments(['--version=2.0.2', '--force']), /Unknown/)
  assert.throws(() => parseArguments(['--version=2.0.2;echo unsafe']), /invalid/)
})

test('GitHub destination follows origin and never forwards embedded credentials', () => {
  for (const remote of ['git@github.com:owner/project.git', 'https://github.com/owner/project.git', 'ssh://git@github.com/owner/project.git']) {
    assert.equal(githubRepository(remote).qualified, 'github.com/owner/project')
  }
  assert.equal(githubRepository('git@github.example.test:owner/project.git').host, 'github.example.test')
  assert.throws(() => githubRepository('https://token@github.com/owner/project.git'), /credential-free/)
  assert.throws(() => githubRepository('D:/local/repo'), /GitHub/)
})

test('prepares only version files, pushes main, then dispatches the exact commit without local suites', () => {
  const scenario = fixture()
  const result = scenario.execute()
  assert.equal(result.sha, releaseSHA)
  assert.equal(result.existing, false)
  const add = scenario.calls.find(call => call[0] === 'git' && call[1] === 'add')
  assert.deepEqual(add, ['git', 'add', '--', ...files])
  const push = scenario.calls.findIndex(call => call[1] === 'push')
  const dispatch = scenario.calls.findIndex(call => call[0] === 'gh' && call[1] === 'workflow')
  assert.ok(push < dispatch)
  assert.deepEqual(scenario.calls[dispatch], ['gh', 'workflow', 'run', 'customer-release.yml', '--repo', 'github.com/owner/project', '--ref', 'main', '-f', 'version=2.0.2', '-f', `source_commit_sha=${releaseSHA}`, '-f', `release_task_id=manual-2.0.2-${releaseSHA.slice(0, 12)}`, '-f', 'free_distribution_id=dist_free_1', '-f', `free_license_sha256=${'d'.repeat(64)}`, '-f', 'free_license_base64=e30='])
  assert.ok(!scenario.calls.some(call => ['npm', 'cargo', 'node'].includes(call[0])))
  assert.ok(!scenario.calls.flat().some(arg => arg.includes('--no-verify') || arg.includes('--force')))
})

test('wrong branch, dirty worktree, ahead main and unavailable workflow stop before preparation', () => {
  for (const options of [{ branch: 'topic' }, { dirty: true }, { ahead: true }, { workflowState: 'disabled_manually' }]) {
    const scenario = fixture(options)
    assert.throws(scenario.execute)
    assert.ok(!scenario.calls.some(call => call[0] === 'prepare'))
    assert.ok(!scenario.calls.some(call => call[0] === 'gh' && call[1] === 'workflow'))
  }
})

test('unexpected prepared changes are not committed', () => {
  const scenario = fixture({ changedFiles: [...files, 'unrelated.txt'] })
  assert.throws(scenario.execute, /unexpected files/)
  assert.ok(!scenario.calls.some(call => call[1] === 'commit'))
})

test('push failure retains the commit and offers dispatch-only recovery without dispatching', () => {
  const scenario = fixture({ fail: (executable, args) => executable === 'git' && args[0] === 'push' })
  assert.throws(scenario.execute, /git push origin main.*--dispatch-only/)
  assert.ok(!scenario.calls.some(call => call[0] === 'gh' && call[1] === 'workflow'))
})

test('a concurrently advanced remote cannot change the requested build source', () => {
  const scenario = fixture({ remoteAdvanced: true })
  assert.throws(scenario.execute, /origin\/main advanced/)
  assert.ok(!scenario.calls.some(call => call[0] === 'gh' && call[1] === 'workflow'))
})

test('dispatch-only does not edit, commit or push and returns an existing failed run', () => {
  const existing = { databaseId: 7, displayTitle: `Customer 2.0.2 · manual-2.0.2-${releaseSHA.slice(0, 12)} · ${releaseSHA} · ${'d'.repeat(64)}`, headSha: releaseSHA, status: 'completed', conclusion: 'failure', url: 'https://github.com/owner/project/actions/runs/7' }
  const scenario = fixture({ dispatchOnly: true, runs: [existing] })
  assert.deepEqual(scenario.execute(), { sha: releaseSHA, existing: true, url: existing.url })
  assert.ok(!scenario.calls.some(call => call[0] === 'prepare' || ['add', 'commit', 'push', 'workflow'].includes(call[1])))
  assert.ok(scenario.messages.some(message => message.includes('gh run rerun 7 --failed')))
})

test('dispatch-only rejects a different committed version', () => {
  const scenario = fixture({ dispatchOnly: true, currentVersion: '2.0.1' })
  assert.throws(scenario.execute, /committed version/)
})

test('an uncertain dispatch response is never automatically retried', () => {
  const scenario = fixture({ fail: (executable, args) => executable === 'gh' && args[0] === 'workflow' })
  assert.throws(scenario.execute, /failed response may still have created a run/)
  assert.equal(scenario.calls.filter(call => call[0] === 'gh' && call[1] === 'workflow').length, 1)
})

test('tag entry creates an annotated tag only after main is pushed, without dispatching twice', () => {
  assert.deepEqual(parseTagArguments(['--version=v2.0.2', '--tag-only']), { version: '2.0.2', tag: true, dispatchOnly: true })
  assert.throws(() => parseTagArguments(['--version=2.0.2', '--dispatch-only']), /Unknown/)
  const scenario = fixture({ tag: true })
  assert.deepEqual(scenario.execute(), { sha: releaseSHA, existing: false, tag: 'v2.0.2' })
  const tagIndex = scenario.calls.findIndex(call => call[1] === 'tag' && call[2] === '-a')
  assert.deepEqual(scenario.calls[tagIndex], ['git', 'tag', '-a', 'v2.0.2', releaseSHA, '-m', 'Customer 2.0.2', '-m', `Free-License-SHA256: ${'d'.repeat(64)}`, '-m', 'Free-Distribution-ID: dist_free_1'])
  assert.ok(scenario.calls.findIndex(call => call[1] === 'push' && call[3] === 'main') < tagIndex)
  assert.ok(scenario.calls.some(call => call[1] === 'push' && call[3] === 'refs/tags/v2.0.2:refs/tags/v2.0.2'))
  assert.ok(!scenario.calls.some(call => call[0] === 'gh' && ['workflow', 'run'].includes(call[1])))
})

test('tag recovery never moves an existing local or remote tag', () => {
  for (const options of [{ remoteTagSHA: 'c'.repeat(40) }, { localTagSHA: 'c'.repeat(40) }]) {
    const scenario = fixture({ tag: true, dispatchOnly: true, ...options })
    assert.throws(scenario.execute, /different commit/)
    assert.ok(!scenario.calls.some(call => call[1] === 'push'))
  }
  const existing = fixture({ tag: true, dispatchOnly: true, remoteTagSHA: releaseSHA })
  assert.equal(existing.execute().existing, true)
  assert.ok(!existing.calls.some(call => call[1] === 'push'))
  const local = fixture({ tag: true, dispatchOnly: true, localTagSHA: releaseSHA })
  assert.equal(local.execute().existing, false)
  assert.ok(!local.calls.some(call => call[1] === 'tag' && call[2] === '-a'))
})

test('existing release tags stop version preparation and failed tag pushes retain recovery state', () => {
  const existing = fixture({ tag: true, remoteTagSHA: releaseSHA })
  assert.throws(existing.execute, /already exists/)
  assert.ok(!existing.calls.some(call => call[0] === 'prepare'))
  const failed = fixture({ tag: true, fail: (exe, args) => exe === 'git' && args[0] === 'push' && args[2].startsWith('refs/tags/') })
  assert.throws(failed.execute, /--tag-only/)
  assert.ok(!failed.calls.flat().some(arg => arg.includes('--force') || arg === '-d'))
})
