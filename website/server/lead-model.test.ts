import assert from 'node:assert/strict'
import test from 'node:test'
import { InvalidLeadError, assessUsage, parseLeadPayload } from './lead-model'

test('assesses normal time-based usage at three users per Pro account', () => {
  assert.deepEqual(assessUsage(10, 'time', null, '1h-4h'), { level: 1, recommended: 4 })
})

test('assesses over-eight-hour usage at two users per Pro account', () => {
  assert.deepEqual(assessUsage(10, 'time', null, 'over8h'), { level: 2, recommended: 5 })
})

test('token evidence never recommends below the active-user baseline', () => {
  assert.deepEqual(assessUsage(10, 'tokens', 32, null), { level: 0, recommended: 4 })
  assert.deepEqual(assessUsage(3, 'tokens', 65, null), { level: 2, recommended: 3 })
})

test('parses a valid lead and recomputes assessment server-side', () => {
  const lead = parseLeadPayload({
    contact: 'team@example.com',
    company: 'Aster Studio',
    teamSize: 10,
    activeUsers: 4,
    evidence: 'time',
    weeklyTokens: 99999,
    dailyTime: '4h-8h',
    usageLevel: 2,
    recommendedProAccounts: 99,
    locale: 'zh',
    turnstileToken: 'token',
    website: '',
  })
  assert.equal(lead.usageLevel, 1)
  assert.equal(lead.recommendedProAccounts, 2)
  assert.equal(lead.weeklyTokens, null)
})

test('rejects active users above team size', () => {
  assert.throws(() => parseLeadPayload({
    contact: 'team@example.com',
    teamSize: 3,
    activeUsers: 4,
    evidence: 'time',
    dailyTime: '1h-4h',
    turnstileToken: 'token',
  }), InvalidLeadError)
})
