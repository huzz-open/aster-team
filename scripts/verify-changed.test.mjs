import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

import { FULL_VERIFICATION_COMMANDS, fullVerificationCommands } from './run-full-verification.mjs'
import { parseNameStatus, resolveVerificationPlan } from './verify-changed.mjs'

const manifest = JSON.parse(readFileSync(resolve('tools/verification-map.json'), 'utf8'))

function commandIDs(plan) {
  return plan.commands.map(command => command.id)
}

test('verification policy schema and risk levels fail closed when malformed', () => {
  const oldSchema = structuredClone(manifest)
  oldSchema.schemaVersion = 1
  assert.throws(() => resolveVerificationPlan(['README.md'], oldSchema), /schemaVersion 2/)

  const invalidRisk = structuredClone(manifest)
  invalidRisk.rules[0].level = 'partial'
  assert.throws(() => resolveVerificationPlan(['README.md'], invalidRisk), /Invalid verification rule/)
})

test('a Customer Admin change runs only its frontend and shared UI policy checks', () => {
  const plan = resolveVerificationPlan(['customer/admin/src/views/HomeView.vue'], manifest)

  assert.equal(plan.level, 'AFFECTED')
  assert.deepEqual(plan.suites, ['common-policy', 'ui-policy', 'customer-admin'])
  assert.deepEqual(commandIDs(plan), [
    'release-boundaries',
    'contracts',
    'table-layout',
    'typography',
    'customer-admin-tests',
    'customer-admin-build',
    'customer-admin-assets',
  ])
})

test('a shared UI change expands to every consuming frontend', () => {
  const plan = resolveVerificationPlan(['packages/ui/src/styles.css'], manifest)

  assert.equal(plan.level, 'AFFECTED')
  assert.deepEqual(plan.suites, [
    'common-policy',
    'ui-policy',
    'customer-admin',
    'customer-member',
    'operations-console',
    'website',
  ])
})

test('shared contracts, dependencies, release foundations, and unknown changes fail closed to repo-full verification', () => {
  for (const path of [
    'Cargo.lock',
    'contracts/schemas/license.v1.schema.yaml',
    'customer/deploy/systemd/aster-control@.service',
    'unexpected/root.config',
  ]) {
    const plan = resolveVerificationPlan([path], manifest)
    assert.equal(plan.level, 'REPO_FULL', path)
    assert.deepEqual(commandIDs(plan), ['full-verification'], path)
  }
})

test('Customer runtime and security changes select the complete Customer source domain', () => {
  for (const path of [
    'customer/backend/control/src/lib.rs',
    'customer/backend/crates/auth-core/src/lib.rs',
    'customer/backend/crates/storage/src/lib.rs',
  ]) {
    const plan = resolveVerificationPlan([path], manifest)
    assert.equal(plan.level, 'DOMAIN', path)
    assert.deepEqual(plan.suites, ['customer-domain'], path)
    assert.deepEqual(commandIDs(plan), ['customer-domain'], path)
  }
})

test('Operations persistence and security changes select the complete Operations source domain', () => {
  const plan = resolveVerificationPlan([
    'operations/backend/internal/adapters/mariadb/store.go',
  ], manifest)

  assert.equal(plan.level, 'DOMAIN')
  assert.deepEqual(plan.suites, ['operations-domain'])
  assert.deepEqual(commandIDs(plan), ['operations-domain'])
})

test('a Customer domain plan supersedes affected checks from the same pull request', () => {
  const plan = resolveVerificationPlan([
    'customer/admin/src/views/OverviewView.vue',
    'customer/admin/test/overview-interactions.test.ts',
    'customer/backend/control/src/lib.rs',
    'customer/member/src/views/ConsumptionLogsView.vue',
    'customer/member/src/views/UsageView.vue',
    'customer/sdk/src/index.ts',
  ], manifest)

  assert.equal(plan.level, 'DOMAIN')
  assert.deepEqual(plan.suites, ['customer-domain'])
  assert.deepEqual(commandIDs(plan), ['customer-domain'])
})

test('repository agent metadata uses deterministic documentation checks without product verification', () => {
  const plan = resolveVerificationPlan(['AGENTS.md'], manifest)

  assert.equal(plan.level, 'AFFECTED')
  assert.deepEqual(plan.suites, ['documentation'])
  assert.deepEqual(commandIDs(plan), ['documentation'])
})

test('documentation-only changes select documentation verification', () => {
  const plan = resolveVerificationPlan(['notes/decision.md'], manifest)

  assert.equal(plan.level, 'AFFECTED')
  assert.deepEqual(plan.suites, ['documentation'])
  assert.deepEqual(commandIDs(plan), ['documentation'])
})

test('Markdown inside a product still runs documentation verification', () => {
  const plan = resolveVerificationPlan(['customer/admin/README.md'], manifest)

  assert.equal(plan.level, 'AFFECTED')
  assert.deepEqual(plan.suites, ['documentation', 'common-policy', 'ui-policy', 'customer-admin'])
})

test('rename parsing keeps both the old and new paths', () => {
  assert.deepEqual(
    parseNameStatus('M\0README.md\0R100\0docs/old.md\0docs/new.md\0D\0docs/removed.md\0'),
    ['README.md', 'docs/old.md', 'docs/new.md', 'docs/removed.md'],
  )
})

test('full verification relies on workspace builds for typechecking exactly once', () => {
  const commandIDs_ = FULL_VERIFICATION_COMMANDS.map(command => command.id)

  assert.equal(commandIDs_.includes('all-typechecks'), false)
  assert.equal(commandIDs_.includes('all-builds'), true)
})

test('full verification runs Windows service tests only when explicitly enabled', () => {
  for (const value of [undefined, '', 'false', 'TRUE']) {
    assert.equal(fullVerificationCommands({ ASTER_ENABLE_WINDOWS_TESTS: value }).some(command => command.id === 'windows-service-launcher'), false)
  }
  assert.equal(fullVerificationCommands({ ASTER_ENABLE_WINDOWS_TESTS: 'true' }).some(command => command.id === 'windows-service-launcher'), true)
})
