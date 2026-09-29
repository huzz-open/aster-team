import assert from 'node:assert/strict'
import test from 'node:test'
import { redactDiagnostic } from './sanitize-diagnostics.mjs'

test('redacts credentials and diagnostic canaries at the source', () => {
  const result = redactDiagnostic('Authorization: Bearer secret-value\npassword=not-for-logs\nhttps://localhost/callback?code=oauth-code&state=oauth-state\nCANARY', { canary: 'CANARY' })
  assert.equal(result.includes('secret-value'), false)
  assert.equal(result.includes('not-for-logs'), false)
  assert.equal(result.includes('oauth-code'), false)
  assert.equal(result.includes('oauth-state'), false)
  assert.equal(result.includes('CANARY'), false)
})
