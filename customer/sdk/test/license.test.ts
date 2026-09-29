import assert from 'node:assert/strict'
import test from 'node:test'
import { licenseFeatureAvailable, licenseFeatureRetained, licenseOperationAvailable, type PublicLicenseState } from '../src/license.ts'
import { BUSINESS_OPERATIONS, type BusinessOperationId } from '../src/generated/product-capabilities.ts'

test('expired data access does not reopen operations or infer unsigned features', () => {
  const expired = { state: 'expired', available: false, features: ['gateway', 'member', 'runner'] }
  assert.equal(licenseFeatureRetained(expired, 'member'), true)
  assert.equal(licenseFeatureAvailable(expired, 'member'), false)
  assert.equal(licenseOperationAvailable(expired, 'upstream_sync'), false)
  assert.equal(licenseFeatureRetained(expired, 'unknown'), false)
  assert.equal(licenseFeatureRetained({ ...expired, available: true }, 'member'), false)
  for (const state of ['missing', 'unavailable', 'unknown']) {
    assert.equal(licenseFeatureRetained({ ...expired, state }, 'member'), false)
  }
  assert.equal(licenseFeatureRetained({ state: 'expired', available: false }, 'member'), false)
})

test('active license only makes its explicit features available', () => {
  const license = { state: 'active', available: true, features: ['runner'] }
  assert.equal(licenseFeatureAvailable(license, 'runner'), true)
  assert.equal(licenseFeatureAvailable(license, 'member'), false)
  assert.equal(licenseFeatureAvailable({ ...license, features: [] }, 'member'), false)
})

test('unavailable states cannot reuse stale features', () => {
  for (const state of ['missing', 'unavailable', 'expired', 'future-state']) {
    assert.equal(licenseFeatureAvailable({ state, available: true, features: ['member'] }, 'member'), false)
  }
  assert.equal(licenseFeatureAvailable({ state: 'active', available: false, features: ['member'] }, 'member'), false)
})

test('legacy responses preserve member availability without granting future features', () => {
  const legacy = { state: 'active', available: true }
  assert.equal(licenseFeatureAvailable(legacy, 'member'), true)
  assert.equal(licenseFeatureAvailable(legacy, 'runner'), false)
  assert.equal(licenseFeatureAvailable(legacy, 'future-feature'), false)
})

test('malformed feature fields do not fall back to legacy availability', () => {
  for (const features of [null, undefined, 'member', { member: true }]) {
    const license = { state: 'active', available: true, features } as unknown as PublicLicenseState
    assert.equal(licenseFeatureAvailable(license, 'member'), false)
  }
})

test('external upstream operations require both capabilities and a current explicit projection', () => {
  for (const operation of BUSINESS_OPERATIONS) {
    for (const features of [[], ['gateway'], ['runner'], ['member']]) {
      assert.equal(licenseOperationAvailable({ state: 'active', available: true, features }, operation.id), false)
    }
    assert.equal(licenseOperationAvailable({ state: 'active', available: true, features: ['gateway', 'runner'] }, operation.id), true)
    assert.equal(licenseOperationAvailable({ state: 'expired', available: true, features: ['gateway', 'runner'] }, operation.id), false)
    assert.equal(licenseOperationAvailable({ state: 'active', available: false, features: ['gateway', 'runner'] }, operation.id), false)
    assert.equal(licenseOperationAvailable({ state: 'active', available: true }, operation.id), false)
    assert.equal(licenseOperationAvailable(undefined, operation.id), false)
  }
  assert.equal(licenseOperationAvailable({ state: 'active', available: true, features: ['gateway', 'runner'] }, 'unknown' as BusinessOperationId), false)
  assert.equal(licenseFeatureAvailable({ state: 'active', available: true, features: ['gateway'] }, 'gateway'), true)
})
