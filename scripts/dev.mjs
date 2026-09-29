import { spawn, spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { resolve } from 'node:path'

const envFile = resolve('data/local/customer.env')
if (!existsSync(envFile)) {
  console.error('缺少本地配置。请先通过本地开发控制台完成初始化。')
  process.exit(1)
}
process.loadEnvFile(envFile)

const env = { ...process.env }
const required = [
  'ASTER_LICENSE_TRUSTED_KEYS_JSON', 'ASTER_RELEASE_TRUSTED_KEYS_JSON',
  'ASTER_INSTALLATION_PROFILE_PATH', 'ASTER_INSTALLATION_KEY_PATH',
  'ASTER_LICENSE_FILE_PATH', 'ASTER_LICENSE_STATE_PATH',
]
const missing = required.filter(name => !env[name]?.trim())
if (missing.length) {
  console.error(`本地开发配置不完整: ${missing.join(', ')}`)
  process.exit(1)
}

const processes = []
let stopping = false

function start(label, command, args) {
  const child = spawn(command, args, {
    stdio: ['ignore', 'pipe', 'pipe'],
    env,
    shell: process.platform === 'win32',
    detached: process.platform !== 'win32',
  })
  child.stdout.on('data', chunk => process.stdout.write(`[${label}] ${chunk}`))
  child.stderr.on('data', chunk => process.stderr.write(`[${label}] ${chunk}`))
  child.on('exit', code => {
    if (!stopping && code !== 0) {
      console.error(`[${label}] exited with code ${code}`)
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

start('control', 'npm', ['run', 'dev:api'])
start('admin', 'npm', ['run', 'dev:admin'])
start('member', 'npm', ['run', 'dev:member'])

console.log('\nAster Team 本地开发环境正在启动：')
console.log('  管理端: http://127.0.0.1:11082')
console.log('  用户端: http://127.0.0.1:11081')
console.log('  API:    http://127.0.0.1:11080')
console.log(`  管理员: ${env.BOOTSTRAP_ADMIN_EMAIL}`)
console.log('  许可证: Rust Control 直接校验离线许可证，不再启动独立 Guard。')
console.log('  Runner: 请在管理端创建注册令牌后启动 Rust Runner。\n')

process.on('SIGINT', () => stop(0))
process.on('SIGTERM', () => stop(0))
if (process.argv.includes('--check-cleanup')) setTimeout(() => stop(0), 4_000)
