import assert from 'node:assert/strict'
import test from 'node:test'

import { createClientRequestId, type BrowserRandomSource } from '../src/random.ts'

test('uses browser randomUUID when the secure-context API is available', () => {
  const expected = '12345678-1234-4123-8123-123456789abc'
  const source: BrowserRandomSource = {
    randomUUID: () => expected,
    getRandomValues: values => values,
  }
  assert.equal(createClientRequestId(source), expected)
})

test('creates an RFC 4122 version 4 request id when randomUUID is unavailable', () => {
  const source: BrowserRandomSource = {
    getRandomValues(values) {
      values.fill(0xab)
      return values
    },
  }
  const requestId = createClientRequestId(source)
  assert.match(requestId, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/)
  assert.equal(requestId, 'abababab-abab-4bab-abab-abababababab')
})

test('fails closed instead of using weak randomness', () => {
  assert.throws(
    () => createClientRequestId({ getRandomValues: undefined as never }),
    /Secure browser randomness is unavailable/,
  )
})
