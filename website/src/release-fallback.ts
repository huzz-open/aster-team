import type { ReleaseCatalog } from '../shared/release-catalog'

// Update this fallback when the website is refreshed after a public Linux release.
const version = '2.1.1'
const tag = `v${version}`
const filename = `aster-team-${version}-linux-amd64.tar.gz`

export const fallbackReleaseCatalog: ReleaseCatalog = {
  schema: 'aster.website-releases.v1',
  latest: tag,
  releases: [{
    tag,
    version,
    filename,
    url: `https://github.com/huzz-open/aster-team/releases/download/${tag}/${filename}`,
    sha256: '955cf0cb649e914ab2a8505130ea6d8db4c97c1996b31961b56492efbd8dd1f0',
    size: 35348616,
  }],
}
