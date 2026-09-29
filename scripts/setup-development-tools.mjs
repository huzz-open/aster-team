import { spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { preferredGoRoot } from '../tools/toolchains/go-toolchain.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const minimumNode = [22, 19, 0]

export function parseArguments(args) {
  let checkOnly = false
  let assumeYes = false
  for (const arg of args) {
    if (arg === '--check') checkOnly = true
    else if (arg === '--yes') assumeYes = true
    else throw new Error(`当前系统不支持参数：${arg}；请使用 ASTER_TOOLS_ROOT 指定统一工具根目录`)
  }
  return { checkOnly, assumeYes }
}

export function versionAtLeast(actual, minimum = minimumNode) {
  const values = String(actual).replace(/^v/, '').split('.').map(value => Number.parseInt(value, 10))
  for (let index = 0; index < minimum.length; index += 1) {
    if ((values[index] || 0) > minimum[index]) return true
    if ((values[index] || 0) < minimum[index]) return false
  }
  return true
}

function execute(command, args = [], options = {}) {
  return spawnSync(command, args, {
    cwd: root, encoding: 'utf8', windowsHide: true,
    stdio: options.inherit ? 'inherit' : 'pipe', env: process.env,
  })
}

function successful(command, args = ['--version']) {
  const result = execute(command, args)
  return !result.error && result.status === 0
}

function workingPython() {
  for (const command of ['python3', 'python']) {
    if (successful(command, ['-c', 'import tkinter'])) return command
  }
  return null
}

function portableReadiness() {
  const goRoot = preferredGoRoot(root)
  const go = resolve(goRoot, 'bin', 'go')
  const goResult = existsSync(go) ? execute(go, ['version']) : null
  const goReady = goResult?.status === 0 && /\bgo1\.25\.14\b/.test(String(goResult.stdout || ''))
  const database = ['mariadb', 'mysql'].find(command => successful(command))
  return [
    { name: `Node.js ${minimumNode.join('.')}+`, ok: versionAtLeast(process.versions.node), detail: process.version, setup: true },
    { name: 'Bash', ok: successful('bash', ['--version']), detail: 'bash', setup: true },
    { name: 'curl', ok: successful('curl', ['--version']), detail: 'curl', setup: true },
    { name: 'tar', ok: successful('tar', ['--version']), detail: 'tar', setup: true },
    { name: 'Python 3 + tkinter', ok: Boolean(workingPython()), detail: workingPython() || '未找到可导入 tkinter 的 Python 3', setup: true },
    { name: 'Rustup、Cargo 与 Rust 编译器', ok: successful('rustup') && successful('cargo') && successful('rustc'), detail: 'rustup / cargo / rustc', setup: true },
    { name: 'MariaDB/MySQL 客户端', ok: Boolean(database), detail: database || '未找到 mariadb 或 mysql', setup: true },
    { name: 'Go 1.25.14', ok: goReady, detail: goReady ? `${go}（go1.25.14）` : `尚未安装：${go}` },
    { name: '锁定的 Node.js 依赖', ok: existsSync(resolve(root, 'node_modules', '.package-lock.json')), detail: 'node_modules/.package-lock.json' },
  ]
}

function printReadiness(entries) {
  console.log(`\n${process.platform === 'darwin' ? 'macOS' : 'Linux'} 开发工具`)
  for (const entry of entries) console.log(`[${entry.ok ? 'PASS' : 'MISS'}] ${entry.name}: ${entry.detail}`)
}

function runRequired(command, args, label) {
  console.log(`\n正在${label}…`)
  const result = execute(command, args, { inherit: true })
  if (result.error || result.status !== 0) throw new Error(`${label}失败（${result.error?.message || `退出码 ${result.status}`}）`)
}

function npmCommand(args) {
  if (process.env.npm_execpath) return runRequired(process.execPath, [process.env.npm_execpath, ...args], args.join(' '))
  return runRequired('npm', args, args.join(' '))
}

function setupPortable(options) {
  let readiness = portableReadiness()
  printReadiness(readiness)
  if (options.checkOnly) {
    process.exitCode = readiness.every(entry => entry.ok) ? 0 : 1
    return
  }
  const missingSystemTools = readiness.filter(entry => entry.setup && !entry.ok)
  if (missingSystemTools.length) {
    throw new Error(`请先通过系统包管理器安装：${missingSystemTools.map(entry => entry.name).join('、')}，然后重新执行 npm run setup`)
  }
  npmCommand(['ci'])
  runRequired(process.execPath, ['./scripts/run-go-toolchain-setup.mjs', ...(options.assumeYes ? ['--yes'] : [])], '准备统一 Go 工具链')
  runRequired(process.execPath, ['./scripts/run-go.mjs', 'mod', 'download'], '下载 Go 模块')
  readiness = portableReadiness()
  printReadiness(readiness)
  if (!readiness.every(entry => entry.ok)) throw new Error('开发工具仍未就绪；请执行 npm run setup:check 查看缺失项')
}

function main() {
  const args = process.argv.slice(2)
  if (process.platform === 'win32') {
    const result = execute(process.execPath, ['./scripts/setup-windows-environment.mjs', 'native', ...args], { inherit: true })
    if (result.error) throw result.error
    process.exitCode = result.status ?? 1
    return
  }
  if (!['darwin', 'linux'].includes(process.platform)) throw new Error(`不支持的操作系统：${process.platform}`)
  setupPortable(parseArguments(args))
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main() } catch (error) { console.error(`Error: ${error.message}`); process.exitCode = 1 }
}
