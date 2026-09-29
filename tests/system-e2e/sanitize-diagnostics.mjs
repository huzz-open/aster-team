import { readdir, readFile, stat, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { mkdir } from 'node:fs/promises'

const secretRules = [
  { pattern: /-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----[\s\S]*?-----END (?:RSA |EC |OPENSSH )?PRIVATE KEY-----/gi, replacement: '[REDACTED_PRIVATE_KEY]' },
  { pattern: /\b(?:authorization|proxy-authorization)\s*[:=]\s*(?:bearer|basic)\s+[^\s"']+/gi, replacement: 'Authorization: [REDACTED]' },
  { pattern: /\b(?:cookie|set-cookie)\s*[:=]\s*[^\r\n]+/gi, replacement: 'Cookie: [REDACTED]' },
  { pattern: /([?&](?:code|state|code_verifier|code_challenge)=)[^&#\s"']+/gi, replacement: '$1[REDACTED]' },
  { pattern: /\bask_[A-Za-z0-9_-]{12,}\b/g, replacement: '[REDACTED_ASTER_KEY]' },
  { pattern: /(["']?(?:access_token|refresh_token|id_token|client_secret|password|private_key)["']?\s*[:=]\s*)["']?[^\s,"'}]{8,}/gi, replacement: '$1[REDACTED]' },
]

export function redactDiagnostic(text, { canary = '' } = {}) {
  let output = String(text)
  if (canary) output = output.replaceAll(canary, '[REDACTED_TEST_MARKER]')
  for (const rule of secretRules) output = output.replace(rule.pattern, rule.replacement)
  return output
}

export async function writeDiagnostic(path, text, options) {
  await mkdir(dirname(path), { recursive: true })
  await writeFile(path, redactDiagnostic(text, options))
}

export async function scanDiagnosticDirectory(root, { canary = '' } = {}) {
  const files = []
  async function walk(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = resolve(directory, entry.name)
      if (entry.isDirectory()) await walk(path)
      else if (entry.isFile()) files.push(path)
    }
  }
  if (!(await stat(root).catch(() => null))?.isDirectory()) return
  await walk(root)
  for (const path of files) {
    const text = await readFile(path, 'utf8')
    if (canary && text.includes(canary)) throw new Error(`diagnostic canary leaked into ${path}`)
    for (const rule of secretRules) {
      rule.pattern.lastIndex = 0
      if (rule.pattern.test(text)) throw new Error(`sensitive value leaked into diagnostic artifact ${path}`)
    }
  }
}
