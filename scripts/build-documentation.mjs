import { cpSync, mkdirSync, readdirSync, statSync } from 'node:fs'
import { dirname, extname, relative, resolve } from 'node:path'
import { createHash } from 'node:crypto'
import { build } from 'vitepress'
import { fileURLToPath } from 'node:url'

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const outputArgument = process.argv.find(value => value.startsWith('--out-dir='))
const target = process.argv.find(value => value.startsWith('--target='))?.slice('--target='.length)
if (!outputArgument || !['customer', 'website'].includes(target)) {
  throw new Error('Usage: node scripts/build-documentation.mjs --target=customer|website --out-dir=PATH')
}

const outputDirectory = resolve(process.cwd(), outputArgument.slice('--out-dir='.length))
const documentationRoot = resolve(repositoryRoot, 'website', 'docs')
// Website, Admin and Member can start together. Keep VitePress temporary bundles and
// search caches separate for each output, while retaining one documentation source.
const buildKey = createHash('sha256').update(outputDirectory).digest('hex').slice(0, 16)
const buildCache = resolve(repositoryRoot, 'node_modules', '.cache', 'aster-docs', buildKey)
process.env.ASTER_DOCS_TARGET = target
await build(documentationRoot, {
  outDir: outputDirectory,
  onAfterConfigResolve(config) {
    config.tempDir = resolve(buildCache, 'temp')
    config.cacheDir = resolve(buildCache, 'cache')
  },
})

function htmlFiles(directory) {
  return readdirSync(directory).flatMap(entry => {
    const path = resolve(directory, entry)
    return statSync(path).isDirectory() ? htmlFiles(path) : extname(path) === '.html' ? [path] : []
  })
}

for (const source of htmlFiles(outputDirectory)) {
  const sourceRelative = relative(outputDirectory, source).replaceAll('\\', '/')
  if (sourceRelative === '404.html' || sourceRelative.endsWith('/index.html') || sourceRelative === 'index.html') continue
  const target = resolve(outputDirectory, sourceRelative.slice(0, -'.html'.length), 'index.html')
  mkdirSync(dirname(target), { recursive: true })
  cpSync(source, target)
}

console.log(`${target} documentation prepared at ${outputDirectory}`)
