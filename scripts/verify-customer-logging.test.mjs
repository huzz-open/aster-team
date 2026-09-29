import assert from 'node:assert/strict'
import test from 'node:test'

import { verifyCustomerLogging, verifyRustLoggingSource } from './verify-customer-logging.mjs'

test('current customer Rust sources do not log sensitive values', () => {
  assert.deepEqual(verifyCustomerLogging(), [])
})

test('logging boundary rejects credentials and full request or response data', () => {
  const failures = verifyRustLoggingSource('sample.rs', `
    info!(request_id = %request_id, authorization = %request.authorization, "unsafe");
    debug!(response_body = ?response_body, "unsafe");
    warn!(?response, "unsafe");
  `)
  assert.equal(failures.length, 3)
  assert.match(failures[0], /authorization/)
  assert.match(failures[1], /response_body/)
  assert.match(failures[2], /response/)
})

test('logging boundary ignores comments, messages and non-sensitive identifiers', () => {
  assert.deepEqual(verifyRustLoggingSource('sample.rs', `
    // error!(payload = ?payload, "comment only");
    const EXAMPLE: &str = r#"warn!(cookie = ?cookie, "text only")"#;
    info!(request_id = %request_id, response_status, key_id, category, "token refresh completed");
  `), [])
})
