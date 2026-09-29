import { InvalidLeadError, parseLeadPayload, type ParsedLead } from './lead-model'
import { clientFingerprint, consumeRateLimit } from './rate-limit'
import { sendLeadEmail } from './smtp'
import { verifyTurnstile } from './turnstile'
import { jsonResponse, positiveInteger, readLimitedJson, smtpConfigured, splitSet } from './lead-http'

type ApiErrorCode = 'invalid_request' | 'origin_not_allowed' | 'rate_limited' | 'verification_failed' | 'server_error'

function apiError(code: ApiErrorCode, status: number): Response {
  return jsonResponse({ ok: false, error: code }, status)
}

async function persistLead(
  env: Env,
  id: string,
  createdAt: string,
  lead: ParsedLead,
  turnstileHostname: string,
  fingerprint: string,
): Promise<void> {
  await env.LEADS_DB.prepare(`
    INSERT INTO trial_leads (
      id, created_at, contact, company, team_size, active_users, evidence,
      weekly_tokens_100m, daily_time, usage_level, recommended_pro_accounts,
      locale, notification_status, turnstile_hostname, client_fingerprint
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
  `).bind(
    id,
    createdAt,
    lead.contact,
    lead.company,
    lead.teamSize,
    lead.activeUsers,
    lead.evidence,
    lead.weeklyTokens,
    lead.dailyTime,
    lead.usageLevel,
    lead.recommendedProAccounts,
    lead.locale,
    smtpConfigured(env) ? 'pending' : 'not_configured',
    turnstileHostname,
    fingerprint,
  ).run()
}

async function sendNotification(env: Env, id: string, lead: ParsedLead, createdAt: string): Promise<void> {
  try {
    await sendLeadEmail({
      host: env.SMTP_HOST,
      port: positiveInteger(env.SMTP_PORT, 465, 65535),
      username: env.SMTP_USERNAME,
      password: env.SMTP_PASSWORD,
      from: env.LEAD_NOTIFICATION_FROM,
      to: env.LEAD_NOTIFICATION_TO,
    }, id, lead, createdAt)
    await env.LEADS_DB.prepare(`
      UPDATE trial_leads SET notification_status = 'sent', notification_error = NULL WHERE id = ?
    `).bind(id).run()
    console.log(JSON.stringify({ message: 'lead notification sent', leadId: id }))
  } catch (error) {
    const detail = error instanceof Error ? error.message.slice(0, 300) : 'unknown email error'
    await env.LEADS_DB.prepare(`
      UPDATE trial_leads SET notification_status = 'failed', notification_error = ? WHERE id = ?
    `).bind(detail, id).run()
    console.error(JSON.stringify({ message: 'lead notification failed', leadId: id, error: detail }))
  }
}

export const handleTrialPost: PagesFunction<Env> = async (context) => {
  const requestId = crypto.randomUUID()
  try {
    const origin = context.request.headers.get('Origin')
    if (!origin || !splitSet(context.env.ALLOWED_ORIGINS).has(origin)) return apiError('origin_not_allowed', 403)

    const lead = parseLeadPayload(await readLimitedJson(context.request))
    if (lead.honeypotTriggered) return jsonResponse({ ok: true, id: requestId }, 202)

    const clientAddress = context.request.headers.get('CF-Connecting-IP') ?? 'local-unknown'
    const verified = await verifyTurnstile(
      lead.turnstileToken,
      context.env.TURNSTILE_SECRET,
      context.env.TURNSTILE_ACTION,
      splitSet(context.env.TURNSTILE_HOSTNAMES),
      clientAddress === 'local-unknown' ? null : clientAddress,
      context.env.TURNSTILE_TEST_MODE === 'true',
    )
    if (!verified) return apiError('verification_failed', 403)

    const fingerprint = await clientFingerprint(clientAddress, context.env.RATE_LIMIT_SALT)
    const nowSeconds = Math.floor(Date.now() / 1000)
    const windowSeconds = positiveInteger(context.env.RATE_LIMIT_WINDOW_SECONDS, 600, 86_400)
    const maximum = positiveInteger(context.env.RATE_LIMIT_MAX_SUBMISSIONS, 3, 100)
    const rateLimit = await consumeRateLimit(context.env.LEADS_DB, fingerprint, nowSeconds, windowSeconds, maximum)
    if (!rateLimit.allowed) {
      const response = apiError('rate_limited', 429)
      response.headers.set('Retry-After', String(Math.max(1, rateLimit.expiresAt - nowSeconds)))
      return response
    }

    const createdAt = new Date().toISOString()
    await persistLead(context.env, requestId, createdAt, lead, verified.hostname, fingerprint)
    if (smtpConfigured(context.env)) context.waitUntil(sendNotification(context.env, requestId, lead, createdAt))
    context.waitUntil(
      context.env.LEADS_DB.prepare('DELETE FROM trial_rate_limits WHERE expires_at < ?').bind(nowSeconds - 86_400).run()
        .then(() => undefined)
        .catch(error => console.error(JSON.stringify({ message: 'rate limit cleanup failed', error: String(error) }))),
    )

    console.log(JSON.stringify({ message: 'lead accepted', leadId: requestId, locale: lead.locale }))
    return jsonResponse({ ok: true, id: requestId }, 202)
  } catch (error) {
    if (error instanceof InvalidLeadError) return apiError('invalid_request', 400)
    console.error(JSON.stringify({ message: 'lead submission failed', requestId, error: error instanceof Error ? error.message : String(error) }))
    return apiError('server_error', 500)
  }
}
