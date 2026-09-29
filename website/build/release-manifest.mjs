import { createHash } from 'node:crypto'

const digest = bytes => createHash('sha256').update(bytes).digest('hex')

// Describes the actual emitted runtime, including the entry HTML and bundled
// JavaScript. Verifying catalog JSON alone cannot detect an old pricing bundle.
export function createReleaseManifest(bundle, identity) {
  if (!identity) return null
  const files = Object.entries(bundle).map(([path, item]) => {
    if (!path || path.startsWith('/') || path.includes('\\') || path.split('/').some(part => !part || part === '.' || part === '..') || /[?#%\u0000-\u0020]/.test(path)) throw new Error('invalid website output path')
    const bytes = Buffer.from(item.type === 'chunk' ? item.code : item.source)
    return { path: `/${path}`, sha256: digest(bytes), size_bytes: bytes.length }
  }).sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0)
  for (const required of ['/index.html', '/catalog-manifest.json', identity.path]) {
    if (!files.some(file => file.path === required)) throw new Error(`website release is missing ${required}`)
  }
  if (files.length > 500 || files.some(file => file.size_bytes > 10 * 1024 * 1024) || files.reduce((sum, file) => sum + file.size_bytes, 0) > 32 * 1024 * 1024) throw new Error('website runtime exceeds publication verification bounds')
  return JSON.stringify({ schema: 'aster.website-release.v1', catalog: identity, files })
}
