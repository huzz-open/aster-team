import { execFileSync } from 'node:child_process'
import { appendFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const MAIN = 'refs/heads/main'
const TARGET_BYTES = 8_000_000_000

// Only these repository-owned namespaces may be pruned. Unknown caches are untouched.
export function cacheFamily(key) {
  // rust-cache puts the custom key BEFORE the job ID, not after it.
  const rust = key.match(/^v0-rust-(?:windows-msvc-static-(?:[0-9a-f]{64}-)?)?((?:asterctl-windows|windows-install|build|customer|preflight|affected)-(?:Windows_NT|Linux)-[^-]+)/)
  if (rust) return `v0-rust-${rust[1]}`.toLowerCase()
  for (const pattern of [
    /^aster-npm-v1-(?:Windows|Linux)-[^-]+-node22/,
    /^node-cache-(?:Windows|Linux)-[^-]+-npm/,
    /^setup-go-(?:Windows|Linux)-[^-]+-(?:ubuntu\d+-)?go-\d+\.\d+\.\d+/,
    /^aster-(?:windows-build-runtime|release-downloads)-(?:Windows|Linux)-[^-]+/,
  ]) {
    const match = key.match(pattern)
    if (match) return match[0].toLowerCase()
  }
  return null
}

export function planCacheCleanup(caches, closedPulls = new Set(), targetBytes = TARGET_BYTES) {
  if (!Number.isSafeInteger(targetBytes) || targetBytes < 0) throw new Error('Invalid cache budget')
  const ids = new Set()
  for (const cache of caches) {
    if (!Number.isSafeInteger(cache.id) || cache.id <= 0 || ids.has(cache.id)
      || !Number.isSafeInteger(cache.size_in_bytes) || cache.size_in_bytes < 0
      || typeof cache.key !== 'string' || typeof cache.ref !== 'string'
      || !Number.isFinite(Date.parse(cache.created_at))) throw new Error('Invalid cache inventory; refusing cleanup')
    ids.add(cache.id)
  }
  const beforeBytes = caches.reduce((sum, cache) => sum + cache.size_in_bytes, 0)
  let afterBytes = beforeBytes
  const deletions = []
  const groups = new Map()
  function remove(cache, reason) {
    deletions.push({ ...cache, reason })
    afterBytes -= cache.size_in_bytes
  }
  for (const cache of caches) {
    const family = cacheFamily(cache.key)
    if (!family) continue
    const pull = cache.ref.match(/^refs\/pull\/(\d+)\/merge$/)
    if (pull && closedPulls.has(Number(pull[1]))) {
      remove(cache, 'closed PR')
    } else if (cache.ref === MAIN) {
      if (family.startsWith('node-cache-') && cache.size_in_bytes < 16_384) {
        remove(cache, 'empty legacy npm cache')
      } else {
        const group = groups.get(family) || []
        group.push(cache)
        groups.set(family, group)
      }
    }
  }
  const fallbacks = []
  for (const [family, group] of groups) {
    group.sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at) || b.id - a.id)
    const replacement = family.replace(/^node-cache-/, 'aster-npm-v1-').replace(/-npm$/, '-node22')
    if (family.startsWith('node-cache-') && groups.get(replacement)?.some(cache => cache.size_in_bytes >= 16_384)) {
      for (const cache of group) remove(cache, 'replaced by populated npm v1 cache')
      continue
    }
    for (const cache of group.slice(2)) remove(cache, 'older than latest and fallback')
    if (group[1]) fallbacks.push(group[1])
  }
  fallbacks.sort((a, b) => Date.parse(a.created_at) - Date.parse(b.created_at) || a.id - b.id)
  for (const cache of fallbacks) {
    if (afterBytes <= targetBytes) break
    remove(cache, 'budget pressure; latest family cache retained')
  }
  return { beforeBytes, afterBytes, targetBytes, budgetMet: afterBytes <= targetBytes, deletions }
}

export function github(args) {
  return execFileSync('gh', args, { encoding: 'utf8', windowsHide: true, maxBuffer: 16 * 1024 * 1024 })
}

export function manageCaches({ repository, apply = false, api = github }) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository || '')) throw new Error('Invalid GitHub repository')
  const endpoint = `repos/${repository}`
  const pages = JSON.parse(api(['api', '--paginate', '--slurp', `${endpoint}/actions/caches?per_page=100`]))
  if (!Array.isArray(pages) || pages.some(page => !Array.isArray(page.actions_caches))) throw new Error('Invalid cache response')
  const caches = pages.flatMap(page => page.actions_caches)
  // Validate the full inventory before making requests or deleting anything.
  planCacheCleanup(caches)
  const pullIds = new Set(caches.filter(cache => cacheFamily(cache.key)).flatMap(cache => {
    const match = cache.ref.match(/^refs\/pull\/(\d+)\/merge$/)
    return match ? [Number(match[1])] : []
  }))
  const closedPulls = new Set()
  for (const id of pullIds) {
    const pull = JSON.parse(api(['api', `${endpoint}/pulls/${id}`]))
    if (!['open', 'closed'].includes(pull.state)) throw new Error('Cannot establish PR state; refusing cleanup')
    if (pull.state === 'closed') closedPulls.add(id)
  }
  const plan = planCacheCleanup(caches, closedPulls)
  console.log(JSON.stringify({ repository, mode: apply ? 'apply' : 'dry-run', ...plan }, null, 2))
  if (apply) {
    for (const cache of plan.deletions) {
      // Delete exact IDs from the validated snapshot, never a broad key/ref match.
      api(['api', '--method', 'DELETE', `${endpoint}/actions/caches/${cache.id}`])
      console.log(`Deleted cache ${cache.id}: ${cache.reason}`)
    }
  }
  if (!plan.budgetMet) console.warn('Cache budget remains exceeded; protected or unknown caches were not deleted.')
  return plan
}

export function main(args = process.argv.slice(2)) {
  if (args.some(arg => !['--apply', '--dry-run'].includes(arg)) || new Set(args).size !== args.length
    || (args.includes('--apply') && args.includes('--dry-run'))) throw new Error('Usage: npm run ci:cache:prune -- [--apply|--dry-run]')
  const repository = process.env.GITHUB_REPOSITORY || github(['repo', 'view', '--json', 'nameWithOwner', '--jq', '.nameWithOwner']).trim()
  const plan = manageCaches({ repository, apply: args.includes('--apply') })
  if (process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY,
      `### Actions cache maintenance\n\nMode: ${args.includes('--apply') ? 'apply' : 'dry-run'}. Selected ${plan.deletions.length} caches.\n\nSnapshot bytes: ${plan.beforeBytes}; projected bytes: ${plan.afterBytes}; target: ${plan.targetBytes}. Budget met: ${plan.budgetMet}.\n`)
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main() } catch (error) { console.error(error.message); process.exitCode = 1 }
}
