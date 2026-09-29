import { linuxReleaseFromGitHub, type ReleaseCatalog } from '../shared/release-catalog'

const githubLatestURL = 'https://api.github.com/repos/huzz-open/aster-team/releases/latest'
const githubListURL = 'https://api.github.com/repos/huzz-open/aster-team/releases?per_page=100'
const maximumResponseBytes = 2 * 1024 * 1024

async function readBoundedJSON(response: Response): Promise<unknown> {
  if (!response.ok || !response.body) throw new Error(`GitHub releases returned HTTP ${response.status}`)
  const contentLength = Number(response.headers.get('Content-Length') || 0)
  if (contentLength > maximumResponseBytes) throw new Error('GitHub release metadata is too large')
  const reader = response.body.getReader()
  const chunks: Uint8Array[] = []
  let length = 0
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      length += value.length
      if (length > maximumResponseBytes) throw new Error('GitHub release metadata is too large')
      chunks.push(value)
    }
  } catch (error) {
    await reader.cancel()
    throw error
  } finally {
    reader.releaseLock()
  }
  const bytes = new Uint8Array(length)
  let offset = 0
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length }
  return JSON.parse(new TextDecoder().decode(bytes)) as unknown
}

export async function loadReleaseCatalog(token: string | undefined, fetcher: typeof fetch = fetch): Promise<ReleaseCatalog> {
  const headers: Record<string, string> = {
    Accept: 'application/vnd.github+json',
    'User-Agent': 'aster-team-website',
    'X-GitHub-Api-Version': '2022-11-28',
  }
  if (token?.trim()) headers.Authorization = `Bearer ${token.trim()}`
  const request = (url: string) => fetcher(url, { headers, signal: AbortSignal.timeout(8000) })
  const [latestResponse, listResponse] = await Promise.all([request(githubLatestURL), request(githubListURL)])
  const [latestValue, listValue] = await Promise.all([readBoundedJSON(latestResponse), readBoundedJSON(listResponse)])
  const latest = linuxReleaseFromGitHub(latestValue)
  if (!latest || !Array.isArray(listValue)) throw new Error('GitHub has no complete stable Linux release')
  const releases = [latest]
  const seen = new Set([latest.tag])
  for (const item of listValue) {
    const release = linuxReleaseFromGitHub(item)
    if (!release || seen.has(release.tag)) continue
    seen.add(release.tag)
    releases.push(release)
  }
  return { schema: 'aster.website-releases.v1', latest: latest.tag, releases }
}
