import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'
import { verifyTurnstile } from './turnstile'

const originalFetch = globalThis.fetch
const expectedHostnames = new Set(['127.0.0.1', 'localhost'])

function siteverifyReturns(body: Record<string, unknown>) {
  globalThis.fetch = async () => Response.json(body)
}

afterEach(() => {
  globalThis.fetch = originalFetch
})

test('local test mode accepts the hostname returned by a browser-generated test token', async () => {
  siteverifyReturns({ success: true, hostname: '127.0.0.1', action: 'trial_request', 'error-codes': [] })

  const result = await verifyTurnstile(
    'browser-test-token',
    'test-secret',
    'trial_request',
    expectedHostnames,
    null,
    true,
  )

  assert.deepEqual(result, { hostname: '127.0.0.1' })
})

test('local test mode accepts the fixed hostname returned for the documented dummy token', async () => {
  siteverifyReturns({ success: true, hostname: 'example.com', 'error-codes': [] })

  const result = await verifyTurnstile(
    'XXXX.DUMMY.TOKEN.XXXX',
    'test-secret',
    'trial_request',
    expectedHostnames,
    null,
    true,
  )

  assert.deepEqual(result, { hostname: 'example.com' })
})

test('local test mode rejects an unexpected hostname or action', async () => {
  siteverifyReturns({ success: true, hostname: 'untrusted.example', action: 'trial_request', 'error-codes': [] })
  assert.equal(await verifyTurnstile('test-token', 'test-secret', 'trial_request', expectedHostnames, null, true), null)

  siteverifyReturns({ success: true, hostname: 'localhost', action: 'different_action', 'error-codes': [] })
  assert.equal(await verifyTurnstile('test-token', 'test-secret', 'trial_request', expectedHostnames, null, true), null)
})

test('production mode still requires the exact hostname and action', async () => {
  siteverifyReturns({ success: true, hostname: 'aster.huzz.top', action: 'trial_request', 'error-codes': [] })

  const result = await verifyTurnstile(
    'production-token',
    'production-secret',
    'trial_request',
    new Set(['aster.huzz.top']),
    '203.0.113.10',
    false,
  )

  assert.deepEqual(result, { hostname: 'aster.huzz.top' })
})
