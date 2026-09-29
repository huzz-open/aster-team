import { beforeEach, expect, test } from 'vitest'
import { submissionJournal } from '../src/commercial/submission-journal'

beforeEach(() => sessionStorage.clear())

test('dispatch is marked uncertain before network work and restored after teardown', () => {
  const journal = submissionJournal('order', 'operator_1')
  journal.prepare({ operation_id: 'order_test' })
  expect(journal.begin().wasUncertain).toBe(false)
  const restored = submissionJournal('order', 'operator_1')
  expect(restored.pending.value).toEqual({ operation_id: 'order_test' })
  expect(restored.begin().wasUncertain).toBe(true)
  // A second operator cannot pick up this operator's request.
  expect(submissionJournal('order', 'operator_2').pending.value).toBeNull()
  restored.clear()
  expect(submissionJournal('order', 'operator_1').pending.value).toBeNull()
})

test('blocked storage prevents preparing a request instead of silently losing retries', () => {
  const unavailable = { getItem: () => null, setItem: () => { throw new Error('storage unavailable') }, removeItem: () => {} } as unknown as Storage
  const journal = submissionJournal('plan', 'operator_1', unavailable)
  expect(() => journal.prepare({ operation_id: 'plan_test' })).toThrow('storage unavailable')
  expect(journal.pending.value).toBeNull()
})

test('malformed stored state is retained and blocks a new identity', () => {
  const key = 'aster.operations.pending.v1:order:operator_1'
  sessionStorage.setItem(key, '{broken')
  const journal = submissionJournal('order', 'operator_1')
  expect(journal.error.value).not.toBe('')
  expect(() => journal.prepare({ operation_id: 'new_order' })).toThrow()
  expect(sessionStorage.getItem(key)).toBe('{broken')
})

test('late old responses cannot clear or overwrite a newer request from a remounted page', () => {
  const old = submissionJournal('order', 'operator_1')
  old.prepare({ operation_id: 'old_order' }); old.begin()
  const active = submissionJournal('order', 'operator_1')
  active.begin(); active.clear()
  active.prepare({ operation_id: 'new_order' }); active.begin()
  expect(() => old.begin()).toThrow('已改变')
  old.clear()
  expect(submissionJournal('order', 'operator_1').pending.value).toEqual({ operation_id: 'new_order' })
  expect(() => old.prepare({ operation_id: 'third_order' })).toThrow('请先核对')
  expect(submissionJournal('order', 'operator_1').pending.value).toEqual({ operation_id: 'new_order' })
})

test('a late first rejection cannot clear a later uncertain retry of the same operation', () => {
  const firstPage = submissionJournal('order', 'operator_1')
  firstPage.prepare({ operation_id: 'same_order' })
  const firstAttempt = firstPage.begin()
  const secondPage = submissionJournal('order', 'operator_1')
  const retry = secondPage.begin()
  expect(retry.id).not.toBe(firstAttempt.id)
  // First request now reports 403, but the subsequent retry may have committed.
  expect(firstPage.reject(firstAttempt)).toBe(false)
  expect(secondPage.reject(retry)).toBe(false)
  const restored = submissionJournal('order', 'operator_1')
  expect(restored.pending.value).toEqual({ operation_id: 'same_order' })
  expect(restored.begin().wasUncertain).toBe(true)
})

test('only the first unsuperseded definitive rejection may release the identity', () => {
  const journal = submissionJournal('plan', 'operator_1')
  journal.prepare({ operation_id: 'rejected_plan' })
  expect(journal.reject(journal.begin())).toBe(true)
  expect(sessionStorage.length).toBe(0)
})
