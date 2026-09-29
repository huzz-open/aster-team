import assert from 'node:assert/strict'
import test from 'node:test'
import { parseArguments, versionAtLeast } from './setup-development-tools.mjs'
import { preferredGoRoot, sharedToolsRoot } from '../tools/toolchains/go-toolchain.mjs'

test('the generic setup parser accepts portable flags and rejects platform-specific paths', () => {
  assert.deepEqual(parseArguments(['--check', '--yes']), { checkOnly: true, assumeYes: true })
  assert.throws(() => parseArguments(['--install-root', '/tmp/tools']), /ASTER_TOOLS_ROOT/)
  assert.equal(versionAtLeast('22.19.0'), true)
  assert.equal(versionAtLeast('22.18.9'), false)
})

test('macOS and Linux use platform-native shared tool roots', () => {
  assert.equal(sharedToolsRoot('/repo', { platform: 'darwin', environment: {}, homeDirectory: '/Users/dev' }), '/Users/dev/Library/Application Support/AsterDev')
  assert.equal(sharedToolsRoot('/repo', { platform: 'linux', environment: {}, homeDirectory: '/home/dev' }), '/home/dev/.local/share/aster-dev')
  assert.equal(sharedToolsRoot('/repo', { platform: 'linux', environment: { XDG_DATA_HOME: '/data' }, homeDirectory: '/home/dev' }), '/data/aster-dev')
})

test('generic and Go-specific overrides work on every supported platform', () => {
  for (const platform of ['win32', 'darwin', 'linux']) {
    const separator = platform === 'win32' ? '\\' : '/'
    const toolsRoot = platform === 'win32' ? 'D:\\DevTools' : '/opt/dev-tools'
    const goRoot = platform === 'win32' ? 'E:\\Go' : '/opt/go'
    assert.equal(preferredGoRoot('ignored', { platform, environment: { ASTER_TOOLS_ROOT: toolsRoot } }), `${toolsRoot}${separator}Go`)
    assert.equal(preferredGoRoot('ignored', { platform, environment: { ASTER_TOOLS_ROOT: toolsRoot, ASTER_GO_ROOT: goRoot } }), goRoot)
  }
})
