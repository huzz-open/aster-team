import assert from 'node:assert/strict'
import { mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import test from 'node:test'
import { isForbiddenReference, verifyArtifact, verifySources } from './verify-release-boundaries.mjs'

test('source imports remain checked without treating prose as a module specifier', () => {
  assert.equal(isForbiddenReference('The local Functions are compiled from the current checkout and use a fresh disposable database.', 'database'), false)
  for (const source of [
    "import { query } from '../database/client.js'",
    'export { query } from "../database/client.js"',
    "import { query }\nfrom\n'../database/client.js'",
    "const store = import('../database/client.js')",
    'const store = require("../database/client.js")',
    'import "aster.local/team/database"',
    "resolve(root, 'database')",
    "cpSync('database', target)",
  ]) assert.equal(isForbiddenReference(source, 'database'), true, source)
})

test('current release domain sources respect dependency boundaries', () => {
  for (const domain of ['customer', 'operations', 'website']) {
    assert.deepEqual(verifySources(domain), [], domain)
  }
})
test('customer artifact rejects internal operations files', () => {
  const directory = resolve(tmpdir(), `aster-boundary-${process.pid}-${Date.now()}`)
  try {
    for (const entry of ['.internal', 'admin', 'bin', 'control', 'member']) mkdirSync(resolve(directory, entry), { recursive: true })
    for (const entry of ['MANIFEST.json', 'SBOM.cdx.json', 'VERSION', 'README.md', 'init.sh']) writeFileSync(resolve(directory, entry), '')
    writeFileSync(resolve(directory, 'bin/operations-api'), '')
    assert.ok(verifyArtifact('customer', directory).some(value => value.includes('operations-api')))
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})
test('customer artifact scans file and binary content for internal modules and signing secrets', () => {
  const directory = resolve(tmpdir(), `aster-boundary-content-${process.pid}-${Date.now()}`)
  try {
    for (const entry of ['.internal', 'admin', 'bin', 'control', 'member']) mkdirSync(resolve(directory, entry), { recursive: true })
    for (const entry of ['MANIFEST.json', 'SBOM.cdx.json', 'VERSION', 'README.md', 'init.sh']) writeFileSync(resolve(directory, entry), '')
    writeFileSync(resolve(directory, 'bin/aster-runner'), Buffer.from([0, 1, 2, ...Buffer.from('ASTER_OPERATIONS_LICENSE_SIGNING_PRIVATE_KEY_PKCS8'), 0]))
    assert.ok(verifyArtifact('customer', directory).some(value => value.includes('ASTER_OPERATIONS_LICENSE_SIGNING_PRIVATE_KEY_PKCS8')))
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

for (const platform of ['linux', 'windows']) test(`${platform} customer artifact permits only the exact optional bundled free license`, () => {
  const directory = resolve(tmpdir(), `aster-boundary-free-license-${process.pid}-${Date.now()}`)
  const boundary = JSON.parse(readFileSync(new URL('../tools/release-boundaries.json', import.meta.url), 'utf8'))
  const required = boundary.domains.customer.artifactProfiles[platform]
  const directories = new Set(['THIRD_PARTY_LICENSES', 'admin', 'bin', 'client-tools', 'libexec', 'member', 'systemd', 'windows'])
  try {
    mkdirSync(directory, { recursive: true })
    for (const entry of required) {
      if (directories.has(entry)) mkdirSync(resolve(directory, entry), { recursive: true })
      else writeFileSync(resolve(directory, entry), '', { flag: 'wx' })
    }
    assert.deepEqual(verifyArtifact('customer', directory, platform), [])

    mkdirSync(resolve(directory, 'licenses'))
    writeFileSync(resolve(directory, 'licenses/free-license.json'), '{"signed":"fixture"}')
    assert.deepEqual(verifyArtifact('customer', directory, platform), [])

    writeFileSync(resolve(directory, 'licenses/private-key.json'), 'BEGIN PRIVATE KEY')
    const extraFailures = verifyArtifact('customer', directory, platform)
    assert.ok(extraFailures.some(value => value.includes('Unexpected optional customer artifact file: licenses/private-key.json')))
    assert.ok(extraFailures.some(value => value.includes('BEGIN PRIVATE KEY')))

    rmSync(resolve(directory, 'licenses'), { recursive: true, force: true })
    mkdirSync(resolve(directory, 'licenses'))
    assert.ok(verifyArtifact('customer', directory, platform).some(value => value.includes('Missing optional customer artifact file: licenses/free-license.json')))

    mkdirSync(resolve(directory, 'licenses/unexpected-empty-directory'))
    assert.ok(verifyArtifact('customer', directory, platform).some(value => value.includes('Unexpected optional customer artifact directory: licenses/unexpected-empty-directory')))

    rmSync(resolve(directory, 'licenses'), { recursive: true, force: true })
    const outside = resolve(tmpdir(), `aster-boundary-free-license-outside-${process.pid}-${Date.now()}`)
    mkdirSync(outside, { recursive: true })
    writeFileSync(resolve(outside, 'free-license.json'), '{"signed":"outside"}')
    symlinkSync(outside, resolve(directory, 'licenses'), 'junction')
    assert.ok(verifyArtifact('customer', directory, platform).some(value => value.includes('Optional customer artifact group must be an ordinary directory: licenses')))
    rmSync(outside, { recursive: true, force: true })
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})
