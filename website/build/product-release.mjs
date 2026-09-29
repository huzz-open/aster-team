import { createHash } from 'node:crypto'
import {
  closeSync, constants, copyFileSync, createReadStream, existsSync, fstatSync, mkdirSync, openSync, readSync,
} from 'node:fs'
import { basename, dirname, isAbsolute, relative, resolve, sep } from 'node:path'
import { readPublicCatalog } from './public-catalog.mjs'
import { readDownloadConfig } from './download-config.mjs'

const digestPattern = /^[a-f0-9]{64}$/
const versionPattern = /^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$/
const maximumManifestBytes = 1024 * 1024
const maximumDocumentBytes = 4 * 1024 * 1024
const maximumArchiveBytes = 1024 * 1024 * 1024

function validRelativePath(path) {
  return typeof path === 'string' && path.length > 0 && !path.includes('\\')
    && !path.split('/').some(part => !part || part === '.' || part === '..')
    && !/[?#%\u0000-\u0020]/.test(path)
}

function readRegularFile(path, maximumBytes) {
  const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0) | (constants.O_NONBLOCK ?? 0))
  try {
    const before = fstatSync(descriptor)
    if (!before.isFile() || before.size < 1 || before.size > maximumBytes) throw new Error('product release input must be a bounded regular file')
    const bytes = Buffer.allocUnsafe(before.size)
    let offset = 0
    while (offset < bytes.length) {
      const count = readSync(descriptor, bytes, offset, bytes.length - offset, null)
      if (!count) break
      offset += count
    }
    const after = fstatSync(descriptor)
    if (offset !== bytes.length || before.dev !== after.dev || before.ino !== after.ino || before.size !== after.size || before.mtimeMs !== after.mtimeMs) {
      throw new Error('product release input changed while it was read')
    }
    return bytes
  } finally {
    closeSync(descriptor)
  }
}

function hashRegularFile(path, maximumBytes) {
  const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0) | (constants.O_NONBLOCK ?? 0))
  try {
    const before = fstatSync(descriptor)
    if (!before.isFile() || before.size < 1 || before.size > maximumBytes) throw new Error('product release artifact must be a bounded regular file')
    const hash = createHash('sha256')
    const chunk = Buffer.allocUnsafe(1024 * 1024)
    let length = 0
    while (length < before.size) {
      const count = readSync(descriptor, chunk, 0, Math.min(chunk.length, before.size - length), null)
      if (!count) break
      hash.update(chunk.subarray(0, count))
      length += count
    }
    const after = fstatSync(descriptor)
    if (length !== before.size || before.dev !== after.dev || before.ino !== after.ino || before.size !== after.size || before.mtimeMs !== after.mtimeMs) {
      throw new Error('product release artifact changed while it was read')
    }
    return { sha256: hash.digest('hex'), size_bytes: length }
  } finally {
    closeSync(descriptor)
  }
}

function checkedDocument(root, files, path) {
  const expected = files.get(path)
  if (!expected) throw new Error(`product release manifest is missing ${path}`)
  const bytes = readRegularFile(resolve(root, path), maximumDocumentBytes)
  if (bytes.length !== expected.size_bytes || createHash('sha256').update(bytes).digest('hex') !== expected.sha256) {
    throw new Error(`product release document does not match ${path}`)
  }
  return bytes
}

function sameEntitlements(left, right) {
  if (!left || !right || left.catalog_version !== right.catalog_version) return false
  const sets = value => JSON.stringify([...(value.feature_sets ?? [])].sort())
  if (sets(left) !== sets(right)) return false
  const leftFeatures = [...(left.features ?? [])].sort()
  const rightFeatures = [...(right.features ?? [])].sort()
  if (JSON.stringify(leftFeatures) !== JSON.stringify(rightFeatures)) return false
  const quotas = value => [...(value.quotas ?? [])].map(entry => [entry.id, entry.limit]).sort(([a], [b]) => a.localeCompare(b))
  return JSON.stringify(quotas(left)) === JSON.stringify(quotas(right))
}

function matchingFreePlan(catalog, bundledLicense) {
  if (!catalog || !Array.isArray(catalog.plans)) throw new Error('product release requires a configured public catalog')
  const plans = catalog.plans.filter(plan => plan.offer?.kind === 'free'
    && plan.plan_id === bundledLicense.plan_id && plan.version === bundledLicense.plan_version)
  if (plans.length !== 1) throw new Error('product release does not identify exactly one public free plan')
  const plan = plans[0]
  if (plan.edition !== bundledLicense.edition || plan.minimum_version !== bundledLicense.minimum_version
    || plan.quota_policy_version !== bundledLicense.quota_policy_version
    || JSON.stringify(plan.offer.expiry) !== JSON.stringify(bundledLicense.expiry)
    || !sameEntitlements(plan.entitlements, bundledLicense.entitlements)) {
    throw new Error('product release bundled license does not match the public free plan')
  }
  return plan
}

export function readProductRelease(env = process.env, catalog = null, platform = 'linux') {
  if (!['linux', 'windows'].includes(platform)) throw new Error('unsupported product release platform')
  const path = env.ASTER_WEBSITE_PRODUCT_RELEASE_MANIFEST_PATH
  const expectedManifestSHA256 = env.ASTER_WEBSITE_PRODUCT_RELEASE_MANIFEST_SHA256
  const environment = env.ASTER_WEBSITE_PRODUCT_RELEASE_ENVIRONMENT
  const archivePath = env.ASTER_WEBSITE_PRODUCT_RELEASE_ARCHIVE_PATH
  const downloadURL = env.ASTER_WEBSITE_PRODUCT_RELEASE_DOWNLOAD_URL
  // A standalone download URL configures the primary link, not a verified release.
  const configured = [path, expectedManifestSHA256, environment, archivePath, ...(platform === 'windows' ? [downloadURL] : [])].some(value => value !== undefined && value !== '')
  if (!configured) return null
  if (!path || !isAbsolute(path) || !archivePath || !isAbsolute(archivePath) || !digestPattern.test(expectedManifestSHA256 || '')
    || !['local', 'production'].includes(environment)) throw new Error('product release requires absolute manifest and archive paths, manifest SHA-256 and environment')
  if (environment === 'production' && (!downloadURL || !/^https:\/\//.test(downloadURL))) throw new Error('production product release requires an HTTPS download URL')
  if (environment === 'local' && downloadURL) throw new Error('local product release must use the verified local artifact')

  const manifestBytes = readRegularFile(path, maximumManifestBytes)
  if (createHash('sha256').update(manifestBytes).digest('hex') !== expectedManifestSHA256) throw new Error('product release manifest SHA-256 mismatch')
  const source = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(manifestBytes)
  const manifest = JSON.parse(source)
  if (`${JSON.stringify(manifest)}\n` !== source || manifest.schema !== 'aster.public-support-release.v1'
    || manifest.product !== 'aster-team' || manifest.environment !== environment || !versionPattern.test(manifest.version || '')) {
    throw new Error('product release manifest identity is invalid')
  }
  if (!catalog || catalog.environment !== environment) throw new Error('product release and public catalog environments do not match')
  const artifact = manifest.artifact
  const bundledLicense = manifest.bundled_license
  const releaseManifest = manifest.release_manifest
  const expectedName = `aster-team-${manifest.version}-${platform}-amd64.tar.gz`
  if (!artifact || artifact.name !== expectedName || artifact.platform !== `${platform}-amd64` || (platform === 'windows' && artifact.channel !== 'experimental') || !digestPattern.test(artifact.sha256)
    || !Number.isSafeInteger(artifact.size_bytes) || artifact.size_bytes < 1 || basename(archivePath) !== expectedName) {
    throw new Error('product release artifact identity is invalid')
  }
  if (!releaseManifest || releaseManifest.path !== 'RELEASE.json' || !digestPattern.test(releaseManifest.sha256)
    || typeof releaseManifest.key_id !== 'string' || !releaseManifest.key_id) throw new Error('product release signed manifest identity is invalid')
  if (environment === 'production') {
    let parsedDownloadURL
    try { parsedDownloadURL = new URL(downloadURL) } catch { throw new Error('production product release download URL is invalid') }
    if (parsedDownloadURL.protocol !== 'https:' || parsedDownloadURL.username || parsedDownloadURL.password || parsedDownloadURL.hash
      || decodeURIComponent(basename(parsedDownloadURL.pathname)) !== expectedName) throw new Error('production product release download URL is invalid')
  }
  if (!bundledLicense || bundledLicense.path !== 'licenses/free-license.json' || !digestPattern.test(bundledLicense.sha256)
    || !Number.isSafeInteger(bundledLicense.size_bytes) || bundledLicense.size_bytes < 1
    || bundledLicense.binding !== 'unbound' || bundledLicense.source !== 'free_distribution') {
    throw new Error('product release bundled license identity is invalid')
  }
  const freePlan = matchingFreePlan(catalog, bundledLicense)
  const actualArtifact = hashRegularFile(archivePath, maximumArchiveBytes)
  if (actualArtifact.sha256 !== artifact.sha256 || actualArtifact.size_bytes !== artifact.size_bytes) throw new Error('product release artifact does not match manifest')

  if (!Array.isArray(manifest.files)) throw new Error('product release file list is invalid')
  const files = new Map()
  for (const file of manifest.files) {
    if (!file || !validRelativePath(file.path) || !digestPattern.test(file.sha256)
      || !Number.isSafeInteger(file.size_bytes) || file.size_bytes < 1 || files.has(file.path)) throw new Error('product release file list is invalid')
    files.set(file.path, file)
  }
  const manifestDirectory = dirname(path)
  if (basename(manifestDirectory) !== manifest.version || basename(dirname(manifestDirectory)) !== 'releases' || basename(path) !== 'manifest.json') {
    throw new Error('product release manifest path is invalid')
  }
  const supportRoot = resolve(manifestDirectory, '..', '..')
  const relativeManifest = relative(supportRoot, path).split(sep).join('/')
  if (relativeManifest !== `releases/${manifest.version}/manifest.json`) throw new Error('product release support root is invalid')
  const documents = {
    manual: checkedDocument(supportRoot, files, 'docs/user-manual.md'),
    guide: checkedDocument(supportRoot, files, platform === 'windows' ? 'README-WINDOWS.md' : 'README-LINUX.md'),
    checksums: checkedDocument(supportRoot, files, `releases/${manifest.version}/SHA256SUMS`),
  }
  if (documents.checksums.toString('utf8') !== `${artifact.sha256}  ${artifact.name}\n`) throw new Error('product release checksum document is invalid')

  const documentRoot = platform === 'windows' ? '/downloads/windows' : '/downloads'
  const publicPaths = {
    manual: `${documentRoot}/aster-team-user-manual.md`,
    guide: `${documentRoot}/README-${platform.toUpperCase()}.md`,
    checksums: `${documentRoot}/${manifest.version}/SHA256SUMS`,
    artifact: environment === 'local' ? `/downloads/${artifact.name}` : downloadURL,
  }
  const release = {
    environment,
    version: manifest.version,
    platform: artifact.platform,
    artifact: { ...artifact, url: publicPaths.artifact },
    documents: { user_manual: publicPaths.manual, [platform === 'windows' ? 'windows_guide' : 'linux_guide']: publicPaths.guide, checksums: publicPaths.checksums },
    support_manifest: { sha256: expectedManifestSHA256, release_manifest_sha256: releaseManifest.sha256, release_key_id: releaseManifest.key_id },
    free_plan: { plan_id: freePlan.plan_id, plan_version: freePlan.version, license_id: bundledLicense.license_id, license_sha256: bundledLicense.sha256 },
  }
  return { release, archivePath, documents }
}

export function productReleasePlugin(env = process.env, catalog = readPublicCatalog(env)?.catalog ?? null) {
  const snapshot = readProductRelease(env, catalog)
  const windowsEnv = Object.fromEntries(['MANIFEST_PATH', 'MANIFEST_SHA256', 'ENVIRONMENT', 'ARCHIVE_PATH', 'DOWNLOAD_URL'].map(key => [`ASTER_WEBSITE_PRODUCT_RELEASE_${key}`, env[`ASTER_WEBSITE_WINDOWS_RELEASE_${key}`]]))
  const windows = readProductRelease(windowsEnv, catalog, 'windows')
  if (windows && !snapshot) throw new Error('Windows experimental download requires the recommended Linux release')
  if (windows && (windows.release.free_plan.plan_id !== snapshot.release.free_plan.plan_id || windows.release.free_plan.plan_version !== snapshot.release.free_plan.plan_version)) throw new Error('Windows and Linux downloads must use the same public free plan version')
  const snapshots = [snapshot, windows].filter(Boolean)
  const publicRelease = snapshot ? { ...snapshot.release, ...(windows ? { windows: windows.release } : {}) } : null
  const downloadConfig = readDownloadConfig(env, publicRelease)
  const moduleId = 'virtual:aster-product-release'
  const resolvedId = `\0${moduleId}`
  const publicManifest = JSON.stringify({ schema: 'aster.website-product-release.v1', state: snapshot ? 'configured' : 'unconfigured', release: publicRelease })
  return {
    name: 'aster-product-release',
    resolveId(id) { if (id === moduleId) return resolvedId },
    load(id) {
      if (id !== resolvedId) return
      return `function freeze(value) { if (value && typeof value === 'object') { Object.values(value).forEach(freeze); Object.freeze(value) } return value }\nexport const downloadConfig = freeze(${JSON.stringify(downloadConfig)});\nexport default freeze(${JSON.stringify(publicRelease)});`
    },
    generateBundle() {
      this.emitFile({ type: 'asset', fileName: 'product-release.json', source: publicManifest })
      for (const item of snapshots) {
        this.emitFile({ type: 'asset', fileName: item.release.documents.user_manual.slice(1), source: item.documents.manual })
        const guide = item.release.documents.windows_guide ?? item.release.documents.linux_guide
        this.emitFile({ type: 'asset', fileName: guide.slice(1), source: item.documents.guide })
        this.emitFile({ type: 'asset', fileName: item.release.documents.checksums.slice(1), source: item.documents.checksums })
      }
    },
    writeBundle(options) {
      for (const item of snapshots) {
        if (item.release.environment !== 'local') continue
        if (!options.dir) throw new Error('local product release requires a directory output')
        const target = resolve(options.dir, item.release.artifact.url.slice(1))
        mkdirSync(dirname(target), { recursive: true })
        if (existsSync(target)) throw new Error('local product release output already exists')
        copyFileSync(item.archivePath, target, constants.COPYFILE_EXCL)
        const copied = hashRegularFile(target, maximumArchiveBytes)
        if (copied.sha256 !== item.release.artifact.sha256 || copied.size_bytes !== item.release.artifact.size_bytes) throw new Error('copied local product release artifact does not match')
      }
    },
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const path = request.url?.split('?')[0]
        if (path === '/product-release.json') {
          response.setHeader('Content-Type', 'application/json; charset=utf-8')
          response.setHeader('Cache-Control', 'no-store')
          response.setHeader('X-Content-Type-Options', 'nosniff')
          response.end(publicManifest)
          return
        }
        for (const item of snapshots) {
          const document = Object.entries(item.release.documents).find(([, value]) => value === path)?.[0]
          if (document) {
            response.setHeader('Content-Type', 'text/markdown; charset=utf-8')
            response.setHeader('X-Content-Type-Options', 'nosniff')
            response.end(item.documents[document === 'user_manual' ? 'manual' : document.endsWith('_guide') ? 'guide' : 'checksums'])
            return
          }
          if (item.release.environment !== 'local' || path !== item.release.artifact.url) continue
          response.setHeader('Content-Type', 'application/gzip')
          response.setHeader('Content-Length', String(item.release.artifact.size_bytes))
          response.setHeader('Content-Disposition', `attachment; filename="${item.release.artifact.name}"`)
          response.setHeader('X-Content-Type-Options', 'nosniff')
          createReadStream(item.archivePath).pipe(response)
          return
        }
        next()
      })
    },
  }
}
