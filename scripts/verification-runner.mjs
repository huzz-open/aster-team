import { spawn, spawnSync } from 'node:child_process'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')

function duration(milliseconds) {
  return `${(milliseconds / 1000).toFixed(1)}s`
}

function displayCommand(command) {
  return [command.executable, ...command.arguments].join(' ')
}

function requiresShell(command) {
  return process.platform === 'win32' && command.executable === 'npm'
}

export function npmCommand(id, ...arguments_) {
  return {
    id,
    executable: 'npm',
    arguments: arguments_,
  }
}

export function runCommands(commands, { cwd = repositoryRoot } = {}) {
  const results = []
  const totalStartedAt = performance.now()

  for (const command of commands) {
    const startedAt = performance.now()
    console.log(`\n[verify] ${command.id}: ${displayCommand(command)}`)
    const result = spawnSync(command.executable, command.arguments, {
      cwd,
      shell: requiresShell(command),
      stdio: 'inherit',
    })
    const elapsed = performance.now() - startedAt
    results.push({ id: command.id, elapsed })
    console.log(`[verify] ${command.id} finished in ${duration(elapsed)}`)

    if (result.error) throw result.error
    if (result.status !== 0) {
      const error = new Error(`${command.id} failed with exit code ${result.status}`)
      error.exitCode = result.status || 1
      throw error
    }
  }

  console.log('\nVerification timing:')
  for (const result of results) console.log(`- ${result.id}: ${duration(result.elapsed)}`)
  console.log(`- total: ${duration(performance.now() - totalStartedAt)}`)
}

function runCommand(command, cwd) {
  return new Promise((resolveCommand, rejectCommand) => {
    const startedAt = performance.now()
    console.log(`\n[verify] ${command.id}: ${displayCommand(command)}`)
    const child = spawn(command.executable, command.arguments, {
      cwd,
      shell: requiresShell(command),
      stdio: 'inherit',
    })
    child.once('error', rejectCommand)
    child.once('close', status => {
      const elapsed = performance.now() - startedAt
      console.log(`[verify] ${command.id} finished in ${duration(elapsed)}`)
      if (status === 0) {
        resolveCommand({ id: command.id, elapsed })
        return
      }
      const error = new Error(`${command.id} failed with exit code ${status}`)
      error.exitCode = status || 1
      rejectCommand(error)
    })
  })
}

export async function runCommandLanes(lanes, { cwd = repositoryRoot } = {}) {
  if (!Array.isArray(lanes) || !lanes.length || lanes.some(lane => !Array.isArray(lane) || !lane.length)) {
    throw new Error('Verification lanes must be a non-empty array of non-empty command arrays')
  }
  const totalStartedAt = performance.now()
  const results = []
  const outcomes = await Promise.allSettled(lanes.map(async lane => {
    for (const command of lane) results.push(await runCommand(command, cwd))
  }))
  const failure = outcomes.find(outcome => outcome.status === 'rejected')

  console.log('\nVerification timing:')
  for (const result of results) console.log(`- ${result.id}: ${duration(result.elapsed)}`)
  console.log(`- total: ${duration(performance.now() - totalStartedAt)}`)

  if (failure) throw failure.reason
}
