import { spawnSync } from 'node:child_process'
import { chmodSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseEnv } from 'node:util'
import { fulfillLocalDemo, newLocalFulfillment, requireLocalFulfillmentEnvironment } from './local-commercial-license.mjs'
import { readCustomerReleaseProfile } from './customer-release-profile.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const credentialsPath = resolve(root, 'data/local/local-admin-credentials.env')
const customerEnvironmentPath = resolve(root, 'data/local/customer.env')
if (!existsSync(credentialsPath) || !existsSync(customerEnvironmentPath)) throw new Error('缺少本地初始化文件，请先执行 npm run setup:local')
const credentials = parseEnv(readFileSync(credentialsPath, 'utf8'))
const customerEnvironment = parseEnv(readFileSync(customerEnvironmentPath, 'utf8'))
const progressPrefix = '@@ASTER_PROGRESS@@'

function progress(step, state, detail) { console.log(progressPrefix + JSON.stringify({ step, state, detail })) }
function required(value, name) {
  const result = String(value || '').trim()
  if (!result || /[\r\n]/.test(result)) throw new Error(`${name} 不能为空或包含换行符`)
  return result
}

class APIClient {
  constructor(baseURL, origin) {
    const url = new URL(baseURL)
    if (url.protocol !== 'http:' || !['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname) || url.username || url.password) throw new Error('本地授权只允许无凭据的回环 HTTP 地址')
    this.baseURL = baseURL.replace(/\/$/, '')
    this.origin = origin
    this.cookies = new Map()
  }
  captureCookies(response) {
    const values = typeof response.headers.getSetCookie === 'function' ? response.headers.getSetCookie() : [response.headers.get('set-cookie') || '']
    for (const value of values) for (const match of value.matchAll(/(?:^|,\s*)([A-Za-z0-9_]+)=([^;,]*)/g)) this.cookies.set(match[1], match[2])
  }
  async request(path, { method = 'GET', body, csrfCookie, format = 'json' } = {}) {
    const headers = { Accept: 'application/json', Origin: this.origin }
    if (this.cookies.size) headers.Cookie = [...this.cookies].map(([name, value]) => `${name}=${value}`).join('; ')
    if (body !== undefined) headers['Content-Type'] = 'application/json'
    if (csrfCookie) headers['X-CSRF-Token'] = decodeURIComponent(this.cookies.get(csrfCookie) || '')
    const payload = body === undefined ? undefined : typeof body === 'string' ? body : JSON.stringify(body)
    const response = await fetch(`${this.baseURL}${path}`, { method, headers, body: payload, redirect: 'error' })
    this.captureCookies(response)
    if (!response.ok) {
      const error = await response.json().catch(() => null)
      throw new Error(`${method} ${path} 失败: ${error?.error?.code || `HTTP_${response.status}`}: ${error?.error?.message || response.statusText || '请求失败'}`)
    }
    if (format === 'bytes') return { bytes: Buffer.from(await response.arrayBuffer()), sha256: response.headers.get('X-Content-SHA256') }
    if (response.status === 204) return null
    return response.json()
  }
}

const operationsEmail = required(credentials.ASTER_LOCAL_OPERATIONS_EMAIL, 'ASTER_LOCAL_OPERATIONS_EMAIL')
const operationsPassword = required(credentials.ASTER_LOCAL_OPERATIONS_PASSWORD, 'ASTER_LOCAL_OPERATIONS_PASSWORD')
const customerEmail = required(credentials.ASTER_LOCAL_CUSTOMER_EMAIL, 'ASTER_LOCAL_CUSTOMER_EMAIL')
const customerPassword = required(credentials.ASTER_LOCAL_CUSTOMER_PASSWORD, 'ASTER_LOCAL_CUSTOMER_PASSWORD')
const operationsAddress = process.env.ASTER_OPERATIONS_ADDR || '127.0.0.1:12090'
const operationsConsolePort = process.env.ASTER_OPERATIONS_CONSOLE_PORT || '12080'
const controlPort = process.env.ASTER_CONTROL_PORT || customerEnvironment.ASTER_CONTROL_PORT || '11080'
const customerAdminPort = process.env.ASTER_CUSTOMER_ADMIN_PORT || '11082'
const operations = new APIClient(
  credentials.ASTER_LOCAL_OPERATIONS_API_URL || `http://${operationsAddress}/api/operations/v1`,
  `http://127.0.0.1:${operationsConsolePort}`,
)
let operationsLogin
try {
  operationsLogin = await operations.request('/session', { method: 'POST', body: { email: operationsEmail, password: operationsPassword } })
} catch (error) {
  if (String(error?.message || error).includes('INVALID_CREDENTIALS')) throw new Error('Operations 当前密码无效；请重新输入 Operations Console 正在使用的密码。')
  throw error
}
if (operationsLogin?.operator?.password_change_required !== false) {
  throw new Error('Operations 仍在使用初始化密码；请先在 Operations Console 完成首次改密，并同步本地账号文件。')
}
progress('operations', 'completed', 'Operations API 登录验证通过')
await requireLocalFulfillmentEnvironment(operations)

const customer = new APIClient(
  process.env.PUBLIC_API_BASE_URL
    || (process.env.ASTER_CONTROL_PORT ? `http://127.0.0.1:${controlPort}` : customerEnvironment.PUBLIC_API_BASE_URL)
    || `http://127.0.0.1:${controlPort}`,
  `http://127.0.0.1:${customerAdminPort}`,
)
let customerLogin
try {
  customerLogin = await customer.request('/api/admin/auth/login', { method: 'POST', body: { email: customerEmail, password: customerPassword } })
} catch (error) {
  if (String(error?.message || error).includes('INVALID_CREDENTIALS')) throw new Error('Customer 当前密码无效；请更新 local-admin-credentials.env 后重试。')
  throw error
}
if (customerLogin?.password_change_required !== false) {
  throw new Error('Customer 仍在使用初始化密码；请先在 Customer Admin 完成首次改密，并同步本地账号文件。')
}

progress('order', 'running', '正在准备演示客户、套餐和订单')
const marker = '[local-demo:v2-commercial]'
let demoCustomer = (await operations.request('/customers?limit=100')).items.find(item => item.notes.includes(marker))
if (!demoCustomer) {
  demoCustomer = await operations.request('/customers', {
    method: 'POST', csrfCookie: 'aster_operations_csrf', body: {
      name: 'Aster 本地演示客户', legal_name: 'Aster 本地演示客户有限公司', status: 'active',
      contact_name: '本地联调负责人', contact_email: 'demo.customer@aster.local', contact_phone: '13800000000',
      contact_wechat: 'aster-local-demo', notes: `${marker} 由本地开发控制台创建。`,
    },
  })
}
progress('request', 'running', '正在读取 Rust Control 生成的 v2 机器申请文件')
const requestPath = resolve(root, required(customerEnvironment.ASTER_LICENSE_REQUEST_PATH, 'ASTER_LICENSE_REQUEST_PATH'))
const requestJSON = readFileSync(requestPath, 'utf8')
const machineRequest = JSON.parse(requestJSON)
if (machineRequest.schema !== 'aster.license-request.v2') throw new Error('机器申请不是 v2，请重新初始化本地环境')
progress('request', 'completed', `机器申请已读取：${machineRequest.request_id}`)
const output = resolve(root, 'data/local/demo-delivery')
mkdirSync(output, { recursive: true, mode: 0o700 })
const statePath = resolve(output, 'fulfillment-input.json')
const state = existsSync(statePath) ? JSON.parse(readFileSync(statePath, 'utf8')) : newLocalFulfillment(requestJSON, demoCustomer.id)
if (!existsSync(statePath)) writeFileSync(statePath, `${JSON.stringify(state)}\n`, { encoding: 'utf8', flag: 'wx', mode: 0o600 })
if (state.customer_id !== demoCustomer.id) throw new Error('本地履约上下文客户不一致，请重新初始化本地环境')
const trust = readCustomerReleaseProfile(customerEnvironment)
progress('policy', 'running', '正在使用商业 v2 流程创建测试套餐与已付款订单')
progress('issuance', 'running', 'Operations 正在核对批准快照与签发范围')
const { free, issued, bytes, sha256 } = await fulfillLocalDemo({
  operations, password: operationsPassword, requestJSON, minimumVersion: machineRequest.product_version,
  trustedKeys: trust.licenseTrustedKeys, state, progress,
})
const copiedRequestPath = resolve(output, 'local-machine-license-request.json')
const licensePath = resolve(output, 'local-machine-license.json')
const freeLicensePath = resolve(output, 'free-license.json')
writeFileSync(copiedRequestPath, requestJSON, { encoding: 'utf8', mode: 0o600 })
writeFileSync(licensePath, bytes, { mode: 0o600 })
writeFileSync(freeLicensePath, free.bytes, { mode: 0o600 })
chmodSync(copiedRequestPath, 0o600); chmodSync(licensePath, 0o600); chmodSync(freeLicensePath, 0o600)
progress('policy', 'completed', `本地测试免费证书已保存：SHA-256 ${free.sha256}`)
progress('issuance', 'completed', `v2 机器许可证已签发：SHA-256 ${sha256}`)

progress('install', 'running', '正在通过 Customer Admin 接口导入并验证许可证')
await customer.request('/api/admin/license', { method: 'POST', body: bytes.toString('utf8') })
progress('install', 'completed', `许可证已验签、安装并生效，到期时间：${issued.document.claims.validity.expiry.expires_at}`)

progress('runner', 'running', '正在检查并注册本机 Runner')
const runnerRoot = resolve(root, 'data/runner')
const runnerTokenPath = resolve(runnerRoot, 'enrollment.token')
const runnerIdentityPath = resolve(runnerRoot, 'identity.json')
const runnerTaskKeysPath = resolve(runnerRoot, 'task-keys.json')
const hasRunnerIdentity = existsSync(runnerIdentityPath)
const hasRunnerTaskKeys = existsSync(runnerTaskKeysPath)
if (hasRunnerIdentity !== hasRunnerTaskKeys) {
  throw new Error('Runner 本地身份文件不完整；请删除 data/runner/identity.json 和 data/runner/task-keys.json 后重新执行本地授权。')
}
if (hasRunnerIdentity) {
  progress('runner', 'completed', '本机 Runner 已注册，继续复用现有身份')
} else {
  const enrollment = await customer.request('/api/admin/runners/enrollments', {
    method: 'POST', body: { name: 'local-runner' },
  })
  mkdirSync(runnerRoot, { recursive: true, mode: 0o700 })
  writeFileSync(runnerTokenPath, `${required(enrollment.token, 'Runner 注册 Token')}\n`, { encoding: 'utf8', mode: 0o600 })
  chmodSync(runnerTokenPath, 0o600)
  try {
    const result = spawnSync('cargo', [
      'run', '-p', 'aster-runner', '--features', 'local-demo', '--', 'enroll',
      '--control-url', customer.baseURL,
      '--token-file', runnerTokenPath,
      '--identity-file', runnerIdentityPath,
      '--task-keys-file', runnerTaskKeysPath,
    ], { cwd: root, env: process.env, encoding: 'utf8', shell: process.platform === 'win32' })
    if (result.error) throw result.error
    if (result.status !== 0) {
      const detail = String(result.stderr || result.stdout || `退出码 ${result.status}`).trim()
      throw new Error(`Runner 注册失败：${detail}`)
    }
  } finally {
    rmSync(runnerTokenPath, { force: true })
  }
  progress('runner', 'completed', '本机 Runner 身份与任务公钥已安全写入 data/runner/')
}

console.log('本地演示订单、机器申请和离线许可证已准备完成。')
console.log(`  Operations:     http://127.0.0.1:12080/commercial/orders`)
console.log(`  Customer Admin: http://127.0.0.1:11082/license`)
console.log(`  离线文件:       ${output}`)
console.log('本机 Runner 已可直接启动；许可证、注册 Token 和登录密码未输出到控制台。')
