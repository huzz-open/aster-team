import assert from 'node:assert/strict'
import test from 'node:test'

import { RUST_TEST_COMMANDS, rustTestEnvironment } from './run-rust-tests.mjs'

test('full Rust tests cover every target and doctests in separate low-memory phases', () => {
  assert.deepEqual(RUST_TEST_COMMANDS.workspace, [
    ['test', '--workspace', '--jobs', '1', '--features', 'aster-control/sqlite-dev', '--lib', '--bins', '--tests', '--examples'],
    ['test', '--workspace', '--jobs', '1', '--features', 'aster-control/sqlite-dev', '--doc'],
  ])
})

test('Customer Rust tests exclude Operations release tooling', () => {
  assert.equal(RUST_TEST_COMMANDS.customer.length, 2)
  assert.ok(RUST_TEST_COMMANDS.customer.every(arguments_ => (
    arguments_.includes('--exclude')
      && arguments_.includes('aster-release-tool')
      && arguments_.includes('--jobs')
      && arguments_.includes('1')
      && arguments_.includes('--features')
      && arguments_.includes('aster-control/sqlite-dev')
  )))
  assert.deepEqual(RUST_TEST_COMMANDS.customer[1], [
    'test', '--workspace', '--exclude', 'aster-release-tool', '--jobs', '1', '--features', 'aster-control/sqlite-dev', '--doc',
  ])
})

test('Rust tests disable debug info without discarding the inherited environment', () => {
  assert.deepEqual(rustTestEnvironment({ SENTINEL: 'present' }), {
    SENTINEL: 'present',
    CARGO_PROFILE_TEST_DEBUG: '0',
  })
})
