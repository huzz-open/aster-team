import { test } from 'node:test'
import assert from 'node:assert/strict'
import { buildInstallCommand } from '../src/install-command'
import { fallbackReleaseCatalog } from '../src/release-fallback'
import { parseReleaseCatalog } from '../shared/release-catalog'

test('bundled fallback is a valid published Linux release', () => {
  assert.deepEqual(parseReleaseCatalog(fallbackReleaseCatalog), fallbackReleaseCatalog)
  assert.equal(fallbackReleaseCatalog.latest, 'v2.1.1')
})

test('install command stays executable and only includes available, validated selections', () => {
  const defaults = { version: 'latest', email: '', protocol: 'http' as const, host: '' }
  const siteOrigin = 'https://aster.huzz.top'
  assert.equal(buildInstallCommand(defaults, [], siteOrigin), 'curl -fsSL https://aster.huzz.top/install.sh | bash')
  assert.equal(buildInstallCommand(defaults, [], 'https://new.example'), 'curl -fsSL https://new.example/install.sh | bash')
  assert.equal(buildInstallCommand({ ...defaults, version: 'v2.1.1', email: 'owner@example.com', protocol: 'https', host: 'team.example.com' }, ['v2.1.1', 'v2.1.0'], siteOrigin),
    'curl -fsSL https://aster.huzz.top/install.sh | bash -s -- --version v2.1.1 --email owner@example.com --protocol https --host team.example.com')
  assert.equal(buildInstallCommand({ ...defaults, version: 'v2.1.0' }, ['v2.1.1', 'v2.1.0'], siteOrigin),
    'curl -fsSL https://aster.huzz.top/install.sh | bash -s -- --version v2.1.0')
  for (const choices of [
    { ...defaults, version: 'v0.0.1' },
    { ...defaults, email: 'owner@example.com;rm -rf /' },
    { ...defaults, host: 'example.com;echo hello' },
  ]) assert.equal(buildInstallCommand(choices, ['v2.1.1'], siteOrigin), null)
  assert.equal(buildInstallCommand(defaults, [], 'https://new.example/;bad'), null)
})
