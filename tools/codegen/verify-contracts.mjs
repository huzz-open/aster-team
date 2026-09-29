import { execFileSync } from 'node:child_process'
import { readFileSync, readdirSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import Ajv2020 from 'ajv/dist/2020.js'
import addFormats from 'ajv-formats'
import { parse as parseYaml } from 'yaml'

const root = resolve(fileURLToPath(new URL('../..', import.meta.url)))
const failures = []

function loadYaml(relativePath) {
  return parseYaml(readFileSync(resolve(root, relativePath), 'utf8'))
}

const schemaFiles = readdirSync(resolve(root, 'contracts/schemas'))
  .filter(name => name.endsWith('.schema.yaml'))
  .sort()
const schemas = new Map(schemaFiles.map(name => [name, loadYaml(`contracts/schemas/${name}`)]))
const ajv = new Ajv2020({ allErrors: true, strict: true })
addFormats(ajv, ['date-time'])
for (const [name, schema] of schemas) {
  try {
    ajv.addSchema(schema, schema.$id.replace(/\.json$/, '.yaml'))
  } catch (error) {
    failures.push(`contracts/schemas/${name}: ${error.message}`)
  }
}

// addSchema is lazy. Compile every root after all references are registered so
// a new contract without a test vector cannot silently skip strict validation.
for (const [name, schema] of schemas) {
  try {
    if (!ajv.getSchema(schema.$id)) failures.push(`contracts/schemas/${name}: schema did not compile`)
  } catch (error) {
    failures.push(`contracts/schemas/${name}: ${error.message}`)
  }
}

function validate(schemaName, value, source, fragment = '') {
  const schema = schemas.get(schemaName)
  if (!schema) {
    failures.push(`missing schema ${schemaName}`)
    return
  }
  const validator = ajv.getSchema(schema.$id + fragment)
  if (!validator) {
    failures.push(`${schemaName}: schema did not compile`)
    return
  }
  if (!validator(value)) failures.push(`${source}: ${ajv.errorsText(validator.errors, { separator: '; ' })}`)
}

const planDefinitionVector = JSON.parse(readFileSync(resolve(root, 'contracts/test-vectors/plan-definition.v1.json'), 'utf8'))
validate('public-catalog.v1.schema.yaml', JSON.parse(readFileSync(resolve(root, 'contracts/test-vectors/public-catalog.v1.json'), 'utf8')), 'public-catalog.v1 actual local export vector')
validate('commercial.v1.schema.yaml', planDefinitionVector, 'plan-definition.v1 test vector', '#/$defs/definition')
const licenseVector = JSON.parse(readFileSync(resolve(root, 'contracts/test-vectors/license.v1.json'), 'utf8'))
validate('license.v1.schema.yaml', licenseVector.document, 'contracts/test-vectors/license.v1.json document')
const entitlementsVector = JSON.parse(readFileSync(resolve(root, 'contracts/test-vectors/entitlements.v1.json'), 'utf8'))
validate('entitlements.v1.schema.yaml', entitlementsVector, 'contracts/test-vectors/entitlements.v1.json')
const licenseV2Vector = JSON.parse(readFileSync(resolve(root, 'contracts/test-vectors/license.v2.json'), 'utf8'))
for (const item of licenseV2Vector.cases) validate('license.v2.schema.yaml', item.document, `license.v2 vector ${item.name}`)
const requestV2Vector = JSON.parse(readFileSync(resolve(root, 'contracts/test-vectors/license-request.v2.json'), 'utf8'))
validate('license-request.v2.schema.yaml', requestV2Vector.request, 'license-request.v2 shared request')
const licenseTrustVector = JSON.parse(readFileSync(resolve(root, 'contracts/test-vectors/license-trust.v1.json'), 'utf8'))
validate('license-trust.v1.schema.yaml', licenseTrustVector.keyring, 'license-trust.v1 test vector')
const runnerVector = JSON.parse(readFileSync(resolve(root, 'contracts/test-vectors/runner-task.v2.json'), 'utf8'))
validate('runner-task.v2.schema.yaml', runnerVector.ticket, 'contracts/test-vectors/runner-task.v2.json ticket')
execFileSync(process.execPath, [resolve(root, 'tools/codegen/generate-runner-task-vector.mjs'), '--check'], { stdio: 'pipe' })
const runnerV3Vector = JSON.parse(readFileSync(resolve(root, 'contracts/test-vectors/runner-task.v3.json'), 'utf8'))
validate('runner-task.v3.schema.yaml', runnerV3Vector.ticket, 'contracts/test-vectors/runner-task.v3.json ticket')
for (const item of runnerV3Vector.cases) validate('runner-task.v3.schema.yaml', item.ticket, `runner-task.v3 ${item.name}`)
execFileSync(process.execPath, [resolve(root, 'scripts/generate-runner-task-vector-v4.mjs'), '--check'], { stdio: 'pipe' })
const runnerV4Vector = JSON.parse(readFileSync(resolve(root, 'contracts/test-vectors/runner-task.v4.json'), 'utf8'))
validate('runner-task.v4.schema.yaml', runnerV4Vector.ticket, 'contracts/test-vectors/runner-task.v4.json ticket')
for (const item of runnerV4Vector.cases) validate('runner-task.v4.schema.yaml', item.ticket, `runner-task.v4 ${item.name}`)

for (const [schema, implementation, marker] of [
  ['license.v2.schema.yaml', 'packages/rust/license-core/src/v2.rs', 'aster.license.v2'],
  ['license.v1.schema.yaml', 'packages/rust/license-core/src/lib.rs', 'aster.license.v1'],
  ['license-request.v2.schema.yaml', 'packages/rust/license-core/src/request_v2.rs', 'aster.license-request.v2'],
  ['license-request.v1.schema.yaml', 'packages/rust/license-core/src/lib.rs', 'aster.license-request.v1'],
  ['runner-task.v4.schema.yaml', 'customer/backend/crates/runner-protocol/src/lib.rs', 'aster.runner-task.v4'],
  ['release-manifest.v1.schema.yaml', 'packages/rust/release-core/src/lib.rs', 'aster.release-manifest.v1'],
]) {
  if (!readFileSync(resolve(root, implementation), 'utf8').includes(marker)) {
    failures.push(`${implementation}: missing protocol marker from ${schema}`)
  }
}

function verifySQLLayout(moduleName, dialects) {
  const schemaRoot = resolve(root, moduleName, 'schema')
  const entries = readdirSync(schemaRoot, { withFileTypes: true })
  const expectedInit = new Set(dialects.map(dialect => `init.${dialect}.sql`))
  for (const entry of entries) {
    if (entry.isFile()) {
      if (entry.name.endsWith('.sql') && !expectedInit.delete(entry.name)) {
        failures.push(`${moduleName}/schema/${entry.name}: only init.<dialect>.sql may be stored at schema root`)
      }
      continue
    }
    if (!entry.isDirectory() || !/^\d{12}$/.test(entry.name)) {
      failures.push(`${moduleName}/schema/${entry.name}: change directories must use YYYYMMDDHHmm`)
      continue
    }
    for (const child of readdirSync(resolve(schemaRoot, entry.name), { withFileTypes: true })) {
      const dialectPattern = dialects.join('|')
      if (!child.isFile() || !new RegExp(`^[a-z][a-z0-9_]*\\.(${dialectPattern})\\.sql$`).test(child.name)) {
        failures.push(`${moduleName}/schema/${entry.name}/${child.name}: invalid dated SQL change filename`)
      }
    }
  }
  for (const missing of expectedInit) failures.push(`${moduleName}/schema/${missing}: required initialization baseline is missing`)
}

verifySQLLayout('customer/backend', ['mariadb', 'sqlcipher'])
verifySQLLayout('operations/backend', ['mariadb'])

const websiteSchemaRoot = resolve(root, 'website/server/schema')
const websiteMigrations = readdirSync(websiteSchemaRoot, { withFileTypes: true })
for (const entry of websiteMigrations) {
  if (!entry.isFile() || !/^\d{4}_[a-z][a-z0-9_]*\.sql$/.test(entry.name)) {
    failures.push(`website/server/schema/${entry.name}: D1 migrations must use NNNN_description.sql filenames`)
  }
}
if (websiteMigrations.length === 0) failures.push('website/server/schema: at least one D1 migration is required')

const trackedSQL = execFileSync('git', ['ls-files', '*.sql'], { cwd: root, encoding: 'utf8' })
  .trim().split(/\r?\n/).filter(Boolean)
for (const path of trackedSQL) {
  if (!path.startsWith('customer/backend/schema/') && !path.startsWith('operations/backend/schema/') && !path.startsWith('website/server/schema/')) {
    failures.push(`${path}: SQL must live in its business module schema directory`)
  }
}

const operationsEmbed = readFileSync(resolve(root, 'operations/backend/schema/files.go'), 'utf8')
for (const entry of readdirSync(resolve(root, 'operations/backend/schema'), { withFileTypes: true })) {
  if (entry.isDirectory() && /^\d{12}$/.test(entry.name) && !operationsEmbed.includes(`${entry.name}/*.mariadb.sql`)) {
    failures.push(`operations/backend/schema/files.go: dated change ${entry.name} is not embedded`)
  }
}

if (failures.length) {
  console.error(failures.join('\n'))
  process.exit(1)
}
console.log(`${schemaFiles.length} public schemas, ${trackedSQL.length} SQL baselines, and cross-language vectors verified`)
