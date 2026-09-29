type TurnstileResponse = {
  success: boolean
  hostname?: string
  action?: string
  'error-codes'?: string[]
}

export type VerifiedTurnstile = { hostname: string }

function isTurnstileResponse(value: unknown): value is TurnstileResponse {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return false
  const candidate = value as Record<string, unknown>
  return typeof candidate.success === 'boolean'
    && (candidate.hostname === undefined || typeof candidate.hostname === 'string')
    && (candidate.action === undefined || typeof candidate.action === 'string')
    && (candidate['error-codes'] === undefined || (Array.isArray(candidate['error-codes']) && candidate['error-codes'].every(code => typeof code === 'string')))
}

export async function verifyTurnstile(
  token: string,
  secret: string,
  expectedAction: string,
  expectedHostnames: ReadonlySet<string>,
  remoteIp: string | null,
  testMode: boolean,
): Promise<VerifiedTurnstile | null> {
  if (!secret || token.length === 0 || token.length > 2048 || expectedHostnames.size === 0) return null

  const body = new URLSearchParams({ secret, response: token })
  if (remoteIp) body.set('remoteip', remoteIp)

  let response: Response
  try {
    response = await fetch('https://challenges.cloudflare.com/turnstile/v0/siteverify', {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body,
      signal: AbortSignal.timeout(10_000),
    })
  } catch {
    return null
  }
  if (!response.ok) return null

  let result: unknown
  try {
    result = await response.json()
  } catch {
    return null
  }
  if (!isTurnstileResponse(result) || !result.success) return null
  if (testMode) {
    if (!result.hostname || (result.hostname !== 'example.com' && !expectedHostnames.has(result.hostname))) return null
    if (result.action && result.action !== expectedAction) return null
    return { hostname: result.hostname }
  }
  if (result.action !== expectedAction) return null
  if (!result.hostname || !expectedHostnames.has(result.hostname)) return null
  return { hostname: result.hostname }
}
