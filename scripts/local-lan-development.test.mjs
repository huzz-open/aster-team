import assert from 'node:assert/strict'
import test from 'node:test'

import {
  localDevelopmentAdvertisedHost,
  localDevelopmentBindHost,
  localLanAccessEnabled,
} from './local-lan-development.mjs'

test('local development stays on loopback by default', () => {
  assert.equal(localLanAccessEnabled({}), false)
  assert.equal(localDevelopmentBindHost({}), '127.0.0.1')
  assert.equal(localDevelopmentAdvertisedHost({}), '127.0.0.1')
})

test('LAN development binds user-facing services and advertises the detected IPv4 address', () => {
  const environment = {
    ASTER_LOCAL_LAN_ENABLED: 'true',
    ASTER_LOCAL_LAN_HOST: '10.213.41.33',
  }
  assert.equal(localLanAccessEnabled(environment), true)
  assert.equal(localDevelopmentBindHost(environment), '0.0.0.0')
  assert.equal(localDevelopmentAdvertisedHost(environment), '10.213.41.33')
})

test('LAN development rejects missing and local-only advertised addresses', () => {
  for (const host of [undefined, 'localhost', '127.0.0.1', '169.254.1.2', '::1']) {
    assert.throws(
      () => localDevelopmentAdvertisedHost({
        ASTER_LOCAL_LAN_ENABLED: 'true',
        ASTER_LOCAL_LAN_HOST: host,
      }),
      /ASTER_LOCAL_LAN_HOST/,
    )
  }
})
