import { readdir, readFile } from 'node:fs/promises'
import { join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const repositoryRoot = fileURLToPath(new URL('../', import.meta.url))
const sourceRoots = [
  fileURLToPath(new URL('../customer/admin/', import.meta.url)),
  fileURLToPath(new URL('../customer/member/', import.meta.url)),
  fileURLToPath(new URL('../operations/console/', import.meta.url)),
  fileURLToPath(new URL('../website/', import.meta.url)),
]

async function vueFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true })
  const nested = await Promise.all(entries.map(async entry => {
    const path = join(directory, entry.name)
    if (entry.isDirectory()) return vueFiles(path)
    return entry.isFile() && entry.name.endsWith('.vue') ? [path] : []
  }))
  return nested.flat()
}

function lineNumber(source, offset) {
  return source.slice(0, offset).split('\n').length
}

export function tableCellViolations(source) {
  const violations = []
  for (const match of source.matchAll(/<(td|th)\b[^>]*>([\s\S]*?)<\/\1>/gi)) {
    const content = match[2]
    const reasons = []
    if (/<br\b/i.test(content)) reasons.push('line break')
    if (/<small\b/i.test(content)) reasons.push('secondary small text')
    if (/<(?:div|span)\b[^>]*class=["'][^"']*\b(?:muted|reviewer)\b/i.test(content)) reasons.push('secondary muted block')
    if (!reasons.length) continue
    violations.push({ line: lineNumber(source, match.index), reason: reasons.join(', ') })
  }

  for (const table of source.matchAll(/<table\b[^>]*>([\s\S]*?)<\/table>/gi)) {
    const header = table[1].match(/<thead\b[^>]*>([\s\S]*?)<\/thead>/i)
    const body = table[1].match(/<tbody\b[^>]*>([\s\S]*?)<\/tbody>/i)
    if (!header || !body) continue
    const columnCount = [...header[1].matchAll(/<th\b/gi)].length
    for (const row of body[1].matchAll(/<tr\b[^>]*>([\s\S]*?)<\/tr>/gi)) {
      const cellCount = [...row[1].matchAll(/<t[dh]\b/gi)].length
      if (cellCount === columnCount) continue
      const offset = table.index + table[0].indexOf(body[0]) + row.index
      violations.push({ line: lineNumber(source, offset), reason: `${columnCount} headers but ${cellCount} cells` })
    }
  }
  return violations
}

async function main() {
  const violations = []
  for (const file of (await Promise.all(sourceRoots.map(vueFiles))).flat()) {
    const source = await readFile(file, 'utf8')
    for (const { line, reason } of tableCellViolations(source)) violations.push(`${relative(repositoryRoot, file)}:${line} (${reason})`)
  }
  if (violations.length) {
    console.error('Table cells must contain one semantic field. Split secondary values into dedicated columns:')
    for (const violation of violations) console.error(`- ${violation}`)
    process.exitCode = 1
  } else {
    console.log('Table cell layout verified: no stacked secondary fields found.')
  }
}

if (resolve(process.argv[1] || '') === fileURLToPath(import.meta.url)) await main()
