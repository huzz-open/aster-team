import { createHash } from 'node:crypto'
import {
  createReadStream, existsSync, mkdirSync, renameSync, rmSync,
} from 'node:fs'
import { basename, resolve } from 'node:path'

export async function fileDigest(path, algorithm) {
  const value = createHash(algorithm)
  for await (const chunk of createReadStream(path)) value.update(chunk)
  return value.digest('hex')
}

export async function prepareVerifiedDownload({
  cacheDirectory,
  fileName,
  algorithm,
  expectedDigest,
  download,
}) {
  if (basename(fileName) !== fileName || fileName === '.' || fileName === '..') {
    throw new Error('download cache file name is invalid')
  }
  mkdirSync(cacheDirectory, { recursive: true })
  const destination = resolve(cacheDirectory, fileName)
  if (existsSync(destination)) {
    const actual = await fileDigest(destination, algorithm)
    if (actual === expectedDigest) return destination
    rmSync(destination, { force: true })
  }

  const partial = `${destination}.${process.pid}.partial`
  if (existsSync(partial)) throw new Error(`download cache partial already exists: ${partial}`)
  try {
    await download(partial)
    const actual = await fileDigest(partial, algorithm)
    if (actual !== expectedDigest) throw new Error(`download digest mismatch: ${actual}`)
    try {
      renameSync(partial, destination)
    } catch (error) {
      if (!existsSync(destination) || await fileDigest(destination, algorithm) !== expectedDigest) throw error
    }
    return destination
  } finally {
    rmSync(partial, { force: true })
  }
}
