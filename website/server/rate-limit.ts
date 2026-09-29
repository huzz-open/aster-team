function toHex(value: ArrayBuffer): string {
  return Array.from(new Uint8Array(value), byte => byte.toString(16).padStart(2, '0')).join('')
}

export async function clientFingerprint(address: string, salt: string): Promise<string> {
  const encoded = new TextEncoder().encode(`${salt}:${address}`)
  return toHex(await crypto.subtle.digest('SHA-256', encoded))
}

export async function consumeRateLimit(
  database: D1Database,
  fingerprint: string,
  nowSeconds: number,
  windowSeconds: number,
  maximum: number,
): Promise<{ allowed: boolean; expiresAt: number }> {
  const windowStartedAt = Math.floor(nowSeconds / windowSeconds) * windowSeconds
  const expiresAt = windowStartedAt + windowSeconds
  const rateKey = `${fingerprint}:${windowStartedAt}`
  const result = await database.prepare(`
    INSERT INTO trial_rate_limits (rate_key, window_started_at, count, expires_at)
    VALUES (?, ?, 1, ?)
    ON CONFLICT(rate_key) DO UPDATE SET count = count + 1
    RETURNING count
  `).bind(rateKey, windowStartedAt, expiresAt).first<number>('count')

  return { allowed: result !== null && result <= maximum, expiresAt }
}
