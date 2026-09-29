import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { npmCommand, runCommands } from './verification-runner.mjs'

export const fullVerificationCommands = (environment = process.env) => [
  ...(environment.ASTER_ENABLE_WINDOWS_TESTS === 'true'
    ? [npmCommand('windows-service-launcher', 'run', 'test:windows-services')]
    : []),
  npmCommand('ci-image-contracts', 'run', 'test:ci-images'),
  npmCommand('ci-image-lock', 'run', 'ci:images:check'),
  npmCommand('ci-cache-contracts', 'run', 'test:ci-cache'),
  npmCommand('customer-sbom', 'run', 'verify:customer-sbom'),
  npmCommand('rust-format-and-lint', 'run', 'check:rust'),
  npmCommand('contracts', 'run', 'verify:contracts'),
  npmCommand('documentation', 'run', 'verify:docs'),
  npmCommand('caddy-format', 'run', 'verify:caddy-format'),
  npmCommand('table-layout', 'run', 'verify:table-layout'),
  npmCommand('typography', 'run', 'verify:typography'),
  npmCommand('all-tests', 'run', 'test'),
  npmCommand('tooling-tests', 'run', 'test:boundaries'),
  npmCommand('system-e2e-unit-tests', 'run', 'test:system-e2e:unit'),
  npmCommand('all-builds', 'run', 'build'),
  npmCommand('customer-assets', 'run', 'verify:customer-assets'),
]

export const FULL_VERIFICATION_COMMANDS = fullVerificationCommands()

function main() {
  try {
    runCommands(FULL_VERIFICATION_COMMANDS)
  } catch (error) {
    console.error(error instanceof Error ? error.message : error)
    process.exitCode = error?.exitCode || 1
  }
}

if (resolve(process.argv[1] || '') === fileURLToPath(import.meta.url)) main()
