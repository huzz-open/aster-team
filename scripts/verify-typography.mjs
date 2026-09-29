import { readdir, readFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourceRoots = [
  'packages/ui/src',
  'customer/admin/src',
  'customer/member/src',
  'customer/demo/src',
  'operations/console/src',
  'website/src',
  'website/docs/.vitepress/theme',
]
const standaloneSourceFiles = ['scripts/local_dev_manager.py']
const typographyRoots = ['packages/ui/src/styles.css', 'website/src/style.css', 'website/docs/.vitepress/theme/style.css']
const fontSizeTokens = [
  '--font-size-caption',
  '--font-size-body',
  '--font-size-title',
  '--font-size-display',
]
const sourceExtensions = new Set(['.css', '.html', '.js', '.mjs', '.ts', '.tsx', '.vue'])
const rawFamily = /(?:Inter|Arial|Helvetica|Consolas|monospace|sans-serif|system-ui|ui-monospace|SFMono-Regular|PingFang|Microsoft YaHei|Noto Sans|Segoe UI)/i
const fontDeclaration = /(?:font-family|font)\s*:\s*[^;}]+/g
const tkFontDeclaration = /font\s*=\s*\(\s*["'][^"']+["'][^)]*\)/g
const fontSizeDeclaration = /font-size\s*:\s*([^;}\r\n]+)/g
const customFontSizeDeclaration = /--[\w-]*font-size\s*:\s*([^;}\r\n]+)/g
const fontShorthandDeclaration = /(?<![-\w])font\s*:\s*([^;}\r\n]+)/g
const inlineFontSizeAttribute = /font-size\s*=\s*["'][^"']+["']/g
const scriptFontSizeProperty = /\bfontSize\s*:/g
const allowedFontSize = /^var\(--font-size-(?:caption|body|title|display)\)(?:!important)?$/
const legacyFontSizeToken = /--(?:type-(?:small|body|title|display|section|hero)|page-title-size)/g

async function collectSourceFiles(relativeDirectory) {
  const absoluteDirectory = path.join(repositoryRoot, relativeDirectory)
  const entries = await readdir(absoluteDirectory, { withFileTypes: true })
  const files = []
  for (const entry of entries) {
    const relativePath = path.posix.join(relativeDirectory.replaceAll('\\', '/'), entry.name)
    if (entry.isDirectory()) files.push(...await collectSourceFiles(relativePath))
    else if (entry.isFile() && sourceExtensions.has(path.extname(entry.name))) files.push(relativePath)
  }
  return files
}

function tokenValue(source, token) {
  const match = source.match(new RegExp(`${token}\\s*:\\s*([^;]+)`))
  return match?.[1].replaceAll(/\s+/g, '') || ''
}

const typographySources = await Promise.all(typographyRoots.map(async relativePath => ({
  relativePath,
  source: await readFile(path.join(repositoryRoot, relativePath), 'utf8'),
})))
for (const token of ['--font-ui', '--font-mono', ...fontSizeTokens]) {
  const definitions = typographySources.map(item => tokenValue(item.source, token))
  if (definitions.some(value => !value) || new Set(definitions).size !== 1) {
    console.error(`${token} must be defined identically in ${typographyRoots.join(' and ')}`)
    process.exit(1)
  }
}
const sizeValues = fontSizeTokens.map(token => tokenValue(typographySources[0].source, token))
if (new Set(sizeValues).size !== fontSizeTokens.length) {
  console.error('The four shared font-size tokens must resolve to four distinct sizes.')
  process.exit(1)
}
if (sizeValues.some(value => !/^\d+(?:\.\d+)?px$/.test(value))) {
  console.error('The four shared font-size tokens must resolve to fixed pixel sizes, not fluid or relative values.')
  process.exit(1)
}
if (tokenValue(typographySources[0].source, '--font-mono') !== 'var(--font-ui)') {
  console.error('--font-mono must resolve to --font-ui so every visible surface uses one font family.')
  process.exit(1)
}

const sharedStyles = typographySources.find(item => item.relativePath === 'packages/ui/src/styles.css')?.source || ''
const requiredSharedRules = [
  ['body UI font', /body\{[^}]*font-family:var\(--font-ui\)/],
  ['body size', /body\{[^}]*font-size:var\(--font-size-body\)/],
  ['inline code inheritance', /code,kbd,samp\{font-family:inherit\}/],
  ['native control inheritance', /button,input,select,textarea,optgroup,option\{font:inherit\}/],
  ['teleported select UI font', /\.a-select-popup\{[^}]*font-family:var\(--font-ui\)/],
  ['teleported date UI font', /--dp-font-family:var\(--font-ui\)/],
]
for (const [label, pattern] of requiredSharedRules) {
  if (pattern.test(sharedStyles)) continue
  console.error(`Shared typography rule is missing: ${label}`)
  process.exit(1)
}

const files = [...new Set([
  ...(await Promise.all(sourceRoots.map(collectSourceFiles))).flat(),
  ...standaloneSourceFiles,
])]
const violations = []
for (const relativePath of files) {
  const source = await readFile(path.join(repositoryRoot, relativePath), 'utf8')
  for (const match of source.matchAll(fontDeclaration)) {
    const declaration = match[0]
    if (!rawFamily.test(declaration)) continue
    const line = source.slice(0, match.index).split('\n').length
    violations.push(`${relativePath}:${line}: ${declaration}`)
  }
  for (const match of source.matchAll(tkFontDeclaration)) {
    const declaration = match[0]
    if (!rawFamily.test(declaration)) continue
    const line = source.slice(0, match.index).split('\n').length
    violations.push(`${relativePath}:${line}: ${declaration}`)
  }
  if (!['.css', '.vue'].includes(path.extname(relativePath))) continue
  for (const match of source.matchAll(fontSizeDeclaration)) {
    const value = match[1].replaceAll(/\s+/g, '')
    if (allowedFontSize.test(value)) continue
    const line = source.slice(0, match.index).split('\n').length
    violations.push(`${relativePath}:${line}: ${match[0]}`)
  }
  for (const match of source.matchAll(customFontSizeDeclaration)) {
    const value = match[1].replaceAll(/\s+/g, '')
    if (allowedFontSize.test(value)) continue
    const line = source.slice(0, match.index).split('\n').length
    violations.push(`${relativePath}:${line}: ${match[0]}`)
  }
  for (const match of source.matchAll(fontShorthandDeclaration)) {
    const value = match[1].replaceAll(/\s+/g, '')
    if (value === 'inherit' || /var\(--font-size-(?:caption|body|title|display)\)/.test(value)) continue
    const line = source.slice(0, match.index).split('\n').length
    violations.push(`${relativePath}:${line}: ${match[0]}`)
  }
  for (const match of source.matchAll(legacyFontSizeToken)) {
    const line = source.slice(0, match.index).split('\n').length
    violations.push(`${relativePath}:${line}: legacy or page-local font-size token ${match[0]}`)
  }
  for (const pattern of [inlineFontSizeAttribute, scriptFontSizeProperty]) {
    for (const match of source.matchAll(pattern)) {
      const line = source.slice(0, match.index).split('\n').length
      violations.push(`${relativePath}:${line}: inline font size bypasses the shared four-size scale (${match[0]})`)
    }
  }
}

if (violations.length) {
  console.error('Typography policy violations: use the one shared font family and only the four shared font-size tokens:')
  console.error(violations.join('\n'))
  process.exit(1)
}

console.log(`Typography verified: ${files.length} UI source files use one font family and the four-size type scale.`)
