import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const CUSTOMER_RUST_ROOTS = [
  'customer/backend/cli',
  'customer/backend/control',
  'customer/backend/runner',
  'customer/backend/crates',
  'packages/rust',
]
const LOG_MACRO = /\b(?:trace|debug|info|warn|error)!\s*\(/g
const FORBIDDEN_LOG_IDENTIFIERS = new Set([
  'access_token', 'accesstoken', 'api_key', 'apikey', 'authorization',
  'authorization_header', 'body', 'body_base64url', 'cookie', 'cookie_header',
  'credential', 'credentials', 'credential_identity_hmac', 'database_key',
  'encrypted_payload', 'headers', 'installation_key', 'messages', 'password',
  'password_hash', 'payload', 'plaintext', 'private_key', 'prompt',
  'proxy_authorization', 'raw_payload', 'refresh_token', 'refreshtoken',
  'request_body', 'response_body', 'secret',
  'session_token', 'token', 'wrapped_data_key',
])

function walk(path) {
  if (!existsSync(path)) return []
  const stats = statSync(path)
  if (stats.isFile()) return path.endsWith('.rs') ? [path] : []
  if (!stats.isDirectory()) return []
  return readdirSync(path).sort().flatMap(entry => walk(resolve(path, entry)))
}

function blank(output, start, end) {
  for (let index = start; index < end; index += 1) {
    if (output[index] !== '\n' && output[index] !== '\r') output[index] = ' '
  }
}

function maskRustNonCode(source) {
  const output = [...source]
  for (let index = 0; index < source.length;) {
    if (source.startsWith('//', index)) {
      const end = source.indexOf('\n', index + 2)
      const next = end < 0 ? source.length : end
      blank(output, index, next)
      index = next
      continue
    }
    if (source.startsWith('/*', index)) {
      let depth = 1
      let end = index + 2
      while (end < source.length && depth > 0) {
        if (source.startsWith('/*', end)) {
          depth += 1
          end += 2
        } else if (source.startsWith('*/', end)) {
          depth -= 1
          end += 2
        } else {
          end += 1
        }
      }
      blank(output, index, end)
      index = end
      continue
    }
    const previous = index === 0 ? '' : source[index - 1]
    const raw = !/[A-Za-z0-9_]/.test(previous)
      ? /^(?:br|rb|r)(#{0,255})"/.exec(source.slice(index))
      : null
    if (raw) {
      const marker = `"${raw[1]}`
      const closing = source.indexOf(marker, index + raw[0].length)
      const end = closing < 0 ? source.length : closing + marker.length
      blank(output, index, end)
      index = end
      continue
    }
    if (source[index] === '"') {
      let end = index + 1
      while (end < source.length) {
        if (source[end] === '\\') end += 2
        else if (source[end++] === '"') break
      }
      blank(output, index, end)
      index = end
      continue
    }
    const character = /^'(?:\\.|[^'\\\r\n])'/.exec(source.slice(index))
    if (character) {
      blank(output, index, index + character[0].length)
      index += character[0].length
      continue
    }
    index += 1
  }
  return output.join('')
}

export function verifyRustLoggingSource(path, source) {
  const masked = maskRustNonCode(source)
  const failures = []
  for (const match of masked.matchAll(LOG_MACRO)) {
    const open = match.index + match[0].lastIndexOf('(')
    let depth = 1
    let end = open + 1
    while (end < masked.length && depth > 0) {
      if (masked[end] === '(') depth += 1
      else if (masked[end] === ')') depth -= 1
      end += 1
    }
    const line = source.slice(0, match.index).split('\n').length
    if (depth !== 0) {
      failures.push(`${path}:${line} has an unterminated tracing log macro`)
      continue
    }
    const body = masked.slice(open + 1, end - 1)
    const identifiers = body.match(/[A-Za-z_][A-Za-z0-9_]*/g) || []
    const forbidden = [...new Set(identifiers.map(value => value.toLowerCase())
      .filter(value => FORBIDDEN_LOG_IDENTIFIERS.has(value)))]
    const directObjects = [...body.matchAll(/(?:^|[=,(])\s*[?%]?\s*(request|response)\b(?!\s*\.)/g)]
      .map(value => value[1])
    forbidden.push(...directObjects.filter(value => !forbidden.includes(value)))
    if (forbidden.length) {
      failures.push(`${path}:${line} logs a sensitive value identifier: ${forbidden.join(', ')}`)
    }
  }
  return failures
}

export function verifyCustomerLogging(repositoryRoot = root) {
  return CUSTOMER_RUST_ROOTS.flatMap(sourceRoot => walk(resolve(repositoryRoot, sourceRoot)))
    .flatMap(path => verifyRustLoggingSource(
      relative(repositoryRoot, path).split('\\').join('/'),
      readFileSync(path, 'utf8'),
    ))
}

function main() {
  const failures = verifyCustomerLogging()
  if (failures.length) {
    for (const failure of failures) console.error(`logging boundary error: ${failure}`)
    process.exitCode = 1
    return
  }
  console.log('customer Rust logging boundary verified')
}

if (resolve(process.argv[1] || '') === fileURLToPath(import.meta.url)) main()
