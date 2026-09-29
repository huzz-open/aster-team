import type { CommercialPlanRecord, CreateCommercialOrderInput } from '../api/client'

export function orderInput(customerID: string, plan: CommercialPlanRecord | undefined, years: number, utcLocalTime: string): CreateCommercialOrderInput {
  if (!customerID) throw new Error('请选择客户')
  if (!plan || plan.snapshot.definition.offer.kind !== 'annual') throw new Error('请选择按年订阅的套餐版本')
  if (!plan.snapshot.definition.offer.terms.some(term => term.years === years)) throw new Error('请选择该版本支持的订阅期限')
  // The field is explicitly UTC. Never reinterpret a contract date using the
  // operator computer's timezone or replace it with the request time on retries.
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2})?$/.test(utcLocalTime)) throw new Error('请填写合同开始时间（UTC）')
  const normalized = utcLocalTime.length === 16 ? `${utcLocalTime}:00` : utcLocalTime
  const startsAt = `${normalized}.000Z`
  if (!Number.isFinite(Date.parse(startsAt)) || new Date(startsAt).toISOString() !== startsAt) throw new Error('合同开始时间无效')
  return { operation_id: `order_${crypto.randomUUID()}`, customer_id: customerID, plan_id: plan.snapshot.plan_id, plan_version: plan.snapshot.version, years, starts_at: startsAt }
}
