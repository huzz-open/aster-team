import { generateKeyPairSync, randomBytes } from 'node:crypto'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'

// Only disposable localhost infrastructure. No real signer/profile is read.
const directory = new URL('../../dist/commercial-validation/', import.meta.url)
await mkdir(directory, { recursive: true })
const v2Key = generateKeyPairSync('ed25519').privateKey.export({ type: 'pkcs8', format: 'der' }).toString('base64url')
const paidV2Key = generateKeyPairSync('ed25519').privateKey.export({ type: 'pkcs8', format: 'der' }).toString('base64url')
const v2Vector = JSON.parse(await readFile(new URL('../../contracts/test-vectors/license.v2.json', import.meta.url), 'utf8'))
const planDefinition = JSON.parse(await readFile(new URL('../../contracts/test-vectors/plan-definition.v1.json', import.meta.url), 'utf8'))
const freeRights = v2Vector.cases[0].document.claims.entitlements
const settings = {
  ASTER_OPERATIONS_ADDR: '127.0.0.1:26390',
  ASTER_OPERATIONS_DB_HOST: '127.0.0.1', ASTER_OPERATIONS_DB_PORT: '26216',
  ASTER_OPERATIONS_DB_NAME: 'aster_commercial_ui_test', ASTER_OPERATIONS_DB_USER: 'root', ASTER_OPERATIONS_DB_PASSWORD: 'aster-test-only',
  ASTER_OPERATIONS_AUTO_MIGRATE: 'true', ASTER_OPERATIONS_CREATE_DATABASE: 'true',
  ASTER_OPERATIONS_BOOTSTRAP_ADMIN_EMAIL: 'commercial-ui@test.invalid', ASTER_OPERATIONS_BOOTSTRAP_ADMIN_PASSWORD: 'Aster-UI-Test-Only-2026!',
  ASTER_OPERATIONS_TRUSTED_ORIGINS: 'http://127.0.0.1:26380',
  ASTER_OPERATIONS_LICENSE_V2_SIGNERS_JSON: JSON.stringify([
    { key_id: 'local-free-test-only', private_key_pkcs8: v2Key, policy: { sources: ['free_distribution'], bindings: ['unbound'], expiries: ['fixed', 'none'], entitlement_ceiling: freeRights } },
    { key_id: 'local-paid-test-only', private_key_pkcs8: paidV2Key, policy: { sources: ['commercial_order'], bindings: ['installation'], expiries: ['fixed'], entitlement_ceiling: { ...planDefinition.entitlements, feature_sets: ['standard'] } } },
  ]),
  ASTER_OPERATIONS_CUSTOMER_REF_SECRET: randomBytes(32).toString('base64url'),
  ASTER_OPERATIONS_ARTIFACT_ROOT: fileURLToPath(new URL('artifacts/', directory)).replaceAll('\\', '/'),
  ASTER_OPERATIONS_PUBLIC_CATALOG_ROOT: fileURLToPath(new URL('public-catalogs/', directory)).replaceAll('\\', '/'),
  ASTER_OPERATIONS_PUBLICATION_LOCAL_ORIGIN: 'http://127.0.0.1:26394',
  ASTER_OPERATIONS_PUBLICATION_PRODUCTION_ORIGIN: '',
  ASTER_OPERATIONS_QUOTATION_ENVIRONMENT: 'local',
  ASTER_OPERATIONS_FULFILLMENT_ENVIRONMENT: 'local',
  ASTER_OPERATIONS_GITHUB_ENABLED: 'false', ASTER_OPERATIONS_GITHUB_PUBLISH_ENABLED: 'false',
}
await writeFile(new URL('operations-ui.env', directory), Object.entries(settings).map(([key, value]) => `${key}=${value}`).join('\n') + '\n')
console.log('Disposable localhost Operations configuration prepared in dist/commercial-validation/operations-ui.env')
