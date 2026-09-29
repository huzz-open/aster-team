import assert from 'node:assert/strict'
import test from 'node:test'
import {
  localRunnerProxyEnvironment,
  parseMacosSystemProxies,
} from './local-system-proxy.mjs'

const SYSTEM_PROXY_OUTPUT = `<dictionary> {
  ExceptionsList : <array> {
    0 : localhost
  }
  HTTPEnable : 1
  HTTPPort : 7897
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7897
  HTTPSProxy : 127.0.0.1
}`

test('parses enabled macOS HTTP proxies', () => {
  assert.deepEqual(parseMacosSystemProxies(SYSTEM_PROXY_OUTPUT), {
    http: 'http://127.0.0.1:7897',
    https: 'http://127.0.0.1:7897',
  })
})

test('adds macOS system proxies and bypasses local services', () => {
  const environment = localRunnerProxyEnvironment(
    { EXISTING: 'kept' },
    { platform: 'darwin', systemProxyOutput: SYSTEM_PROXY_OUTPUT },
  )
  assert.equal(environment.EXISTING, 'kept')
  assert.equal(environment.HTTP_PROXY, 'http://127.0.0.1:7897')
  assert.equal(environment.HTTPS_PROXY, 'http://127.0.0.1:7897')
  assert.equal(environment.no_proxy, '127.0.0.1,localhost,::1')
})

test('preserves explicit proxy configuration', () => {
  const environment = localRunnerProxyEnvironment(
    {
      HTTPS_PROXY: 'http://explicit.example:8080',
      ALL_PROXY: 'socks5://explicit.example:1080',
      NO_PROXY: 'internal.example,localhost',
    },
    { platform: 'darwin', systemProxyOutput: SYSTEM_PROXY_OUTPUT },
  )
  assert.equal(environment.HTTPS_PROXY, 'http://explicit.example:8080')
  assert.equal(environment.ALL_PROXY, 'socks5://explicit.example:1080')
  assert.equal(environment.HTTP_PROXY, undefined)
  assert.equal(environment.NO_PROXY, 'internal.example,localhost,127.0.0.1,::1')
})

test('does not use macOS proxy settings on other platforms', () => {
  assert.deepEqual(
    localRunnerProxyEnvironment(
      { EXISTING: 'kept' },
      { platform: 'linux', systemProxyOutput: SYSTEM_PROXY_OUTPUT },
    ),
    { EXISTING: 'kept' },
  )
})
