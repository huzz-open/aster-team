import type { ParsedLead } from './lead-model'

function usageTimeLabel(value: ParsedLead['dailyTime']): string {
  if (value === 'under1h') return '小于 1 小时'
  if (value === '1h-4h') return '1–4 小时'
  if (value === '4h-8h') return '4–8 小时'
  if (value === 'over8h') return '8 小时以上'
  return '未提供'
}

function usageLevelLabel(value: ParsedLead['usageLevel']): string {
  if (value === 0) return '轻量使用'
  if (value === 1) return '正常使用'
  return '超高强度'
}

export function buildLeadEmailContent(id: string, lead: ParsedLead, createdAt: string): { subject: string; body: string } {
  const subject = `[Aster Team] 新的企业试用申请 · ${lead.teamSize} 人团队`
  const evidenceLines = lead.evidence === 'tokens'
    ? ['评估方式：近 7 天 Token 用量', `团队近 7 天 Token 用量：${lead.weeklyTokens} 亿 Token`]
    : ['评估方式：平均每天使用 AI 的时间', `平均每天使用 AI 的时间：${usageTimeLabel(lead.dailyTime)}`]
  const submittedFields = {
    contact: lead.contact,
    company: lead.company,
    teamSize: lead.teamSize,
    activeUsers: lead.activeUsers,
    evidence: lead.evidence,
    weeklyTokens: lead.weeklyTokens,
    dailyTime: lead.dailyTime,
    locale: lead.locale,
  }
  const body = [
    'Aster Team 官网收到新的企业试用申请。',
    '',
    `申请编号：${id}`,
    `提交时间：${createdAt}`,
    '',
    '【用户填写内容】',
    `联系方式：${lead.contact}`,
    `企业 / 团队：${lead.company || '未填写'}`,
    `团队成员：${lead.teamSize} 人`,
    `经常使用 AI：${lead.activeUsers} 人`,
    ...evidenceLines,
    '',
    '【系统评估结果（仅供参考）】',
    `使用强度：${usageLevelLabel(lead.usageLevel)}`,
    `建议 Pro 20x 容量：${lead.recommendedProAccounts} 个账号容量`,
    '',
    '【表单字段值（安全校验后）】',
    JSON.stringify(submittedFields, null, 2),
    '',
    '该邮件由 Aster Team 官网申请表单自动发送。',
  ].join('\r\n')
  return { subject, body }
}
