import assert from 'node:assert/strict'
import test from 'node:test'
import { generatedContentsMatch } from './generate-contracts.mjs'

test('generatedContentsMatch accepts platform line endings without ignoring content changes', () => {
  assert.equal(generatedContentsMatch('first\r\nsecond\r\n', 'first\nsecond\n'), true)
  assert.equal(generatedContentsMatch('first\rsecond\r', 'first\nsecond\n'), true)
  assert.equal(generatedContentsMatch('first\r\nchanged\r\n', 'first\nsecond\n'), false)
})

import { readFileSync } from 'node:fs'
import { parse } from 'yaml'
import { productCatalogOutputs, validateProductCatalog } from './product-capabilities.mjs'
import { publicSchema, websiteContractOutputs } from './website-contracts.mjs'

const sourceCatalog = parse(readFileSync(new URL('../../contracts/catalogs/product-capabilities.yaml', import.meta.url), 'utf8'))
const freshCatalog = () => structuredClone(sourceCatalog)

test('public website generation follows this run of the capability catalog and excludes private definitions', async () => {
  const catalog = freshCatalog()
  catalog.capabilities.push({ id: 'future', label: '测试未来能力', description: '仅测试', feature_set: 'standard', free_default: false, requires: ['member'] })
  const outputs = await websiteContractOutputs(productCatalogOutputs(catalog))
  assert.match(outputs.get('website/shared/generated/contracts.ts'), /"future"/)
  const schema = JSON.stringify(publicSchema('public-catalog.v1.schema.yaml'))
  for (const internal of ['approved_by', 'operation_id', 'current_password', 'expected_sha256', 'transfer_limit']) assert.ok(!schema.includes(internal), internal)
  assert.ok(!outputs.get('website/shared/generated/inquiry-validator.js').includes('require('))
})

test('capability catalog rejects accidental grants, ambiguous IDs and undeclared fields', () => {
  for (const mutate of [
    value => { value.catalog_version = 0 },
    value => { value.catalog_version = 0x1_0000_0000 },
    value => { value.product = 'another-product' },
    value => { value.default_features = ['all'] },
    value => { value.capabilities[0].price = 1 },
    value => { value.capabilities.push(structuredClone(value.capabilities[0])) },
    value => { value.capabilities[0].id = 'Self' },
    value => { value.capabilities[0].id = 'self' },
    value => { value.capabilities[0].id = 'i_d' },
    value => { value.capabilities[0].id = 'a_l_l' },
    value => { value.quotas[0].id = 'i_d' },
    value => { value.quotas[0].id = 'a_l_l' },
    value => { value.quotas[0].id = 'scope' },
    value => { value.quotas[0].id = 'limit' },
    value => { value.quotas[0].id = 'grant' },
    value => { value.capabilities[0].id = 'x__y' },
    value => { value.capabilities[0].label = '' },
    value => { value.capabilities[0].requires = ['missing'] },
    value => { value.capabilities[0].requires = [value.capabilities[1].id, value.capabilities[1].id] },
    value => { value.quotas[0].scope = 'company' },
    value => { value.quotas.push(structuredClone(value.quotas[0])) },
    value => { delete value.quotas[0].scope },
    value => { delete value.operations },
    value => { value.operations = [] },
    value => { value.operations[0].id = 'self' },
    value => { value.operations[0].id = 'i_d' },
    value => { value.operations[0].requires = [] },
    value => { value.operations[0].requires = ['unknown'] },
    value => { value.operations[0].requires = ['gateway', 'gateway'] },
    value => { value.operations[0].label = '' },
    value => { value.operations[0].default_enabled = true },
    value => { value.operations.push(structuredClone(value.operations[0])) },
  ]) {
    const catalog = freshCatalog()
    mutate(catalog)
    assert.throws(() => validateProductCatalog(catalog))
  }
})

test('capability dependencies support a DAG and reject indirect cycles', () => {
  const catalog = freshCatalog()
  catalog.capabilities[0].requires = [catalog.capabilities[1].id]
  catalog.capabilities[1].requires = [catalog.capabilities[2].id]
  assert.equal(validateProductCatalog(catalog), catalog)
  catalog.capabilities[2].requires = [catalog.capabilities[0].id]
  assert.throws(() => validateProductCatalog(catalog), /cycle/)
})

test('a new capability is generated for every consumer without editing a second ID list', () => {
  const catalog = freshCatalog()
  catalog.capabilities.push({ id: 'usage_export', label: '用量导出', description: '导出使用记录', feature_set: 'standard', free_default: false, requires: ['member'] })
  const outputs = productCatalogOutputs(catalog)
  assert.equal(outputs.size, 6)
  for (const [path, contents] of outputs) {
    assert.ok(contents.includes('usage_export'), path)
    assert.ok(contents.includes('member_seats'), path)
    assert.ok(contents.includes('api_keys_per_member'), path)
    assert.ok(!contents.includes('5999'), path)
  }
  const ts = [...outputs].filter(([path]) => path.endsWith('.ts')).map(([, value]) => value)
  assert.equal(new Set(ts).size, 1)
  const changed = freshCatalog()
  changed.capabilities[0].label = '新显示名'
  const oldOutputs = productCatalogOutputs(sourceCatalog)
  for (const [path, contents] of productCatalogOutputs(changed)) {
    if (!path.endsWith('.schema.yaml')) assert.equal(generatedContentsMatch(oldOutputs.get(path), contents), false, path)
  }
})


test('generated entitlement schema rejects omitted, duplicate and ambiguous quota grants', async () => {
  const { default: Ajv2020 } = await import('ajv/dist/2020.js')
  const schema = parse(productCatalogOutputs(sourceCatalog).get('contracts/schemas/entitlements.v1.schema.yaml'))
  const validate = new Ajv2020({ strict: true }).compile(schema)
  const vector = JSON.parse(readFileSync(new URL('../../contracts/test-vectors/entitlements.v1.json', import.meta.url), 'utf8'))
  assert.equal(validate(vector), true)
  for (const change of [
    value => { value.features.push('unknown') },
    value => { value.features.push('member') },
    value => { value.quotas.pop() },
    value => { value.quotas[1].id = value.quotas[0].id },
    value => { value.quotas[0].limit = {} },
    value => { value.quotas[0].limit = { mode: 'unlimited', value: null } },
    value => { value.quotas[0].limit = { mode: 'limited', value: -1 } },
    value => { value.quotas[0].limit = { mode: 'limited', value: 0x1_0000_0000 } },
  ]) {
    const invalid = structuredClone(vector)
    change(invalid)
    assert.equal(validate(invalid), false, JSON.stringify(invalid))
  }
  const empty = structuredClone(vector)
  empty.features = []
  empty.quotas[0].limit = { mode: 'limited', value: 0 }
  assert.equal(validate(empty), true)
  empty.quotas[0].limit = { mode: 'unlimited' }
  assert.equal(validate(empty), true)
  const futureCatalog = freshCatalog()
  futureCatalog.capabilities.push({ id: 'usage_export', label: '用量导出', description: '导出使用记录', feature_set: 'standard', free_default: false, requires: ['member'] })
  const futureSchema = parse(productCatalogOutputs(futureCatalog).get('contracts/schemas/entitlements.v1.schema.yaml'))
  const futureValidate = new Ajv2020({ strict: true }).compile(futureSchema)
  empty.features = ['usage_export']
  assert.equal(futureValidate(empty), false)
  empty.features.push('member')
  assert.equal(futureValidate(empty), true)
})

test('v2 schema requires explicit source, binding, expiry and all signed rights', async () => {
  const { default: Ajv2020 } = await import('ajv/dist/2020.js')
  const ajv = new Ajv2020({ strict: true })
  const entitlements = parse(readFileSync(new URL('../../contracts/schemas/entitlements.v1.schema.yaml', import.meta.url), 'utf8'))
  ajv.addSchema(entitlements, entitlements.$id.replace(/\.json$/, '.yaml'))
  const validate = ajv.compile(parse(readFileSync(new URL('../../contracts/schemas/license.v2.schema.yaml', import.meta.url), 'utf8')))
  const fixture = JSON.parse(readFileSync(new URL('../../contracts/test-vectors/license.v2.json', import.meta.url), 'utf8'))
  for (const item of fixture.cases) assert.equal(validate(item.document), true, item.name)
  for (const change of [
    value => { delete value.claims.binding },
    value => { value.claims.binding = {} },
    value => { value.claims.binding = { mode: 'unbound', installation_id: 'test_installation' } },
    value => { value.claims.source = { kind: 'unknown' } },
    value => { value.claims.validity.expiry = { mode: 'none', expires_at: '2027-09-01T00:00:00.000Z' } },
    value => { value.claims.plan_version = null },
    value => { value.claims.quota_policy_version = 2 },
    value => { value.claims.entitlements.quotas.pop() },
    value => { value.claims.entitlements.quotas[0].limit = { mode: 'unlimited', value: 3 } },
  ]) {
    const invalid = structuredClone(fixture.cases[0].document)
    change(invalid)
    assert.equal(validate(invalid), false, JSON.stringify(invalid))
  }
  const paid = structuredClone(fixture.cases[2].document)
  paid.claims.binding = { mode: 'unbound' }
  assert.equal(validate(paid), false)
  paid.claims.binding = structuredClone(fixture.cases[2].document.claims.binding)
  paid.claims.validity.expiry = { mode: 'none' }
  assert.equal(validate(paid), false)
})

test('commercial wire schema shares capability IDs and separates free, annual and contact offers', async () => {
  const { default: Ajv2020 } = await import('ajv/dist/2020.js')
  const ajv = new Ajv2020({ strict: true })
  for (const name of ['entitlements.v1.schema.yaml', 'commercial.v1.schema.yaml']) {
    const schema = parse(readFileSync(new URL('../../contracts/schemas/' + name, import.meta.url), 'utf8'))
    ajv.addSchema(schema, schema.$id.replace(/\.json$/, '.yaml'))
  }
  const validate = ajv.getSchema('https://aster-team.local/contracts/commercial.v1.schema.json#/$defs/definition')
  const source = JSON.parse(readFileSync(new URL('../../contracts/test-vectors/plan-definition.v1.json', import.meta.url), 'utf8'))
  assert.equal(validate(source), true)
  for (const offer of [{ kind: 'free', expiry: { mode: 'none' } }, { kind: 'free', expiry: { mode: 'fixed', expires_at: '2027-09-06T00:00:00.000Z' } }, { kind: 'contact' }]) {
    assert.equal(validate({ ...source, offer }), true)
  }
  for (const change of [
    value => { value.offer = { kind: 'free' } },
    value => { value.offer = { kind: 'contact', annual_amount_minor: 0 } },
    value => { value.offer.annual_amount_minor = 0 },
    value => { value.offer.annual_amount_minor = 9000000000001 },
    value => { value.offer.terms[0].discount_basis_points = 10001 },
    value => { value.offer.terms[0].years = 0 },
    value => { value.offer.term_timezone = 'unconfigured' },
    value => { value.minimum_version = '1.0.0+' + 'a'.repeat(59) },
    value => { value.entitlements.features.push('invented') },
    value => { value.entitlements.quotas.pop() },
    value => { value.entitlements.quotas[0].limit = { mode: 'unlimited', value: 3 } },
  ]) {
    const invalid = structuredClone(source); change(invalid)
    assert.equal(validate(invalid), false, JSON.stringify(invalid))
  }
})

test('free distribution schema separates reauthentication requests and persisted issuance states', async () => {
  const { default: Ajv2020 } = await import('ajv/dist/2020.js')
  const ajv = new Ajv2020({ strict: true })
  for (const name of ['entitlements.v1.schema.yaml', 'license-trust.v1.schema.yaml', 'license.v2.schema.yaml', 'commercial.v1.schema.yaml', 'free-distribution.v1.schema.yaml']) {
    const schema = parse(readFileSync(new URL('../../contracts/schemas/' + name, import.meta.url), 'utf8'))
    ajv.addSchema(schema, schema.$id.replace(/\.json$/, '.yaml'))
  }
  const validate = ajv.getSchema('https://aster-team.local/contracts/free-distribution.v1.schema.json')
  const approval = ajv.getSchema('https://aster-team.local/contracts/free-distribution.v1.schema.json#/$defs/approval_input')
  const definition = JSON.parse(readFileSync(new URL('../../contracts/test-vectors/plan-definition.v1.json', import.meta.url), 'utf8'))
  const document = JSON.parse(readFileSync(new URL('../../contracts/test-vectors/license.v2.json', import.meta.url), 'utf8')).cases[0].document
  definition.offer = { kind: 'free', expiry: document.claims.validity.expiry }
  definition.entitlements = document.claims.entitlements
  const hash = 'a'.repeat(64)
  const input = { operation_id: 'approval_test', plan_id: 'free_plan', plan_version: 1, expected_sha256: hash, not_before: '2026-09-06T00:00:00.000Z', reason: 'schema test', current_password: 'test-only' }
  assert.equal(approval(input), true, JSON.stringify(approval.errors))
  for (const change of [
    value => { delete value.current_password },
    value => { value.current_password = '' },
    value => { value.current_password = null },
    value => { value.status = 'issued' },
    value => { value.expected_sha256 = 'z'.repeat(64) },
  ]) {
    const invalid = structuredClone(input); change(invalid)
    assert.equal(approval(invalid), false)
  }
  const approved = { snapshot: { schema: 'aster.free-distribution.v1', id: 'dist_test', plan: { schema: 'aster.plan-snapshot.v1', plan_id: 'free_plan', version: 1, definition }, plan_sha256: hash, not_before: input.not_before, reason: input.reason, approved_by: 'operator_test', approved_at: input.not_before }, sha256: hash, operation_id: input.operation_id, status: 'approved' }
  const prepared = { ...approved, status: 'prepared', claims: document.claims }
  const issued = { ...prepared, status: 'issued', document, document_sha256: hash }
  for (const record of [approved, prepared, issued]) assert.equal(validate(record), true, JSON.stringify(validate.errors))
  for (const invalid of [
    { ...approved, current_password: 'must-not-persist' },
    { ...approved, claims: document.claims },
    { ...approved, status: 'prepared' },
    { ...prepared, document },
    { ...prepared, status: 'issued' },
    { ...issued, document_sha256: null },
    { ...issued, status: 'draft' },
  ]) assert.equal(validate(invalid), false, JSON.stringify(invalid))
})


test('v2 installation request schema rejects lost or forged compatibility fields', async () => {
  const { default: Ajv2020 } = await import('ajv/dist/2020.js')
  const ajv = new Ajv2020({ allErrors: true, strict: true })
  for (const name of ['entitlements.v1.schema.yaml', 'license.v2.schema.yaml', 'license-request.v2.schema.yaml']) {
    const schema = parse(readFileSync(new URL(`../../contracts/schemas/${name}`, import.meta.url), 'utf8'))
    ajv.addSchema(schema, schema.$id.replace(/\.json$/, '.yaml'))
  }
  const validate = ajv.getSchema('https://aster-team.local/contracts/license-request.v2.schema.json')
  const fixture = JSON.parse(readFileSync(new URL('../../contracts/test-vectors/license-request.v2.json', import.meta.url), 'utf8'))
  assert.equal(validate(fixture.request), true, ajv.errorsText(validate.errors))
  for (const item of fixture.invalid_raw.filter(item => item.schema_invalid)) {
    assert.equal(validate(JSON.parse(item.raw)), false, item.name)
  }
})

test('business operations generate shared requirements without changing signed entitlement grants', () => {
  const catalog = freshCatalog()
  const before = productCatalogOutputs(catalog)
  catalog.operations.push({ id: 'usage_export', label: '测试导出操作', requires: ['member', 'runner'] })
  const after = productCatalogOutputs(catalog)
  for (const [path, contents] of after) {
    if (path.endsWith('.schema.yaml')) assert.equal(contents, before.get(path))
    else {
      assert.ok(contents.includes('usage_export'), path)
      assert.ok(!generatedContentsMatch(contents, before.get(path)), path)
    }
  }
  assert.deepEqual(catalog.capabilities, sourceCatalog.capabilities)
  assert.deepEqual(catalog.quotas, sourceCatalog.quotas)
})

test('v2 request compatibility follows the generated catalog revision', async () => {
  const { default: Ajv2020 } = await import('ajv/dist/2020.js')
  const catalog = { ...freshCatalog(), catalog_version: 2 }
  const outputs = productCatalogOutputs(catalog)
  const ajv = new Ajv2020({ allErrors: true, strict: true })
  for (const name of ['entitlements.v1.schema.yaml', 'license.v2.schema.yaml', 'license-request.v2.schema.yaml']) {
    const schema = parse(outputs.get(`contracts/schemas/${name}`) ?? readFileSync(new URL(`../../contracts/schemas/${name}`, import.meta.url), 'utf8'))
    ajv.addSchema(schema, schema.$id.replace(/\.json$/, '.yaml'))
  }
  const validate = ajv.getSchema('https://aster-team.local/contracts/license-request.v2.schema.json')
  const { request } = JSON.parse(readFileSync(new URL('../../contracts/test-vectors/license-request.v2.json', import.meta.url), 'utf8'))
  assert.equal(validate({ ...request, capability_catalog_version: 2 }), true, ajv.errorsText(validate.errors))
  assert.equal(validate(request), false)
  assert.equal(validate({ ...request, capability_catalog_version: 2, quota_policy_version: 2 }), false)
})

test('future standard and extension capabilities remain separate in generated runtime projection', async () => {
  const { transpileModule, ModuleKind, ScriptTarget } = await import('typescript')
  const { runInNewContext } = await import('node:vm')
  const catalog = freshCatalog()
  catalog.feature_sets.push({ id: 'audit', label: '审计模块', description: '测试扩展模块' })
  catalog.capabilities.push(
    { id: 'future_standard', label: '标准新增', description: '测试', feature_set: 'standard', free_default: false, requires: ['member'] },
    { id: 'advanced_audit', label: '扩展新增', description: '测试', feature_set: 'audit', free_default: false, requires: ['member'] },
  )
  const outputs = productCatalogOutputs(catalog)
  const source = outputs.get('customer/sdk/src/generated/product-capabilities.ts')
  const context = { exports: {} }
  runInNewContext(transpileModule(source, { compilerOptions: { module: ModuleKind.CommonJS, target: ScriptTarget.ES2022 } }).outputText, context)
  const resolve = value => Array.from(context.exports.effectiveFeatures(value))
  const originalFree = { features: ['gateway', 'member', 'runner'] }
  const originalPaid = { features: [], feature_sets: ['standard'] }
  assert.deepEqual(resolve(originalFree), originalFree.features)
  assert.ok(resolve(originalPaid).includes('future_standard'))
  assert.ok(!resolve(originalPaid).includes('advanced_audit'))
  assert.ok(resolve({ ...originalPaid, feature_sets: ['standard', 'audit'] }).includes('advanced_audit'))
  catalog.capabilities[0].requires = ['advanced_audit']
  assert.throws(() => validateProductCatalog(catalog), /ungranted extension/)
})
