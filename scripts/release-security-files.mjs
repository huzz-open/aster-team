import {
  existsSync,
  lstatSync,
  readFileSync,
  realpathSync,
  statSync,
} from 'node:fs'
import { basename, isAbsolute, relative, resolve, sep } from 'node:path'
import { parseEnv } from 'node:util'

function ordinaryFile(directory, fileName, label) {
  if (basename(fileName) !== fileName || fileName === '.' || fileName === '..') {
    throw new Error(`${label} file name is invalid`)
  }
  const path = resolve(directory, fileName)
  if (!existsSync(path)) throw new Error(`${label} is missing: ${fileName}`)
  const information = lstatSync(path)
  if (!information.isFile() || information.isSymbolicLink()) {
    throw new Error(`${label} must be an ordinary file: ${fileName}`)
  }
  return path
}

function isInside(parent, candidate) {
  const child = relative(parent, candidate)
  return child === '' || (child !== '..' && !child.startsWith(`..${sep}`) && !isAbsolute(child))
}

export function inspectBundledFreeLicense(bytes, maximumBytes = 64 * 1024) {
  if (bytes.length < 1 || bytes.length > maximumBytes) {
    throw new Error(`Bundled free license must contain between 1 byte and ${maximumBytes / 1024} KiB`)
  }
  let document
  try { document = JSON.parse(bytes) } catch { throw new Error('Bundled free license is not valid JSON') }
  const distributionID = String(document?.claims?.source?.distribution_id || '')
  if (document?.claims?.source?.kind !== 'free_distribution'
    || document?.claims?.binding?.mode !== 'unbound'
    || document?.claims?.validity?.expiry?.mode !== 'none'
    || !/^[A-Za-z0-9_.:-]{3,64}$/.test(distributionID)
    || typeof document?.signature !== 'string'
    || !document.signature) {
    throw new Error('Bundled free license must be a signed, unbound, non-expiring free-distribution document')
  }
  return { distributionID }
}

export function loadLocalReleaseSecurityFiles(rootDirectory, envPath = resolve(rootDirectory, '.env')) {
  if (!existsSync(envPath)) throw new Error(`Local environment file is missing: ${envPath}`)
  let environment
  try {
    environment = parseEnv(readFileSync(envPath, 'utf8'))
  } catch (error) {
    throw new Error(`Local environment file is invalid: ${error.message}`)
  }
  const configured = String(environment.ASTER_LOCAL_SECURITY_CONFIG_DIR || '').trim()
  if (!isAbsolute(configured)) {
    throw new Error('ASTER_LOCAL_SECURITY_CONFIG_DIR must be an absolute path')
  }
  if (!existsSync(configured)) throw new Error('ASTER_LOCAL_SECURITY_CONFIG_DIR does not exist')
  const directory = realpathSync(configured)
  if (!statSync(directory).isDirectory()) {
    throw new Error('ASTER_LOCAL_SECURITY_CONFIG_DIR must be a directory')
  }
  if (isInside(realpathSync(rootDirectory), directory)) {
    throw new Error('ASTER_LOCAL_SECURITY_CONFIG_DIR must be outside the source repository')
  }

  const licenseKeyringFile = ordinaryFile(
    directory,
    'license-v2.public-keyring.json',
    'License keyring',
  )
  const releaseKeyringFile = ordinaryFile(
    directory,
    'release-v1.public-keyring.json',
    'Release keyring',
  )
  const releaseSeedFile = ordinaryFile(directory, 'release-v1.seed', 'Release signing seed')
  const pluginKeyringFile = ordinaryFile(directory, 'plugin-v1.public-keyring.json', 'Plugin keyring')
  const pluginSeedFile = ordinaryFile(directory, 'plugin-v1.seed', 'Plugin signing seed')
  const freeLicenseFile = ordinaryFile(directory, 'free-license.json', 'Bundled free license')
  const freeLicenseBytes = readFileSync(freeLicenseFile)
  inspectBundledFreeLicense(freeLicenseBytes)
  return {
    directory,
    licenseKeyringFile,
    releaseKeyringFile,
    releaseSeedFile,
    pluginKeyringFile,
    pluginSeedFile,
    freeLicenseFile,
    freeLicenseBytes,
  }
}
