import { spawnSync } from 'node:child_process'
import { mkdirSync, existsSync, statSync } from 'node:fs'
import { join, isAbsolute } from 'node:path'
import { homedir } from 'node:os'
import { fileURLToPath } from 'node:url'
import { resolveCiImage } from './ci-images.mjs'

// CI compilation only. Windows local development uses linux-lab.sh's isolated Docker volumes.
if (process.platform !== 'linux') throw new Error('ci:builder runs on Linux; use test:linux:primary/full from Windows')
const root = fileURLToPath(new URL('../../', import.meta.url)).replace(/\/$/, '')
const image = resolveCiImage('linux-builder', 'builder')
const command = process.argv.slice(2)
if (!command.length) throw new Error('Usage: npm run ci:builder -- COMMAND [ARG...]')
function docker(args, capture = false) {
  const result = spawnSync('docker', args, { cwd: root, encoding: 'utf8', stdio: capture ? 'pipe' : 'inherit' })
  if (result.error || result.status !== 0) throw new Error(`Docker builder ${args[0]} failed`)
  return result.stdout
}
if (!spawnSync('docker', ['image', 'inspect', image.image], { stdio: 'ignore' }).status) {
  // Exact immutable image is already present.
} else docker(['pull', '--platform', 'linux/amd64', image.image])
const info = JSON.parse(docker(['image', 'inspect', image.image], true))[0]
if (info.Os !== 'linux' || info.Architecture !== 'amd64'
  || info.Config?.Labels?.['io.aster.ci.recipe'] !== image.recipe
  || info.Config?.Labels?.['io.aster.ci.target'] !== image.target) throw new Error('Builder identity differs from validated lock')
const args = ['run', '--rm', '--network', 'host', '--user', `${process.getuid()}:${process.getgid()}`,
  '--workdir', root, '--volume', `${root}:${root}:rw`, '--env', 'RUSTUP_HOME=/usr/local/rustup',
  '--env', 'CARGO_HOME=/aster-cache/cargo', '--env', 'HOME=/aster-cache/home']
const cache = join(root, 'target/ci-builder-cache')
mkdirSync(join(cache, 'home'), { recursive: true })
// Create parent directories as the invoking user before Docker creates nested
// bind mount destinations; otherwise CARGO_HOME can become root-owned.
mkdirSync(join(cache, 'cargo'), { recursive: true })
args.push('--volume', `${cache}:/aster-cache:rw`)
for (const name of ['registry', 'git']) {
  const path = join(homedir(), '.cargo', name)
  mkdirSync(path, { recursive: true })
  args.push('--volume', `${path}:/aster-cache/cargo/${name}:rw`)
}
for (const [key, path] of [['GOPATH', join(homedir(), 'go')], ['GOCACHE', join(homedir(), '.cache/go-build')]]) {
  mkdirSync(path, { recursive: true })
  args.push('--volume', `${path}:${path}:rw`, '--env', `${key}=${path}`)
}
// Release inputs are supplied by the already-authorized release job, not baked into images.
if (process.env.RUNNER_TEMP) args.push('--volume', `${process.env.RUNNER_TEMP}:${process.env.RUNNER_TEMP}:ro`)
for (const path of new Set(command.filter(arg => isAbsolute(arg) && existsSync(arg) && statSync(arg).isFile()))) {
  if (!path.startsWith(`${root}/`) && !path.startsWith(`${process.env.RUNNER_TEMP}/`)) args.push('--volume', `${path}:${path}:ro`)
}
for (const key of ['GITHUB_OUTPUT', 'GITHUB_STEP_SUMMARY']) {
  if (process.env[key] && existsSync(process.env[key])) args.push('--volume', `${process.env[key]}:${process.env[key]}:rw`, '--env', key)
}
for (const key of Object.keys(process.env)) {
  if (/^(ASTER_|RELEASE_|SOURCE_DATE_EPOCH$|CANDIDATE_VERSION$)/.test(key)) args.push('--env', key)
}
docker([...args, image.image, ...command])
