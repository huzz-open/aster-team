import { stringify } from 'yaml'

const identifier = /^[a-z][a-z0-9]*(?:_[a-z0-9]+)*$/
const symbol = id => id.split('_').map(part => part[0].toUpperCase() + part.slice(1)).join('')
const quote = value => JSON.stringify(value)

function exactFields(value, fields, path) {
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || Object.keys(value).length !== fields.length
    || fields.some(field => !Object.hasOwn(value, field))) {
    throw new Error(`${path}: fields must be exactly ${fields.join(', ')}`)
  }
}

function text(value, path) {
  if (typeof value !== 'string' || !value.trim() || value !== value.trim() || value.length > 256 || /[\x00-\x1f]/.test(value)) {
    throw new Error(`${path}: expected non-empty trimmed text (max 256 characters)`)
  }
}

export function validateProductCatalog(catalog) {
  exactFields(catalog, ['catalog_version', 'product', 'feature_sets', 'capabilities', 'operations', 'quotas'], 'catalog')
  if (!Number.isSafeInteger(catalog.catalog_version) || catalog.catalog_version < 1 || catalog.catalog_version > 0xffff_ffff) throw new Error('catalog_version must be a positive integer')
  if (catalog.product !== 'aster-team') throw new Error('unsupported product')
  if (!Array.isArray(catalog.feature_sets) || !catalog.feature_sets.length || catalog.feature_sets.length > 64) throw new Error('feature_sets: expected 1–64 entries')
  const featureSets = new Set()
  for (const entry of catalog.feature_sets) {
    exactFields(entry, ['id', 'label', 'description'], 'feature_set')
    if (typeof entry.id !== 'string' || !identifier.test(entry.id) || entry.id.length > 64 || ['Self', 'ALL', 'ID'].includes(symbol(entry.id)) || featureSets.has(entry.id)) throw new Error('invalid or duplicate feature set')
    text(entry.label, 'feature_set.label')
    text(entry.description, 'feature_set.description')
    featureSets.add(entry.id)
  }
  if (!featureSets.has('standard')) throw new Error('standard feature set is required')
  for (const collection of ['capabilities', 'quotas']) {
    const entries = catalog[collection]
    if (!Array.isArray(entries) || !entries.length || entries.length > 64) throw new Error(`${collection}: expected 1–64 entries`)
    const ids = new Set()
    const symbols = new Set()
    for (const [index, entry] of entries.entries()) {
      const path = `${collection}[${index}]`
      exactFields(entry, collection === 'capabilities' ? ['id', 'label', 'description', 'feature_set', 'free_default', 'requires'] : ['id', 'label', 'unit', 'scope'], path)
      if (typeof entry.id !== 'string' || !identifier.test(entry.id) || entry.id.length > 64) throw new Error(`${path}: invalid ID`)
      const reserved = collection === 'capabilities' ? ['Self', 'ALL', 'ID'] : ['Self', 'ALL', 'ID', 'Scope', 'Limit', 'Grant']
      if (reserved.includes(symbol(entry.id))) throw new Error(`${path}: ID collides with a generated type or member`)
      if (ids.has(entry.id) || symbols.has(symbol(entry.id))) throw new Error(`${path}: duplicate ID or generated symbol`)
      ids.add(entry.id)
      symbols.add(symbol(entry.id))
      text(entry.label, `${path}.label`)
      if (collection === 'capabilities') {
        if (!featureSets.has(entry.feature_set) || typeof entry.free_default !== 'boolean') throw new Error(`${path}: invalid feature classification`)
        text(entry.description, `${path}.description`)
        if (!Array.isArray(entry.requires) || entry.requires.some(id => typeof id !== 'string' || !identifier.test(id))
          || new Set(entry.requires).size !== entry.requires.length) throw new Error(`${path}.requires: invalid or duplicate dependencies`)
      } else {
        text(entry.unit, `${path}.unit`)
        if (!['installation', 'member'].includes(entry.scope)) throw new Error(`${path}.scope: invalid scope`)
      }
    }
  }
  const capabilities = new Map(catalog.capabilities.map(entry => [entry.id, entry]))
  const visiting = new Set()
  const visited = new Set()
  const visit = id => {
    if (!capabilities.has(id)) throw new Error(`unknown capability dependency ${id}`)
    if (visiting.has(id)) throw new Error(`capability dependency cycle at ${id}`)
    if (visited.has(id)) return
    visiting.add(id)
    for (const dependency of capabilities.get(id).requires) {
      visit(dependency)
      const entry = capabilities.get(id), required = capabilities.get(dependency)
      if (required.feature_set !== 'standard' && required.feature_set !== entry.feature_set) throw new Error('feature set depends on an ungranted extension')
      if (entry.free_default && !required.free_default) throw new Error('free default depends on a non-free feature')
    }
    visiting.delete(id)
    visited.add(id)
  }
  for (const id of capabilities.keys()) visit(id)
  if (!Array.isArray(catalog.operations) || !catalog.operations.length || catalog.operations.length > 128) throw new Error('operations: expected 1–128 entries')
  const operationIds = new Set()
  const operationSymbols = new Set()
  for (const [index, operation] of catalog.operations.entries()) {
    const path = `operations[${index}]`
    exactFields(operation, ['id', 'label', 'requires'], path)
    if (typeof operation.id !== 'string' || !identifier.test(operation.id) || operation.id.length > 64
      || ['Self', 'ALL', 'ID'].includes(symbol(operation.id))) throw new Error(`${path}: invalid or reserved ID`)
    if (operationIds.has(operation.id) || operationSymbols.has(symbol(operation.id))) throw new Error(`${path}: duplicate ID or generated symbol`)
    operationIds.add(operation.id)
    operationSymbols.add(symbol(operation.id))
    text(operation.label, `${path}.label`)
    if (!Array.isArray(operation.requires) || !operation.requires.length
      || operation.requires.some(id => !capabilities.has(id))
      || new Set(operation.requires).size !== operation.requires.length) throw new Error(`${path}: unknown, missing or duplicate capability requirements`)
  }
  return catalog
}

function rustEnum(name, entries) {
  return `#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize)]
pub enum ${name} {
${entries.map(entry => `    #[serde(rename = ${quote(entry.id)})]\n    ${symbol(entry.id)},`).join('\n')}
}
impl<'de> Deserialize<'de> for ${name} {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(|_| serde::de::Error::custom("unknown ${name}"))
    }
}
impl ${name} {
    pub const ALL: &'static [Self] = &[${entries.map(entry => `Self::${symbol(entry.id)}`).join(', ')}];
    pub const fn as_str(self) -> &'static str {
        match self { ${entries.map(entry => `Self::${symbol(entry.id)} => ${quote(entry.id)},`).join('\n')} }
    }
}
impl std::str::FromStr for ${name} {
    type Err = UnknownCatalogId;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value { ${entries.map(entry => `${quote(entry.id)} => Ok(Self::${symbol(entry.id)}),`).join('\n')}
            _ => Err(UnknownCatalogId),
        }
    }
}
`
}

function rustSource(catalog) {
  return `// @generated by tools/codegen/generate-contracts.mjs; DO NOT EDIT.
use serde::{Deserialize, Serialize};
pub const CATALOG_VERSION: u32 = ${catalog.catalog_version};
pub const PRODUCT: &str = ${quote(catalog.product)};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnknownCatalogId;
${rustEnum('FeatureSetId', catalog.feature_sets)}
${rustEnum('CapabilityId', catalog.capabilities)}
${rustEnum('QuotaId', catalog.quotas)}
${rustEnum('BusinessOperationId', catalog.operations)}
impl BusinessOperationId {
    pub const fn required_capabilities(self) -> &'static [CapabilityId] {
        match self {
${catalog.operations.map(entry => `Self::${symbol(entry.id)} => &[${entry.requires.map(id => `CapabilityId::${symbol(id)}`).join(', ')}],`).join('\n')}
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaScope { Installation, Member }
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct CapabilityDescriptor {
    pub id: CapabilityId,
    pub label: &'static str,
    pub description: &'static str,
    pub feature_set: FeatureSetId,
    pub free_default: bool,
    pub requires: &'static [CapabilityId],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct QuotaDescriptor {
    pub id: QuotaId,
    pub label: &'static str,
    pub unit: &'static str,
    pub scope: QuotaScope,
}
pub const CAPABILITIES: &[CapabilityDescriptor] = &[
${catalog.capabilities.map(entry => `CapabilityDescriptor { id: CapabilityId::${symbol(entry.id)}, label: ${quote(entry.label)}, description: ${quote(entry.description)}, feature_set: FeatureSetId::${symbol(entry.feature_set)}, free_default: ${entry.free_default}, requires: &[${entry.requires.map(id => `CapabilityId::${symbol(id)}`).join(', ')}] },`).join('\n')}
];
pub const QUOTAS: &[QuotaDescriptor] = &[
${catalog.quotas.map(entry => `QuotaDescriptor { id: QuotaId::${symbol(entry.id)}, label: ${quote(entry.label)}, unit: ${quote(entry.unit)}, scope: QuotaScope::${symbol(entry.scope)} },`).join('\n')}
];
`
}

function goSource(catalog) {
  return `// Code generated by tools/codegen/generate-contracts.mjs; DO NOT EDIT.
package productcatalog
const Version uint32 = ${catalog.catalog_version}
const Product = ${quote(catalog.product)}
type FeatureSetID string
type CapabilityID string
type QuotaID string
type QuotaScope string
type BusinessOperationID string
const (
${catalog.feature_sets.map(entry => `FeatureSet${symbol(entry.id)} FeatureSetID = ${quote(entry.id)}`).join('\n')}
${catalog.capabilities.map(entry => `Capability${symbol(entry.id)} CapabilityID = ${quote(entry.id)}`).join('\n')}
${catalog.quotas.map(entry => `Quota${symbol(entry.id)} QuotaID = ${quote(entry.id)}`).join('\n')}
${catalog.operations.map(entry => `BusinessOperation${symbol(entry.id)} BusinessOperationID = ${quote(entry.id)}`).join('\n')}
ScopeInstallation QuotaScope = "installation"
ScopeMember QuotaScope = "member"
)
type Capability struct {
ID CapabilityID \x60json:"id"\x60
Label string \x60json:"label"\x60
Description string \x60json:"description"\x60
FeatureSet FeatureSetID \x60json:"feature_set"\x60
FreeDefault bool \x60json:"free_default"\x60
Requires []CapabilityID \x60json:"requires"\x60
}
type Quota struct {
ID QuotaID \x60json:"id"\x60
Label string \x60json:"label"\x60
Unit string \x60json:"unit"\x60
Scope QuotaScope \x60json:"scope"\x60
}
type BusinessOperation struct {
ID BusinessOperationID \x60json:"id"\x60
Label string \x60json:"label"\x60
Requires []CapabilityID \x60json:"requires"\x60
}
func BusinessOperations() []BusinessOperation {
return []BusinessOperation{
${catalog.operations.map(entry => `{ID: BusinessOperation${symbol(entry.id)}, Label: ${quote(entry.label)}, Requires: []CapabilityID{${entry.requires.map(id => `Capability${symbol(id)}`).join(', ')}}},`).join('\n')}
}
}
// Return fresh slices so callers cannot mutate the catalog used by later requests.
func Capabilities() []Capability {
return []Capability{
${catalog.capabilities.map(entry => `{ID: Capability${symbol(entry.id)}, Label: ${quote(entry.label)}, Description: ${quote(entry.description)}, FeatureSet: FeatureSet${symbol(entry.feature_set)}, FreeDefault: ${entry.free_default}, Requires: []CapabilityID{${entry.requires.map(id => `Capability${symbol(id)}`).join(', ')}}},`).join('\n')}
}
}
func Quotas() []Quota {
return []Quota{
${catalog.quotas.map(entry => `{ID: Quota${symbol(entry.id)}, Label: ${quote(entry.label)}, Unit: ${quote(entry.unit)}, Scope: Scope${symbol(entry.scope)}},`).join('\n')}
}
}
func FeatureSets() []FeatureSetID { return []FeatureSetID{${catalog.feature_sets.map(entry => `FeatureSet${symbol(entry.id)}`).join(', ')}} }
func FindCapability(id CapabilityID) (Capability, bool) {
for _, entry := range Capabilities() { if entry.ID == id { return entry, true } }
return Capability{}, false
}
func FindQuota(id QuotaID) (Quota, bool) {
for _, entry := range Quotas() { if entry.ID == id { return entry, true } }
return Quota{}, false
}
`
}

function typeScriptSource(catalog) {
  return `// @generated by tools/codegen/generate-contracts.mjs; DO NOT EDIT.
export const CAPABILITY_CATALOG_VERSION = ${catalog.catalog_version} as const
export const CAPABILITY_PRODUCT = ${quote(catalog.product)} as const
export const FEATURE_SETS = ${JSON.stringify(catalog.feature_sets, null, 2)} as const
export type FeatureSetId = (typeof FEATURE_SETS)[number]['id']
export const CAPABILITIES = ${JSON.stringify(catalog.capabilities, null, 2)} as const
export const QUOTAS = ${JSON.stringify(catalog.quotas, null, 2)} as const
export const BUSINESS_OPERATIONS = ${JSON.stringify(catalog.operations, null, 2)} as const
export type BusinessOperationId = (typeof BUSINESS_OPERATIONS)[number]['id']
export type CapabilityId = (typeof CAPABILITIES)[number]['id']
export type QuotaId = (typeof QUOTAS)[number]['id']
export type QuotaScope = (typeof QUOTAS)[number]['scope']
export type QuotaLimit = { mode: 'limited'; value: number } | { mode: 'unlimited' }
export type QuotaGrant = { id: QuotaId; limit: QuotaLimit }
export type Entitlements = { catalog_version: number; features: CapabilityId[]; feature_sets?: FeatureSetId[]; quotas: QuotaGrant[] }
export function effectiveFeatures(value: Pick<Entitlements, 'features' | 'feature_sets'>): CapabilityId[] {
  const sets: readonly string[] = value.feature_sets ?? []
  return CAPABILITIES.filter(entry => value.features.includes(entry.id) || sets.includes(entry.feature_set)).map(entry => entry.id)
}
export function isCapabilityId(value: unknown): value is CapabilityId {
  return typeof value === 'string' && CAPABILITIES.some(entry => entry.id === value)
}
export function isQuotaId(value: unknown): value is QuotaId {
  return typeof value === 'string' && QUOTAS.some(entry => entry.id === value)
}
`
}

function entitlementsSchema(catalog) {
  const hasFeature = entry => ({ anyOf: [
    { required: ['features'], properties: { features: { type: 'array', contains: { const: entry.id } } } },
    { required: ['feature_sets'], properties: { feature_sets: { type: 'array', contains: { const: entry.feature_set } } } },
  ] })
  const dependencyRules = catalog.capabilities.flatMap(entry => entry.requires.map(dependency => ({
    if: hasFeature(entry), then: hasFeature(catalog.capabilities.find(item => item.id === dependency)),
  })))
  const schema = {
    $schema: 'https://json-schema.org/draft/2020-12/schema',
    $id: 'https://aster-team.local/contracts/entitlements.v1.schema.json',
    title: 'Aster explicit entitlement payload',
    description: 'Generated from product-capabilities.yaml; describes rights but does not authenticate or grant them.',
    type: 'object', additionalProperties: false,
    ...(dependencyRules.length ? { allOf: dependencyRules } : {}),
    required: ['catalog_version', 'features', 'quotas'],
    properties: {
      catalog_version: { const: catalog.catalog_version },
      feature_sets: { type: 'array', uniqueItems: true, maxItems: catalog.feature_sets.length, items: { enum: catalog.feature_sets.map(entry => entry.id) } },
      features: { type: 'array', uniqueItems: true, maxItems: catalog.capabilities.length,
        items: { enum: catalog.capabilities.map(entry => entry.id) },

      },
      quotas: { type: 'array', minItems: catalog.quotas.length, maxItems: catalog.quotas.length,
        items: { $ref: '#/$defs/quota' },
        allOf: catalog.quotas.map(entry => ({
          contains: { type: 'object', required: ['id'], properties: { id: { const: entry.id } } },
          minContains: 1, maxContains: 1,
        })),
      },
    },
    $defs: {
      quota: { type: 'object', additionalProperties: false, required: ['id', 'limit'], properties: {
        id: { enum: catalog.quotas.map(entry => entry.id) }, limit: { $ref: '#/$defs/limit' },
      } },
      limit: { oneOf: [
        { type: 'object', additionalProperties: false, required: ['mode', 'value'], properties: {
          mode: { const: 'limited' }, value: { type: 'integer', minimum: 0, maximum: 0xffff_ffff },
        } },
        { type: 'object', additionalProperties: false, required: ['mode'], properties: { mode: { const: 'unlimited' } } },
      ] },
    },
  }
  // Reference the instance payload separately so OpenAPI generators do not
  // mistake JSON Schema's $defs metadata for required request properties.
  const { type, additionalProperties, required, properties, allOf, $defs, ...metadata } = schema
  return stringify({
    ...metadata,
    $ref: '#/$defs/entitlement_payload',
    $defs: { entitlement_payload: { type, additionalProperties, required, properties, ...(allOf ? { allOf } : {}) }, ...$defs },
  })
}

export function productCatalogOutputs(catalog) {
  validateProductCatalog(catalog)
  return new Map([
    ['contracts/schemas/entitlements.v1.schema.yaml', entitlementsSchema(catalog)],
    ['packages/rust/license-core/src/catalog.rs', rustSource(catalog)],
    ['operations/backend/internal/productcatalog/catalog_generated.go', goSource(catalog)],
    ...['customer/sdk/src/generated', 'operations/console/src/api/generated', 'website/src/generated']
      .map(directory => [`${directory}/product-capabilities.ts`, typeScriptSource(catalog)]),
  ])
}
