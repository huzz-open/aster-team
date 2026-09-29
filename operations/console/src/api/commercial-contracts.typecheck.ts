import type { components } from './generated/schema'
import type { Entitlements, CAPABILITY_CATALOG_VERSION } from './generated/product-capabilities'

type Assert<T extends true> = T
type WireEntitlements = components['schemas']['CommercialPlanDefinition']['entitlements']
type CatalogEntitlements = Omit<Entitlements, 'catalog_version'> & { catalog_version: typeof CAPABILITY_CATALOG_VERSION }

// Both generators must describe the same instance data. In particular, JSON
// Schema metadata such as $defs must never become required request fields.
export type WireAcceptsCatalog = Assert<CatalogEntitlements extends WireEntitlements ? true : false>
export type CatalogAcceptsWire = Assert<WireEntitlements extends CatalogEntitlements ? true : false>
export type WireHasOnlyInstanceFields = Assert<Exclude<keyof WireEntitlements, keyof CatalogEntitlements> extends never ? true : false>
