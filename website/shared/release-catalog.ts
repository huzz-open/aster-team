export type LinuxRelease = {
  tag: string
  version: string
  filename: string
  url: string
  sha256: string
  size: number
}

export type ReleaseCatalog = {
  schema: 'aster.website-releases.v1'
  latest: string
  releases: LinuxRelease[]
}

const tagPattern = /^v(\d+)\.(\d+)\.(\d+)$/
const digestPattern = /^sha256:([a-f0-9]{64})$/
const maximumArchiveBytes = 1024 * 1024 * 1024

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

export function linuxReleaseFromGitHub(value: unknown): LinuxRelease | null {
  if (!record(value) || value.draft !== false || value.prerelease !== false
    || typeof value.tag_name !== 'string' || !Array.isArray(value.assets)) return null
  const tag = value.tag_name
  const match = tagPattern.exec(tag)
  if (!match) return null
  const version = tag.slice(1)
  const filename = `aster-team-${version}-linux-amd64.tar.gz`
  const url = `https://github.com/huzz-open/aster-team/releases/download/${tag}/${filename}`
  const checksumName = `${filename}.sha256`
  const checksumAssets = value.assets.filter(asset => record(asset) && asset.name === checksumName)
  if (checksumAssets.length !== 1) return null
  const checksumAsset = checksumAssets[0]
  if (!record(checksumAsset) || checksumAsset.state !== 'uploaded'
    || checksumAsset.browser_download_url !== `${url}.sha256`
    || typeof checksumAsset.size !== 'number' || !Number.isSafeInteger(checksumAsset.size)
    || checksumAsset.size < 1 || checksumAsset.size > 1024) return null
  const assets = value.assets.filter(asset => record(asset) && asset.name === filename)
  if (assets.length !== 1) return null
  const asset = assets[0]
  if (!record(asset) || asset.state !== 'uploaded' || asset.browser_download_url !== url
    || typeof asset.digest !== 'string' || typeof asset.size !== 'number'
    || !Number.isSafeInteger(asset.size) || asset.size < 1 || asset.size > maximumArchiveBytes) return null
  const digest = digestPattern.exec(asset.digest)
  if (!digest) return null
  return { tag, version, filename, url, sha256: digest[1], size: asset.size }
}

export function parseReleaseCatalog(value: unknown): ReleaseCatalog | null {
  if (!record(value) || value.schema !== 'aster.website-releases.v1'
    || typeof value.latest !== 'string' || !Array.isArray(value.releases)
    || value.releases.length < 1 || value.releases.length > 100) return null
  const releases: LinuxRelease[] = []
  const tags = new Set<string>()
  for (const item of value.releases) {
    if (!record(item) || typeof item.tag !== 'string' || !tagPattern.test(item.tag)
      || item.version !== item.tag.slice(1) || typeof item.filename !== 'string'
      || typeof item.url !== 'string' || typeof item.sha256 !== 'string'
      || typeof item.size !== 'number' || !Number.isSafeInteger(item.size)
      || item.size < 1 || item.size > maximumArchiveBytes || !/^[a-f0-9]{64}$/.test(item.sha256)) return null
    const filename = `aster-team-${item.version}-linux-amd64.tar.gz`
    const url = `https://github.com/huzz-open/aster-team/releases/download/${item.tag}/${filename}`
    if (item.filename !== filename || item.url !== url || tags.has(item.tag)) return null
    tags.add(item.tag)
    releases.push({ tag: item.tag, version: item.version as string, filename, url, sha256: item.sha256, size: item.size })
  }
  if (releases[0]?.tag !== value.latest) return null
  return { schema: 'aster.website-releases.v1', latest: value.latest, releases }
}
