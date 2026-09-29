import { readFileSync, readdirSync, lstatSync } from 'node:fs'
import { resolve, join } from 'node:path'
import { fileURLToPath } from 'node:url'

export function verifyStaticElf(bytes, name = 'binary') {
  const fail = reason => { throw new Error(`${name}: ${reason}`) }
  if (bytes.length < 64 || bytes.subarray(0, 4).toString('hex') !== '7f454c46'
    || bytes[4] !== 2 || bytes[5] !== 1 || bytes.readUInt16LE(18) !== 62
    || ![2, 3].includes(bytes.readUInt16LE(16))) fail('expected Linux x86-64 ELF executable')
  const offset = Number(bytes.readBigUInt64LE(32))
  const size = bytes.readUInt16LE(54)
  const count = bytes.readUInt16LE(56)
  if (!count || size < 56 || !Number.isSafeInteger(offset) || offset < 64
    || offset + size * count > bytes.length) fail('invalid ELF program headers')
  for (let index = 0; index < count; index++) {
    const header = offset + index * size
    const type = bytes.readUInt32LE(header)
    if (type === 3) fail('dynamic interpreter is forbidden')
    if (type !== 2) continue
    const start = Number(bytes.readBigUInt64LE(header + 8))
    const length = Number(bytes.readBigUInt64LE(header + 32))
    if (!Number.isSafeInteger(start) || !Number.isSafeInteger(length) || length % 16
      || start < 64 || start + length > bytes.length) fail('invalid dynamic section')
    let terminated = false
    for (let cursor = start; cursor < start + length; cursor += 16) {
      const tag = bytes.readBigInt64LE(cursor)
      if (tag === 0n) { terminated = true; break }
      if ([1n, 0x7ffffffdn, 0x7fffffffn].includes(tag)) fail('external shared library dependency is forbidden')
    }
    if (!terminated) fail('unterminated dynamic section')
  }
}

export function verifyBundle(directory) {
  const bin = join(directory, 'bin')
  for (const required of ['aster-team-cli', 'aster-control', 'aster-runner', 'caddy']) {
    if (!lstatSync(join(bin, required)).isFile()) throw new Error(`Missing executable: ${required}`)
  }
  const paths = []
  function visit(directory) {
    for (const item of readdirSync(directory)) {
      const path = join(directory, item)
      const stat = lstatSync(path)
      if (stat.isDirectory()) visit(path)
      else if (stat.isFile()) { verifyStaticElf(readFileSync(path), path); paths.push(path) }
      else throw new Error(`Non-regular executable path: ${path}`)
    }
  }
  visit(bin)
  return paths
}

if (resolve(process.argv[1] || '') === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 3) throw new Error('Usage: node scripts/ci/verify-static-elf.mjs BUNDLE')
    const paths = verifyBundle(resolve(process.argv[2]))
    console.log(`Verified ${paths.length} static x86-64 ELF executables; no runtime libraries supplied by the test image.`)
  } catch (error) { console.error(error.message); process.exitCode = 1 }
}
