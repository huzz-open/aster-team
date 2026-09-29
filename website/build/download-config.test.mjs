import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readDownloadConfig } from './download-config.mjs'
import { productReleasePlugin } from './product-release.mjs'

test('a standalone HTTPS download URL needs no package, manifest or catalog', async () => {
  const env = { ASTER_WEBSITE_PRODUCT_RELEASE_DOWNLOAD_URL: 'https://downloads.example.com/latest.tar.gz', ASTER_WEBSITE_PRODUCT_RELEASE_VERSION: 'Aster 2.1.0' }
  const plugin = productReleasePlugin(env, null)
  const source = plugin.load(plugin.resolveId('virtual:aster-product-release'))
  const module = await import(`data:text/javascript,${encodeURIComponent(source)}`)
  assert.equal(module.default, null)
  assert.equal(module.downloadConfig.url, env.ASTER_WEBSITE_PRODUCT_RELEASE_DOWNLOAD_URL)
  assert.equal(module.downloadConfig.version, 'Aster 2.1.0')
})

test('configuration wins, then verified local release, otherwise runtime releases provide the link', () => {
  const release = { version: '2.1.0', platform: 'linux-amd64', environment: 'local', artifact: { name: 'package.tar.gz', url: '/downloads/package.tar.gz' } }
  assert.equal(readDownloadConfig({ ASTER_WEBSITE_PRODUCT_RELEASE_DOWNLOAD_URL: 'https://cdn.example.com/latest' }, release).url, 'https://cdn.example.com/latest')
  assert.equal(readDownloadConfig({}, release).filename, 'package.tar.gz')
  assert.equal(readDownloadConfig({}, null), null)
  assert.equal(readDownloadConfig({ ASTER_WEBSITE_PRODUCT_RELEASE_DOWNLOAD_URL: 'https://cdn.example.com/latest' }, null).version, null)
})

test('invalid configuration fails instead of silently falling back', () => {
  for (const url of ['javascript:alert(1)', 'http://example.com/file', '/file', 'https://user:password@example.com/file', 'https://example.com/file#fragment']) {
    assert.throws(() => productReleasePlugin({ ASTER_WEBSITE_PRODUCT_RELEASE_DOWNLOAD_URL: url }, null), /HTTPS/)
  }
  assert.throws(() => readDownloadConfig({ ASTER_WEBSITE_PRODUCT_RELEASE_VERSION: '2.1.0' }, null), /requires/)
  assert.throws(() => productReleasePlugin({ ASTER_WEBSITE_PRODUCT_RELEASE_DOWNLOAD_URL: 'https://example.com/file', ASTER_WEBSITE_PRODUCT_RELEASE_ENVIRONMENT: 'production' }, null), /requires/)
  assert.throws(() => productReleasePlugin({ ASTER_WEBSITE_WINDOWS_RELEASE_DOWNLOAD_URL: 'https://example.com/windows.tar.gz' }, null), /requires/)
})
