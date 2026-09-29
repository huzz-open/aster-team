import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs'
import { dirname, extname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const DEFAULT_ASSET_ROOTS = [
  resolve(root, 'customer/admin/dist'),
  resolve(root, 'customer/member/dist'),
]
const TEXT_EXTENSIONS = new Set(['.css', '.html', '.js', '.json'])
const FORBIDDEN_MARKERS = [
  '@aster/demo',
  'asterDataMode',
  'mockServiceWorker.js',
  'Mock Service Worker',
  'setupWorker',
]

function filesUnder(directory) {
  const files = []
  for (const entry of readdirSync(directory)) {
    const path = resolve(directory, entry)
    if (statSync(path).isDirectory()) files.push(...filesUnder(path))
    else files.push(path)
  }
  return files
}

export function verifyCustomerProductionAssets(assetRoots = DEFAULT_ASSET_ROOTS) {
  const failures = []
  for (const assetRoot of assetRoots) {
    if (!existsSync(assetRoot) || !statSync(assetRoot).isDirectory()) {
      failures.push(`Customer production asset directory is unavailable: ${assetRoot}`)
      continue
    }
    for (const path of filesUnder(assetRoot)) {
      if (path.endsWith('mockServiceWorker.js')) {
        failures.push(`Customer production assets contain Mock Service Worker: ${path}`)
        continue
      }
      if (!TEXT_EXTENSIONS.has(extname(path))) continue
      const source = readFileSync(path, 'utf8')
      for (const marker of FORBIDDEN_MARKERS) {
        if (source.includes(marker)) failures.push(`Customer production asset ${path} contains demo marker: ${marker}`)
      }
    }
  }
  if (failures.length) throw new Error(failures.join('\n'))
}

export function assetRootsFromArguments(arguments_) {
  if (!arguments_.length) return DEFAULT_ASSET_ROOTS
  return arguments_.map(argument => {
    if (!argument.startsWith('--root=') || argument.length === '--root='.length) {
      throw new Error('Usage: node scripts/verify-customer-production-assets.mjs [--root=PATH ...]')
    }
    return resolve(root, argument.slice('--root='.length))
  })
}

if (resolve(process.argv[1] || '') === fileURLToPath(import.meta.url)) {
  verifyCustomerProductionAssets(assetRootsFromArguments(process.argv.slice(2)))
  console.log('Customer production assets exclude Demo and MSW runtime code')
}
