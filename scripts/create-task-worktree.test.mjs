import assert from 'node:assert/strict'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'
import test from 'node:test'
import { parseTaskArguments } from './create-task-worktree.mjs'

const scriptPath = resolve(dirname(fileURLToPath(import.meta.url)), 'create-task-worktree.mjs')

function run(command, args, cwd) {
  const result = spawnSync(command, args, { cwd, encoding: 'utf8' })
  assert.equal(result.status, 0, result.stderr || result.stdout)
  return result.stdout.trim()
}

test('parseTaskArguments creates the standard branch and directory names', () => {
  assert.deepEqual(parseTaskArguments(['ui', 'dashboard-cards']), {
    branch: 'codex/ui-dashboard-cards',
    directoryName: 'ui-dashboard-cards',
    name: 'dashboard-cards',
    type: 'ui',
  })
  assert.throws(() => parseTaskArguments(['feature', 'dashboard']), /Unsupported task type/)
  assert.throws(() => parseTaskArguments(['fix', '../dashboard']), /Task name must use/)
})

test('CLI creates a task worktree beside the primary checkout from origin/main', async () => {
  const sandbox = await mkdtemp(resolve(tmpdir(), 'aster-worktree-test-'))
  const repository = resolve(sandbox, 'aster-team')
  const remote = resolve(sandbox, 'origin.git')
  const expectedWorktree = resolve(sandbox, 'aster-team_worktrees', 'fix-sample-bug')

  try {
    await mkdir(repository)
    run('git', ['init', '--initial-branch=main'], repository)
    run('git', ['config', 'user.name', 'Aster Test'], repository)
    run('git', ['config', 'user.email', 'aster-test@example.invalid'], repository)
    await writeFile(resolve(repository, 'README.md'), 'test\n', 'utf8')
    run('git', ['add', 'README.md'], repository)
    run('git', ['commit', '-m', 'Initial commit'], repository)
    run('git', ['init', '--bare', remote], repository)
    run('git', ['remote', 'add', 'origin', remote], repository)
    run('git', ['push', '-u', 'origin', 'main'], repository)

    const output = run(process.execPath, [scriptPath, 'fix', 'sample-bug'], repository)
    assert.match(output, new RegExp(`WORKTREE_PATH=${expectedWorktree.replaceAll('\\', '\\\\')}`, 'i'))
    assert.equal(run('git', ['branch', '--show-current'], expectedWorktree), 'codex/fix-sample-bug')
    assert.equal((await readFile(resolve(expectedWorktree, 'README.md'), 'utf8')).replaceAll('\r\n', '\n'), 'test\n')
  } finally {
    await rm(sandbox, { recursive: true, force: true, maxRetries: 3 })
  }
})
