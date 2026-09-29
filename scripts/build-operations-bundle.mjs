import { createHash } from 'node:crypto'
import { chmodSync, cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { createTarGz } from './release-archive.mjs'
import { verifyArtifact, verifySources } from './verify-release-boundaries.mjs'
import { resolveGoCommand } from '../tools/toolchains/go-toolchain.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm'
const go = resolveGoCommand('go', root)
const rawVersion = process.env.ASTER_OPERATIONS_RELEASE_VERSION || '0.1.0'
const version = rawVersion.replace(/^operations-v/, '').replace(/^v/, '')
const arch = process.argv.find(value => value.startsWith('--arch='))?.slice(7) || 'amd64'
if (!['amd64', 'arm64'].includes(arch)) throw new Error('--arch must be amd64 or arm64')
if (!/^[0-9A-Za-z._-]+$/.test(version)) throw new Error('ASTER_OPERATIONS_RELEASE_VERSION is invalid')

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: 'inherit',
    env: { ...process.env, ...options.env },
    shell: process.platform === 'win32' && /\.(cmd|bat)$/i.test(command),
  })
  if (result.error) throw result.error
  if (result.status !== 0) process.exit(result.status ?? 1)
}

const sourceFailures = verifySources('operations')
if (sourceFailures.length) throw new Error(sourceFailures.join('\n'))
run(npm, ['run', 'build', '--workspace', '@aster/operations-console'])

const bundleName = `aster-operations-${version}-linux-${arch}`
const outputRoot = resolve(root, 'dist', 'operations')
const bundle = resolve(outputRoot, bundleName)
rmSync(bundle, { recursive: true, force: true })
mkdirSync(resolve(bundle, 'bin'), { recursive: true })
mkdirSync(resolve(bundle, 'systemd'), { recursive: true })

run(go, ['build', '-trimpath', '-ldflags=-s -w', '-o', resolve(bundle, 'bin', 'aster-operations-api'), './operations/backend/cmd/api'], {
  env: { GOOS: 'linux', GOARCH: arch, CGO_ENABLED: '0' },
})
run(go, ['build', '-trimpath', '-ldflags=-s -w', '-o', resolve(bundle, 'bin', 'aster-operations-backup'), './operations/backend/cmd/backup'], {
  env: { GOOS: 'linux', GOARCH: arch, CGO_ENABLED: '0' },
})
run(go, ['build', '-trimpath', '-ldflags=-s -w', '-o', resolve(bundle, 'bin', 'aster-webhost'), './operations/backend/cmd/webhost'], {
  env: { GOOS: 'linux', GOARCH: arch, CGO_ENABLED: '0' },
})
cpSync(resolve(root, 'operations/console/dist'), resolve(bundle, 'console'), { recursive: true })
cpSync(resolve(root, 'operations/deploy/systemd'), resolve(bundle, 'systemd'), { recursive: true })
cpSync(resolve(root, 'operations/deploy/operations.env.example'), resolve(bundle, 'operations.env.example'))
cpSync(resolve(root, 'operations/deploy/install.sh'), resolve(bundle, 'install.sh'))
cpSync(resolve(root, 'operations/deploy/README.md'), resolve(bundle, 'README.md'))
cpSync(resolve(root, 'docs/operations-guide.md'), resolve(bundle, 'OPERATIONS-GUIDE.md'))
writeFileSync(resolve(bundle, 'VERSION'), `${version}\n`)
chmodSync(resolve(bundle, 'install.sh'), 0o755)
chmodSync(resolve(bundle, 'bin/aster-operations-api'), 0o755)
chmodSync(resolve(bundle, 'bin/aster-operations-backup'), 0o755)
chmodSync(resolve(bundle, 'bin/aster-webhost'), 0o755)

const artifactFailures = verifyArtifact('operations', bundle)
if (artifactFailures.length) throw new Error(artifactFailures.join('\n'))
mkdirSync(outputRoot, { recursive: true })
const archive = resolve(outputRoot, `${bundleName}.tar.gz`)
rmSync(archive, { force: true })
createTarGz(bundle, archive, bundleName)
const checksum = createHash('sha256').update(readFileSync(archive)).digest('hex')
writeFileSync(`${archive}.sha256`, `${checksum}  ${bundleName}.tar.gz\n`)
console.log(`Operations bundle: ${archive}`)
console.log(`SHA-256: ${checksum}`)
