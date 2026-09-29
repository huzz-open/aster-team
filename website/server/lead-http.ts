import { InvalidLeadError } from './lead-model'

export function jsonResponse(body: unknown, status: number): Response {
  return Response.json(body, {
    status,
    headers: {
      'Cache-Control': 'no-store',
      'Content-Security-Policy': "default-src 'none'",
      'X-Content-Type-Options': 'nosniff',
    },
  })
}

export function splitSet(value: string): ReadonlySet<string> {
  return new Set(value.split(',').map(item => item.trim()).filter(Boolean))
}

export function positiveInteger(value: string, fallback: number, maximum: number): number {
  const parsed = Number(value)
  return Number.isInteger(parsed) && parsed > 0 && parsed <= maximum ? parsed : fallback
}

export function smtpConfigured(env: Env): boolean {
  return typeof env.SMTP_PASSWORD === 'string' && env.SMTP_PASSWORD.length > 0
}

export async function readLimitedJson(request: Request, maximumRequestBytes = 16 * 1024): Promise<unknown> {
  const contentLength = Number(request.headers.get('Content-Length') ?? '0')
  if (Number.isFinite(contentLength) && contentLength > maximumRequestBytes) throw new InvalidLeadError('request body is too large')
  if (request.headers.get('Content-Type')?.split(';')[0].trim().toLowerCase() !== 'application/json') {
    throw new InvalidLeadError('content type must be application/json')
  }
  const reader = request.body?.getReader()
  if (!reader) throw new InvalidLeadError('request body is empty')
  const chunks: Uint8Array[] = []
  let total = 0
  try {
    while (true) {
      const chunk = await reader.read()
      if (chunk.done) break
      total += chunk.value.byteLength
      if (total > maximumRequestBytes) {
        await reader.cancel()
        throw new InvalidLeadError('request body is too large')
      }
      chunks.push(chunk.value)
    }
  } finally {
    reader.releaseLock()
  }
  const merged = new Uint8Array(total)
  let offset = 0
  for (const chunk of chunks) { merged.set(chunk, offset); offset += chunk.byteLength }
  try {
    return JSON.parse(new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(merged)) as unknown
  } catch {
    throw new InvalidLeadError('request body is not valid JSON')
  }
}
