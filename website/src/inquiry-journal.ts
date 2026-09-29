import validate from '../shared/generated/inquiry-validator.js'
import type { ProductInquiry } from '../shared/generated/contracts'

export type PendingInquiry = { payload: Omit<ProductInquiry, 'turnstile_token'>; label: string }
const key = 'aster:product-inquiry-pending:v1'

export function readPendingInquiry(storage: Pick<Storage, 'getItem'>): PendingInquiry | null {
  const raw = storage.getItem(key)
  if (raw === null) return null
  if (raw.length > 40_000) throw new Error('invalid inquiry recovery record')
  const value: unknown = JSON.parse(raw)
  if (typeof value !== 'object' || value === null || Array.isArray(value)) throw new Error('invalid inquiry recovery record')
  const record = value as Record<string, unknown>
  if (Object.keys(record).length !== 3 || record.version !== 1 || typeof record.label !== 'string' || record.label.length > 400
    || typeof record.payload !== 'object' || record.payload === null || Array.isArray(record.payload)) throw new Error('invalid inquiry recovery record')
  if ('turnstile_token' in record.payload) throw new Error('inquiry recovery must not store verification tokens')
  const candidate = { ...record.payload, turnstile_token: 'pending' }
  if (!validate(candidate)) throw new Error('invalid inquiry recovery record')
  const { turnstile_token: _token, ...payload } = candidate
  return { payload, label: record.label }
}

export function savePendingInquiry(storage: Pick<Storage, 'setItem'>, value: PendingInquiry): void {
  storage.setItem(key, JSON.stringify({ version: 1, payload: value.payload, label: value.label }))
}

export function clearPendingInquiry(storage: Pick<Storage, 'getItem' | 'removeItem'>, requestId?: string): void {
  if (requestId && readPendingInquiry(storage)?.payload.request_id !== requestId) throw new Error('inquiry recovery identity changed')
  storage.removeItem(key)
}
