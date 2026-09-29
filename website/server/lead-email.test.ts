import assert from 'node:assert/strict'
import test from 'node:test'
import { buildLeadEmailContent } from './lead-email'
import type { ParsedLead } from './lead-model'

function lead(overrides: Partial<ParsedLead> = {}): ParsedLead {
  return {
    contact: '系统邮件链路测试（请忽略）',
    company: 'Aster Team deployment verification',
    teamSize: 3,
    activeUsers: 1,
    evidence: 'time',
    weeklyTokens: null,
    dailyTime: 'under1h',
    usageLevel: 0,
    recommendedProAccounts: 1,
    locale: 'zh',
    turnstileToken: 'not-in-email',
    honeypotTriggered: false,
    ...overrides,
  }
}

test('email lists the validated user fields exactly and labels computed values separately', () => {
  const message = buildLeadEmailContent('lead-123', lead(), '2026-08-29T17:36:47.467Z')
  assert.match(message.body, /【用户填写内容】/)
  assert.match(message.body, /联系方式：系统邮件链路测试（请忽略）/)
  assert.match(message.body, /企业 \/ 团队：Aster Team deployment verification/)
  assert.match(message.body, /平均每天使用 AI 的时间：小于 1 小时/)
  assert.match(message.body, /"dailyTime": "under1h"/)
  assert.match(message.body, /【系统评估结果（仅供参考）】/)
  assert.match(message.body, /使用强度：轻量使用/)
  assert.doesNotMatch(message.body, /评估等级：0/)
  assert.doesNotMatch(message.body, /not-in-email/)
})

test('email preserves the submitted token amount for token-based assessment', () => {
  const message = buildLeadEmailContent('lead-456', lead({
    evidence: 'tokens', weeklyTokens: 41.75, dailyTime: null, usageLevel: 1, recommendedProAccounts: 2,
  }), '2026-08-30T01:00:00.000Z')
  assert.match(message.body, /团队近 7 天 Token 用量：41\.75 亿 Token/)
  assert.match(message.body, /"weeklyTokens": 41\.75/)
  assert.match(message.body, /"dailyTime": null/)
})
