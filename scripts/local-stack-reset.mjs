import { existsSync, readFileSync, rmSync } from 'node:fs'
import { isAbsolute, relative, resolve } from 'node:path'
import { parseEnv } from 'node:util'

function readEnvironment(path) {
  return existsSync(path) ? parseEnv(readFileSync(path, 'utf8')) : {}
}

function inside(root, path) {
  const pathFromRoot = relative(root, path)
  return pathFromRoot !== '' && !pathFromRoot.startsWith('..') && !isAbsolute(pathFromRoot)
}

function databaseLabel(environment, prefix) {
  const host = environment[`${prefix}_HOST`] || '127.0.0.1'
  const port = environment[`${prefix}_PORT`] || '3306'
  const name = environment[`${prefix}_NAME`] || '(未配置数据库名)'
  return `MariaDB/MySQL ${host}:${port}/${name}`
}

export function localResetPlan(root = resolve('.')) {
  const localRoot = resolve(root, 'data/local')
  const customer = readEnvironment(resolve(localRoot, 'customer.env'))
  const operationsPath = resolve(localRoot, 'operations.env')
  const operations = existsSync(operationsPath) ? readEnvironment(operationsPath) : readEnvironment(resolve(root, '.env'))
  const files = []
  const directories = []
  const externalStores = []

  if (Object.keys(customer).some(name => name.startsWith('ASTER_CONTROL_DB_'))) {
    externalStores.push(databaseLabel(customer, 'ASTER_CONTROL_DB'))
  }
  if (Object.keys(operations).some(name => name.startsWith('ASTER_OPERATIONS_DB_'))) {
    externalStores.push(databaseLabel(operations, 'ASTER_OPERATIONS_DB'))
  }

  for (const path of [
    resolve(root, 'data/runner/runner.json'), resolve(root, 'data/runner/chatgpt-credential.enc'),
    resolve(root, 'data/control/license/license.json'), resolve(root, 'data/control/license/state.json'),
    resolve(root, 'data/control/license/state.json.staged'),
    resolve(root, 'data/control/license/state.json.activation'),
  ]) {
    if (existsSync(path)) files.push(path)
  }

  for (const path of [resolve(localRoot, 'local-admin-credentials.env'), resolve(localRoot, 'pending-admin-passwords.env')]) {
    if (existsSync(path)) files.push(path)
  }
  for (const path of [
    resolve(localRoot, 'demo-delivery'),
    resolve(operations.ASTER_OPERATIONS_ARTIFACT_ROOT || resolve(localRoot, 'operations-artifacts')),
  ]) {
    if (inside(root, path) && existsSync(path)) directories.push(path)
    else if (!inside(root, path) && existsSync(path)) externalStores.push(`ArtifactStore ${path}`)
  }
  // The local install root is `data/`, and the layout contract places the
  // settlement outbox at `data/settlements` inside it. Its owner tag is an HMAC
  // over a constant keyed by the installation identity, while setup-local-stack
  // regenerates that identity on every run: a surviving tag makes
  // `aster-control serve` fail closed with DataIntegrityInvalid, so this
  // identity-bound directory leaves together with the identity itself.
  const settlementOutbox = resolve(root, 'data/data/settlements')
  if (inside(root, settlementOutbox) && existsSync(settlementOutbox)) directories.push(settlementOutbox)

  return { files: [...new Set(files)], directories: [...new Set(directories)], externalStores: [...new Set(externalStores)] }
}

export function removeLocalResetFiles(plan) {
  for (const path of plan.files) rmSync(path, { force: true })
  for (const path of plan.directories || []) rmSync(path, { recursive: true, force: true })
}
