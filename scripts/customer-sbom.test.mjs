import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

import { buildCustomerSbom, collectNpmComponents, customerStorageFeatures } from './generate-customer-sbom.mjs'

const cargoMetadata = {
  packages: [
    { id: 'control', name: 'aster-control', version: '1.1.0', source: null, license: 'LicenseRef-Aster-Proprietary' },
    { id: 'runner', name: 'aster-runner', version: '1.1.0', source: null, license: 'LicenseRef-Aster-Proprietary' },
    { id: 'cli', name: 'aster-team-cli', version: '1.1.0', source: null, license: 'LicenseRef-Aster-Proprietary' },
    { id: 'asterctl', name: 'asterctl', version: '1.1.0', source: null, license: 'LicenseRef-Aster-Proprietary' },
    { id: 'runtime', name: 'runtime-dependency', version: '2.0.0', source: 'registry+https://example.invalid', license: 'MIT', checksum: 'a'.repeat(64) },
    { id: 'dev', name: 'dev-only', version: '3.0.0', source: 'registry+https://example.invalid', license: 'MIT' },
  ],
  resolve: {
    nodes: [
      { id: 'control', deps: [
        { pkg: 'runtime', dep_kinds: [{ kind: null, target: null }] },
        { pkg: 'dev', dep_kinds: [{ kind: 'dev', target: null }] },
      ] },
      { id: 'runner', deps: [] },
      { id: 'cli', deps: [{ pkg: 'runtime', dep_kinds: [{ kind: null, target: null }] }] },
      { id: 'asterctl', deps: [{ pkg: 'runtime', dep_kinds: [{ kind: null, target: null }] }] },
      { id: 'runtime', deps: [] },
      { id: 'dev', deps: [] },
    ],
  },
}

const packageLock = {
  packages: {
    'customer/admin': { name: '@aster/admin', version: '0.1.0', dependencies: { '@aster/ui': '*', vue: '^3' } },
    'customer/member': { name: '@aster/member', version: '0.1.0', dependencies: { vue: '^3' } },
    'node_modules/@aster/ui': { resolved: 'packages/ui', link: true },
    'packages/ui': { name: '@aster/ui', version: '0.1.0', dependencies: { vue: '^3' } },
    'node_modules/vue': { version: '3.5.0', license: 'MIT', integrity: `sha512-${Buffer.from('vue-integrity').toString('base64')}` },
    'node_modules/dev-only': { version: '9.0.0', license: 'MIT' },
  },
}

const actualPackageLock = JSON.parse(readFileSync(new URL('../package-lock.json', import.meta.url), 'utf8'))

test('customer SBOM contains only reachable production Rust and frontend components', () => {
  const input = {
    cargoMetadata,
    packageLock,
    version: '1.1.0',
    timestamp: '2026-08-28T00:00:00.000Z',
    target: {
      platform: 'windows',
      architecture: 'amd64',
      rustTarget: 'x86_64-pc-windows-msvc',
      runtime: 'msvc-static',
    },
  }
  const first = buildCustomerSbom(input)
  const second = buildCustomerSbom(input)
  assert.deepEqual(first, second)
  assert.equal(first.bomFormat, 'CycloneDX')
  assert.equal(first.specVersion, '1.6')
  assert.deepEqual(first.components.map(value => value.name).sort(), [
    '@aster/admin', '@aster/member', '@aster/ui', 'Caddy', 'aster-control', 'aster-runner', 'aster-team-cli', 'asterctl',
    'runtime-dependency', 'vue',
  ])
  assert.ok(!first.components.some(value => value.name === 'dev-only'))
  assert.ok(first.components.every(value => value.licenses[0].expression))
  assert.equal(new Set(first.components.map(value => value['bom-ref'])).size, first.components.length)
  assert.equal(first.dependencies[0].ref, first.metadata.component['bom-ref'])
  assert.equal(first.components.find(value => value.name === 'vue').hashes[0].alg, 'SHA-512')
  const caddy = first.components.find(value => value.name === 'Caddy')
  assert.equal(caddy.licenses[0].expression, 'Apache-2.0')
  assert.equal(caddy.hashes[0].alg, 'SHA-512')
})

test('customer SBOM rejects an external component without license metadata', () => {
  const invalid = structuredClone(cargoMetadata)
  invalid.packages.find(value => value.id === 'runtime').license = null
  assert.throws(() => buildCustomerSbom({
    cargoMetadata: invalid,
    packageLock,
    version: '1.1.0',
    timestamp: '2026-08-28T00:00:00.000Z',
    target: { platform: 'windows', architecture: 'amd64', rustTarget: 'x86_64-pc-windows-msvc', runtime: 'msvc-static' },
  }), /missing license metadata/)
})

test('customer SBOM accepts the permissive 0BSD license', () => {
  const metadata = structuredClone(cargoMetadata)
  metadata.packages.find(value => value.id === 'runtime').license = '0BSD'
  const document = buildCustomerSbom({
    cargoMetadata: metadata,
    packageLock,
    version: '1.1.0',
    timestamp: '2026-08-28T00:00:00.000Z',
    target: { platform: 'windows', architecture: 'amd64', rustTarget: 'x86_64-pc-windows-msvc', runtime: 'msvc-static' },
  })
  assert.equal(
    document.components.find(value => value.name === 'runtime-dependency').licenses[0].expression,
    '0BSD',
  )
})

test('customer SBOM rejects a newly introduced reciprocal license pending review', () => {
  const invalid = structuredClone(cargoMetadata)
  invalid.packages.find(value => value.id === 'runtime').license = 'GPL-3.0-only'
  assert.throws(() => buildCustomerSbom({
    cargoMetadata: invalid,
    packageLock,
    version: '1.1.0',
    timestamp: '2026-08-28T00:00:00.000Z',
    target: { platform: 'windows', architecture: 'amd64', rustTarget: 'x86_64-pc-windows-msvc', runtime: 'msvc-static' },
  }), /unapproved license: GPL-3.0-only/)
})

test('customer SBOM rejects ambiguous duplicate package references', () => {
  const invalid = structuredClone(cargoMetadata)
  invalid.packages.push({
    id: 'runtime-from-another-source',
    name: 'runtime-dependency',
    version: '2.0.0',
    source: 'git+https://example.invalid/runtime',
    license: 'MIT',
  })
  invalid.resolve.nodes.find(value => value.id === 'runner').deps.push({
    pkg: 'runtime-from-another-source',
    dep_kinds: [{ kind: null, target: null }],
  })
  invalid.resolve.nodes.push({ id: 'runtime-from-another-source', deps: [] })
  assert.throws(() => buildCustomerSbom({
    cargoMetadata: invalid,
    packageLock,
    version: '1.1.0',
    timestamp: '2026-08-28T00:00:00.000Z',
    target: { platform: 'windows', architecture: 'amd64', rustTarget: 'x86_64-pc-windows-msvc', runtime: 'msvc-static' },
  }), /duplicate component references|conflicts across target graphs/)
})

test('Linux customer SBOM includes the downloadable Windows initializer', () => {
  const clientCargoMetadata = structuredClone(cargoMetadata)
  clientCargoMetadata.packages.push({
    id: 'windows-registry', name: 'windows-registry', version: '0.6.1',
    source: 'registry+https://example.invalid', license: 'MIT', checksum: 'b'.repeat(64),
  })
  clientCargoMetadata.resolve.nodes
    .find(value => value.id === 'asterctl').deps
    .push({ pkg: 'windows-registry', dep_kinds: [{ kind: null, target: 'cfg(windows)' }] })
  clientCargoMetadata.resolve.nodes.push({ id: 'windows-registry', deps: [] })
  const document = buildCustomerSbom({
    cargoMetadata,
    clientCargoMetadata,
    packageLock,
    version: '1.1.0',
    timestamp: '2026-08-28T00:00:00.000Z',
    target: { platform: 'linux', architecture: 'amd64', rustTarget: 'x86_64-unknown-linux-musl', runtime: 'musl-static' },
  })
  assert.ok(document.components.some(value => value.name === 'asterctl'))
  assert.ok(document.components.some(value => value.name === 'windows-registry'))
  assert.equal(document.metadata.properties.find(value => value.name === 'aster:target').value, 'linux-amd64')
})

test('actual customer production dependency graph excludes demo-only tooling', () => {
  for (const root of ['customer/admin', 'customer/member']) {
    assert.equal(actualPackageLock.packages[root].dependencies?.['@aster/demo'], undefined)
    assert.equal(actualPackageLock.packages[root].devDependencies?.['@aster/demo'], '*')
  }
  const names = new Set(collectNpmComponents(actualPackageLock).components.map(value => value.name))
  assert.ok(!names.has('@aster/demo'))
  assert.ok(!names.has('msw'))
})

test('real lock graph catches a demo dependency accidentally promoted to production', () => {
  const invalid = structuredClone(actualPackageLock)
  invalid.packages['customer/admin'].dependencies['@aster/demo'] = '*'
  assert.throws(() => collectNpmComponents(invalid), /component type-fest uses an unapproved license: CC0-1.0/)
})

test('SBOM production storage features follow the platform build matrix', () => {
  assert.deepEqual(customerStorageFeatures('linux'), ['sqlcipher', 'mariadb'])
  assert.deepEqual(customerStorageFeatures('windows'), ['sqlcipher'])
  assert.deepEqual(customerStorageFeatures('macos'), ['sqlcipher'])
  const document = buildCustomerSbom({ cargoMetadata, packageLock, version: '1.1.0', timestamp: '2026-08-28T00:00:00.000Z',
    target: { platform: 'linux', architecture: 'amd64', rustTarget: 'x86_64-unknown-linux-musl', runtime: 'musl-static' } })
  assert.equal(document.metadata.properties.find(property => property.name === 'aster:storage').value, 'sqlcipher,mariadb')
  const builder = readFileSync(new URL('./build-linux-bundle.mjs', import.meta.url), 'utf8')
  assert.ok(builder.includes(`'--features', '${customerStorageFeatures('linux').join(',')}'`))
})
