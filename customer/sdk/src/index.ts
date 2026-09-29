export * from './generated/error-codes'
export * from './generated/product-capabilities'
export * from './format'
export * from './random'
export * from './license'

export type AdminMemberOption = { id: string; email: string; display_name: string }
export type RunnerConnectionInfo = { public_api_base_url: string }

export type User = {
  id: string
  email: string
  display_name: string
  status: string
  password_change_required?: boolean
  money_currency?: 'CNY' | 'USD' | null
  money_balance?: string | null
  money_credited?: string | null
  money_debited?: string | null
  balance_tokens: number
  granted_tokens?: number
  used_tokens?: number
  request_count?: number
  raw_tokens?: number
  billed_tokens?: number
  uncached_input_tokens?: number
  cached_input_tokens?: number
  cache_write_tokens?: number
  output_tokens?: number
  created_at: string
}

export type MoneyEntry = {
  id: string
  kind: 'grant' | 'charge' | 'anomaly' | 'request_failed'
  status: string
  amount: string
  reference_id: string
  created_at: string
  details: Record<string, unknown>
}

export type MoneySnapshot = {
  identity_id: string
  currency: 'CNY' | 'USD'
  balance: string
  credited: string
  debited: string
  billing_errors: number
  daily: Array<{ date: string; cost: string; requests: number }>
  models: Array<{ model: string; cost: string; requests: number }>
  entries: MoneyEntry[]
}

export type AdminBillingOverview = { configured: false } | (Omit<MoneySnapshot, 'identity_id' | 'entries'> & {
  configured: true
  members: Array<{ identity_id: string; name: string; email: string; balance: string; credited: string; debited: string }>
  entries: Array<MoneyEntry & { identity_id: string; member_name: string; member_email: string }>
})

export function formatMoney(amount: string | null | undefined, currency: 'CNY' | 'USD' | null | undefined, digits: 2 | 4 = 4): string {
  if (amount == null || !currency) return '—'
  const match = /^(-?)(\d+)(?:\.(\d{1,9}))?$/.exec(amount)
  if (!match) return '—'
  const fraction = (match[3] ?? '').padEnd(9, '0')
  const nanos = BigInt(match[2] ?? '0') * 1_000_000_000n + BigInt(fraction)
  const divisor = 10n ** BigInt(9 - digits)
  const rounded = (nanos + divisor / 2n) / divisor
  const whole = rounded / (10n ** BigInt(digits))
  const decimal = String(rounded % (10n ** BigInt(digits))).padStart(digits, '0')
  const negative = match[1] === '-' && rounded !== 0n ? '-' : ''
  return `${negative}${currency === 'CNY' ? '¥' : '$'}${whole}.${decimal}`
}

export function sumMoneyAmounts(amounts: string[]): string {
  const nanos = amounts.reduce((sum, amount) => {
    const sign = amount.startsWith('-') ? -1n : 1n
    const [whole = '0', fraction = ''] = amount.replace(/^[+-]/, '').split('.')
    return sum + sign * (BigInt(whole) * 1_000_000_000n + BigInt((fraction + '000000000').slice(0, 9)))
  }, 0n)
  const sign = nanos < 0n ? '-' : ''
  const magnitude = nanos < 0n ? -nanos : nanos
  const whole = magnitude / 1_000_000_000n
  const fraction = String(magnitude % 1_000_000_000n).padStart(9, '0').replace(/0+$/, '')
  return `${sign}${whole}${fraction ? `.${fraction}` : ''}`
}

export type APIKey = {
  id: string
  name: string
  key_prefix: string
  status: string
  created_at: string
  last_used_at?: string
}

export type Voucher = {
  id: string
  code_prefix: string
  name: string
  quota_tokens: number
  status: string
  max_redemptions: number
  redeemed_count: number
  delivery_count?: number
  delivered_to_emails?: string
  redeemed_by_emails?: string
  last_redeemed_at?: string
  expires_at?: string | null
  created_at: string
}

export type VoucherDelivery = {
  delivery_id: string
  delivery_status: string
  delivered_at: string
  redeemed_at?: string
  id: string
  name: string
  code_prefix: string
  quota_tokens: number
  expires_at?: string | null
  availability: 'available' | 'redeemed' | 'expired' | 'unavailable'
}

export type LedgerEntry = {
  id: string
  kind: string
  amount_tokens: number
  uncached_input_tokens: number
  cached_input_tokens: number
  cache_write_tokens: number
  output_tokens: number
  uncovered_tokens: number
  raw_tokens: number
  billed_tokens: number
  multiplier: number
  protocol: string
  model: string
  requested_model?: string
  processing_tier?: string
  reasoning_effort?: string
  api_key_id?: string
  runner_id?: string
  runner_name?: string
  client_request_id?: string
  attempt_count: number
  description: string
  created_at: string
}

export type AdminConsumptionEntry = LedgerEntry & {
  user_id: string
  user_email: string
  user_display_name: string
  api_key_name?: string
  api_key_prefix?: string
}

export type ConsumptionSummary = {
  request_count: number
  raw_tokens: number
  billed_tokens: number
  uncovered_tokens: number
}

export type AuditEvent = {
  id: string
  sequence: number
  actor_identity_id?: string
  actor_role: 'owner' | 'admin' | 'member' | 'system' | 'runner'
  actor_email?: string
  actor_display_name?: string
  action: string
  target_type: string
  target_id?: string
  outcome: 'succeeded' | 'failed'
  created_at: string
}

export type ModelPricePreviewRate =
  | { kind: 'tokens'; rates: Record<string, number | null> }
  | { kind: 'context_tokens'; rates: { input_threshold: number; short: Record<string, number | null>; long: Record<string, number | null> } }
  | { kind: 'images'; rates: { unit_prices: Array<{ spec: { size: string; quality: string }; unit_price: { currency: 'CNY' | 'USD'; nanos: number } }> } }

export type Model = {
  id: string
  public_name: string
  upstream_name: string
  display_name: string
  enabled: boolean
  discovered_at?: string
  provider?: string
  source_account_email?: string
  available?: boolean
  price_source?: 'manual' | 'official' | 'builtin' | null
  price_preview?: {
    currency: 'CNY' | 'USD'
    rate: ModelPricePreviewRate
    has_other_prices: boolean
  } | null
}

export type Runner = {
  id: string
  name: string
  enabled: boolean
  online: boolean
  version: string
  platform: string
  architecture: string
  protocol_version: number
  max_inflight: number
  inflight: number
  recent_request_count: number
  recent_error_count: number
  latency_ms: number
  last_seen_at?: string
  created_at: string
}

export class APIError extends Error {
  constructor(public code: string, message: string, public status: number, public number?: number) {
    super(number ? `${message}（错误码：${number}）` : message)
    this.name = 'APIError'
  }
}

export async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers)
  if (init.body && !(init.body instanceof FormData) && !(init.body instanceof Blob) && !headers.has('Content-Type')) headers.set('Content-Type', 'application/json')
  const response = await fetch(path, { credentials: 'include', ...init, headers })
  if (response.status === 204) return undefined as T
  const body = await response.json().catch(() => ({}))
  if (!response.ok) {
    const error = body?.error
    const number = Number.isSafeInteger(error?.number) ? error.number as number : undefined
    throw new APIError(error?.code ?? 'REQUEST_FAILED', error?.message ?? '请求失败', response.status, number)
  }
  return body as T
}

export function formatTokens(value: number, locale = 'zh-CN'): string {
  return new Intl.NumberFormat(locale).format(value)
}

export function formatDate(value?: string, locale = 'zh-CN'): string {
  return value
    ? new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(value))
    : '—'
}

export async function copyText(value: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(value)
      return
    } catch { /* fall back for HTTP pages and denied clipboard permissions */ }
  }
  const textarea = document.createElement('textarea')
  textarea.value = value
  textarea.setAttribute('readonly', '')
  textarea.style.position = 'fixed'
  textarea.style.left = '-9999px'
  textarea.style.opacity = '0'
  document.body.appendChild(textarea)
  textarea.select()
  textarea.setSelectionRange(0, textarea.value.length)
  try {
    if (!document.execCommand('copy')) throw new Error('浏览器拒绝了复制操作')
  } finally {
    textarea.remove()
  }
}
