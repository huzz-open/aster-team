import { spawnSync } from 'node:child_process'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export const RUST_TEST_COMMANDS = {
  workspace: [
    ['test', '--workspace', '--jobs', '1', '--features', 'aster-control/sqlite-dev', '--lib', '--bins', '--tests', '--examples'],
    ['test', '--workspace', '--jobs', '1', '--features', 'aster-control/sqlite-dev', '--doc'],
  ],
  customer: [
    [
      'test', '--workspace', '--exclude', 'aster-release-tool', '--jobs', '1', '--features', 'aster-control/sqlite-dev',
      '--lib', '--bins', '--tests', '--examples',
    ],
    ['test', '--workspace', '--exclude', 'aster-release-tool', '--jobs', '1', '--features', 'aster-control/sqlite-dev', '--doc'],
  ],
}

export function rustTestEnvironment(environment = process.env) {
  return {
    ...environment,
    CARGO_PROFILE_TEST_DEBUG: '0',
  }
}

function main() {
  const scope = process.argv[2]
  const commands = RUST_TEST_COMMANDS[scope]
  if (!commands || process.argv.length !== 3) {
    console.error('Usage: node scripts/run-rust-tests.mjs workspace|customer')
    process.exitCode = 1
    return
  }

  for (const arguments_ of commands) {
    const result = spawnSync('cargo', arguments_, {
      cwd: resolve(fileURLToPath(new URL('..', import.meta.url))),
      env: rustTestEnvironment(),
      shell: process.platform === 'win32',
      stdio: 'inherit',
    })
    if (result.error) throw result.error
    if (result.status !== 0) {
      process.exitCode = result.status || 1
      return
    }
  }
}

if (resolve(process.argv[1] || '') === fileURLToPath(import.meta.url)) main()
