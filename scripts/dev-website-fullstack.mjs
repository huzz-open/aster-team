import { spawn, spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { localDevelopmentAdvertisedHost } from './local-lan-development.mjs'

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const websiteRoot = resolve(repositoryRoot, 'website')
const wranglerBin = resolve(repositoryRoot, 'node_modules', 'wrangler', 'bin', 'wrangler.js')
const viteBin = resolve(repositoryRoot, 'node_modules', 'vite', 'bin', 'vite.js')

function portFromEnvironment(name, fallback) {
  const value = Number(process.env[name] || fallback)
  if (!Number.isInteger(value) || value < 1 || value > 65535) {
    console.error(`${name} must be an integer between 1 and 65535`)
    process.exit(1)
  }
  return value
}

const frontendPort = portFromEnvironment('ASTER_WEBSITE_FRONTEND_PORT', 14080)
const backendPort = portFromEnvironment('ASTER_WEBSITE_BACKEND_PORT', 8788)
const advertisedHost = localDevelopmentAdvertisedHost(process.env)
const childEnvironment = {
  ...process.env,
  ASTER_WEBSITE_FRONTEND_PORT: String(frontendPort),
  ASTER_WEBSITE_BACKEND_PORT: String(backendPort),
}

for (const requiredPath of [wranglerBin, viteBin]) {
  if (!existsSync(requiredPath)) {
    console.error(`Missing local development dependency: ${requiredPath}`)
    process.exit(1)
  }
}

const migration = spawnSync(process.execPath, [
  wranglerBin,
  'd1', 'migrations', 'apply', 'aster-team-website-leads', '--local',
], { cwd: websiteRoot, env: childEnvironment, stdio: 'inherit' })
if (migration.error) throw migration.error
if (migration.status !== 0) process.exit(migration.status ?? 1)

const processes = []
let stopping = false

function writePrefixed(stream, label, chunk) {
  const lines = String(chunk).split(/(?<=\n)/)
  for (const line of lines) {
    if (line) stream.write(`[${label}] ${line}`)
  }
}

function start(label, executable, args) {
  const child = spawn(executable, args, {
    cwd: websiteRoot,
    env: childEnvironment,
    stdio: ['ignore', 'pipe', 'pipe'],
    detached: process.platform !== 'win32',
  })
  child.stdout.on('data', chunk => writePrefixed(process.stdout, label, chunk))
  child.stderr.on('data', chunk => writePrefixed(process.stderr, label, chunk))
  child.on('error', error => {
    if (!stopping) {
      console.error(`[${label}] failed to start: ${error.message}`)
      stop(1)
    }
  })
  child.on('exit', code => {
    if (!stopping) {
      console.error(`[${label}] exited with code ${code ?? 1}`)
      stop(code || 1)
    }
  })
  processes.push(child)
}

function stop(code = 0) {
  if (stopping) return
  stopping = true
  for (const child of processes) {
    if (!child.pid) continue
    if (process.platform === 'win32') {
      spawnSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore' })
    } else {
      try { process.kill(-child.pid, 'SIGTERM') } catch { child.kill('SIGTERM') }
    }
  }
  process.exit(code)
}

start('pages', process.execPath, [
  wranglerBin,
  'pages', 'dev', 'public',
  '--ip', '127.0.0.1',
  '--port', String(backendPort),
])
start('vite', process.execPath, [viteBin])

console.log('\nAster Team website full-stack development:')
console.log(`  Frontend (Vite HMR): http://${advertisedHost}:${frontendPort}`)
console.log(`  Pages API (Wrangler): http://127.0.0.1:${backendPort}/api/trial`)
console.log('  Browser requests to /api are proxied by Vite to Wrangler.\n')

process.on('SIGINT', () => stop(0))
process.on('SIGTERM', () => stop(0))
if (process.argv.includes('--check-cleanup')) setTimeout(() => stop(0), 5_000)
