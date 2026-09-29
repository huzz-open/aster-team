import { describe, expect, it } from 'vitest'
import { presentationLabel } from '../src/presentation'

describe('operations presentation labels', () => {
  it('presents workflow states and actions in user-facing Chinese', () => {
    expect(presentationLabel('customerStatus', 'lead')).toBe('线索')
    expect(presentationLabel('orderStatus', 'fulfillment_pending')).toBe('待办理授权')
    expect(presentationLabel('trialStatus', 'approved')).toBe('待创建许可证策略')
    expect(presentationLabel('trialStatus', 'converted')).toBe('已转正式')
    expect(presentationLabel('licenseStatus', 'expired')).toBe('已到期')
    expect(presentationLabel('auditAction', 'license.issued')).toBe('签发机器许可证文件')
    expect(presentationLabel('backupStatus', 'consistency_failed')).toBe('一致性校验失败')
    expect(presentationLabel('billingCycle', 'year')).toBe('年')
    expect(presentationLabel('auditAction', 'order.offline_payment_confirmed')).toBe('确认线下到账')
    expect(presentationLabel('resourceType', 'license')).toBe('许可证')
  })

  it('keeps unknown future values visible instead of hiding them', () => {
    expect(presentationLabel('licenseStatus', 'future_state')).toBe('future_state')
    expect(presentationLabel('licenseStatus', '')).toBe('—')
  })
})
