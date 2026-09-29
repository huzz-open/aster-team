import { randomBytes } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseEnv } from 'node:util'
import { spawnSync } from 'node:child_process'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const customerEnvironmentPath = resolve(root, 'data/local/customer.env')
const adminCredentialsPath = resolve(root, 'data/local/local-admin-credentials.env')
const mockCredentialsPath = resolve(root, 'data/local/ui-mock-credentials.env')
const marker = '[mock-ui:v1]'
const mockEmail = 'mock.dashboard@aster.local'

if (!existsSync(customerEnvironmentPath) || !existsSync(adminCredentialsPath)) {
  throw new Error('缺少本地环境文件，请先通过本地开发管理器完成初始化与授权。')
}

const customerEnvironment = parseEnv(readFileSync(customerEnvironmentPath, 'utf8'))
const adminCredentials = parseEnv(readFileSync(adminCredentialsPath, 'utf8'))

function required(value, name) {
  const result = String(value || '').trim()
  if (!result || /[\r\n]/.test(result)) throw new Error(`${name} 不能为空或包含换行符`)
  return result
}

function localSecret() {
  return `Aster-${randomBytes(24).toString('base64url')}!9a`
}

function ensureMockCredentials() {
  if (existsSync(mockCredentialsPath)) return parseEnv(readFileSync(mockCredentialsPath, 'utf8'))
  const credentials = {
    ASTER_UI_MOCK_EMAIL: mockEmail,
    ASTER_UI_MOCK_INITIAL_PASSWORD: localSecret(),
    ASTER_UI_MOCK_PASSWORD: localSecret(),
  }
  mkdirSync(dirname(mockCredentialsPath), { recursive: true, mode: 0o700 })
  writeFileSync(
    mockCredentialsPath,
    Object.entries(credentials).map(([name, value]) => `${name}=${value}`).join('\n') + '\n',
    { encoding: 'utf8', mode: 0o600 },
  )
  return credentials
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
      const error = await response.json().catch(() => null)
      throw new Error(`${method} ${path} 失败: ${error?.error?.code || `HTTP_${response.status}`} ${error?.error?.message || response.statusText}`)
    }
    return response.status === 204 ? null : response.json()
  }
}

const baseURL = customerEnvironment.PUBLIC_API_BASE_URL || 'http://127.0.0.1:11080'
const admin = new APIClient(baseURL, 'http://127.0.0.1:11082')
await admin.request('/api/admin/auth/login', {
  method: 'POST',
  body: {
    email: required(adminCredentials.ASTER_LOCAL_CUSTOMER_EMAIL, 'ASTER_LOCAL_CUSTOMER_EMAIL'),
    password: required(adminCredentials.ASTER_LOCAL_CUSTOMER_PASSWORD, 'ASTER_LOCAL_CUSTOMER_PASSWORD'),
  },
})

const mockCredentials = ensureMockCredentials()
const initialPassword = required(mockCredentials.ASTER_UI_MOCK_INITIAL_PASSWORD, 'ASTER_UI_MOCK_INITIAL_PASSWORD')
const finalPassword = required(mockCredentials.ASTER_UI_MOCK_PASSWORD, 'ASTER_UI_MOCK_PASSWORD')
const desiredEmails = [
  mockEmail,
  ...Array.from({ length: 12 }, (_, index) => `mock.member.${String(index + 1).padStart(2, '0')}@aster.local`),
]
let members = (await admin.request('/api/admin/users')).items
const existingEmails = new Set(members.map(item => item.email))
const missingEmails = desiredEmails.filter(email => !existingEmails.has(email))
if (missingEmails.length) {
  await admin.request('/api/admin/users/batch', {
    method: 'POST',
    csrfCookie: 'aster_admin_csrf',
    body: { emails: missingEmails, password_mode: 'fixed', fixed_password: initialPassword },
  })
  members = (await admin.request('/api/admin/users')).items
}
const mockMember = members.find(item => item.email === mockEmail)
if (!mockMember) throw new Error(`没有找到演示成员 ${mockEmail}`)
console.log(`成员数据已就绪：${members.length} 条。`)

async function loginMockMember() {
  let member = new APIClient(baseURL, 'http://127.0.0.1:11081')
  try {
    await member.request('/api/member/auth/login', { method: 'POST', body: { email: mockEmail, password: finalPassword } })
    return member
  } catch {
    member = new APIClient(baseURL, 'http://127.0.0.1:11081')
    const result = await member.request('/api/member/auth/login', { method: 'POST', body: { email: mockEmail, password: initialPassword } })
    if (result.password_change_required) {
      await member.request('/api/member/auth/password', {
        method: 'POST',
        csrfCookie: 'aster_member_csrf',
        body: { current_password: initialPassword, new_password: finalPassword },
      })
      member = new APIClient(baseURL, 'http://127.0.0.1:11081')
      await member.request('/api/member/auth/login', { method: 'POST', body: { email: mockEmail, password: finalPassword } })
    }
    return member
  }
}

const member = await loginMockMember()
let keys = (await member.request('/api/member/keys')).items
for (let index = keys.filter(key => key.name.startsWith(`${marker} `)).length; index < 12; index += 1) {
  await member.request('/api/member/keys', {
    method: 'POST',
    csrfCookie: 'aster_member_csrf',
    body: { name: `${marker} 演示密钥 ${String(index + 1).padStart(2, '0')}` },
  })
}
keys = (await member.request('/api/member/keys')).items
console.log(`API Key 数据已就绪：${keys.length} 条。`)

const cargo = spawnSync(
  'cargo',
  ['run', '-p', 'aster-control', '--bin', 'seed_local_mock_usage', '--', mockEmail],
  {
    cwd: root,
    env: { ...process.env, ...customerEnvironment },
    encoding: 'utf8',
    shell: process.platform === 'win32',
  },
)
if (cargo.error) throw cargo.error
if (cargo.status !== 0) throw new Error(String(cargo.stderr || cargo.stdout || `额度流水生成失败，退出码 ${cargo.status}`).trim())
console.log(String(cargo.stdout || '').trim())

let quotaRequests = (await member.request(`/api/member/quota-requests?limit=100&offset=0&keyword=${encodeURIComponent(marker)}`)).items
for (const pending of quotaRequests.filter(item => item.status === 'pending')) {
  await admin.request(`/api/admin/quota-requests/${encodeURIComponent(pending.id)}`, {
    method: 'PATCH',
    csrfCookie: 'aster_admin_csrf',
    body: { status: 'rejected', review_note: `${marker} 本地界面验收模拟审批` },
  })
}
for (let index = quotaRequests.length; index < 60; index += 1) {
  await member.request('/api/member/quota-requests', {
    method: 'POST',
    csrfCookie: 'aster_member_csrf',
    body: {
      amount_tokens: 100_000 + (index % 8) * 50_000,
      reason: `${marker} 第 ${String(index + 1).padStart(2, '0')} 次额度申请，用于验证审批列表与筛选。`,
    },
  })
  const pending = (await admin.request(`/api/admin/quota-requests?limit=100&offset=0&status=pending&keyword=${encodeURIComponent(mockEmail)}`)).items
    .find(item => item.user_id === mockMember.id)
  if (!pending) throw new Error('演示额度申请创建后未找到待审批记录')
  await admin.request(`/api/admin/quota-requests/${encodeURIComponent(pending.id)}`, {
    method: 'PATCH',
    csrfCookie: 'aster_admin_csrf',
    body: {
      status: index % 4 === 0 ? 'approved' : 'rejected',
      review_note: `${marker} 本地界面验收模拟审批`,
    },
  })
}
quotaRequests = (await member.request(`/api/member/quota-requests?limit=100&offset=0&keyword=${encodeURIComponent(marker)}`)).items
console.log(`额度申请数据已就绪：${quotaRequests.length} 条。`)

async function ensureVouchers(kind, target, recipientUserIDs) {
  const prefix = `${marker} ${kind}`
  const current = await admin.request(`/api/admin/vouchers?limit=100&offset=0&keyword=${encodeURIComponent(prefix)}`)
  for (let index = current.total; index < target; index += 1) {
    await admin.request('/api/admin/vouchers', {
      method: 'POST',
      csrfCookie: 'aster_admin_csrf',
      body: {
        name: `${prefix} ${String(index + 1).padStart(3, '0')}`,
        quota_tokens: 80_000 + (index % 10) * 20_000,
        max_redemptions: recipientUserIDs.length ? 1 : 5 + (index % 6),
        valid_days: index % 5 === 0 ? null : 30 + (index % 6) * 15,
        recipient_user_ids: recipientUserIDs,
      },
    })
  }
}

await ensureVouchers('成员券', 70, [mockMember.id])
await ensureVouchers('公开券', 60, [])
const voucherTotal = (await admin.request(`/api/admin/vouchers?limit=1&offset=0&keyword=${encodeURIComponent(marker)}`)).total
console.log(`兑换券数据已就绪：${voucherTotal} 条。`)

let deliveries = (await member.request(`/api/member/vouchers?limit=100&offset=0&keyword=${encodeURIComponent(`${marker} 成员券`)}`)).items
const alreadyRedeemed = deliveries.filter(item => item.redeemed_at).length
for (const delivery of deliveries.filter(item => !item.redeemed_at).slice(0, Math.max(0, 20 - alreadyRedeemed))) {
  await member.request('/api/member/vouchers/redeem', {
    method: 'POST',
    csrfCookie: 'aster_member_csrf',
    body: { delivery_id: delivery.delivery_id },
  })
}
deliveries = (await member.request(`/api/member/vouchers?limit=100&offset=0&keyword=${encodeURIComponent(`${marker} 成员券`)}`)).items
console.log(`成员兑换券数据已就绪：${deliveries.length} 条，其中已领取 ${deliveries.filter(item => item.redeemed_at).length} 条。`)

const adminLogs = await admin.request('/api/admin/consumption-logs?limit=1&offset=0')
const audits = await admin.request('/api/admin/audit-events?limit=1&offset=0')
console.log(`消费日志共 ${adminLogs.total} 条；安全审计共 ${audits.total} 条。`)

for (const [period, expectedBuckets] of [['1d', 24], ['7d', 7], ['14d', 14], ['30d', 30]]) {
  const usage = await member.request(`/api/member/usage-summary?period=${period}&granularity=auto&utc_offset_minutes=480`)
  if (usage.trend.length !== expectedBuckets) {
    throw new Error(`${period} 用量趋势应返回 ${expectedBuckets} 个桶，实际为 ${usage.trend.length}`)
  }
}
const paginationChecks = await Promise.all([
  admin.request('/api/admin/consumption-logs?limit=50&offset=50'),
  admin.request('/api/admin/vouchers?limit=50&offset=50'),
  admin.request('/api/admin/quota-requests?limit=50&offset=50'),
  admin.request('/api/admin/audit-events?limit=50&offset=50'),
  member.request('/api/member/quota-requests?limit=6&offset=6'),
  member.request('/api/member/vouchers?limit=8&offset=8'),
])
if (paginationChecks.some(result => !result.items.length)) {
  throw new Error('至少一个演示列表的第二页没有数据')
}
console.log('1/7/14/30 天趋势桶数与六个列表的第二页数据均已验证。')
console.log(`演示成员登录信息保存在 ${mockCredentialsPath}（未输出密码）。`)
