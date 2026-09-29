import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const CUSTOMER_RUST_ROOTS = ['aster-control', 'aster-runner', 'aster-team-cli']
const CUSTOMER_NPM_ROOTS = ['customer/admin', 'customer/member']
const PROPRIETARY_LICENSE = 'LicenseRef-Aster-Proprietary'
const APPROVED_LICENSE_TOKENS = new Set([
  '0BSD', 'AND', 'Apache-2.0', 'BSD-2-Clause', 'BSD-3-Clause', 'BSL-1.0',
  'CDLA-Permissive-2.0', 'ISC', 'LLVM-exception', PROPRIETARY_LICENSE,
  'MIT', 'OR', 'Unicode-3.0', 'Unlicense', 'WITH', 'Zlib',
])

function license(expression, local, componentName) {
  const value = (expression || (local ? PROPRIETARY_LICENSE : ''))
    .replaceAll('MIT/Apache-2.0', 'MIT OR Apache-2.0')
  const component = componentName ? ` ${componentName}` : ''
  if (!value) throw new Error(`SBOM component${component} is missing license metadata`)
  const tokens = value.match(/[A-Za-z0-9][A-Za-z0-9.+-]*/g) || []
  const unsupported = tokens.filter(token => !APPROVED_LICENSE_TOKENS.has(token))
  if (unsupported.length) throw new Error(`SBOM component${component} uses an unapproved license: ${[...new Set(unsupported)].join(', ')}`)
  return [{ expression: value }]
}

function cargoPurl(name, version) {
  return `pkg:cargo/${encodeURIComponent(name)}@${encodeURIComponent(version)}`
}

function npmPurl(name, version) {
  if (name.startsWith('@')) {
    const [scope, packageName] = name.slice(1).split('/')
    if (!scope || !packageName) throw new Error(`Invalid scoped npm package name: ${name}`)
    return `pkg:npm/%40${encodeURIComponent(scope)}/${encodeURIComponent(packageName)}@${encodeURIComponent(version)}`
  }
  return `pkg:npm/${encodeURIComponent(name)}@${encodeURIComponent(version)}`
}

function npmNameFromKey(key) {
  const marker = 'node_modules/'
  const index = key.lastIndexOf(marker)
  if (index >= 0) return key.slice(index + marker.length)
  return key
}

function resolvedNpmKey(packages, fromKey, dependencyName) {
  let directory = fromKey
  while (true) {
    const candidate = directory
      ? `${directory}/node_modules/${dependencyName}`
      : `node_modules/${dependencyName}`
    const record = packages[candidate]
    if (record) {
      if (record.link) {
        const target = String(record.resolved || '').replace(/^\.\//, '')
        if (!packages[target]) throw new Error(`Broken npm workspace link ${candidate} -> ${target}`)
        return target
      }
      return candidate
    }
    const slash = directory.lastIndexOf('/')
    if (slash < 0) directory = ''
    else directory = directory.slice(0, slash)
    if (!directory && candidate === `node_modules/${dependencyName}`) break
  }
  throw new Error(`Unable to resolve production npm dependency ${dependencyName} from ${fromKey}`)
}

function dependencyNames(record) {
  const names = new Set([
    ...Object.keys(record.dependencies || {}),
    ...Object.keys(record.optionalDependencies || {}),
  ])
  for (const name of Object.keys(record.peerDependencies || {})) {
    if (!record.peerDependenciesMeta?.[name]?.optional) names.add(name)
  }
  return [...names].sort()
}

function npmHash(integrity) {
  const match = /^(sha256|sha384|sha512)-([A-Za-z0-9+/=]+)$/.exec(integrity || '')
  if (!match) return undefined
  return [{ alg: match[1].toUpperCase().replace('SHA', 'SHA-'), content: Buffer.from(match[2], 'base64').toString('hex') }]
}

export function collectNpmComponents(packageLock) {
  const packages = packageLock.packages || {}
  const queue = [...CUSTOMER_NPM_ROOTS]
  const visited = new Set()
  const names = new Map()
  const graph = new Map()
  for (const key of CUSTOMER_NPM_ROOTS) names.set(key, packages[key]?.name || key)

  while (queue.length) {
    const key = queue.shift()
    if (visited.has(key)) continue
    const record = packages[key]
    if (!record) throw new Error(`Missing npm package-lock entry: ${key}`)
    visited.add(key)
    const dependencies = []
    for (const dependencyName of dependencyNames(record)) {
      const dependencyKey = resolvedNpmKey(packages, key, dependencyName)
      names.set(dependencyKey, packages[dependencyKey]?.name || dependencyName)
      dependencies.push(dependencyKey)
      if (!visited.has(dependencyKey)) queue.push(dependencyKey)
    }
    graph.set(key, dependencies)
  }

  const refs = new Map()
  const components = [...visited].map(key => {
    const record = packages[key]
    const name = names.get(key) || record.name || npmNameFromKey(key)
    const version = record.version
    if (!version) throw new Error(`npm SBOM component ${name} has no locked version`)
    const purl = npmPurl(name, version)
    refs.set(key, purl)
    const component = {
      type: CUSTOMER_NPM_ROOTS.includes(key) ? 'application' : 'library',
      'bom-ref': purl,
      name,
      version,
      licenses: license(record.license, !key.startsWith('node_modules/'), name),
      purl,
      properties: [{ name: 'aster:ecosystem', value: 'npm' }],
    }
    const hashes = npmHash(record.integrity)
    if (hashes) component.hashes = hashes
    return component
  })
  const dependencies = [...visited].map(key => ({
    ref: refs.get(key),
    dependsOn: (graph.get(key) || []).map(value => refs.get(value)).sort(),
  }))
  return {
    components,
    dependencies,
    roots: CUSTOMER_NPM_ROOTS.map(key => refs.get(key)),
  }
}

export function collectCargoComponents(metadata, rootNames = CUSTOMER_RUST_ROOTS) {
  const rustRoots = new Set(rootNames)
  const packages = new Map(metadata.packages.map(value => [value.id, value]))
  const nodes = new Map((metadata.resolve?.nodes || []).map(value => [value.id, value]))
  const rootIDs = metadata.packages
    .filter(value => rustRoots.has(value.name) && value.source === null)
    .map(value => value.id)
  if (rootIDs.length !== rustRoots.size) throw new Error('Cargo metadata is missing a customer runtime root')

  const queue = [...rootIDs]
  const visited = new Set()
  const graph = new Map()
  while (queue.length) {
    const id = queue.shift()
    if (visited.has(id)) continue
    visited.add(id)
    const node = nodes.get(id)
    if (!node) throw new Error(`Cargo resolve graph is missing ${id}`)
    const dependencies = node.deps
      .filter(dependency => dependency.dep_kinds.some(kind => kind.kind !== 'dev'))
      .map(dependency => dependency.pkg)
    graph.set(id, dependencies)
    for (const dependency of dependencies) if (!visited.has(dependency)) queue.push(dependency)
  }

  const refs = new Map()
  const components = [...visited].map(id => {
    const value = packages.get(id)
    if (!value) throw new Error(`Cargo package metadata is missing ${id}`)
    const purl = cargoPurl(value.name, value.version)
    refs.set(id, purl)
    const component = {
      type: rustRoots.has(value.name) && value.source === null ? 'application' : 'library',
      'bom-ref': purl,
      name: value.name,
      version: value.version,
      licenses: license(value.license, value.source === null, value.name),
      purl,
      properties: [{ name: 'aster:ecosystem', value: 'cargo' }],
    }
    if (value.checksum) component.hashes = [{ alg: 'SHA-256', content: value.checksum }]
    return component
  })
  const dependencies = [...visited].map(id => ({
    ref: refs.get(id),
    dependsOn: (graph.get(id) || []).map(value => refs.get(value)).sort(),
  }))
  return {
    components,
    dependencies,
    roots: rootIDs.map(id => refs.get(id)),
  }
}

function mergeCargoGraphs(graphs) {
  const components = new Map()
  const dependencies = new Map()
  const roots = new Set()
  for (const graph of graphs) {
    for (const component of graph.components) {
      const reference = component['bom-ref']
      const existing = components.get(reference)
      if (existing && JSON.stringify(existing) !== JSON.stringify(component)) {
        throw new Error(`SBOM Cargo component conflicts across target graphs: ${reference}`)
      }
      components.set(reference, component)
    }
    for (const dependency of graph.dependencies) {
      const targets = dependencies.get(dependency.ref) || new Set()
      for (const target of dependency.dependsOn) targets.add(target)
      dependencies.set(dependency.ref, targets)
    }
    for (const root of graph.roots) roots.add(root)
  }
  return {
    components: [...components.values()],
    dependencies: [...dependencies].map(([ref, dependsOn]) => ({ ref, dependsOn: [...dependsOn].sort() })),
    roots: [...roots],
  }
}

export function customerStorageFeatures(platform) {
  return platform === 'linux' ? ['sqlcipher', 'mariadb'] : ['sqlcipher']
}

function validateTarget(target) {
  const required = ['platform', 'architecture', 'rustTarget', 'runtime']
  for (const field of required) {
    if (!/^[a-z0-9][a-z0-9._-]*$/.test(target?.[field] || '')) {
      throw new Error(`SBOM target ${field} is invalid`)
    }
  }
  if (!['linux', 'windows', 'macos'].includes(target.platform)) throw new Error('SBOM target platform is unsupported')
  if (!['amd64', 'arm64'].includes(target.architecture)) throw new Error('SBOM target architecture is unsupported')
  if (target.platform === 'windows' && target.architecture !== 'amd64') {
    throw new Error('SBOM target Windows architecture is unsupported')
  }
}

export function buildCustomerSbom({ cargoMetadata, clientCargoMetadata = cargoMetadata, packageLock, version, timestamp, target }) {
  if (!/^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) throw new Error('SBOM version is invalid')
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/.test(timestamp)) throw new Error('SBOM timestamp is invalid')
  validateTarget(target)
  const cargo = target.platform === 'macos'
    ? collectCargoComponents(cargoMetadata, CUSTOMER_RUST_ROOTS)
    : mergeCargoGraphs([
        collectCargoComponents(cargoMetadata, CUSTOMER_RUST_ROOTS),
        collectCargoComponents(clientCargoMetadata, ['asterctl']),
      ])
  const npm = collectNpmComponents(packageLock)
  const caddyRuntime = JSON.parse(readFileSync(resolve(root, 'tools/caddy-runtime.json'), 'utf8'))
  const caddyHashKey = `${target.platform}_${target.architecture}_sha512`
  const caddyHash = caddyRuntime[caddyHashKey]
  if (!/^\d+\.\d+\.\d+$/.test(caddyRuntime.version) || !/^[0-9a-f]{128}$/.test(caddyHash || '')) {
    throw new Error('tools/caddy-runtime.json is invalid')
  }
  const caddyRef = `pkg:github/caddyserver/caddy@${encodeURIComponent(caddyRuntime.version)}`
  const caddy = {
    type: 'application',
    'bom-ref': caddyRef,
    name: 'Caddy',
    version: caddyRuntime.version,
    licenses: [{ expression: 'Apache-2.0' }],
    purl: caddyRef,
    hashes: [{ alg: 'SHA-512', content: caddyHash }],
    properties: [{ name: 'aster:ecosystem', value: 'third-party-runtime' }],
  }
  const productRef = `pkg:generic/aster-team-customer@${encodeURIComponent(version)}?arch=${target.architecture}&os=${target.platform}`
  const components = [...cargo.components, ...npm.components, caddy].sort((left, right) => left['bom-ref'].localeCompare(right['bom-ref']))
  const componentRefs = components.map(value => value['bom-ref'])
  const duplicateRefs = componentRefs.filter((value, index) => componentRefs.indexOf(value) !== index)
  if (duplicateRefs.length) {
    throw new Error(`SBOM contains duplicate component references: ${[...new Set(duplicateRefs)].join(', ')}`)
  }
  const dependencies = [...cargo.dependencies, ...npm.dependencies]
    .sort((left, right) => left.ref.localeCompare(right.ref))
  const knownRefs = new Set(componentRefs)
  for (const dependency of dependencies) {
    if (!knownRefs.has(dependency.ref) || dependency.dependsOn.some(value => !knownRefs.has(value))) {
      throw new Error(`SBOM dependency graph references an unknown component: ${dependency.ref}`)
    }
  }
  dependencies.push({ ref: caddyRef, dependsOn: [] })
  dependencies.sort((left, right) => left.ref.localeCompare(right.ref))
  dependencies.unshift({ ref: productRef, dependsOn: [...cargo.roots, ...npm.roots, caddyRef].sort() })
  return {
    $schema: 'https://cyclonedx.org/schema/bom-1.6.schema.json',
    bomFormat: 'CycloneDX',
    specVersion: '1.6',
    version: 1,
    metadata: {
      timestamp,
      component: {
        type: 'application',
        'bom-ref': productRef,
        name: 'aster-team-customer',
        version,
        licenses: [{ expression: PROPRIETARY_LICENSE }],
        purl: productRef,
      },
      properties: [
        { name: 'aster:target', value: `${target.platform}-${target.architecture}` },
        { name: 'aster:rust-target', value: target.rustTarget },
        { name: 'aster:runtime', value: target.runtime },
        { name: 'aster:storage', value: customerStorageFeatures(target.platform).join(',') },
      ],
    },
    components,
    dependencies,
  }
}

function argument(name) {
  return process.argv.find(value => value.startsWith(`${name}=`))?.slice(name.length + 1) || ''
}

function currentCustomerSbom(version, timestamp, target) {
  const cargo = process.platform === 'win32' ? 'cargo.exe' : 'cargo'
  const metadata = (rustTarget, platform) => JSON.parse(execFileSync(cargo, [
    'metadata', '--locked', '--format-version', '1',
    '--filter-platform', rustTarget,
    '--no-default-features', '--features', customerStorageFeatures(platform).map(feature => `aster-control/${feature}`).join(','),
  ], { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }))
  const cargoMetadata = metadata(target.rustTarget, target.platform)
  const clientCargoMetadata = target.platform === 'macos'
    ? cargoMetadata
    : metadata('x86_64-pc-windows-msvc', 'windows')
  const packageLock = JSON.parse(readFileSync(resolve(root, 'package-lock.json'), 'utf8'))
  return buildCustomerSbom({ cargoMetadata, clientCargoMetadata, packageLock, version, timestamp, target })
}

function targetFromArguments(defaultToLinux = false) {
  const target = {
    platform: argument('--platform'),
    architecture: argument('--architecture'),
    rustTarget: argument('--rust-target'),
    runtime: argument('--runtime'),
  }
  if (defaultToLinux && Object.values(target).every(value => !value)) {
    return {
      platform: 'linux',
      architecture: 'amd64',
      rustTarget: 'x86_64-unknown-linux-musl',
      runtime: 'musl-static',
    }
  }
  validateTarget(target)
  return target
}

function main() {
  if (process.argv.includes('--check')) {
    const packageManifest = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf8'))
    const document = currentCustomerSbom(
      packageManifest.version,
      '1970-01-01T00:00:00.000Z',
      targetFromArguments(true),
    )
    const names = new Set(document.components.map(component => component.name))
    const storage = document.metadata.properties.find(property => property.name === 'aster:storage')?.value
    if (storage === 'sqlcipher,mariadb' && !names.has('sqlx-mysql')) {
      throw new Error('Linux dual-driver SBOM is missing the MariaDB driver dependency')
    }
    console.log(`Customer SBOM preflight passed (${document.components.length} production components)`)
    return
  }
  const output = argument('--output')
  const version = argument('--version')
  if (!output || !version) {
    throw new Error('Usage: node scripts/generate-customer-sbom.mjs --output=FILE --version=VERSION --platform=PLATFORM --architecture=ARCH --rust-target=TRIPLE --runtime=RUNTIME')
  }
  const sourceDateEpoch = Number(process.env.SOURCE_DATE_EPOCH || '')
  if (!Number.isSafeInteger(sourceDateEpoch) || sourceDateEpoch < 0) throw new Error('SOURCE_DATE_EPOCH is required for deterministic SBOM generation')
  const document = currentCustomerSbom(
    version,
    new Date(sourceDateEpoch * 1000).toISOString(),
    targetFromArguments(),
  )
  const target = resolve(output)
  if (existsSync(target)) throw new Error(`Refusing to overwrite SBOM: ${target}`)
  writeFileSync(target, `${JSON.stringify(document, null, 2)}\n`, { flag: 'wx' })
  console.log(`Customer CycloneDX SBOM: ${target} (${document.components.length} components)`)
}

if (resolve(process.argv[1] || '') === fileURLToPath(import.meta.url)) main()
