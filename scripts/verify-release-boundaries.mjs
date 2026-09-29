import { closeSync, existsSync, lstatSync, openSync, readFileSync, readSync, readdirSync, statSync } from 'node:fs'
import { dirname, relative, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const manifestPath = resolve(root, 'tools/release-boundaries.json')
const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))
const textExtensions = new Set(['.cjs', '.css', '.go', '.html', '.js', '.json', '.jsonc', '.md', '.mjs', '.sh', '.sql', '.ts', '.tsx', '.vue', '.yaml', '.yml'])

function normalizePath(path) {
  return path.split(sep).join('/')
}

function extension(path) {
  const name = path.slice(path.lastIndexOf('/') + 1)
  const dot = name.lastIndexOf('.')
  return dot < 0 ? '' : name.slice(dot)
}

function generatedSource(path) {
  return path.split('/').some(part => part === 'dist' || part === 'node_modules' || part === '.wrangler')
}

function walk(path) {
  if (!existsSync(path)) return []
  const stats = statSync(path)
  if (stats.isFile()) return [path]
  if (!stats.isDirectory()) return []
  return readdirSync(path).sort().flatMap(entry => walk(resolve(path, entry)))
}

function walkExactFiles(path, relativeRoot = '') {
  const entries = []
  for (const name of readdirSync(path).sort()) {
    const absolute = resolve(path, name)
    const relativePath = relativeRoot ? `${relativeRoot}/${name}` : name
    const stats = lstatSync(absolute)
    if (stats.isSymbolicLink()) {
      entries.push({ path: relativePath, type: 'link' })
    } else if (stats.isDirectory()) {
      entries.push({ path: relativePath, type: 'directory' })
      entries.push(...walkExactFiles(absolute, relativePath))
    } else if (stats.isFile()) {
      entries.push({ path: relativePath, type: 'file' })
    } else {
      entries.push({ path: relativePath, type: 'special' })
    }
  }
  return entries
}

function fileContains(path, fragment) {
  const needle = Buffer.from(fragment)
  const chunkSize = 1024 * 1024
  const buffer = Buffer.allocUnsafe(chunkSize + Math.max(0, needle.length - 1))
  const descriptor = openSync(path, 'r')
  let carry = 0
  try {
    while (true) {
      const bytes = readSync(descriptor, buffer, carry, chunkSize, null)
      if (bytes === 0) return false
      const length = carry + bytes
      if (buffer.subarray(0, length).includes(needle)) return true
      carry = Math.min(needle.length - 1, length)
      if (carry > 0) buffer.copyWithin(0, length - carry, length)
    }
  } finally {
    closeSync(descriptor)
  }
}

export function isForbiddenReference(contents, reference) {
  const escaped = reference.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const importPattern = new RegExp(`(?:\\bfrom\\s+['\"]|import\\s*\\(|require\\s*\\(|aster\\.local/team/|resolve\\s*\\([^\\n]*|cpSync\\s*\\([^\\n]*)['\"]?[^\\n'\"]*${escaped}`, 'i')
  return importPattern.test(contents)
}

export function verifySources(domainName) {
  const domain = manifest.domains[domainName]
  if (!domain) throw new Error(`Unknown release domain: ${domainName}`)
  const failures = []
  for (const sourceRoot of domain.sourceRoots) {
    const absoluteRoot = resolve(root, sourceRoot)
    for (const file of walk(absoluteRoot)) {
      const repositoryPath = normalizePath(relative(root, file))
      if (generatedSource(repositoryPath)) continue
      if (!textExtensions.has(extension(repositoryPath))) continue
      const contents = readFileSync(file, 'utf8')
      for (const reference of domain.forbiddenReferences) {
        if (isForbiddenReference(contents, reference)) {
          failures.push(`${repositoryPath} references forbidden ${domainName} boundary: ${reference}`)
        }
      }
    }
  }
  return failures
}

export function verifyArtifact(domainName, artifactPath, profileName = '') {
  const domain = manifest.domains[domainName]
  if (!domain) throw new Error(`Unknown release domain: ${domainName}`)
  const absoluteArtifact = resolve(artifactPath)
  if (!existsSync(absoluteArtifact) || !statSync(absoluteArtifact).isDirectory()) {
    return [`Artifact directory does not exist: ${absoluteArtifact}`]
  }
  const failures = []
  const topLevel = readdirSync(absoluteArtifact).sort()
  const expectedTopLevel = profileName ? domain.artifactProfiles?.[profileName] : domain.artifactTopLevel
  if (!expectedTopLevel) return [`Unknown ${domainName} artifact profile: ${profileName}`]
  const optionalGroups = profileName ? domain.artifactOptionalGroups?.[profileName] || {} : {}
  const allowed = new Set([...expectedTopLevel, ...Object.keys(optionalGroups)])
  for (const entry of topLevel) {
    if (!allowed.has(entry)) failures.push(`Unexpected top-level ${domainName} artifact entry: ${entry}`)
  }
  for (const required of expectedTopLevel) {
    if (!topLevel.includes(required)) failures.push(`Missing top-level ${domainName} artifact entry: ${required}`)
  }
  for (const [entry, expectedFiles] of Object.entries(optionalGroups)) {
    if (!topLevel.includes(entry)) continue
    const expected = new Set(expectedFiles)
    const groupRoot = resolve(absoluteArtifact, entry)
    const groupStats = lstatSync(groupRoot)
    if (!groupStats.isDirectory() || groupStats.isSymbolicLink()) {
      failures.push(`Optional ${domainName} artifact group must be an ordinary directory: ${entry}`)
      continue
    }
    const entries = walkExactFiles(groupRoot)
    const actualFiles = entries
      .filter(candidate => candidate.type === 'file')
      .map(candidate => `${entry}/${candidate.path}`)
    for (const candidate of entries) {
      const file = `${entry}/${candidate.path}`
      if (candidate.type !== 'file') {
        failures.push(`Unexpected optional ${domainName} artifact ${candidate.type}: ${file}`)
      } else if (!expected.has(file)) {
        failures.push(`Unexpected optional ${domainName} artifact file: ${file}`)
      }
    }
    for (const file of expected) {
      if (!actualFiles.includes(file)) failures.push(`Missing optional ${domainName} artifact file: ${file}`)
    }
  }
  for (const file of walk(absoluteArtifact)) {
    const artifactRelativePath = normalizePath(relative(absoluteArtifact, file)).toLowerCase()
    for (const fragment of domain.forbiddenArtifactFragments) {
      const normalizedFragment = fragment.toLowerCase()
      const matched = normalizedFragment === '.env'
        ? artifactRelativePath.split('/').some(part => part === '.env' || part.endsWith('.env'))
        : artifactRelativePath.includes(normalizedFragment)
      if (matched) {
        failures.push(`Forbidden ${domainName} artifact path: ${artifactRelativePath}`)
      }
    }
    if (!statSync(file).isFile()) continue
    for (const fragment of domain.forbiddenArtifactContent || []) {
      if (fileContains(file, fragment)) failures.push(`Forbidden ${domainName} artifact content in ${artifactRelativePath}: ${fragment}`)
    }
  }
  return failures
}

function parseArgs(argv) {
  const values = new Map()
  for (const argument of argv) {
    const match = /^--([^=]+)=(.*)$/.exec(argument)
    if (!match) throw new Error(`Invalid argument: ${argument}`)
    values.set(match[1], match[2])
  }
  return values
}

function main() {
  const args = parseArgs(process.argv.slice(2))
  const domain = args.get('domain')
  if (!domain) {
    if (args.has('artifact') || args.has('profile')) throw new Error('--domain is required when verifying an artifact')
    const failures = Object.keys(manifest.domains).flatMap(name => verifySources(name))
    if (failures.length) {
      for (const failure of failures) console.error(`boundary error: ${failure}`)
      process.exitCode = 1
      return
    }
    console.log(`${Object.keys(manifest.domains).join(', ')} release boundaries verified`)
    return
  }
  const failures = verifySources(domain)
  if (args.has('artifact')) failures.push(...verifyArtifact(domain, args.get('artifact'), args.get('profile') || ''))
  if (failures.length) {
    for (const failure of failures) console.error(`boundary error: ${failure}`)
    process.exitCode = 1
    return
  }
  console.log(`${domain} release boundary verified`)
}

if (resolve(process.argv[1] || '') === fileURLToPath(import.meta.url)) main()
