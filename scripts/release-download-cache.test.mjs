import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'

import { prepareVerifiedDownload } from './release-download-cache.mjs'

const sha512 = value => createHash('sha512').update(value).digest('hex')

test('verified downloads reuse a matching cache entry', async () => {
  const cacheDirectory = mkdtempSync(join(tmpdir(), 'aster-download-cache-'))
  writeFileSync(join(cacheDirectory, 'runtime.zip'), 'expected')
  let downloads = 0
  const path = await prepareVerifiedDownload({
    cacheDirectory,
    fileName: 'runtime.zip',
    algorithm: 'sha512',
    expectedDigest: sha512('expected'),
    download: () => { downloads += 1 },
  })
  assert.equal(downloads, 0)
  assert.equal(readFileSync(path, 'utf8'), 'expected')
})

test('verified downloads replace a corrupt cache entry atomically', async () => {
  const cacheDirectory = mkdtempSync(join(tmpdir(), 'aster-download-cache-'))
  writeFileSync(join(cacheDirectory, 'runtime.zip'), 'corrupt')
  let downloads = 0
  const path = await prepareVerifiedDownload({
    cacheDirectory,
    fileName: 'runtime.zip',
    algorithm: 'sha512',
    expectedDigest: sha512('expected'),
    download: temporary => {
      downloads += 1
      writeFileSync(temporary, 'expected')
    },
  })
  assert.equal(downloads, 1)
  assert.equal(readFileSync(path, 'utf8'), 'expected')
})
