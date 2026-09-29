import { ref, shallowRef } from 'vue'

type Input = { operation_id: string }
type Entry<T extends Input> = { schema: 'aster.operations.pending.v1'; owner: string; uncertain: boolean; attempt_id?: string; input: T }
type Attempt = { id: string; wasUncertain: boolean }

// Per operator and tab: survives component teardown, route changes and reloads.
// Never stores credentials or authorizes an operation. The server remains the
// authority for permissions, payload validation and idempotency.
export function submissionJournal<T extends Input>(kind: 'plan' | 'order' | 'quotation-order' | 'payment' | 'distribution' | 'fulfillment' | 'fulfillment-issue' | 'fulfillment-redelivery' | 'paid-transfer' | 'paid-transfer-issue' | 'draft' | 'draft-freeze' | 'catalog' | 'publication', owner: string, storage: Storage = sessionStorage) {
  const key = `aster.operations.pending.v1:${kind}:${owner}`
  const pending = shallowRef<T | null>(null)
  const error = ref('')
  try {
    if (!owner) throw new Error('缺少当前运营账号身份，请重新登录')
    const raw = storage.getItem(key)
    if (raw) {
      const entry = JSON.parse(raw) as Entry<T>
      if (entry.schema !== 'aster.operations.pending.v1' || entry.owner !== owner || typeof entry.uncertain !== 'boolean' || !entry.input || typeof entry.input.operation_id !== 'string' || !/^[A-Za-z0-9._:@+/-]{1,128}$/.test(entry.input.operation_id) || (entry.attempt_id !== undefined && typeof entry.attempt_id !== 'string')) throw new Error('待确认请求记录损坏，请保留记录并核对原操作')
      pending.value = entry.input
    }
  } catch (value) { error.value = value instanceof Error ? value.message : '读取待确认请求失败' }
  function persist(input: T, nextUncertain: boolean, attemptID: string) {
    if (error.value) throw new Error(error.value)
    storage.setItem(key, JSON.stringify({ schema: 'aster.operations.pending.v1', owner, uncertain: nextUncertain, attempt_id: attemptID, input } satisfies Entry<T>))
    pending.value = input
  }
  function isCurrent(input: T) {
    const raw = storage.getItem(key)
    if (!raw) return null
    const entry = JSON.parse(raw) as Entry<T>
    return entry.schema === 'aster.operations.pending.v1' && entry.owner === owner && entry.input?.operation_id === input.operation_id && JSON.stringify(entry.input) === JSON.stringify(input) ? entry : null
  }
  function clear() {
    // Only a confirmed success/absence, or the unsuperseded first rejection,
    // may clear. A late result for another operation cannot delete its record.
    if (pending.value && isCurrent(pending.value)) storage.removeItem(key)
    pending.value = null
  }
  return {
    pending, error,
    prepare(input: T) {
      if (pending.value || storage.getItem(key)) throw new Error('请先核对待确认请求')
      persist(JSON.parse(JSON.stringify(input)) as T, false, '')
    },
    begin() {
      if (!pending.value) throw new Error('没有可重试的请求')
      const current = isCurrent(pending.value)
      if (!current) throw new Error('待确认操作已改变，请刷新页面核对当前请求')
      const attempt: Attempt = { id: crypto.randomUUID(), wasUncertain: current.uncertain }
      // Persist before dispatch: reloading while a request is in flight is
      // equivalent to losing its response, even if no catch block ran.
      persist(pending.value, true, attempt.id)
      return attempt
    },
    reject(attempt: Attempt) {
      const current = pending.value && isCurrent(pending.value)
      if (attempt.wasUncertain || !current || current.attempt_id !== attempt.id) return false
      clear()
      return true
    },
    clear,
  }
}
