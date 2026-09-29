import { chmodSync, existsSync, readFileSync, writeFileSync } from 'node:fs'

function serializeEnvValue(value) {
  const text = String(value)
  if (/\r|\n/.test(text)) throw new Error('env 值不能包含换行符')
  if (/^[A-Za-z0-9_!@%+./:=-]*$/.test(text)) return text
  if (!text.includes("'")) return `'${text}'`
  if (!text.includes('"')) return `"${text}"`
  throw new Error('env 值不能同时包含单引号和双引号')
}

export function updateEnvFile(path, updates) {
  const lines = existsSync(path) ? readFileSync(path, 'utf8').split(/\r?\n/) : []
  const pending = new Map(Object.entries(updates).map(([key, value]) => [key, String(value)]))
  const output = []

  for (const line of lines) {
    const match = line.match(/^([A-Za-z_][A-Za-z0-9_]*)=/)
    if (!match || !pending.has(match[1])) {
      output.push(line)
      continue
    }
    output.push(`${match[1]}=${serializeEnvValue(pending.get(match[1]))}`)
    pending.delete(match[1])
  }

  while (output.length && output.at(-1) === '') output.pop()
  for (const [key, value] of pending) output.push(`${key}=${serializeEnvValue(value)}`)
  writeFileSync(path, `${output.join('\n')}\n`, { encoding: 'utf8', mode: 0o600 })
  chmodSync(path, 0o600)
}
