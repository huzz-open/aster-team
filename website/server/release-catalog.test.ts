import { test } from 'node:test'
import assert from 'node:assert/strict'
import { linuxReleaseFromGitHub, parseReleaseCatalog } from '../shared/release-catalog'
import { loadReleaseCatalog } from './release-catalog'
import { onRequestGet } from '../functions/api/releases'

function release(version: string, overrides: Record<string, unknown> = {}) {
  const tag = `v${version}`
  const filename = `aster-team-${version}-linux-amd64.tar.gz`
  return {
    tag_name: tag, draft: false, prerelease: false,
    assets: [
      { name: filename, state: 'uploaded', size: 12345, digest: `sha256:${'a'.repeat(64)}`,
        browser_download_url: `https://github.com/huzz-open/aster-team/releases/download/${tag}/${filename}` },
      { name: `${filename}.sha256`, state: 'uploaded', size: 102,
        browser_download_url: `https://github.com/huzz-open/aster-team/releases/download/${tag}/${filename}.sha256` },
    ],
    ...overrides,
  }
}

test('accepts only a complete stable Linux asset with GitHub SHA-256', () => {
  const good = release('2.1.1')
  assert.equal(linuxReleaseFromGitHub(good)?.sha256, 'a'.repeat(64))
  for (const bad of [
    release('2.1.1', { draft: true }), release('2.1.1', { prerelease: true }), release('2.1.1-rc.1'),
    release('2.1.1', { assets: [] }), release('2.1.1', { assets: [...good.assets, ...good.assets] }),
    release('2.1.1', { assets: [good.assets[0]] }),
    release('2.1.1', { assets: [{ ...good.assets[0], digest: null }, good.assets[1]] }),
    release('2.1.1', { assets: [{ ...good.assets[0], browser_download_url: 'https://evil.example/file' }, good.assets[1]] }),
    release('2.1.1', { assets: [{ ...good.assets[0], size: 0 }, good.assets[1]] }),
    release('2.1.1', { assets: [good.assets[0], { ...good.assets[1], size: 0 }] }),
  ]) assert.equal(linuxReleaseFromGitHub(bad), null)
})

test('latest release and historical versions come from GitHub, with one validated catalog', async () => {
  const calls: string[] = []
  const fetcher: typeof fetch = async (input, init) => {
    const url = String(input)
    calls.push(url)
    assert.equal(new Headers(init?.headers).get('Authorization'), 'Bearer read-only-token')
    return Response.json(url.endsWith('/latest') ? release('2.1.1') : [release('2.1.1'), release('2.1.0'), release('2.0.9', { prerelease: true })])
  }
  const catalog = await loadReleaseCatalog('read-only-token', fetcher)
  assert.equal(catalog.latest, 'v2.1.1')
  assert.deepEqual(catalog.releases.map(item => item.tag), ['v2.1.1', 'v2.1.0'])
  assert.equal(calls.length, 2)
  assert.deepEqual(parseReleaseCatalog(catalog), catalog)
  assert.equal(parseReleaseCatalog({ ...catalog, releases: [catalog.releases[0], catalog.releases[0]] }), null)
  assert.equal(parseReleaseCatalog({ ...catalog, releases: [{ ...catalog.releases[0], url: 'https://evil.example/file' }] }), null)
})

test('GitHub failures and incomplete latest release cannot produce a catalog', async () => {
  const fetcher: typeof fetch = async input => Response.json(String(input).endsWith('/latest') ? release('2.1.1', { assets: [] }) : [release('2.1.0')])
  await assert.rejects(loadReleaseCatalog(undefined, fetcher), /no complete stable Linux release/)
  const limited: typeof fetch = async () => Response.json({ message: 'rate limit' }, { status: 403 })
  await assert.rejects(loadReleaseCatalog(undefined, limited), /HTTP 403/)
})

test('the release API caches its validated catalog for one day', async () => {
  const originalFetch = globalThis.fetch
  const originalCaches = Object.getOwnPropertyDescriptor(globalThis, 'caches')
  const stored = new Map<string, Response>()
  let githubCalls = 0
  globalThis.fetch = async input => {
    githubCalls++
    return Response.json(String(input).endsWith('/latest') ? release('2.1.1') : [release('2.1.1'), release('2.1.0')])
  }
  Object.defineProperty(globalThis, 'caches', { configurable: true, value: { default: {
    match: async (request: Request) => stored.get(request.url)?.clone(),
    put: async (request: Request, response: Response) => { stored.set(request.url, response.clone()) },
  } } })
  const background: Promise<unknown>[] = []
  const context = (path: string) => ({
    request: new Request(`https://aster.huzz.top${path}`),
    env: { GITHUB_RELEASES_TOKEN: 'read-only-token' },
    waitUntil: (promise: Promise<unknown>) => { background.push(promise) },
  }) as Parameters<typeof onRequestGet>[0]
  try {
    const json = await onRequestGet(context('/api/releases')) as Response
    assert.equal(json.status, 200)
    assert.equal(json.headers.get('Cache-Control'), 'public, max-age=86400')
    const body = await json.json() as { latest: string }
    assert.equal(body.latest, 'v2.1.1')
    await Promise.all(background)
    const selected = await onRequestGet(context('/api/releases')) as Response
    assert.equal(selected.status, 200)
    assert.equal((await selected.json() as { releases: unknown[] }).releases.length, 2)
    assert.equal(githubCalls, 2)
  } finally {
    globalThis.fetch = originalFetch
    if (originalCaches) Object.defineProperty(globalThis, 'caches', originalCaches)
    else Reflect.deleteProperty(globalThis, 'caches')
  }
})
