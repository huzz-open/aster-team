export type PresentationDomain =
  | 'customerStatus'
  | 'orderStatus'
  | 'trialStatus'
  | 'licenseStatus'
  | 'backupKind'
  | 'backupStatus'
  | 'billingCycle'
  | 'auditAction'
  | 'resourceType'

const labels: Record<PresentationDomain, Record<string, string>> = {
  customerStatus: { lead: '线索', active: '正式客户', inactive: '已停用' },
  orderStatus: {
    pending_payment: '待收款', fulfillment_pending: '待办理授权', fulfilled: '已签发',
    cancelled: '已取消', refunded: '已退款',
  },
  trialStatus: { approved: '待创建许可证策略', active: '试用中', converted: '已转正式', rejected: '已拒绝', expired: '已到期', cancelled: '已取消' },
  licenseStatus: { active: '有效', expired: '已到期' },
  backupKind: { backup: '创建备份', verify: '校验备份', restore: '恢复备份' },
  backupStatus: { created: '已创建', verified: '校验通过', restored: '恢复完成', failed: '失败', consistency_failed: '一致性校验失败' },
  billingCycle: { month: '月', year: '年', one_time: '一次性' },
  auditAction: {
    'customer.created': '创建客户', 'customer.updated': '更新客户', 'contact.created': '新增联系人',
    'billing_profile.upserted': '更新开票资料', 'plan.created': '创建套餐', 'plan.price_published': '发布价格',
    'order.created': '创建订单', 'order.offline_payment_confirmed': '确认线下到账', 'order.refund_recorded': '记录退款',
    'trial.approved': '批准试用', 'trial.extended': '延长试用', 'trial.risk_noted': '记录试用风险',
    'trial.converted': '试用转正式', 'license.policy_created': '创建许可证策略',
    'license.issued': '签发机器许可证文件',
    'release.imported': '导入发布包', 'delivery.created': '创建交付', 'operations.exported': '导出运营数据',
    'operator.password_changed': '修改操作员密码',
  },
  resourceType: {
    customer: '客户', contact: '联系人', billing_profile: '开票资料', plan: '套餐', order: '订单',
    trial: '试用', license: '许可证', release: '发布包', delivery: '交付', operations: '运营数据', operator: '操作员',
  },
}

export function presentationLabel(domain: PresentationDomain, value: string | null | undefined): string {
  if (!value) return '—'
  return labels[domain][value] || value
}
