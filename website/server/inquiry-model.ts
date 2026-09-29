import validate from '../shared/generated/inquiry-validator.js'
import type { ProductInquiry, InquiryReference } from '../shared/generated/contracts'
import { InvalidLeadError } from './lead-model'

export type InquiryContent = Pick<ProductInquiry, 'request_id' | 'contact' | 'message' | 'locale' | 'reference'>
export type ParsedInquiry = { content: InquiryContent; token: string }

export function parseInquiry(value: unknown): ParsedInquiry {
  if (!validate(value)) throw new InvalidLeadError('invalid inquiry')
  const contact = value.contact.trim()
  const message = value.message.trim()
  if ([...contact].length < 2 || [...message].length < 2 || value.website.trim()) throw new InvalidLeadError('invalid inquiry')
  const content: InquiryContent = { request_id: value.request_id, contact, message, locale: value.locale }
  if (value.reference) {
    const reference: InquiryReference = {
      catalog_revision: value.reference.catalog_revision,
      plan_id: value.reference.plan_id,
      plan_version: value.reference.plan_version,
    }
    if (value.reference.years !== undefined) reference.years = value.reference.years
    content.reference = reference
  }
  return { content, token: value.turnstile_token }
}

export async function inquiryDigest(content: InquiryContent): Promise<string> {
  // Explicit property order is shared by all accepted requests; excludes the
  // single-use CAPTCHA token, so an uncertain response can retry the same ID.
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(JSON.stringify(content)))
  return Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, '0')).join('')
}

export function buildInquiryEmailContent(id: string, inquiry: InquiryContent, createdAt: string): { subject: string; body: string } {
  return {
    subject: '[Aster Team] 新的产品咨询',
    body: [
      `咨询编号：${id}`, `提交时间：${createdAt}`, `联系方式：${inquiry.contact}`, '',
      '需求说明：', inquiry.message, '',
      '访客所见套餐引用（未经受理核验，不构成报价或订单）：',
      JSON.stringify(inquiry.reference ?? null, null, 2),
    ].join('\r\n'),
  }
}
