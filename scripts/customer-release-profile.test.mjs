import assert from 'node:assert/strict'
import { generateKeyPairSync } from 'node:crypto'
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import test from 'node:test'
import { localEntitlementsWithin } from './local-commercial-license.mjs'
import { readCustomerReleaseProfile } from './customer-release-profile.mjs'
import { generateLicenseSigners, localLicensePolicies, readLicenseSigningProfile } from './license-signing-profile.mjs'

const trustFixture = JSON.parse(readFileSync(new URL('../contracts/test-vectors/license-trust.v1.json', import.meta.url), 'utf8'))
const policy = trustFixture.keyring[0].policy
const publicKey = () => generateKeyPairSync('ed25519').publicKey.export({ format: 'der', type: 'spki' }).toString('base64url')
const licenseOld = publicKey()
const licenseNew = publicKey()
const releaseOld = publicKey()
const releaseNew = publicKey()
const configuration = {
  ASTER_LICENSE_TRUSTED_KEYS_JSON: JSON.stringify([
    { key_id: 'license-2026-01', public_key_spki: licenseOld, policy },
    { key_id: 'license-2027-01', public_key_spki: licenseNew, policy },
  ]),
  ASTER_RELEASE_TRUSTED_KEYS_JSON: JSON.stringify([
    { key_id: 'release-2026-01', public_key_spki: releaseOld },
    { key_id: 'release-2027-01', public_key_spki: releaseNew },
  ]),
}

test('customer release profile accepts separate bridge keyrings', () => {
  const profile = readCustomerReleaseProfile(configuration)
  assert.equal(profile.licenseTrustedKeys.length, 2)
  assert.equal(profile.releaseTrustedKeys.length, 2)
  assert.deepEqual(JSON.parse(profile.licenseTrustedKeysJSON), profile.licenseTrustedKeys)
  assert.deepEqual(JSON.parse(profile.releaseTrustedKeysJSON), profile.releaseTrustedKeys)
})

test('customer release profile rejects missing, malformed, duplicate, or shared keys', () => {
  assert.throws(() => readCustomerReleaseProfile({}), /valid JSON/)
  assert.throws(() => readCustomerReleaseProfile({ ...configuration, ASTER_LICENSE_TRUSTED_KEYS_JSON: '[]' }), /one and eight/)
  assert.throws(() => readCustomerReleaseProfile({
    ...configuration,
    ASTER_LICENSE_TRUSTED_KEYS_JSON: JSON.stringify([
      { key_id: 'duplicate', public_key_spki: licenseOld, policy },
      { key_id: 'duplicate', public_key_spki: licenseNew, policy },
    ]),
  }), /duplicated/)
  assert.throws(() => readCustomerReleaseProfile({
    ...configuration,
    ASTER_RELEASE_TRUSTED_KEYS_JSON: JSON.stringify([{ key_id: 'release-shared', public_key_spki: licenseOld }]),
  }), /cryptographically distinct/)
})

test('v2 private profiles reject ambiguous, noncanonical and mismatched trusted configuration without exposing secrets', () => {
  const signers = generateLicenseSigners(localLicensePolicies())
  assert.deepEqual(signers[0].policy.entitlement_ceiling.quotas.map(entry => entry.limit.value), [3, 1, 1, 1])
  const release = configuration.ASTER_RELEASE_TRUSTED_KEYS_JSON
  const raw = JSON.stringify(signers)
  const profile = readLicenseSigningProfile(raw, release)
  assert.deepEqual(profile.licenseTrustedKeys.map(entry => entry.policy), signers.map(entry => entry.policy))
  assert.doesNotMatch(profile.licenseTrustedKeysJSON, /private_key_pkcs8/)
  for (const change of [
    entries => { entries[0].private_key_pkcs8 += '=' },
    entries => { entries[0].private_key_pkcs8 = 'PRIVATE-CANARY' },
    entries => { delete entries[0].policy },
    entries => { entries[0].policy = null },
    entries => { entries[0].policy.sources.push('commercial_order') },
    entries => { entries[0].policy.sources.unshift('approved_trial') },
    entries => { entries[0].skip = true },
    entries => { entries[1].key_id = entries[0].key_id },
    entries => { entries[1].private_key_pkcs8 = entries[0].private_key_pkcs8 },
  ]) {
    const changed = structuredClone(signers); change(changed)
    assert.throws(() => readLicenseSigningProfile(JSON.stringify(changed), release), error => {
      assert.ok(!String(error).includes(signers[0].private_key_pkcs8))
      assert.doesNotMatch(String(error), /PRIVATE-CANARY/)
      return true
    })
  }
  assert.throws(() => readLicenseSigningProfile(raw.replace('"policy":', '"policy":null,"policy":'), release), /unique fields/)
  assert.throws(() => readLicenseSigningProfile(raw.replace('"key_id":', '"k\\u0065y_id":"alias","key_id":'), release), /unique fields/)
  assert.throws(() => readLicenseSigningProfile(raw, JSON.stringify([{ key_id: 'release-shared', public_key_spki: profile.licenseTrustedKeys[0].public_key_spki }])), /cryptographically distinct/)
  for (const change of [
    keys => { keys[0].key_id = 'another-key' },
    keys => { keys[0].public_key_spki = publicKey() },
    keys => { keys[0].policy.entitlement_ceiling.quotas[0].limit.value = 4 },
  ]) {
    const changed = structuredClone(profile.licenseTrustedKeys); change(changed)
    assert.throws(() => readLicenseSigningProfile(raw, release, JSON.stringify(changed)), /does not match/)
  }
})

test('signing key generator requires explicit policies and emits matching scoped keys without overwriting recovery files', () => {
  const temporary = mkdtempSync(resolve(tmpdir(), 'aster-v2-keygen-test-'))
  const output = resolve(temporary, 'test-only-keys')
  const policiesPath = resolve(temporary, 'test-only-policies.json')
  const policies = localLicensePolicies()
  writeFileSync(policiesPath, JSON.stringify(policies))
  const run = (...args) => spawnSync(process.execPath, [resolve(import.meta.dirname, 'generate-production-signing-keys.mjs'), ...args], { encoding: 'utf8', windowsHide: true })
  assert.notEqual(run(output).status, 0)
  assert.equal(existsSync(output), false)
  const generated = run(output, policiesPath)
  assert.equal(generated.status, 0, generated.stderr)
  const raw = readFileSync(resolve(output, 'license-v2.signers.json'), 'utf8')
  const publicJSON = readFileSync(resolve(output, 'license-v2.public-keyring.json'), 'utf8')
  const profile = readLicenseSigningProfile(raw, readFileSync(resolve(output, 'release-v1.public-keyring.json'), 'utf8'), publicJSON)
  assert.deepEqual(profile.licenseTrustedKeys.map(entry => entry.policy), policies.map(entry => entry.policy))
  assert.equal(readFileSync(resolve(output, 'release-v1.seed')).length, 32)
  for (const signer of JSON.parse(raw)) assert.ok(!generated.stdout.includes(signer.private_key_pkcs8))
  assert.notEqual(run(output, policiesPath).status, 0)
  assert.equal(readFileSync(resolve(output, 'license-v2.signers.json'), 'utf8'), raw)
  const invalidOutput = resolve(temporary, 'invalid')
  policies[0].policy = null
  writeFileSync(policiesPath, JSON.stringify(policies))
  assert.notEqual(run(invalidOutput, policiesPath).status, 0)
  assert.equal(existsSync(invalidOutput), false)
  const repositoryOutput = resolve(import.meta.dirname, '..', 'dist', 'must-not-generate-keys')
  assert.notEqual(run(repositoryOutput, policiesPath).status, 0)
  assert.equal(existsSync(repositoryOutput), false)
  writeFileSync(policiesPath, JSON.stringify(localLicensePolicies()))
  const deceptiveOutput = resolve(import.meta.dirname, '..', `..signing-test-${Date.now()}`)
  assert.notEqual(run(deceptiveOutput, policiesPath).status, 0)
  assert.equal(existsSync(deceptiveOutput), false)
})

const scopedConfiguration = keyring => ({ ...configuration, ASTER_LICENSE_TRUSTED_KEYS_JSON: JSON.stringify(keyring) })

test('Customer builds retain the exact Operations scope in v2 trust', () => {
  const profile = readCustomerReleaseProfile(scopedConfiguration(trustFixture.keyring))
  assert.deepEqual(profile.licenseTrustedKeys, trustFixture.keyring)
  assert.deepEqual(JSON.parse(profile.licenseTrustedKeysJSON), trustFixture.keyring)
  for (const keyID of ['x', 'license/a+b@v2']) {
    const keyring = structuredClone(trustFixture.keyring)
    keyring[1].key_id = keyID
    assert.equal(readCustomerReleaseProfile(scopedConfiguration(keyring)).licenseTrustedKeys[1].key_id, keyID)
  }
})

test('Customer builds reject malformed scopes instead of dropping policy', () => {
  for (const change of [
    entry => { delete entry.policy },
    entry => { entry.policy = null },
    entry => { entry.policy = {} },
    entry => { entry.policy.sources = [] },
    entry => { entry.policy.sources.push('free_distribution') },
    entry => { entry.policy.sources.push('commercial_order') },
    entry => { entry.policy.sources.unshift('approved_trial') },
    entry => { entry.policy.sources = [{ free_distribution: null }] },
    entry => { entry.policy.bindings = [{ unbound: null }] },
    entry => { entry.policy.expiries = [{ none: null }] },
    entry => { entry.policy.expiries = ['unknown'] },
    entry => { entry.policy.entitlement_ceiling.catalog_version = 2 },
    entry => { entry.policy.entitlement_ceiling.quotas.pop() },
    entry => { entry.policy.entitlement_ceiling.quotas[1].id = 'member_seats' },
    entry => { entry.policy.entitlement_ceiling.quotas[0].limit = { mode: 'unlimited', value: 3 } },
    entry => { entry.policy.entitlement_ceiling.features.push('unknown') },
    entry => { entry.policy.skip = true },
    entry => { entry.private_key_pkcs8 = 'must-not-be-compiled' },
    entry => { entry.key_id = 'invalid key' },
    entry => { entry.public_key_spki += '=' },
  ]) {
    const keyring = structuredClone(trustFixture.keyring)
    change(keyring[1])
    assert.throws(() => readCustomerReleaseProfile(scopedConfiguration(keyring)))
  }
  // Release keys are still a separate protocol and cannot receive License scope.
  assert.throws(() => readCustomerReleaseProfile({
    ...configuration, ASTER_RELEASE_TRUSTED_KEYS_JSON: JSON.stringify([trustFixture.keyring[1]]),
  }), /invalid fields/)
})

test('Customer builds reject scope aliases, duplicate source fields and shared release material', () => {
  const free = trustFixture.keyring[1]
  const alias = { key_id: 'legacy-alias', public_key_spki: free.public_key_spki }
  for (const keyring of [[free, { ...free, key_id: 'free-alias' }], [{ ...free, key_id: 'free-alias' }, free]]) {
    assert.throws(() => readCustomerReleaseProfile(scopedConfiguration(keyring)), /duplicated public key/)
  }
  assert.throws(() => readCustomerReleaseProfile({
    ...scopedConfiguration(trustFixture.keyring),
    ASTER_RELEASE_TRUSTED_KEYS_JSON: JSON.stringify([alias]),
  }), /cryptographically distinct/)
  for (const keyring of [[free, alias], [alias, free]]) assert.throws(() => readCustomerReleaseProfile(scopedConfiguration(keyring)), /invalid fields/)
  const raw = JSON.stringify(trustFixture.keyring)
  for (const [from, to] of [
    ['"policy":{', '"policy":null,"policy":{'],
    ['"sources":[', '"sources":[],"sources":['],
    ['"bindings":', '"b\\u0069ndings":[],"bindings":'],
    ['"key_id":', '"key_id":"alias","key_id":'],
  ]) {
    assert.ok(raw.includes(from))
    assert.throws(() => readCustomerReleaseProfile({
      ...configuration, ASTER_LICENSE_TRUSTED_KEYS_JSON: raw.replace(from, to),
    }), /unique fields/)
  }
  assert.throws(() => readCustomerReleaseProfile({
    ...configuration, ASTER_LICENSE_TRUSTED_KEYS_JSON: ' '.repeat((1 << 20) + 1),
  }), /valid JSON/)
})

test('local issuer selection accepts symbolic ceilings but cannot manufacture future rights or quota', () => {
  const rights = localLicensePolicies()[1].policy.entitlement_ceiling
  const symbolic = { ...structuredClone(rights), features: [] }
  assert.equal(localEntitlementsWithin(rights, symbolic), true)
  assert.equal(localEntitlementsWithin(rights, { ...symbolic, feature_sets: [] }), false)
  symbolic.quotas[0].limit.value = 1
  assert.equal(localEntitlementsWithin(rights, symbolic), false)
})

test('build-time trust refuses symbolic grants for free or trial issuers', () => {
  for (const source of ['free_distribution', 'approved_trial']) {
    const entry = { key_id: 'bounded-test', public_key_spki: licenseOld, policy: localLicensePolicies()[1].policy }
    entry.policy.sources = [source]
    assert.throws(() => readCustomerReleaseProfile({ ...configuration, ASTER_LICENSE_TRUSTED_KEYS_JSON: JSON.stringify([entry]) }), /invalid fields/)
  }
})
