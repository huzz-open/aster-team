import { createPrivateKey, generateKeyPairSync, randomBytes } from 'node:crypto'
import { chmodSync, existsSync, lstatSync, mkdirSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { basename, isAbsolute, relative, resolve, sep } from 'node:path'
import { createInterface } from 'node:readline/promises'
import { parseEnv } from 'node:util'
import mysql from 'mysql2/promise'
import { updateEnvFile } from './local-env-file.mjs'
import { inspectLocalDatabases, localDatabasePlans, provisionLocalDatabase, runtimeDatabaseEnvironment } from './local-database-provision.mjs'
import { localResetPlan, removeLocalResetFiles } from './local-stack-reset.mjs'
import { generateLicenseSigners, localLicensePolicies, readLicenseSigningProfile } from './license-signing-profile.mjs'

function required(value, name) {
  const result = String(value || '').trim()
  if (!result || /[\r\n]/.test(result)) throw new Error(`${name} 不能为空或包含换行符`)
  return result
}
function localPort(value, fallback, name) {
  const port = Number(value || fallback)
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error(`${name} 必须是 1-65535 的端口号`)
  return port
}
function publicSPKI(pair) { return pair.publicKey.export({ format: 'der', type: 'spki' }).toString('base64url') }
function secret(bytes = 32) { return randomBytes(bytes).toString('base64url') }
function exactKeys(value, expected, label) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${label} 必须是 JSON 对象`)
  const actual = Object.keys(value).sort()
  const wanted = [...expected].sort()
  if (actual.length !== wanted.length || actual.some((key, index) => key !== wanted[index])) {
    throw new Error(`${label} 字段必须严格为：${wanted.join(', ')}`)
  }
}
function securityFile(directory, name, label) {
  if (typeof name !== 'string' || name === '' || basename(name) !== name || name === '.' || name === '..') {
    throw new Error(`${label} 必须是安全配置目录中的文件名，不能包含路径`)
  }
  const path = resolve(directory, name)
  if (!existsSync(path)) throw new Error(`${label} 不存在：${name}`)
  const information = lstatSync(path)
  if (information.isSymbolicLink() || !information.isFile()) throw new Error(`${label} 必须是普通文件，不能是符号链接：${name}`)
  return path
}
function readGitHubApp(directory, value, label) {
  exactKeys(value, ['app_id', 'installation_id', 'private_key_file'], label)
  for (const name of ['app_id', 'installation_id']) {
    if (!Number.isSafeInteger(value[name]) || value[name] <= 0) throw new Error(`${label}.${name} 必须是正整数`)
  }
  const pem = readFileSync(securityFile(directory, value.private_key_file, `${label}.private_key_file`), 'utf8')
  let key
  try { key = createPrivateKey(pem) }
  catch (error) { throw new Error(`${label} RSA 私钥无效：${error.message}`) }
  if (key.asymmetricKeyType !== 'rsa' || (key.asymmetricKeyDetails?.modulusLength || 0) < 2048) {
    throw new Error(`${label} 必须使用至少 2048 位的 RSA 私钥`)
  }
  return {
    appID: String(value.app_id), installationID: String(value.installation_id),
    privateKeyPEMBase64: Buffer.from(pem, 'utf8').toString('base64'),
  }
}
function loadSecurityDirectory(rootDirectory, rawDirectory) {
  const configured = String(rawDirectory || '').trim()
  if (!configured) return null
  if (!isAbsolute(configured)) throw new Error('ASTER_LOCAL_SECURITY_CONFIG_DIR 必须是绝对路径')
  if (!existsSync(configured)) throw new Error(`安全配置目录不存在：${configured}`)
  const directory = realpathSync(configured)
  const repositoryRelative = relative(realpathSync(rootDirectory), directory)
  if (repositoryRelative === '' || (repositoryRelative !== '..' && !repositoryRelative.startsWith(`..${sep}`) && !isAbsolute(repositoryRelative))) {
    throw new Error('安全配置目录必须位于源码仓库之外')
  }
  if (!statSync(directory).isDirectory()) throw new Error(`安全配置目录不是目录：${directory}`)

  const profile = readLicenseSigningProfile(
    readFileSync(securityFile(directory, 'license-v2.signers.json', 'License v2 签发配置'), 'utf8'),
    readFileSync(securityFile(directory, 'release-v1.public-keyring.json', 'Release 公钥环'), 'utf8'),
    readFileSync(securityFile(directory, 'license-v2.public-keyring.json', 'License v2 公钥环'), 'utf8'),
  )

  let github = null
  const manifestCandidate = resolve(directory, 'operations-release-center.json')
  if (existsSync(manifestCandidate)) {
    const manifestPath = securityFile(directory, 'operations-release-center.json', 'GitHub App 配置清单')
    let manifest
    try { manifest = JSON.parse(readFileSync(manifestPath, 'utf8')) }
    catch (error) { throw new Error(`operations-release-center.json 不是有效 JSON：${error.message}`) }
    const hasPublishApp = manifest.publish_app !== undefined
    const hasPublishRepository = manifest.publish_repository !== undefined
    if (hasPublishApp !== hasPublishRepository) {
      throw new Error('operations-release-center.json 必须同时配置 publish_repository 和 publish_app')
    }
    const rootKeys = hasPublishApp
      ? ['schema', 'repository', 'build_app', 'publish_repository', 'publish_app']
      : ['schema', 'repository', 'build_app']
    exactKeys(manifest, rootKeys, 'operations-release-center.json')
    if (manifest.schema !== 'aster.operations-release-center.v1') throw new Error('operations-release-center.json schema 无效')
    if (typeof manifest.repository !== 'string' || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(manifest.repository)) {
      throw new Error('operations-release-center.json repository 必须是 owner/repository')
    }
    if (hasPublishRepository && (typeof manifest.publish_repository !== 'string' || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(manifest.publish_repository))) {
      throw new Error('operations-release-center.json publish_repository 必须是 owner/repository')
    }
    github = {
      repository: manifest.repository,
      publishRepository: manifest.publish_repository || '',
      build: readGitHubApp(directory, manifest.build_app, 'build_app'),
      publish: hasPublishApp ? readGitHubApp(directory, manifest.publish_app, 'publish_app') : null,
    }
  }
  return {
    directory, licenseSignersJSON: profile.licenseSignersJSON,
    licenseTrustedKeysJSON: profile.licenseTrustedKeysJSON, releaseTrustedKeysJSON: profile.releaseTrustedKeysJSON, github,
  }
}
async function terminalConfirmation(question) {
  const prompt = createInterface({ input: process.stdin, output: process.stdout })
  try { return ['y', 'yes'].includes((await prompt.question(question)).trim().toLowerCase()) }
  finally { prompt.close() }
}

export function runLocalInitializationProcess(command, args, {
  cwd, label, log = console.log, heartbeatMs = 10000,
} = {}) {
  return new Promise((accept, reject) => {
    const started = Date.now()
    let lastOutput = started
    let errorTail = ''
    let spawnError
    const child = spawn(command, args, {
      cwd, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true,
      env: { ...process.env, CARGO_TERM_COLOR: 'never' },
    })
    for (const [stream, isError] of [[child.stdout, false], [child.stderr, true]]) {
      stream.setEncoding('utf8')
      stream.on('data', chunk => {
        lastOutput = Date.now()
        if (isError) errorTail = (errorTail + chunk).slice(-4000)
      })
      const lines = createInterface({ input: stream, crlfDelay: Infinity, terminal: false })
      lines.on('line', line => { if (line.trim()) log(line) })
    }
    const heartbeat = setInterval(() => {
      const now = Date.now()
      if (now - lastOutput >= heartbeatMs) {
        log(`[等待 ${Math.floor((now - started) / 1000)} 秒] ${label}；进程仍在运行，最近 ${Math.floor((now - lastOutput) / 1000)} 秒无新输出（可能在下载、等待锁或编译）`)
      }
    }, heartbeatMs)
    child.once('error', error => { spawnError = error })
    // close occurs after both streams drain, including a final line without a newline.
    child.once('close', (code, signal) => {
      clearInterval(heartbeat)
      if (spawnError || code !== 0) {
        reject(new Error(`${label}失败：${spawnError?.message || errorTail.trim() || `退出码 ${code ?? signal ?? 'unknown'}`}`))
      } else accept()
    })
  })
}

export async function setupLocalStack(options = {}) {
  const rootDirectory = resolve(options.rootDirectory || '.')
  const atRoot = (...parts) => resolve(rootDirectory, ...parts)
  const args = options.args || []
  const interactive = options.interactive ?? process.stdin.isTTY
  const confirm = options.confirm || terminalConfirmation
  const connect = options.connect || mysql.createConnection
  const log = options.log || console.log
  const environment = options.environment || process.env
  const runControlInitialization = options.runControlInitialization || (flag => runLocalInitializationProcess(
    process.execPath, ['./scripts/run-rust-control.mjs', flag],
    { cwd: rootDirectory, log, label: `Rust Control 初始化（${flag}）` },
  ))
  if (args.length) throw new Error('setup:local 不接受命令行参数，请编辑根目录 .env')

  const localRoot = atRoot('data/local')
  const customerEnv = resolve(localRoot, 'customer.env')
  const operationsEnv = resolve(localRoot, 'operations.env')
  const localAdminCredentialsEnv = resolve(localRoot, 'local-admin-credentials.env')
  const installationProfile = atRoot('data/control/license/installation.json')
  const licenseFile = atRoot('data/control/license/license.json')
  const licenseStateFile = atRoot('data/control/license/state.json')
  const installationKey = atRoot('data/control/keys/installation.key')
  const runnerTaskKey = atRoot('data/control/keys/runner-task.key')
  const runnerTaskPublicKeys = atRoot('data/control/keys/runner-task-public.json')
  const customerDatabasePasswordFile = atRoot('data/control/keys/mariadb.password')
  const ownerPasswordFile = atRoot('data/control/keys/owner-bootstrap.password')
  const licenseRequestFile = atRoot('data/control/license/request.json')
  const generatedPaths = [customerEnv, operationsEnv, installationProfile]
  const existingGeneratedPaths = generatedPaths.filter(existsSync)
  if (existingGeneratedPaths.length === generatedPaths.length && !interactive) {
    log('本地配置已经生成，无需重复初始化。')
    return { status: 'unchanged' }
  }
  if (existingGeneratedPaths.length && !interactive) throw new Error('检测到不完整的本地配置，请在开发控制台重新初始化')

  const sourceEnvPath = atRoot('.env')
  if (!existsSync(sourceEnvPath)) throw new Error('缺少 .env，请先复制 .env.example 并填写本地配置')
  const sourceEnv = parseEnv(readFileSync(sourceEnvPath, 'utf8'))
  const email = required(sourceEnv.ASTER_LOCAL_ADMIN_EMAIL, 'ASTER_LOCAL_ADMIN_EMAIL')
  if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) throw new Error('ASTER_LOCAL_ADMIN_EMAIL 格式无效')
  const securityConfiguration = loadSecurityDirectory(rootDirectory, sourceEnv.ASTER_LOCAL_SECURITY_CONFIG_DIR)
  const operationsAddress = required(
    environment.ASTER_OPERATIONS_ADDR || sourceEnv.ASTER_OPERATIONS_ADDR || '127.0.0.1:12090',
    'ASTER_OPERATIONS_ADDR',
  )
  const operationsConsolePort = localPort(
    environment.ASTER_OPERATIONS_CONSOLE_PORT || sourceEnv.ASTER_OPERATIONS_CONSOLE_PORT,
    12080,
    'ASTER_OPERATIONS_CONSOLE_PORT',
  )
  const controlPort = localPort(
    environment.ASTER_CONTROL_PORT || sourceEnv.ASTER_CONTROL_PORT,
    11080,
    'ASTER_CONTROL_PORT',
  )
  const databasePlans = localDatabasePlans(sourceEnv)
  const databaseInspection = await inspectLocalDatabases(databasePlans, connect)
  const resetPlan = localResetPlan(rootDirectory)
  const databaseSummary = databaseInspection.map(({ plan, exists }) => `${plan.label}=${plan.host}:${plan.port}/${plan.database}${exists ? '（删除重建）' : '（新建）'}；账号=${plan.serviceUser}@${plan.serviceHost}`).join('；')
  if (!interactive) throw new Error(`初始化需要确认数据库创建/覆盖操作：${databaseSummary}`)
  const approved = await confirm(`即将创建专用数据库账号并处理数据库：${databaseSummary}。现有本地配置和演示数据会被覆盖。确认继续？[y/N] `)
  if (!approved) { log('已取消，未修改数据库或本地配置。'); return { status: 'cancelled' } }

  const releaseSigner = securityConfiguration ? null : generateKeyPairSync('ed25519')
  const customerPassword = secret(18)
  const operationsPassword = secret(18)
  const customerDatabasePassword = secret(24)
  const operationsDatabasePassword = secret(24)
  const customerDatabase = runtimeDatabaseEnvironment(databasePlans.find(plan => plan.label === 'Customer'), customerDatabasePassword)
  const operationsDatabase = runtimeDatabaseEnvironment(databasePlans.find(plan => plan.label === 'Operations'), operationsDatabasePassword)
  const productVersion = JSON.parse(readFileSync(atRoot('package.json'), 'utf8')).version
  const artifactRoot = resolve(localRoot, 'operations-artifacts')

  mkdirSync(localRoot, { recursive: true, mode: 0o700 })
  mkdirSync(atRoot('data/control/license'), { recursive: true, mode: 0o700 })
  mkdirSync(atRoot('data/control/keys'), { recursive: true, mode: 0o700 })
  for (const plan of databasePlans) {
    log(`正在重建 ${plan.label} 数据库 ${plan.host}:${plan.port}/${plan.database}…`)
    await provisionLocalDatabase(plan, plan.label === 'Customer' ? customerDatabasePassword : operationsDatabasePassword, connect)
    log(`${plan.label} 数据库已就绪。`)
  }
  removeLocalResetFiles(resetPlan)
  for (const path of [installationProfile, installationKey, runnerTaskKey, runnerTaskPublicKeys, licenseFile, licenseStateFile, licenseRequestFile]) rmSync(path, { force: true })
  mkdirSync(artifactRoot, { recursive: true, mode: 0o700 })

  const writeFlag = existingGeneratedPaths.length ? 'w' : 'wx'
  const releaseTrustedKeysJSON = securityConfiguration?.releaseTrustedKeysJSON || JSON.stringify([{ key_id: 'release-local-v1', public_key_spki: publicSPKI(releaseSigner) }])
  const signingProfile = securityConfiguration || readLicenseSigningProfile(JSON.stringify(generateLicenseSigners(localLicensePolicies())), releaseTrustedKeysJSON)
  const { licenseSignersJSON, licenseTrustedKeysJSON } = signingProfile
  const github = securityConfiguration?.github
  const operationsLines = [
    'ASTER_OPERATIONS_DB_DRIVER=mysql', `ASTER_OPERATIONS_DB_HOST=${operationsDatabase.ASTER_OPERATIONS_DB_HOST}`,
    `ASTER_OPERATIONS_DB_PORT=${operationsDatabase.ASTER_OPERATIONS_DB_PORT}`, `ASTER_OPERATIONS_DB_NAME=${operationsDatabase.ASTER_OPERATIONS_DB_NAME}`,
    `ASTER_OPERATIONS_DB_USER=${operationsDatabase.ASTER_OPERATIONS_DB_USER}`, `ASTER_OPERATIONS_DB_PASSWORD=${operationsDatabase.ASTER_OPERATIONS_DB_PASSWORD}`,
    'ASTER_OPERATIONS_DB_TLS=false', 'ASTER_OPERATIONS_DB_CHARSET=utf8mb4', 'ASTER_OPERATIONS_DB_LOCATION=Local',
    'ASTER_OPERATIONS_DB_MAX_OPEN_CONNS=10', 'ASTER_OPERATIONS_DB_MAX_IDLE_CONNS=10', `ASTER_OPERATIONS_ADDR=${operationsAddress}`,
    'ASTER_OPERATIONS_AUTO_MIGRATE=true', 'ASTER_OPERATIONS_CREATE_DATABASE=false',
    `ASTER_OPERATIONS_BOOTSTRAP_ADMIN_EMAIL=${email}`, `ASTER_OPERATIONS_BOOTSTRAP_ADMIN_PASSWORD=${operationsPassword}`,
    `ASTER_OPERATIONS_LICENSE_V2_SIGNERS_JSON=${licenseSignersJSON}`,
    `ASTER_OPERATIONS_LICENSE_V2_VERIFIERS_JSON=${licenseTrustedKeysJSON}`,
    'ASTER_OPERATIONS_FULFILLMENT_ENVIRONMENT=local', 'ASTER_OPERATIONS_QUOTATION_ENVIRONMENT=local',
    `ASTER_OPERATIONS_CUSTOMER_REF_SECRET=${secret()}`, `ASTER_OPERATIONS_ARTIFACT_ROOT=${artifactRoot}`,
    'ASTER_OPERATIONS_SESSION_SECURE=false', 'ASTER_OPERATIONS_SESSION_TTL=12h',
    `ASTER_OPERATIONS_TRUSTED_ORIGINS=http://127.0.0.1:${operationsConsolePort},http://localhost:${operationsConsolePort}`,
    `ASTER_OPERATIONS_GITHUB_ENABLED=${github ? 'true' : 'false'}`,
    `ASTER_OPERATIONS_GITHUB_APP_ID=${github?.build.appID || ''}`,
    `ASTER_OPERATIONS_GITHUB_INSTALLATION_ID=${github?.build.installationID || ''}`,
    `ASTER_OPERATIONS_GITHUB_APP_PRIVATE_KEY_PEM_BASE64=${github?.build.privateKeyPEMBase64 || ''}`,
    `ASTER_OPERATIONS_GITHUB_REPOSITORY=${github?.repository || 'huzz-open/aster-team'}`,
    'ASTER_OPERATIONS_GITHUB_WORKFLOW=customer-release.yml', 'ASTER_OPERATIONS_GITHUB_ENVIRONMENT=customer-release',
    'ASTER_OPERATIONS_GITHUB_DEFAULT_REF=main', 'ASTER_OPERATIONS_GITHUB_POLL_INTERVAL=15s', 'ASTER_OPERATIONS_GITHUB_REQUEST_TIMEOUT=20s',
    'ASTER_OPERATIONS_GITHUB_ARTIFACT_DOWNLOAD_TIMEOUT=5m',
    `ASTER_OPERATIONS_RELEASE_TRUSTED_KEYS_JSON=${releaseTrustedKeysJSON}`,
    'ASTER_OPERATIONS_RELEASE_ARTIFACT_MAX_BYTES=2147483648', 'ASTER_OPERATIONS_RELEASE_EXPANDED_MAX_BYTES=4294967296',
    `ASTER_OPERATIONS_GITHUB_PUBLISH_ENABLED=${github?.publish ? 'true' : 'false'}`,
    `ASTER_OPERATIONS_GITHUB_PUBLISH_APP_ID=${github?.publish?.appID || ''}`,
    `ASTER_OPERATIONS_GITHUB_PUBLISH_INSTALLATION_ID=${github?.publish?.installationID || ''}`,
    `ASTER_OPERATIONS_GITHUB_PUBLISH_APP_PRIVATE_KEY_PEM_BASE64=${github?.publish?.privateKeyPEMBase64 || ''}`,
    `ASTER_OPERATIONS_GITHUB_PUBLISH_REPOSITORY=${github?.publishRepository || 'huzz-open/aster-team'}`,
    'ASTER_OPERATIONS_GITHUB_PUBLISH_API_URL=https://api.github.com', 'ASTER_OPERATIONS_GITHUB_PUBLISH_UPLOAD_URL=https://uploads.github.com',
    'ASTER_OPERATIONS_GITHUB_PUBLISH_REQUEST_TIMEOUT=2m', '',
  ]
  const customerLines = [
    `BOOTSTRAP_ADMIN_EMAIL=${email}`, 'BOOTSTRAP_ADMIN_PASSWORD_FILE=data/control/keys/owner-bootstrap.password',
    `ASTER_LICENSE_TRUSTED_KEYS_JSON=${licenseTrustedKeysJSON}`,
    `ASTER_RELEASE_TRUSTED_KEYS_JSON=${releaseTrustedKeysJSON}`,
    'ASTER_RUNNER_TASK_KEY_ID=runner-task-local-v1', 'ASTER_RUNNER_TASK_KEY_FILE=data/control/keys/runner-task.key',
    'ASTER_RUNNER_TASK_PUBLIC_KEYS_PATH=data/control/keys/runner-task-public.json',
    'ASTER_CONTROL_HOST=127.0.0.1', `ASTER_CONTROL_PORT=${controlPort}`,
    `ASTER_CONTROL_DB_HOST=${customerDatabase.ASTER_CONTROL_DB_HOST}`, `ASTER_CONTROL_DB_PORT=${customerDatabase.ASTER_CONTROL_DB_PORT}`,
    `ASTER_CONTROL_DB_NAME=${customerDatabase.ASTER_CONTROL_DB_NAME}`, `ASTER_CONTROL_DB_USER=${customerDatabase.ASTER_CONTROL_DB_USER}`,
    'ASTER_CONTROL_DB_PASSWORD_FILE=data/control/keys/mariadb.password', 'ASTER_CONTROL_DB_TLS=false', 'ASTER_CONTROL_DB_MAX_CONNECTIONS=10',
    `PUBLIC_API_BASE_URL=http://127.0.0.1:${controlPort}`, `ASTER_PRODUCT_VERSION=${productVersion}`,
    'ASTER_INSTALLATION_PROFILE_PATH=data/control/license/installation.json', 'ASTER_INSTALLATION_KEY_PATH=data/control/keys/installation.key',
    'ASTER_LICENSE_FILE_PATH=data/control/license/license.json', 'ASTER_LICENSE_STATE_PATH=data/control/license/state.json',
    'ASTER_LICENSE_REQUEST_PATH=data/control/license/request.json', '',
  ]
  for (const [path, lines] of [[operationsEnv, operationsLines], [customerEnv, customerLines]]) {
    writeFileSync(path, lines.join('\n'), { encoding: 'utf8', flag: writeFlag, mode: 0o600 }); chmodSync(path, 0o600)
  }
  if (securityConfiguration) {
    log(`已从仓库外安全目录导入 License 私钥和 License/Release 公钥环：${securityConfiguration.directory}`)
    log(github ? `已写入 Build GitHub App 配置${github.publish ? `，正式发布目标为 ${github.publishRepository}` : ''}；Operations API 启动后可使用远端验证构建。` : '未找到 operations-release-center.json；发布中心保持只读。')
    log('release-v1.seed 未读取、未复制，也未写入 Operations 环境。')
  }
  writeFileSync(customerDatabasePasswordFile, `${customerDatabase.ASTER_CONTROL_DB_PASSWORD}\n`, { encoding: 'utf8', flag: 'w', mode: 0o600 })
  writeFileSync(ownerPasswordFile, `${customerPassword}\n`, { encoding: 'utf8', flag: 'w', mode: 0o600 })
  const controlInitializationSteps = [
    ['--initialize-installation', '生成本机安装身份（首次编译 Rust 可能需要较长时间）'],
    ['--initialize-database', '初始化 Customer 数据库结构'],
    ['--initialize-owner', '创建 Customer 初始管理员'],
    ['--initialize-runner-key', '生成 Runner 任务签名密钥'],
    ['--export-runner-keys', '导出 Runner 公钥'],
    ['--generate-license-request', '生成本机授权申请'],
  ]
  for (const [flag, label] of controlInitializationSteps) {
    log(`正在${label}…`)
    await runControlInitialization(flag)
  }
  rmSync(ownerPasswordFile, { force: true })
  chmodSync(installationProfile, 0o600)
  updateEnvFile(localAdminCredentialsEnv, {
    ASTER_LOCAL_CUSTOMER_EMAIL: email, ASTER_LOCAL_CUSTOMER_PASSWORD: customerPassword,
    ASTER_LOCAL_OPERATIONS_EMAIL: email, ASTER_LOCAL_OPERATIONS_PASSWORD: operationsPassword,
  })
  log('本地离线许可证开发环境已生成。')
  log(`  Customer env:   ${customerEnv}`); log(`  Operations env: ${operationsEnv}`)
  log(`  Customer DB:    ${customerDatabase.ASTER_CONTROL_DB_NAME}`); log(`  Operations DB:  ${operationsDatabase.ASTER_OPERATIONS_DB_NAME}`)
  log(`  本地管理员账号: ${localAdminCredentialsEnv}（权限 0600）`)
  return { status: 'created', customerEnv, operationsEnv, localAdminCredentialsEnv }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await setupLocalStack({ args: process.argv.slice(2) })
