import { loadReleaseCatalog } from '../../server/release-catalog'
import { parseReleaseCatalog, type ReleaseCatalog } from '../../shared/release-catalog'

const cacheSeconds = 24 * 60 * 60
const publicHeaders = {
  'Cache-Control': `public, max-age=${cacheSeconds}`,
  'X-Content-Type-Options': 'nosniff',
}

async function cachedCatalog(context: EventContext<Env, string, unknown>): Promise<ReleaseCatalog> {
  const cache = typeof caches === 'undefined' ? null : caches.default
  const cacheKey = new Request(new URL('/api/releases?catalog=v1', context.request.url))
  try {
    const cached = await cache?.match(cacheKey)
    if (cached) {
      const catalog = parseReleaseCatalog(await cached.json())
      if (catalog) return catalog
    }
  } catch {
    // A cache failure must not prevent a fresh release lookup.
  }
  const catalog = await loadReleaseCatalog(context.env.GITHUB_RELEASES_TOKEN)
  if (cache) {
    const response = Response.json(catalog, { headers: { 'Cache-Control': `public, max-age=${cacheSeconds}` } })
    context.waitUntil(cache.put(cacheKey, response))
  }
  return catalog
}

export const onRequestGet: PagesFunction<Env> = async (context) => {
  try {
    const catalog = await cachedCatalog(context)
    return Response.json(catalog, { headers: publicHeaders })
  } catch {
    return Response.json({ error: 'release_metadata_unavailable' }, {
      status: 503, headers: { 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' },
    })
  }
}
