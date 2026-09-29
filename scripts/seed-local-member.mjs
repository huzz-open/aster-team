import { randomBytes } from 'node:crypto'
import { existsSync, readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseEnv } from 'node:util'
import { updateEnvFile } from './local-env-file.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const customerEnvironmentPath = resolve(root, 'data/local/customer.env')
const credentialsPath = resolve(root, 'data/local/local-admin-credentials.env')
const memberEmail = 'test@at.com'
const initialQuotaTokens = 1_000_000

if (!existsSync(customerEnvironmentPath) || !existsSync(credentialsPath)) {
  throw new Error('缺少本地环境文件，请先通过本地开发管理器完成初始化与授权。')
}

const customerEnvironment = parseEnv(readFileSync(customerEnvironmentPath, 'utf8'))
const credentials = parseEnv(readFileSync(credentialsPath, 'utf8'))

function required(value, name) {
  const result = String(value || '').trim()
  if (!result || /[\r\n]/.test(result)) throw new Error(`${name} 不能为空或包含换行符`)
  return result
}

function temporaryPassword() {
  return `Aster-${randomBytes(24).toString('base64url')}!9a`
}

class APIError extends Error {
  constructor(message, code, status) {
    super(message)
    this.code = code
    this.status = status
  }
}

class APIClient {
  constructor(baseURL, origin) {
    this.baseURL = baseURL.replace(/\/$/, '')
    this.origin = origin
    this.cookies = new Map()
  }

  captureCookies(response) {
    const values = typeof response.headers.getSetCookie === 'function'
      ? response.headers.getSetCookie()
      : [response.headers.get('set-cookie') || '']
    for (const value of values) {
      for (const match of value.matchAll(/(?:^|,\s*)([A-Za-z0-9_]+)=([^;,]*)/g)) {
        this.cookies.set(match[1], match[2])
      }
    }
  }

  async request(path, { method = 'GET', body, csrfCookie } = {}) {
    const headers = { Accept: 'application/json', Origin: this.origin }
    if (this.cookies.size) headers.Cookie = [...this.cookies].map(([name, value]) => `${name}=${value}`).join('; ')
    if (body !== undefined) headers['Content-Type'] = 'application/json'
    if (csrfCookie) headers['X-CSRF-Token'] = decodeURIComponent(this.cookies.get(csrfCookie) || '')
    const response = await fetch(`${this.baseURL}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    })
    this.captureCookies(response)
    if (!response.ok) {
      const payload = await response.json().catch(() => null)
      const code = payload?.error?.code || `HTTP_${response.status}`
      const detail = payload?.error?.message || response.statusText || '请求失败'
      throw new APIError(`${method} ${path} 失败: ${code}: ${detail}`, code, response.status)
    }
    return response.status === 204 ? null : response.json()
  }
}

const controlPort = process.env.ASTER_CONTROL_PORT || customerEnvironment.ASTER_CONTROL_PORT || '11080'
const customerAdminPort = process.env.ASTER_CUSTOMER_ADMIN_PORT || '11082'
const baseURL = process.env.PUBLIC_API_BASE_URL
  || (process.env.ASTER_CONTROL_PORT ? `http://127.0.0.1:${controlPort}` : customerEnvironment.PUBLIC_API_BASE_URL)
  || `http://127.0.0.1:${controlPort}`
const customerEmail = required(credentials.ASTER_LOCAL_CUSTOMER_EMAIL, 'ASTER_LOCAL_CUSTOMER_EMAIL')
const customerPassword = required(credentials.ASTER_LOCAL_CUSTOMER_PASSWORD, 'ASTER_LOCAL_CUSTOMER_PASSWORD')
const recordedMemberPassword = String(credentials.ASTER_LOCAL_MEMBER_PASSWORD || '')
const admin = new APIClient(baseURL, `http://127.0.0.1:${customerAdminPort}`)
const adminLogin = await admin.request('/api/admin/auth/login', {
  method: 'POST',
  body: { email: customerEmail, password: customerPassword },
})
if (adminLogin?.password_change_required !== false) {
  throw new Error('Customer 仍在使用初始化密码；请先完成管理端首次改密。')
}

let members = (await admin.request('/api/admin/users')).items
let member = members.find(item => item.email === memberEmail)
let currentPassword = ''

if (!member) {
  currentPassword = temporaryPassword()
  member = await admin.request('/api/admin/users', {
    method: 'POST',
    csrfCookie: 'aster_admin_csrf',
    body: { email: memberEmail, display_name: 'Local Test', password: currentPassword },
  })
  console.log(`用户侧测试账号已创建：${memberEmail}`)
}

if (member.status && member.status !== 'active') {
  await admin.request(`/api/admin/users/${encodeURIComponent(member.id)}`, {
    method: 'PATCH',
    csrfCookie: 'aster_admin_csrf',
    body: { enabled: true },
  })
  member = { ...member, status: 'active' }
  console.log(`用户侧测试账号已重新启用：${memberEmail}`)
}

if (Number(member.balance_tokens || 0) <= 0) {
  const quota = await admin.request(`/api/admin/users/${encodeURIComponent(member.id)}/quota-adjustments`, {
    method: 'POST',
    csrfCookie: 'aster_admin_csrf',
    body: {
      amount_tokens: initialQuotaTokens,
      reason: '本地快速授权初始化测试额度',
      request_id: `local-demo-quota-${randomBytes(12).toString('hex')}`,
    },
  })
  member = { ...member, balance_tokens: quota.balance_tokens }
  console.log(`已向用户侧测试账号发放 ${initialQuotaTokens} Token 初始额度。`)
}

async function tryMemberLogin(password) {
  const client = new APIClient(baseURL, 'http://127.0.0.1:11081')
  try {
    const result = await client.request('/api/member/auth/login', {
      method: 'POST',
      body: { email: memberEmail, password },
    })
    return { client, result }
  } catch (error) {
    if (error instanceof APIError && error.code === 'MEMBER_INVALID_CREDENTIALS') return null
    throw error
  }
}

let verified = await tryMemberLogin(customerPassword)
if (verified?.result?.password_change_required) {
  verified = null
  currentPassword = ''
}

if (!verified && !currentPassword && recordedMemberPassword && recordedMemberPassword !== customerPassword) {
  const recorded = await tryMemberLogin(recordedMemberPassword)
  if (recorded) currentPassword = recordedMemberPassword
}

if (!verified && !currentPassword) {
  const reset = await admin.request(`/api/admin/users/${encodeURIComponent(member.id)}/password-reset`, {
    method: 'POST',
    csrfCookie: 'aster_admin_csrf',
  })
  currentPassword = required(reset?.temporary_password, '成员临时密码')
  console.log(`用户侧测试账号已重置为一次性临时密码：${memberEmail}`)
}

if (!verified) {
  const initial = await tryMemberLogin(currentPassword)
  if (!initial) throw new Error(`无法使用刚生成的临时密码登录用户侧账号 ${memberEmail}`)
  await initial.client.request('/api/member/auth/password', {
    method: 'POST',
    csrfCookie: 'aster_member_csrf',
    body: { current_password: currentPassword, new_password: customerPassword },
  })
  verified = await tryMemberLogin(customerPassword)
}

if (!verified || verified.result?.password_change_required !== false) {
  throw new Error(`用户侧账号 ${memberEmail} 的目标密码验证失败，未写入本地凭据文件。`)
}

updateEnvFile(credentialsPath, {
  ASTER_LOCAL_MEMBER_EMAIL: memberEmail,
  ASTER_LOCAL_MEMBER_PASSWORD: customerPassword,
  ASTER_LOCAL_MEMBER_QUOTA_READY: 'true',
})
console.log(`用户侧测试账号已就绪：${memberEmail}；密码与初始额度已验证并安全写入本地账号文件。`)
