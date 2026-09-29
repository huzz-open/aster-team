import { existsSync, mkdirSync, realpathSync } from 'node:fs'
import { basename, dirname, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { spawnSync } from 'node:child_process'

export const TASK_TYPES = new Set(['feat', 'fix', 'ui', 'opt', 'docs', 'chore'])

function usage() {
  return 'Usage: npm run worktree:new -- <feat|fix|ui|opt|docs|chore> <task-name>'
}

export function parseTaskArguments(args) {
  if (args.length !== 2) throw new Error(usage())

  const [type, name] = args
  if (!TASK_TYPES.has(type)) {
    throw new Error(`Unsupported task type: ${type}\n${usage()}`)
  }
  if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(name)) {
    throw new Error(`Task name must use lowercase letters, numbers, and single hyphens: ${name}`)
  }

  return {
    branch: `codex/${type}-${name}`,
    directoryName: `${type}-${name}`,
    name,
    type,
  }
}

function runGit(args, cwd, { allowFailure = false } = {}) {
  const result = spawnSync('git', args, {
    cwd,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  })

  if (result.error) throw result.error
  if (result.status !== 0 && !allowFailure) {
    const detail = result.stderr.trim() || result.stdout.trim() || `git exited with ${result.status}`
    throw new Error(`git ${args.join(' ')} failed:\n${detail}`)
  }

  return result
}

function primaryWorktreePath(cwd) {
  const output = runGit(['worktree', 'list', '--porcelain'], cwd).stdout
  const entry = output.split(/\r?\n/).find((line) => line.startsWith('worktree '))
  if (!entry) throw new Error('Unable to locate the primary Git worktree.')
  return resolve(entry.slice('worktree '.length))
}

function refExists(ref, cwd) {
  return runGit(['show-ref', '--verify', '--quiet', ref], cwd, { allowFailure: true }).status === 0
}

export function createTaskWorktree({ cwd = process.cwd(), type, name }) {
  const task = parseTaskArguments([type, name])
  const primaryRoot = primaryWorktreePath(cwd)
  const worktreesRoot = resolve(dirname(primaryRoot), `${basename(primaryRoot)}_worktrees`)
  const worktreePath = resolve(worktreesRoot, task.directoryName)

  runGit(['remote', 'get-url', 'origin'], primaryRoot)
  runGit(['fetch', 'origin', '--prune'], primaryRoot)

  if (!refExists('refs/remotes/origin/main', primaryRoot)) {
    throw new Error('origin/main does not exist after fetching origin.')
  }
  if (refExists(`refs/heads/${task.branch}`, primaryRoot)) {
    throw new Error(`Local branch already exists: ${task.branch}`)
  }
  if (refExists(`refs/remotes/origin/${task.branch}`, primaryRoot)) {
    throw new Error(`Remote branch already exists: origin/${task.branch}`)
  }
  if (existsSync(worktreePath)) {
    throw new Error(`Worktree directory already exists: ${worktreePath}`)
  }

  mkdirSync(worktreesRoot, { recursive: true })
  runGit(['worktree', 'add', '--no-track', '-b', task.branch, worktreePath, 'origin/main'], primaryRoot)

  const actualRoot = realpathSync(runGit(['rev-parse', '--show-toplevel'], worktreePath).stdout.trim())
  const expectedRoot = realpathSync(worktreePath)
  if (actualRoot !== expectedRoot) {
    throw new Error(`Created worktree root mismatch: expected ${expectedRoot}, got ${actualRoot}`)
  }

  const actualBranch = runGit(['branch', '--show-current'], worktreePath).stdout.trim()
  if (actualBranch !== task.branch) {
    throw new Error(`Created worktree branch mismatch: expected ${task.branch}, got ${actualBranch}`)
  }

  return {
    base: runGit(['rev-parse', '--short=12', 'origin/main'], worktreePath).stdout.trim(),
    branch: task.branch,
    worktreePath: actualRoot,
  }
}

function main() {
  try {
    const task = parseTaskArguments(process.argv.slice(2))
    const result = createTaskWorktree(task)
    console.log(`WORKTREE_PATH=${result.worktreePath}`)
    console.log(`BRANCH=${result.branch}`)
    console.log(`BASE=${result.base}`)
    console.log('Switch the coding session to WORKTREE_PATH before reading or modifying project files.')
  } catch (error) {
    console.error(error instanceof Error ? error.message : error)
    process.exitCode = 1
  }
}

const entryPoint = process.argv[1] ? pathToFileURL(resolve(process.argv[1])).href : ''
if (import.meta.url === entryPoint) main()
