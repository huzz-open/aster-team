import assert from 'node:assert/strict'
import { generateKeyPairSync } from 'node:crypto'
import { existsSync } from 'node:fs'
import { mkdtemp, readFile, writeFile, mkdir } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, resolve } from 'node:path'
import test from 'node:test'
import { parseEnv } from 'node:util'
import { runLocalInitializationProcess, setupLocalStack } from './setup-local-stack.mjs'
import { generateLicenseSigners, localLicensePolicies, readLicenseSigningProfile } from './license-signing-profile.mjs'

test('initialization streams Cargo diagnostics before completion and drains the final output', async () => {
  const lines = []
  let reportFirst
  const first = new Promise(accept => { reportFirst = accept })
  let completed = false
  const running = runLocalInitializationProcess(process.execPath, ['-e', `
    process.stderr.write('Updating crates.io index\\n');
    setTimeout(() => { process.stderr.write('Compiling 编译依赖\\n'); process.stdout.write('identity created'); }, 250);
  `], { label: '测试初始化', log: line => { lines.push(line); reportFirst() } }).then(() => { completed = true })
  await first
  assert.equal(completed, false)
  assert.deepEqual(lines, ['Updating crates.io index'])
  await running
  assert.ok(lines.includes('Compiling 编译依赖'))
  assert.ok(lines.includes('identity created'))
})

test('silent initialization reports a wait without claiming compilation progress and stops after exit', async () => {
  const lines = []
  await runLocalInitializationProcess(process.execPath, ['-e', 'setTimeout(() => {}, 220)'], {
    label: '测试初始化', heartbeatMs: 40, log: line => lines.push(line),
  })
  assert.ok(lines.some(line => /无新输出.*下载、等待锁或编译/.test(line)))
  const count = lines.length
  await new Promise(accept => setTimeout(accept, 90))
  assert.equal(lines.length, count)
})

test('initialization forwards network failures and spawn errors without leaving a heartbeat', async () => {
  const lines = []
  await assert.rejects(runLocalInitializationProcess(process.execPath, ['-e', `process.stderr.write('network timeout'); process.exitCode = 7`], {
    label: '测试初始化', log: line => lines.push(line),
  }), /测试初始化失败：network timeout/)
  assert.deepEqual(lines, ['network timeout'])
  await assert.rejects(runLocalInitializationProcess(resolve(tmpdir(), 'aster-missing-command-579da2.exe'), [], {
    label: '测试初始化', heartbeatMs: 40, log: line => lines.push(line),
  }), /ENOENT/)
  const count = lines.length
  await new Promise(accept => setTimeout(accept, 90))
  assert.equal(lines.length, count)
})


test('local authorization uses Customer Admin APIs and enrolls the Rust Runner without compiling another Control', async () => {
  const source = await readFile(resolve(import.meta.dirname, 'seed-local-demo.mjs'), 'utf8')
  const fulfillment = await readFile(resolve(import.meta.dirname, 'local-commercial-license.mjs'), 'utf8')
  assert.match(source, /customer\.request\('\/api\/admin\/auth\/login'/)
  assert.match(source, /customer\.request\('\/api\/admin\/license'/)
  assert.match(source, /customer\.request\('\/api\/admin\/runners\/enrollments'/)
  assert.match(source, /const freeLicensePath = resolve\(output, 'free-license\.json'\)/)
  assert.match(source, /writeFileSync\(freeLicensePath, free\.bytes/)
  assert.match(fulfillment, /write\('\/commercial\/distributions'/)
  assert.match(fulfillment, /operations\.request\(`\$\{freePath\}\/download`, \{ format: 'bytes' \}\)/)
  assert.match(fulfillment, /'free_distribution', 'unbound', 'none'/)
  assert.match(source, /operationsLogin\?\.operator\?\.password_change_required !== false/)
  assert.match(source, /customerLogin\?\.password_change_required !== false/)
  assert.ok(source.indexOf("customer.request('/api/admin/auth/login'") < source.indexOf("operations.request('/customers?limit=100')"))
  assert.ok(source.indexOf('await requireLocalFulfillmentEnvironment(operations)') < source.indexOf("operations.request('/customers?limit=100')"))
  assert.match(source, /'run', '-p', 'aster-runner'.*?'enroll'/s)
  assert.match(source, /data\/runner.*?identity\.json/s)
  assert.match(source, /data\/runner.*?task-keys\.json/s)
  assert.doesNotMatch(source, /run-rust-control\.mjs/)
})

test('quick authorization prepares the fixed local member without generating the full UI mock dataset', async () => {
  const manager = await readFile(resolve(import.meta.dirname, 'local_dev_manager.py'), 'utf8')
  const packageSource = JSON.parse(await readFile(resolve(import.meta.dirname, '..', 'package.json'), 'utf8'))
  const memberSource = await readFile(resolve(import.meta.dirname, 'seed-local-member.mjs'), 'utf8')
  assert.equal(packageSource.scripts['seed:local-member'], 'node ./scripts/seed-local-member.mjs')
  assert.match(manager, /\[self\.npm, "run", "seed:local-member"\]/)
  assert.match(memberSource, /const memberEmail = 'test@at\.com'/)
  assert.match(memberSource, /\/api\/admin\/users\/\$\{encodeURIComponent\(member\.id\)\}\/password-reset/)
  assert.match(memberSource, /new_password: customerPassword/)
  assert.match(memberSource, /\/api\/admin\/users\/\$\{encodeURIComponent\(member\.id\)\}\/quota-adjustments/)
  assert.match(memberSource, /amount_tokens: initialQuotaTokens/)
  assert.match(memberSource, /ASTER_LOCAL_MEMBER_PASSWORD: customerPassword/)
  assert.match(memberSource, /ASTER_LOCAL_MEMBER_QUOTA_READY: 'true'/)
  assert.doesNotMatch(memberSource, /seed_local_mock_usage/)
  const memberVite = await readFile(resolve(import.meta.dirname, '..', 'customer/member/vite.config.ts'), 'utf8')
  const memberLogin = await readFile(resolve(import.meta.dirname, '..', 'customer/member/src/views/LoginView.vue'), 'utf8')
  const memberLayout = await readFile(resolve(import.meta.dirname, '..', 'customer/member/src/views/MemberLayout.vue'), 'utf8')
  const imageMatrix = await readFile(resolve(import.meta.dirname, '..', 'website/docs/.vitepress/theme/ImageFieldMatrix.vue'), 'utf8')
  assert.match(memberVite, /localAdminCredentials\('member'\)/)
  assert.match(memberLogin, /route\.query\.local_login !== '1'/)
  assert.match(memberLogin, /fetch\('\/__aster_local_admin_credentials'/)
  assert.match(memberLayout, /window\.location\.origin}\/docs\//)
  assert.match(memberLayout, /locale\.value === 'en-US' \? 'en' : 'zh-cn'/)
  assert.match(memberLayout, /href: documentationUrl\.value/)
  assert.match(imageMatrix, /@click="model=id"/)
  assert.match(imageMatrix, /const selected = computed/)
  assert.doesNotMatch(imageMatrix, /configuration\.default_image_model/)
})

function fakeMySQL(existingDatabases = []) {
  const existing = new Set(existingDatabases); const statements = []
  return {
    statements,
    async connect() {
      return {
        async query(sql, parameters = []) { statements.push({ sql, parameters }); if (sql.startsWith('SELECT SCHEMA_NAME')) return [existing.has(parameters[0]) ? [{}] : [], []]; return [[], []] },
        escapeId(value) { return `\`${String(value)}\`` }, escape(value) { return `'${String(value).replaceAll("'", "''")}'` }, async end() {},
      }
    },
  }
}

async function project() {
  const root = await mkdtemp(resolve(tmpdir(), 'aster-local-stack-'))
  await writeFile(resolve(root, 'package.json'), '{"version":"1.0.5"}\n')
  await writeFile(resolve(root, '.env'), [
    'ASTER_LOCAL_ADMIN_EMAIL=developer@example.com',
    'ASTER_OPERATIONS_DB_HOST=127.0.0.1', 'ASTER_OPERATIONS_DB_PORT=3306', 'ASTER_OPERATIONS_DB_NAME=aster_operations_test',
    'ASTER_OPERATIONS_DB_ADMIN_USER=root', 'ASTER_OPERATIONS_DB_ADMIN_PASSWORD=admin-secret', 'ASTER_OPERATIONS_DB_SERVICE_USER=aster_operations_test', 'ASTER_OPERATIONS_DB_SERVICE_HOST=localhost',
    'ASTER_CUSTOMER_DB_HOST=127.0.0.1', 'ASTER_CUSTOMER_DB_PORT=3306', 'ASTER_CUSTOMER_DB_NAME=aster_customer_test',
    'ASTER_CUSTOMER_DB_ADMIN_USER=root', 'ASTER_CUSTOMER_DB_ADMIN_PASSWORD=admin-secret', 'ASTER_CUSTOMER_DB_SERVICE_USER=aster_customer_test', 'ASTER_CUSTOMER_DB_SERVICE_HOST=localhost', '',
  ].join('\n'))
  return root
}

async function machine(path) {
  await mkdir(dirname(path), { recursive: true })
  await writeFile(path, JSON.stringify({
    schema: 'aster.installation.v1', installation_id: 'installation_local_test', machine_fingerprint_sha256: 'A'.repeat(43),
    machine_factors: [{ kind: 'dmi_product_uuid', sha256: 'B'.repeat(43) }, { kind: 'machine_id', sha256: 'C'.repeat(43) }],
  }))
}

async function securityDirectory(root, includeGitHub = true) {
  const directory = resolve(root, '..', `aster-security-${Date.now()}-${Math.random().toString(16).slice(2)}`)
  await mkdir(directory, { recursive: true })
  const release = generateKeyPairSync('ed25519')
  const buildApp = generateKeyPairSync('rsa', { modulusLength: 2048 })
  const publishApp = generateKeyPairSync('rsa', { modulusLength: 2048 })
  const keyring = (keyID, key) => JSON.stringify([{
    key_id: keyID,
    public_key_spki: key.publicKey.export({ format: 'der', type: 'spki' }).toString('base64url'),
  }])
  const policies = localLicensePolicies()
  policies[0].key_id = 'license-test-free-v2'
  policies[1].key_id = 'license-test-paid-v2'
  const profile = readLicenseSigningProfile(JSON.stringify(generateLicenseSigners(policies)), keyring('release-test-v1', release))
  await writeFile(resolve(directory, 'license-v2.signers.json'), profile.licenseSignersJSON)
  await writeFile(resolve(directory, 'license-v2.public-keyring.json'), profile.licenseTrustedKeysJSON)
  await writeFile(resolve(directory, 'release-v1.public-keyring.json'), keyring('release-test-v1', release))
  await writeFile(resolve(directory, 'release-v1.seed'), 'THIS-RELEASE-SEED-MUST-NEVER-BE-COPIED')
  if (includeGitHub) {
    await writeFile(resolve(directory, 'github-build-app.private-key.pem'), buildApp.privateKey.export({ format: 'pem', type: 'pkcs8' }))
    await writeFile(resolve(directory, 'github-publish-app.private-key.pem'), publishApp.privateKey.export({ format: 'pem', type: 'pkcs8' }))
    await writeFile(resolve(directory, 'operations-release-center.json'), JSON.stringify({
      schema: 'aster.operations-release-center.v1', repository: 'example/aster-team',
      publish_repository: 'example-public/aster-team',
      build_app: { app_id: 12345, installation_id: 67890, private_key_file: 'github-build-app.private-key.pem' },
      publish_app: { app_id: 54321, installation_id: 9876, private_key_file: 'github-publish-app.private-key.pem' },
    }))
  }
  return directory
}

function runtimeInitializer(root) {
  return async flag => {
    if (flag === '--initialize-installation') await machine(resolve(root, 'data/control/license/installation.json'))
  }
}

test('setup creates only customer and operations profiles for offline machine licensing', async () => {
  const root = await project(); const database = fakeMySQL(['aster_customer_test']); let confirmation = ''; const logs = []
  const result = await setupLocalStack({
    rootDirectory: root, interactive: true, connect: database.connect, runControlInitialization: runtimeInitializer(root),
    confirm: async value => { confirmation = value; return true }, log: value => logs.push(value),
  })
  assert.equal(result.status, 'created')
  assert.match(confirmation, /aster_customer_test（删除重建）/)
  const customer = parseEnv(await readFile(resolve(root, 'data/local/customer.env'), 'utf8'))
  const operations = parseEnv(await readFile(resolve(root, 'data/local/operations.env'), 'utf8'))
  const credentials = parseEnv(await readFile(resolve(root, 'data/local/local-admin-credentials.env'), 'utf8'))
  assert.match(customer.ASTER_LICENSE_TRUSTED_KEYS_JSON, /^\[/)
  assert.match(customer.ASTER_RELEASE_TRUSTED_KEYS_JSON, /^\[/)
  assert.notEqual(
    JSON.parse(customer.ASTER_LICENSE_TRUSTED_KEYS_JSON)[0].public_key_spki,
    JSON.parse(customer.ASTER_RELEASE_TRUSTED_KEYS_JSON)[0].public_key_spki,
  )
  assert.equal(customer.ASTER_CONTROL_PORT, '11080')
  assert.match(customer.ASTER_INSTALLATION_PROFILE_PATH, /installation\.json$/)
  assert.match(JSON.parse(operations.ASTER_OPERATIONS_LICENSE_V2_SIGNERS_JSON)[0].private_key_pkcs8, /^[A-Za-z0-9_-]+$/)
  assert.equal(operations.ASTER_OPERATIONS_LICENSE_SIGNING_PRIVATE_KEY_PKCS8, undefined)
  assert.equal(operations.ASTER_OPERATIONS_LICENSE_SIGNING_KEY_ID, undefined)
  assert.equal(operations.ASTER_OPERATIONS_FULFILLMENT_ENVIRONMENT, 'local')
  assert.equal(operations.ASTER_OPERATIONS_LICENSE_V2_VERIFIERS_JSON, customer.ASTER_LICENSE_TRUSTED_KEYS_JSON)
  assert.match(operations.ASTER_OPERATIONS_CUSTOMER_REF_SECRET, /^[A-Za-z0-9_-]+$/)
  assert.ok(credentials.ASTER_LOCAL_CUSTOMER_PASSWORD.length >= 16)
  assert.equal(customer.BOOTSTRAP_ADMIN_PASSWORD, undefined)
  assert.equal(credentials.ASTER_LOCAL_OPERATIONS_PASSWORD, operations.ASTER_OPERATIONS_BOOTSTRAP_ADMIN_PASSWORD)
  assert.deepEqual(Object.keys(credentials).sort(), [
    'ASTER_LOCAL_CUSTOMER_EMAIL', 'ASTER_LOCAL_CUSTOMER_PASSWORD',
    'ASTER_LOCAL_OPERATIONS_EMAIL', 'ASTER_LOCAL_OPERATIONS_PASSWORD',
  ])
  assert.doesNotMatch((await readFile(resolve(root, 'data/local/customer.env'), 'utf8')) + (await readFile(resolve(root, 'data/local/operations.env'), 'utf8')), /admin-secret/)
  assert.doesNotMatch(logs.join('\n'), new RegExp(credentials.ASTER_LOCAL_CUSTOMER_PASSWORD))
})

test('setup persists the active manager port profile', async () => {
  const root = await project(); const database = fakeMySQL()
  await setupLocalStack({
    rootDirectory: root,
    interactive: true,
    connect: database.connect,
    runControlInitialization: runtimeInitializer(root),
    confirm: async () => true,
    log() {},
    environment: {
      ASTER_OPERATIONS_ADDR: '127.0.0.1:22090',
      ASTER_OPERATIONS_CONSOLE_PORT: '22080',
      ASTER_CONTROL_PORT: '21080',
    },
  })

  const customer = parseEnv(await readFile(resolve(root, 'data/local/customer.env'), 'utf8'))
  const operations = parseEnv(await readFile(resolve(root, 'data/local/operations.env'), 'utf8'))
  assert.equal(customer.ASTER_CONTROL_PORT, '21080')
  assert.equal(customer.PUBLIC_API_BASE_URL, 'http://127.0.0.1:21080')
  assert.equal(operations.ASTER_OPERATIONS_ADDR, '127.0.0.1:22090')
  assert.equal(
    operations.ASTER_OPERATIONS_TRUSTED_ORIGINS,
    'http://127.0.0.1:22080,http://localhost:22080',
  )
})

test('completed non-interactive setup is unchanged and does not reconnect databases', async () => {
  const root = await project(); const database = fakeMySQL()
  await setupLocalStack({ rootDirectory: root, interactive: true, connect: database.connect, runControlInitialization: runtimeInitializer(root), confirm: async () => true, log() {} })
  const result = await setupLocalStack({ rootDirectory: root, interactive: false, connect: async () => { throw new Error('must not connect') }, log() {} })
  assert.equal(result.status, 'unchanged')
})

test('setup clears the identity-bound settlement outbox so Control can rebind it', async () => {
  const root = await project()
  const outbox = resolve(root, 'data/data/settlements')
  await mkdir(outbox, { recursive: true })
  await writeFile(resolve(outbox, 'installation.json'), 'stale-owner-tag')
  await writeFile(resolve(outbox, 'registry.lock'), '')
  const result = await setupLocalStack({
    rootDirectory: root, interactive: true, connect: fakeMySQL().connect,
    runControlInitialization: runtimeInitializer(root), confirm: async () => true, log() {},
  })
  assert.equal(result.status, 'created')
  // A surviving tag was signed by the previous installation identity, so
  // `aster-control serve` refuses to start with DataIntegrityInvalid.
  assert.equal(existsSync(outbox), false)
})

test('cancelling setup does not provision databases or write profiles', async () => {
  const root = await project(); const database = fakeMySQL()
  const result = await setupLocalStack({ rootDirectory: root, interactive: true, connect: database.connect, runControlInitialization: runtimeInitializer(root), confirm: async () => false, log() {} })
  assert.equal(result.status, 'cancelled')
  assert.equal(database.statements.some(value => value.sql.startsWith('DROP DATABASE')), false)
})

test('setup rejects a repository directory starting with two dots before reading keys or connecting databases', async () => {
  const root = await project()
  const directory = resolve(root, '..security')
  await mkdir(directory)
  await writeFile(resolve(root, '.env'), `${await readFile(resolve(root, '.env'), 'utf8')}ASTER_LOCAL_SECURITY_CONFIG_DIR=${directory}\n`)
  await assert.rejects(setupLocalStack({ rootDirectory: root, interactive: true,
    connect: async () => { throw new Error('database must not be reached') }, log() {},
  }), /安全配置目录必须位于源码仓库之外/)
})

test('setup imports an external security directory without copying the release seed', async () => {
  const root = await project()
  const directory = await securityDirectory(root)
  await writeFile(resolve(root, '.env'), `${await readFile(resolve(root, '.env'), 'utf8')}ASTER_LOCAL_SECURITY_CONFIG_DIR=${directory}\n`)
  const result = await setupLocalStack({
    rootDirectory: root, interactive: true, connect: fakeMySQL().connect,
    runControlInitialization: runtimeInitializer(root), confirm: async () => true, log() {},
  })
  assert.equal(result.status, 'created')
  const operationsSource = await readFile(resolve(root, 'data/local/operations.env'), 'utf8')
  const operations = parseEnv(operationsSource)
  const customer = parseEnv(await readFile(resolve(root, 'data/local/customer.env'), 'utf8'))
  assert.equal(JSON.parse(operations.ASTER_OPERATIONS_LICENSE_V2_SIGNERS_JSON)[0].key_id, 'license-test-free-v2')
  assert.equal(operations.ASTER_OPERATIONS_GITHUB_ENABLED, 'true')
  assert.equal(operations.ASTER_OPERATIONS_GITHUB_APP_ID, '12345')
  assert.equal(operations.ASTER_OPERATIONS_GITHUB_INSTALLATION_ID, '67890')
  assert.equal(operations.ASTER_OPERATIONS_GITHUB_REPOSITORY, 'example/aster-team')
  assert.equal(operations.ASTER_OPERATIONS_GITHUB_PUBLISH_ENABLED, 'true')
  assert.equal(operations.ASTER_OPERATIONS_GITHUB_PUBLISH_APP_ID, '54321')
  assert.equal(operations.ASTER_OPERATIONS_GITHUB_PUBLISH_INSTALLATION_ID, '9876')
  assert.equal(operations.ASTER_OPERATIONS_GITHUB_PUBLISH_REPOSITORY, 'example-public/aster-team')
  assert.equal(operations.ASTER_OPERATIONS_GITHUB_REQUEST_TIMEOUT, '20s')
  assert.equal(operations.ASTER_OPERATIONS_GITHUB_ARTIFACT_DOWNLOAD_TIMEOUT, '5m')
  assert.equal(JSON.parse(operations.ASTER_OPERATIONS_RELEASE_TRUSTED_KEYS_JSON)[0].key_id, 'release-test-v1')
  assert.equal(JSON.parse(customer.ASTER_LICENSE_TRUSTED_KEYS_JSON)[0].key_id, 'license-test-free-v2')
  assert.equal(JSON.parse(customer.ASTER_RELEASE_TRUSTED_KEYS_JSON)[0].key_id, 'release-test-v1')
  assert.doesNotMatch(operationsSource, /THIS-RELEASE-SEED-MUST-NEVER-BE-COPIED/)
})

test('external signing keys without a GitHub App manifest keep the release center read-only', async () => {
  const root = await project()
  const directory = await securityDirectory(root, false)
  await writeFile(resolve(root, '.env'), `${await readFile(resolve(root, '.env'), 'utf8')}ASTER_LOCAL_SECURITY_CONFIG_DIR=${directory}\n`)
  await setupLocalStack({
    rootDirectory: root, interactive: true, connect: fakeMySQL().connect,
    runControlInitialization: runtimeInitializer(root), confirm: async () => true, log() {},
  })
  const operations = parseEnv(await readFile(resolve(root, 'data/local/operations.env'), 'utf8'))
  assert.equal(JSON.parse(operations.ASTER_OPERATIONS_LICENSE_V2_SIGNERS_JSON)[0].key_id, 'license-test-free-v2')
  assert.equal(operations.ASTER_OPERATIONS_GITHUB_ENABLED, 'false')
  assert.equal(operations.ASTER_OPERATIONS_GITHUB_APP_PRIVATE_KEY_PEM_BASE64, '')
})

test('local setup retains historical public scope and rejects a private/public policy mismatch before provisioning', async () => {
  const root = await project()
  const directory = await securityDirectory(root, false)
  const path = resolve(directory, 'license-v2.public-keyring.json')
  const active = JSON.parse(await readFile(path, 'utf8'))
  const scoped = JSON.parse(await readFile(new URL('../contracts/test-vectors/license-trust.v1.json', import.meta.url), 'utf8')).keyring[1]
  await writeFile(path, JSON.stringify([...active, scoped]))
  await writeFile(resolve(root, '.env'), `${await readFile(resolve(root, '.env'), 'utf8')}ASTER_LOCAL_SECURITY_CONFIG_DIR=${directory}\n`)
  await setupLocalStack({
    rootDirectory: root, interactive: true, connect: fakeMySQL().connect,
    runControlInitialization: runtimeInitializer(root), confirm: async () => true, log() {},
  })
  const customer = parseEnv(await readFile(resolve(root, 'data/local/customer.env'), 'utf8'))
  assert.deepEqual(JSON.parse(customer.ASTER_LICENSE_TRUSTED_KEYS_JSON), [...active, scoped])
  // Matching key material cannot hide a changed issuer policy.
  const fresh = await project()
  await writeFile(resolve(fresh, '.env'), `${await readFile(resolve(fresh, '.env'), 'utf8')}ASTER_LOCAL_SECURITY_CONFIG_DIR=${directory}\n`)
  active[1].policy.entitlement_ceiling.quotas[0].limit.value = 51
  await writeFile(path, JSON.stringify(active))
  await assert.rejects(setupLocalStack({
    rootDirectory: fresh, interactive: true,
    connect: async () => { throw new Error('must not provision with mismatched signing policy') },
    runControlInitialization: runtimeInitializer(fresh), confirm: async () => true, log() {},
  }), /does not match the public keyring and policy/)
})
