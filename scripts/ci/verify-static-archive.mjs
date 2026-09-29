import { spawnSync } from 'node:child_process'
import { basename, dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { verifyStaticElf } from './verify-static-elf.mjs'

export function verifyArchive(path) {
  const archive = resolve(path)
  // Git Bash's GNU tar treats a Windows drive colon as a remote-host prefix.
  // Give tar a local basename and let Node resolve the absolute working directory.
  const options = { cwd: dirname(archive), windowsHide: true }
  const archiveName = `./${basename(archive)}`
  const list = spawnSync('tar', ['-tzf', archiveName], { ...options, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024 })
  if (list.error || list.status !== 0) throw new Error('Cannot inspect package archive with tar')
  const names = list.stdout.trim().split(/\r?\n/)
  const entries = names.map(value => value.replace(/^\.\//, ''))
  if (entries.some(value => value.startsWith('/') || value.includes('\\') || value.split('/').includes('..'))) throw new Error('Unsafe archive paths')
  const binaries = entries.filter(value => /^[^/]+\/bin\/.+/.test(value) && !value.endsWith('/'))
  if (new Set(binaries).size !== binaries.length) throw new Error('Duplicate executable in archive')
  const roots = new Set(binaries.map(value => value.split('/')[0]))
  if (roots.size !== 1) throw new Error('Expected one package root')
  for (const name of ['aster-team-cli', 'aster-control', 'aster-runner', 'caddy']) {
    if (!binaries.includes(`${[...roots][0]}/bin/${name}`)) throw new Error(`Missing executable: ${name}`)
  }
  for (const binary of binaries) {
    // Stream to memory: never extract untrusted archive paths onto the host.
    const result = spawnSync('tar', ['-xzOf', archiveName, names[entries.indexOf(binary)]], { ...options, maxBuffer: 256 * 1024 * 1024 })
    if (result.error || result.status !== 0) throw new Error(`Cannot read executable: ${binary}`)
    verifyStaticElf(result.stdout, binary)
  }
  return binaries
}

if (resolve(process.argv[1] || '') === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 3) throw new Error('Usage: node scripts/ci/verify-static-archive.mjs ARCHIVE')
    console.log(`Verified ${verifyArchive(process.argv[2]).length} static ELF executables in final archive`)
  } catch (error) { console.error(error.message); process.exitCode = 1 }
}
