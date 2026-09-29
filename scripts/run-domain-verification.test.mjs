import assert from 'node:assert/strict'
import test from 'node:test'

import {
  DOMAIN_VERIFICATION_COMMANDS,
  DOMAIN_VERIFICATION_PHASES,
} from './run-domain-verification.mjs'

function commandIDs(domain) {
  return DOMAIN_VERIFICATION_COMMANDS[domain].map(command => command.id)
}

test('Customer domain verification covers policies, Rust, both applications, SDK, and assets', () => {
  assert.deepEqual(commandIDs('customer'), [
    'customer-rust-format-and-lint',
    'customer-rust-tests',
    'customer-boundaries',
    'contracts',
    'table-layout',
    'typography',
    'customer-logging',
    'customer-admin-tests',
    'customer-sdk-tests',
    'customer-admin-build',
    'customer-member-build',
    'customer-assets',
  ])
  assert.deepEqual(DOMAIN_VERIFICATION_PHASES.customer.map(phase => phase.length), [1, 2])
  const rustCommands = DOMAIN_VERIFICATION_PHASES.customer[0][0]
  assert.ok(rustCommands.every(command => command.arguments.at(-1).includes(':customer')))
})

test('Operations domain verification excludes unrelated Customer and release checks', () => {
  assert.deepEqual(commandIDs('operations'), [
    'operations-boundaries',
    'contracts',
    'table-layout',
    'typography',
    'operations-go-tests',
    'operations-go-build',
    'operations-console-tests',
    'operations-console-build',
  ])
  assert.deepEqual(DOMAIN_VERIFICATION_PHASES.operations.map(phase => phase.length), [3])
})
