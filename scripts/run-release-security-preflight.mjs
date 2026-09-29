import { spawnSync } from 'node:child_process'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { npmCommand, runCommands } from './verification-runner.mjs'

export const CARGO_AUDIT_VERSION = '0.22.2'

export const RELEASE_SECURITY_COMMANDS = [
  npmCommand(
    'production-node-dependency-audit',
    'audit',
    '--omit=dev',
    '--audit-level=high',
    '--registry=https://registry.npmjs.org',
  ),
  {
    id: 'rust-dependency-audit',
    executable: 'cargo',
    arguments: ['audit', '--deny', 'warnings'],
  },
]

function cargoAuditVersion(execute = spawnSync) {
  const result = execute('cargo', ['audit', '--version'], {
    encoding: 'utf8',
    shell: process.platform === 'win32',
  })
  if (result.error || result.status !== 0) return ''
  return `${result.stdout || ''}${result.stderr || ''}`.trim()
}

export function ensureCargoAudit(execute = spawnSync) {
  if (cargoAuditVersion(execute) === `cargo-audit ${CARGO_AUDIT_VERSION}`) return

  console.log(`[preflight] Installing cargo-audit ${CARGO_AUDIT_VERSION}...`)
  const result = execute('cargo', [
    'install',
    'cargo-audit',
    '--locked',
    '--version',
    CARGO_AUDIT_VERSION,
  ], {
    shell: process.platform === 'win32',
    stdio: 'inherit',
  })
  if (result.error) throw result.error
  if (result.status !== 0) {
    const error = new Error(`cargo-audit installation failed with exit code ${result.status}`)
    error.exitCode = result.status || 1
    throw error
  }
}

function main() {
  try {
    runCommands([RELEASE_SECURITY_COMMANDS[0]])
    ensureCargoAudit()
    runCommands([RELEASE_SECURITY_COMMANDS[1]])
  } catch (error) {
    console.error(error instanceof Error ? error.message : error)
    process.exitCode = error?.exitCode || 1
  }
}

if (resolve(process.argv[1] || '') === fileURLToPath(import.meta.url)) main()
