import { readFileSync, statSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { fileURLToPath } from 'node:url'
import { resolve } from 'node:path'

const root = fileURLToPath(new URL('..', import.meta.url))
const lock = JSON.parse(readFileSync(new URL('../tools/ci-base-images.lock.json', import.meta.url), 'utf8'))
const image = lock.images.find(entry => entry.target === 'linux-builder')?.image
if (!image || !image.includes('@sha256:')) throw new Error('Pinned Linux builder is missing')
const options = process.argv.slice(2)
if (options.length && (options.length !== 2 || options[0] !== '--registry-cache')) throw new Error('Usage: npm run check:customer:external-db:linux -- [--registry-cache DIRECTORY]')
const registry = options.length ? resolve(options[1]) : null
if (registry && !statSync(registry).isDirectory()) throw new Error('Registry cache must be a directory')
const registryMount = registry ? `${registry}:/usr/local/cargo/registry:ro` : 'aster-external-db-check-registry:/usr/local/cargo/registry'
const result = spawnSync('docker', [
  'run', '--rm', '--name', `aster-external-db-check-${randomBytes(6).toString('hex')}`,
  '-v', `${root}:/workspace:ro`, '-v', registryMount,
  '-v', 'aster-external-db-check-target:/target', '-w', '/workspace',
  '-e', 'CARGO_TARGET_DIR=/target', '-e', 'CARGO_HTTP_MULTIPLEXING=false',
  '-e', 'CARGO_HTTP_TIMEOUT=30', '-e', 'CARGO_NET_RETRY=2', '-e', 'CC_x86_64_unknown_linux_musl=musl-gcc',
  '--entrypoint', 'cargo', image, 'check', '--locked', '-p', 'aster-control', '-p', 'aster-team-cli',
  '--no-default-features', '--features', 'aster-control/sqlcipher,aster-control/mariadb', '--target', 'x86_64-unknown-linux-musl', '--jobs', '1', ...(registry ? ['--offline'] : []),
], { stdio: 'inherit', windowsHide: true })
if (result.error) throw new Error('Cannot start isolated Linux dual-driver check')
process.exitCode = result.status ?? 1
