import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { deflateRawSync, gzipSync } from 'node:zlib'

function writeTarText(header, offset, length, value) {
  const encoded = Buffer.from(value)
  if (encoded.length > length) throw new Error(`Tar header field is too long: ${value}`)
  encoded.copy(header, offset)
}

function writeTarOctal(header, offset, length, value) {
  writeTarText(header, offset, length, Math.trunc(value).toString(8).padStart(length - 1, '0') + '\0')
}

function splitTarPath(path) {
  if (Buffer.byteLength(path) <= 100) return { name: path, prefix: '' }
  for (let index = path.lastIndexOf('/'); index > 0; index = path.lastIndexOf('/', index - 1)) {
    const prefix = path.slice(0, index)
    const name = path.slice(index + 1)
    if (Buffer.byteLength(prefix) <= 155 && Buffer.byteLength(name) <= 100) return { name, prefix }
  }
  throw new Error(`Path is too long for a portable ustar archive: ${path}`)
}

function createTarHeader(path, size, type, mode, epochSeconds) {
  const header = Buffer.alloc(512)
  const normalizedPath = type === '5' && !path.endsWith('/') ? `${path}/` : path
  const { name, prefix } = splitTarPath(normalizedPath)
  writeTarText(header, 0, 100, name)
  writeTarOctal(header, 100, 8, mode)
  writeTarOctal(header, 108, 8, 0)
  writeTarOctal(header, 116, 8, 0)
  writeTarOctal(header, 124, 12, type === '5' ? 0 : size)
  writeTarOctal(header, 136, 12, epochSeconds)
  header.fill(0x20, 148, 156)
  writeTarText(header, 156, 1, type)
  writeTarText(header, 257, 6, 'ustar\0')
  writeTarText(header, 263, 2, '00')
  writeTarText(header, 265, 32, 'root')
  writeTarText(header, 297, 32, 'root')
  writeTarText(header, 345, 155, prefix)
  const checksum = header.reduce((sum, byte) => sum + byte, 0).toString(8).padStart(6, '0')
  writeTarText(header, 148, 8, `${checksum}\0 `)
  return header
}

export function createTarGz(sourceDirectory, archivePath, archiveRoot, epochSeconds = 0) {
  if (!Number.isSafeInteger(epochSeconds) || epochSeconds < 0) throw new Error('archive epoch must be a non-negative integer')
  const blocks = []
  function append(currentPath, archivePathname) {
    const stats = statSync(currentPath)
    if (stats.isDirectory()) {
      blocks.push(createTarHeader(archivePathname, 0, '5', 0o755, epochSeconds))
      for (const entry of readdirSync(currentPath).sort()) append(resolve(currentPath, entry), `${archivePathname}/${entry}`)
      return
    }
    if (!stats.isFile()) throw new Error(`Unsupported release entry: ${currentPath}`)
    const executable = archivePathname.endsWith('/init.sh') ||
      archivePathname.endsWith('/init-macos.sh') ||
      archivePathname.endsWith('/install.sh') ||
      archivePathname.endsWith('/install-macos.sh') ||
      archivePathname.endsWith('/restore-backup.sh') ||
      archivePathname.endsWith('/restore-backup-macos.sh') ||
      archivePathname.endsWith('/service-launch.sh') ||
      archivePathname.includes('/bin/')
    blocks.push(createTarHeader(archivePathname, stats.size, '0', executable ? 0o755 : 0o644, epochSeconds))
    const contents = readFileSync(currentPath)
    blocks.push(contents)
    const remainder = contents.length % 512
    if (remainder) blocks.push(Buffer.alloc(512 - remainder))
  }
  append(sourceDirectory, archiveRoot)
  blocks.push(Buffer.alloc(1024))
  writeFileSync(archivePath, gzipSync(Buffer.concat(blocks), { level: 9, mtime: 0 }))
}

const crcTable = Array.from({ length: 256 }, (_, start) => {
  let value = start
  for (let bit = 0; bit < 8; bit += 1) value = (value & 1) ? 0xedb88320 ^ (value >>> 1) : value >>> 1
  return value >>> 0
})

function crc32(buffer) {
  let value = 0xffffffff
  for (const byte of buffer) value = crcTable[(value ^ byte) & 0xff] ^ (value >>> 8)
  return (value ^ 0xffffffff) >>> 0
}

function dosTimestamp(date) {
  const year = Math.max(1980, date.getFullYear())
  const time = (date.getHours() << 11) | (date.getMinutes() << 5) | Math.floor(date.getSeconds() / 2)
  const day = ((year - 1980) << 9) | ((date.getMonth() + 1) << 5) | date.getDate()
  return { time, day }
}

function releaseFiles(sourceDirectory, archiveRoot) {
  const files = []
  function visit(currentPath, archivePathname) {
    for (const entry of readdirSync(currentPath).sort()) {
      const localPath = resolve(currentPath, entry)
      const stats = statSync(localPath)
      const zipPath = `${archivePathname}/${entry}`.replaceAll('\\', '/')
      if (stats.isDirectory()) visit(localPath, zipPath)
      else if (stats.isFile()) files.push({ localPath, zipPath, stats })
      else throw new Error(`Unsupported release entry: ${localPath}`)
    }
  }
  visit(sourceDirectory, archiveRoot)
  return files
}

export function createZip(sourceDirectory, archivePath, archiveRoot) {
  const localParts = []
  const centralParts = []
  let offset = 0
  for (const { localPath, zipPath, stats } of releaseFiles(sourceDirectory, archiveRoot)) {
    const name = Buffer.from(zipPath)
    const contents = readFileSync(localPath)
    const compressed = deflateRawSync(contents, { level: 9 })
    const checksum = crc32(contents)
    const { time, day } = dosTimestamp(stats.mtime)
    const localHeader = Buffer.alloc(30)
    localHeader.writeUInt32LE(0x04034b50, 0)
    localHeader.writeUInt16LE(20, 4)
    localHeader.writeUInt16LE(0x0800, 6)
    localHeader.writeUInt16LE(8, 8)
    localHeader.writeUInt16LE(time, 10)
    localHeader.writeUInt16LE(day, 12)
    localHeader.writeUInt32LE(checksum, 14)
    localHeader.writeUInt32LE(compressed.length, 18)
    localHeader.writeUInt32LE(contents.length, 22)
    localHeader.writeUInt16LE(name.length, 26)
    localHeader.writeUInt16LE(0, 28)
    localParts.push(localHeader, name, compressed)

    const centralHeader = Buffer.alloc(46)
    centralHeader.writeUInt32LE(0x02014b50, 0)
    centralHeader.writeUInt16LE(0x031e, 4)
    centralHeader.writeUInt16LE(20, 6)
    centralHeader.writeUInt16LE(0x0800, 8)
    centralHeader.writeUInt16LE(8, 10)
    centralHeader.writeUInt16LE(time, 12)
    centralHeader.writeUInt16LE(day, 14)
    centralHeader.writeUInt32LE(checksum, 16)
    centralHeader.writeUInt32LE(compressed.length, 20)
    centralHeader.writeUInt32LE(contents.length, 24)
    centralHeader.writeUInt16LE(name.length, 28)
    centralHeader.writeUInt32LE((0o100644 << 16) >>> 0, 38)
    centralHeader.writeUInt32LE(offset, 42)
    centralParts.push(centralHeader, name)
    offset += localHeader.length + name.length + compressed.length
  }
  const centralDirectory = Buffer.concat(centralParts)
  const end = Buffer.alloc(22)
  end.writeUInt32LE(0x06054b50, 0)
  end.writeUInt16LE(centralParts.length / 2, 8)
  end.writeUInt16LE(centralParts.length / 2, 10)
  end.writeUInt32LE(centralDirectory.length, 12)
  end.writeUInt32LE(offset, 16)
  writeFileSync(archivePath, Buffer.concat([...localParts, centralDirectory, end]))
}
