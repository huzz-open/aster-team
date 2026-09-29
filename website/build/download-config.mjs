// Public link metadata only. This never certifies a package, license or catalog.
export function readDownloadConfig(env, release) {
  const raw = env.ASTER_WEBSITE_PRODUCT_RELEASE_DOWNLOAD_URL?.trim()
  const version = env.ASTER_WEBSITE_PRODUCT_RELEASE_VERSION?.trim()
  if (raw) {
    let url
    try { url = new URL(raw) } catch { throw new Error('download URL must be absolute HTTPS') }
    if (url.protocol !== 'https:' || url.username || url.password || url.hash) throw new Error('download URL must be credential-free HTTPS without a fragment')
    return { url: url.href, version: version || release?.version || null, platform: release?.platform ?? null, filename: null }
  }
  if (version && !release) throw new Error('download version requires a download URL')
  if (!release) return null
  return { url: release.artifact.url, version: release.version, platform: release.platform, filename: release.environment === 'local' ? release.artifact.name : null }
}
