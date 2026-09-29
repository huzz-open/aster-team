import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { npmCommand, runCommandLanes } from './verification-runner.mjs'

export const DOMAIN_VERIFICATION_PHASES = {
  customer: [
    [
      [
        npmCommand('customer-rust-format-and-lint', 'run', 'check:rust:customer'),
        npmCommand('customer-rust-tests', 'run', 'test:rust:customer'),
      ],
    ],
    [
      [
        npmCommand('customer-boundaries', 'run', 'verify:boundaries', '--', '--domain=customer'),
        npmCommand('contracts', 'run', 'verify:contracts'),
        npmCommand('table-layout', 'run', 'verify:table-layout'),
        npmCommand('typography', 'run', 'verify:typography'),
        npmCommand('customer-logging', 'run', 'verify:customer-logging'),
      ],
      [
        npmCommand('customer-admin-tests', 'test', '--workspace', '@aster/admin'),
        npmCommand('customer-sdk-tests', 'test', '--workspace', '@aster/sdk'),
        npmCommand('customer-admin-build', 'run', 'build', '--workspace', '@aster/admin'),
        npmCommand('customer-member-build', 'run', 'build', '--workspace', '@aster/member'),
        npmCommand('customer-assets', 'run', 'verify:customer-assets'),
      ],
    ],
  ],
  operations: [
    [
      [
        npmCommand('operations-boundaries', 'run', 'verify:boundaries', '--', '--domain=operations'),
        npmCommand('contracts', 'run', 'verify:contracts'),
        npmCommand('table-layout', 'run', 'verify:table-layout'),
        npmCommand('typography', 'run', 'verify:typography'),
      ],
      [
        npmCommand('operations-go-tests', 'run', 'test:go'),
        npmCommand('operations-go-build', 'run', 'build:go'),
      ],
      [
        npmCommand('operations-console-tests', 'test', '--workspace', '@aster/operations-console'),
        npmCommand('operations-console-build', 'run', 'build', '--workspace', '@aster/operations-console'),
      ],
    ],
  ],
}

export const DOMAIN_VERIFICATION_COMMANDS = Object.fromEntries(
  Object.entries(DOMAIN_VERIFICATION_PHASES)
    .map(([domain, phases]) => [domain, phases.flat(2)]),
)

async function main() {
  const domain = process.argv[2]
  const phases = DOMAIN_VERIFICATION_PHASES[domain]
  if (!phases || process.argv.length !== 3) {
    console.error('Usage: node scripts/run-domain-verification.mjs customer|operations')
    process.exitCode = 1
    return
  }

  try {
    for (const lanes of phases) await runCommandLanes(lanes)
  } catch (error) {
    console.error(error instanceof Error ? error.message : error)
    process.exitCode = error?.exitCode || 1
  }
}

if (resolve(process.argv[1] || '') === fileURLToPath(import.meta.url)) void main()
