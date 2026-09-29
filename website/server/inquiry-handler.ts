import type { InquiryResponse } from '../shared/generated/contracts'
import { parseInquiry, inquiryDigest, buildInquiryEmailContent, type InquiryContent } from './inquiry-model'
import { InvalidLeadError } from './lead-model'
import { jsonResponse, positiveInteger, readLimitedJson, smtpConfigured, splitSet } from './lead-http'
import { clientFingerprint, consumeRateLimit } from './rate-limit'
import { sendWebsiteEmail } from './smtp'
import { verifyTurnstile } from './turnstile'

type ErrorCode = Extract<InquiryResponse, { ok: false }>['error']
function apiError(error: ErrorCode, status: number): Response { return jsonResponse({ ok: false, error } satisfies InquiryResponse, status) }
function accepted(id: string): Response { return jsonResponse({ ok: true, id } satisfies InquiryResponse, 202) }

async function notify(env: Env, content: InquiryContent, createdAt: string): Promise<void> {
  try {
    await sendWebsiteEmail({
      host: env.SMTP_HOST, port: positiveInteger(env.SMTP_PORT, 465, 65535),
      username: env.SMTP_USERNAME, password: env.SMTP_PASSWORD,
      from: env.LEAD_NOTIFICATION_FROM, to: env.LEAD_NOTIFICATION_TO,
    }, content.request_id, { ...buildInquiryEmailContent(content.request_id, content, createdAt), contact: content.contact }, createdAt)
    await env.LEADS_DB.prepare("UPDATE product_inquiries SET notification_status = 'sent', notification_error = NULL WHERE id = ?").bind(content.request_id).run()
  } catch {
    await env.LEADS_DB.prepare("UPDATE product_inquiries SET notification_status = 'failed', notification_error = 'notification_failed' WHERE id = ?").bind(content.request_id).run()
    console.error(JSON.stringify({ message: 'inquiry notification failed', inquiryId: content.request_id }))
  }
}

export const handleInquiryPost: PagesFunction<Env> = async (context) => {
  const traceId = crypto.randomUUID()
  try {
    const origin = context.request.headers.get('Origin')
    if (!origin || !splitSet(context.env.ALLOWED_ORIGINS).has(origin)) return apiError('origin_not_allowed', 403)
    const { content, token } = parseInquiry(await readLimitedJson(context.request, 32 * 1024))
    if (!context.env.RATE_LIMIT_SALT) return apiError('server_error', 500)
    const address = context.request.headers.get('CF-Connecting-IP') ?? 'local-unknown'
    const verified = await verifyTurnstile(token, context.env.TURNSTILE_SECRET, context.env.TURNSTILE_ACTION,
      splitSet(context.env.TURNSTILE_HOSTNAMES), address === 'local-unknown' ? null : address, context.env.TURNSTILE_TEST_MODE === 'true')
    if (!verified) return apiError('verification_failed', 403)
    const digest = await inquiryDigest(content)
    const existing = await context.env.LEADS_DB.prepare('SELECT request_sha256 FROM product_inquiries WHERE id = ?').bind(content.request_id).first<string>('request_sha256')
    if (existing !== null) return existing === digest ? accepted(content.request_id) : apiError('request_conflict', 409)

    const fingerprint = await clientFingerprint(address, context.env.RATE_LIMIT_SALT)
    const now = Math.floor(Date.now() / 1000)
    const rate = await consumeRateLimit(context.env.LEADS_DB, fingerprint, now,
      positiveInteger(context.env.RATE_LIMIT_WINDOW_SECONDS, 600, 86_400), positiveInteger(context.env.RATE_LIMIT_MAX_SUBMISSIONS, 3, 100))
    if (!rate.allowed) {
      const response = apiError('rate_limited', 429)
      response.headers.set('Retry-After', String(Math.max(1, rate.expiresAt - now)))
      return response
    }
    const createdAt = new Date().toISOString()
    const inserted = await context.env.LEADS_DB.prepare(`
      INSERT INTO product_inquiries (id, created_at, request_sha256, contact, message, locale,
        catalog_reference_json, reference_status, notification_status, turnstile_hostname, client_fingerprint)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(id) DO NOTHING RETURNING id
    `).bind(content.request_id, createdAt, digest, content.contact, content.message, content.locale,
      content.reference ? JSON.stringify(content.reference) : null, content.reference ? 'unverified' : 'none',
      smtpConfigured(context.env) ? 'pending' : 'not_configured', verified.hostname, fingerprint).first<string>('id')
    if (inserted === null) {
      const saved = await context.env.LEADS_DB.prepare('SELECT request_sha256 FROM product_inquiries WHERE id = ?').bind(content.request_id).first<string>('request_sha256')
      return saved === digest ? accepted(content.request_id) : apiError('request_conflict', 409)
    }
    if (smtpConfigured(context.env)) context.waitUntil(notify(context.env, content, createdAt)
      .catch(() => console.error(JSON.stringify({ message: 'inquiry notification status unavailable', inquiryId: content.request_id }))))
    context.waitUntil(context.env.LEADS_DB.prepare('DELETE FROM trial_rate_limits WHERE expires_at < ?').bind(now - 86_400).run()
      .catch(() => console.error(JSON.stringify({ message: 'inquiry rate cleanup failed', traceId }))))
    console.log(JSON.stringify({ message: 'inquiry accepted', inquiryId: content.request_id, locale: content.locale }))
    return accepted(content.request_id)
  } catch (error) {
    if (error instanceof InvalidLeadError) return apiError('invalid_request', 400)
    console.error(JSON.stringify({ message: 'inquiry submission failed', traceId }))
    return apiError('server_error', 500)
  }
}
