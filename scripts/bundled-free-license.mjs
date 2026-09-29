import {
  closeSync, constants, fstatSync, mkdirSync, openSync, readSync, writeFileSync,
} from 'node:fs'
import { resolve } from 'node:path'

export const MAX_BUNDLED_FREE_LICENSE_BYTES = 64 * 1024

export function freezeBundledFreeLicense(path) {
  let descriptor
  try {
    descriptor = openSync(
      path,
      constants.O_RDONLY | constants.O_NOFOLLOW | (constants.O_NONBLOCK ?? 0),
    )
    const before = fstatSync(descriptor)
    if (!before.isFile() || before.size === 0 || before.size > MAX_BUNDLED_FREE_LICENSE_BYTES) {
      throw new Error('invalid bundled free license input')
    }
    const buffer = Buffer.allocUnsafe(MAX_BUNDLED_FREE_LICENSE_BYTES + 1)
    let length = 0
    while (length < buffer.length) {
      const read = readSync(descriptor, buffer, length, buffer.length - length, null)
      if (read === 0) break
      length += read
    }
    const after = fstatSync(descriptor)
    if (length !== before.size || after.size !== before.size) {
      throw new Error('bundled free license changed while it was read')
    }
    return Buffer.from(buffer.subarray(0, length))
  } catch (error) {
    throw new Error('ASTER_CUSTOMER_FREE_LICENSE_FILE must be an ordinary file between 1 byte and 64 KiB', { cause: error })
  } finally {
    if (descriptor !== undefined) closeSync(descriptor)
  }
}

export function stageBundledFreeLicense(bundle, bytes) {
  if (!Buffer.isBuffer(bytes) || bytes.length === 0 || bytes.length > MAX_BUNDLED_FREE_LICENSE_BYTES) {
    throw new Error('bundled free license bytes are invalid')
  }
  mkdirSync(resolve(bundle, 'licenses'), { recursive: true })
  const target = resolve(bundle, 'licenses/free-license.json')
  writeFileSync(target, bytes, { flag: 'wx' })
  return target
}

export function verifyStagedBundledFreeLicense(bundle, target, execute, platform = 'linux') {
  if (!['linux', 'windows'].includes(platform)) throw new Error('unsupported bundled license verifier platform')
  const verifier = platform === 'windows' ? resolve(bundle, 'bin/aster-control.exe') : resolve(bundle, 'bin/aster-control')
  execute(verifier, ['verify-bundled-free-license', '--source', target])
}
