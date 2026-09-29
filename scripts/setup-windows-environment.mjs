import { spawn, spawnSync } from 'node:child_process'
import { closeSync, copyFileSync, createReadStream, existsSync, mkdtempSync, openSync, readFileSync, readSync, readdirSync, rmSync, statSync } from 'node:fs'
import { StringDecoder } from 'node:string_decoder'
import { createHash } from 'node:crypto'
import { tmpdir } from 'node:os'
import { createInterface } from 'node:readline/promises'
import { dirname, resolve, win32 } from 'node:path'
import { fileURLToPath } from 'node:url'
import { resolveWindowsDockerRuntime } from './windows-docker-runtime.mjs'
import { fileDigest, prepareVerifiedDownload } from './release-download-cache.mjs'
import { acquireSetupLock, githubEnvironment, githubRepository, prepareGitHubAuthentication, readGitHubAuth, runNativeSetupPhases } from './windows-setup-github.mjs'
import { preferredGoRoot } from '../tools/toolchains/go-toolchain.mjs'
import {
  collectNativeChoices, nativePackageOverride, parseSetupArguments, readSetupSettings,
  rootPasswordSql, selectInstallRoot, setupEnvironment, windowsInstallPaths, writeSetupSettings,
} from './windows-setup-options.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
let setupSettings = readSetupSettings(root)
const configuredPaths = () => setupSettings.installRoot ? windowsInstallPaths(setupSettings.installRoot) : null
const minimumNode = [22, 19, 0]
const buildToolsOverride = '--quiet --wait --norestart --add Microsoft.VisualStudio.Component.VC.Tools.x86.x64 --add Microsoft.VisualStudio.Component.Windows11SDK.26100 --addProductLang En-us'

// Digest published in the official winx64-packages/sha256sums.txt for this release.
export const mariaDBDownload = Object.freeze({
  version: '12.3.3',
  bytes: 92852224,
  sha256: '811a38a862c1c55325b6ba8a757b381923e51dee4fe3cde95887b0ae2490c02d',
  urls: [
    'https://dlm.mariadb.com/4805482/MariaDB/mariadb-12.3.3/winx64-packages/mariadb-12.3.3-winx64.msi',
    'https://mirror.mariadb.org/mariadb-12.3.3/winx64-packages/mariadb-12.3.3-winx64.msi',
    'https://archive.mariadb.org/mariadb-12.3.3/winx64-packages/mariadb-12.3.3-winx64.msi',
  ],
})

export const nativePackages = Object.freeze([
  { key: 'bash', name: 'Git for Windows（提供 Git Bash）', id: 'Git.Git' },
  { key: 'gh', name: 'GitHub CLI（gh）', id: 'GitHub.cli' },
  { key: 'python', name: 'Python 3（包含 tkinter）', id: 'Python.Python.3.13' },
  { key: 'rustup', name: 'Rustup', id: 'Rustlang.Rustup' },
  { key: 'msvc', name: 'MSVC Build Tools 与 Windows SDK', id: 'Microsoft.VisualStudio.BuildTools', override: buildToolsOverride },
  { key: 'mariadb', name: 'MariaDB Server', id: 'MariaDB.Server' },
])

export const linuxLabPackages = Object.freeze([
  { key: 'wsl', name: 'Windows Subsystem for Linux', id: 'Microsoft.WSL' },
  { key: 'docker', name: 'Docker Desktop', id: 'Docker.DockerDesktop' },
])

function usage() {
  console.log(`Usage:
  node scripts/setup-windows-environment.mjs native [--check] [--yes] [--install-root D:\\AsterDev]
  node scripts/setup-windows-environment.mjs linux-lab [--check] [--yes]

Profiles:
  native     Prepare the Windows-native source development environment.
  linux-lab  Prepare optional WSL 2 and Docker Desktop Linux release validation.

Options:
  --check    Report readiness without changing the machine.
  --yes      Use defaults and a random database password; existing GitHub authentication is required.
  --install-root PATH  Choose the shared Windows native tool installation root.

Run native installation in an administrator terminal. All choices are collected before downloads.`)
}

function execute(command, args = [], options = {}) {
  return spawnSync(command, args, {
    cwd: options.cwd || root,
    encoding: 'utf8',
    stdio: options.capture === false ? 'inherit' : 'pipe',
    windowsHide: true,
    env: options.env || process.env,
    timeout: options.timeout,
    input: options.input,
  })
}

export function createSetupLogProgress(directory = tmpdir()) {
  const pattern = /^dd_(?:setup|installer|bootstrapper)_\d+\.log$/
  const previous = new Set(readdirSync(directory).filter(name => pattern.test(name)))
  let latest = ''
  let lastChange = Date.now()
  return () => {
    const messages = []
    for (const name of readdirSync(directory).filter(name => pattern.test(name) && !previous.has(name))) {
      // Only read logs created after this command started; never print command lines or telemetry.
      try {
        const lines = readFileSync(resolve(directory, name), 'utf8').slice(-65536).split(/\r?\n/)
        for (const line of lines) {
          const match = line.match(/\[(\d{4}-\d\d-\d\dT[\d:]+)\] ((?:BEGIN|END): (?:Downloading package|Installing) .+|(?:Started|Completed): Installing .+|Package executed successfully\..+)/)
          if (match) messages.push(`${match[1]} ${match[2]}`)
        }
      } catch { /* The installer may be rotating or locking a log. */ }
    }
    const current = messages.sort().at(-1)
    if (current && current !== latest) {
      latest = current
      lastChange = Date.now()
      return latest
    }
    const seconds = Math.floor((Date.now() - lastChange) / 1000)
    return `${latest ? `最近进度：${latest}；` : '等待安装器输出组件进度；'}${seconds} 秒无新日志（可能正在下载或等待系统响应）`
  }
}

export async function executeWithProgress(command, args = [], options = {}) {
  const { label = command, log = console.log, intervalMs = 10000, progress } = options
  const started = Date.now()
  log(`[开始] ${label}`)
  const output = options.output || process.stdout
  const inline = output.isTTY && typeof options.progressBar === 'function'
  let lineActive = false
  let childOutput = ''
  let lastLogAt = started
  let child
  const drawBar = () => {
    output.write(`\r\x1b[2K${options.progressBar(Date.now() - started, output.columns || 80)}`)
    lineActive = true
  }
  const restoreCursor = () => {
    if (lineActive) { output.write('\n'); lineActive = false }
    if (output.isTTY) output.write('\x1b[?25h')
  }
  const interrupt = () => { child?.kill(); restoreCursor(); process.exit(130) }
  const terminate = () => { child?.kill(); restoreCursor(); process.exit(143) }
  if (output.isTTY) {
    output.write('\x1b[?25l')
    process.once('exit', restoreCursor)
    process.once('SIGINT', interrupt)
    process.once('SIGTERM', terminate)
  }
  let timer
  let logTimer
  let logReadError = ''
  const drainLogs = (flush = false) => {
    try {
      const lines = options.readLogLines?.(flush) || []
      if (lines.length) lastLogAt = Date.now()
      for (const line of lines) log(`[MSI] ${line}`)
      logReadError = ''
    } catch (error) {
      if (logReadError !== error.message) log(`[日志] 暂时无法读取安装日志：${error.message}`)
      logReadError = error.message
    }
  }
  try {
    return await new Promise(resolveResult => {
      child = spawn(command, args, {
        cwd: options.cwd || root, env: options.env || process.env,
        stdio: inline ? ['inherit', 'pipe', 'pipe'] : 'inherit', windowsHide: true,
      })
      if (inline) {
        // The progress renderer owns the terminal line. Print child diagnostics after it finishes.
        for (const stream of [child.stdout, child.stderr]) {
          stream.setEncoding('utf8')
          stream.on('data', chunk => { childOutput = (childOutput + chunk).slice(-8192) })
        }
        drawBar()
      }
      if (options.readLogLines) logTimer = setInterval(drainLogs, 500)
      timer = setInterval(() => {
        if (inline) { drawBar(); return }
        if (options.readLogLines && Date.now() - lastLogAt < intervalMs) return
        let detail = ''
        try { detail = progress?.() || '' } catch { detail = '暂时无法读取安装日志' }
        log(`[等待 ${Math.floor((Date.now() - started) / 1000)} 秒] ${label}；${detail || '进程尚未退出，等待完成'}`)
      }, inline ? 250 : intervalMs)
      child.once('error', error => resolveResult({ error, status: null }))
      child.once('close', (status, signal) => resolveResult({ status, signal }))
    })
  } finally {
    clearInterval(timer)
    clearInterval(logTimer)
    drainLogs(true)
    if (inline) drawBar()
    restoreCursor()
    process.removeListener('exit', restoreCursor)
    process.removeListener('SIGINT', interrupt)
    process.removeListener('SIGTERM', terminate)
    if (childOutput.trim()) log(childOutput.trim())
    log(`[结束] ${label}，耗时 ${Math.floor((Date.now() - started) / 1000)} 秒`)
  }
}

export function renderDownloadBar(destination, totalBytes, elapsedMs, columns = 80) {
  let bytes = 0
  try { bytes = statSync(destination).size } catch { /* Download has not started yet. */ }
  const ratio = totalBytes > 0 ? Math.min(1, bytes / totalBytes) : 0
  const percent = `${(ratio * 100).toFixed(1)}%`
  const size = `${(bytes / 1048576).toFixed(1)}/${(totalBytes / 1048576).toFixed(1)} MiB`
  const elapsed = `${Math.floor(elapsedMs / 1000)}s`
  // ASCII characters occupy one terminal cell; leave the last column free to avoid wrapping.
  const available = Math.max(1, columns - 1)
  let details = `${percent} | ${size} | ${elapsed}`
  if (details.length + 10 > available) details = `${percent} | ${elapsed}`
  if (details.length + 10 > available) return percent.slice(0, available)
  const width = Math.min(28, available - details.length - 3)
  const filled = Math.floor(ratio * width)
  const bar = '='.repeat(filled) + (filled < width ? '>' + ' '.repeat(width - filled - 1) : '')
  return `[${bar}] ${details}`
}

export function downloadProgress(destination, totalBytes) {
  let bytes = 0
  try { bytes = statSync(destination).size } catch { /* curl has not created the file yet. */ }
  const downloaded = (bytes / 1024 / 1024).toFixed(1)
  if (!totalBytes) return `已下载 ${downloaded} MiB`
  return `${Math.min(100, bytes / totalBytes * 100).toFixed(1)}% | ${downloaded} / ${(totalBytes / 1024 / 1024).toFixed(1)} MiB`
}

export async function downloadVerifiedMariaDB(destination, {
  download = mariaDBDownload, run = executeWithProgress, log = console.log,
} = {}) {
  for (const [index, url] of download.urls.entries()) {
    log(`下载源 ${index + 1}/${download.urls.length}：${url}`)
    const result = await run('curl.exe', [
      '--fail', '--location', '--silent', '--show-error', '--connect-timeout', '30',
      '--max-time', '1800', '--speed-limit', '1024', '--speed-time', '120',
      '--proto', '=https', '--proto-redir', '=https', '--output', destination, url,
    ], {
      label: `下载 MariaDB ${download.version}`,
      progress: () => downloadProgress(destination, download.bytes),
      progressBar: (elapsedMs, columns) => renderDownloadBar(destination, download.bytes, elapsedMs, columns),
    })
    if (result.error || result.status !== 0) {
      rmSync(destination, { force: true })
      log(`下载失败（${result.error?.message || `退出码 ${result.status}`}）${index + 1 < download.urls.length ? '，尝试备用官方源。' : '。'}`)
      continue
    }
    const hash = createHash('sha256')
    for await (const chunk of createReadStream(destination)) hash.update(chunk)
    if (hash.digest('hex') !== download.sha256) {
      rmSync(destination, { force: true })
      throw new Error('MariaDB 安装包 SHA-256 不匹配，已删除，未执行安装')
    }
    log(`[PASS] 下载完成（${downloadProgress(destination, download.bytes)}）；MariaDB 安装包 SHA-256 校验通过`)
    return
  }
  throw new Error('MariaDB 官方下载源均失败；请检查网络或代理后重试 npm run setup')
}

export function installerSucceeded(result) {
  return !result.error && [0, 3010, 1641].includes(result.status)
}

export function installerNeedsRestart(result) {
  // Native MSI codes and winget's signed/unsigned reboot HRESULTs.
  return !result.error && Number.isInteger(result.status)
    && [3010, 1641, 0x8a150109, 0x8a15010a, 0x8a15010b].includes(result.status >>> 0)
}

export async function prepareMariaDBInstaller({
  cacheDirectory = resolve(root, '.aster-tools', 'downloads', 'mariadb'),
  legacyDirectory = tmpdir(), download = mariaDBDownload, run = executeWithProgress, log = console.log,
} = {}) {
  const fileName = `mariadb-${download.version}-winx64.msi`
  const installer = await prepareVerifiedDownload({
    cacheDirectory, fileName, algorithm: 'sha256', expectedDigest: download.sha256,
    download: async partial => {
      const oldCache = resolve(root, '.aster-tools', 'downloads', 'mariadb', fileName)
      if (existsSync(oldCache) && await fileDigest(oldCache, 'sha256') === download.sha256) {
        copyFileSync(oldCache, partial)
        log(`[缓存] 复用原仓库缓存：${oldCache}`)
        return
      }
      // Recover complete packages downloaded by older setup versions; leave running installers' files intact.
      for (const entry of readdirSync(legacyDirectory, { withFileTypes: true })) {
        if (!entry.isDirectory() || !/^aster-mariadb-[a-zA-Z0-9]+$/.test(entry.name)) continue
        const candidate = resolve(legacyDirectory, entry.name, fileName)
        try {
          if (!existsSync(candidate) || (download.bytes && statSync(candidate).size !== download.bytes)) continue
          if (await fileDigest(candidate, 'sha256') !== download.sha256) continue
          copyFileSync(candidate, partial)
        } catch { continue }
        log(`[缓存] 复用此前已下载并通过 SHA-256 校验的安装包：${candidate}`)
        return
      }
      await downloadVerifiedMariaDB(partial, { download, run, log })
    },
  })
  log(`[缓存] MariaDB 安装包已校验：${installer}（安装失败或重试时保留）`)
  return installer
}

export function mariaDBMsiArguments(installer, logPath) {
  return ['/i', installer, '/qn', '/norestart', '/L*V!', logPath, 'SERVICENAME=MariaDB']
}

export function mariaDBElevatedScript(installer, logPath, startedPath, { installRoot, elevate = true } = {}) {
  for (const path of [installer, logPath, startedPath]) {
    if (/["\r\n]/.test(path)) throw new Error('MSI 路径包含不支持的字符')
  }
  // MSI switches and PROPERTY=value tokens must remain unquoted. Quote only path arguments.
  const paths = installRoot ? windowsInstallPaths(installRoot) : null
  const location = paths ? ` INSTALLDIR="${paths.mariaDB}" DATADIR="${paths.mariaDBData}"` : ''
  const argumentList = `/i "${installer}" /qn /norestart /L*V! "${logPath}" SERVICENAME=MariaDB${location}`.replaceAll("'", "''")
  const marker = startedPath.replaceAll("'", "''")
  return `$ErrorActionPreference = 'Stop'; $installerProcess = Start-Process -FilePath msiexec.exe -ArgumentList '${argumentList}'${elevate ? ' -Verb RunAs' : ''} -WindowStyle Hidden -PassThru; [IO.File]::WriteAllText('${marker}', [string]$installerProcess.Id); $installerProcess.WaitForExit(); exit $installerProcess.ExitCode`
}

export function createMsiProgress(logPath, startedPath, needsElevation) {
  const started = Date.now()
  let authorizedAt
  return () => {
    if (existsSync(logPath)) {
      const buffer = readFileSync(logPath)
      const text = buffer.toString(buffer[0] === 0xff && buffer[1] === 0xfe ? 'utf16le' : 'utf8')
      const action = text.split(/\r?\n/).filter(line => /^(?:Action start|Action ended)|Doing action:/.test(line)).at(-1)
      return `MSI 安装已启动；${action || '正在初始化安装引擎'}（暂无新的安装日志）`
    }
    if (existsSync(startedPath) || !needsElevation) {
      authorizedAt ??= Date.now()
      const seconds = Math.floor((Date.now() - authorizedAt) / 1000)
      return `已获管理员权限，MSI 尚未生成安装日志（等待 ${seconds} 秒）${seconds >= 30 ? '；启动异常，请检查安装器，不是仍在等待 UAC' : ''}`
    }
    return `等待 Windows 权限确认或进程启动（${Math.floor((Date.now() - started) / 1000)} 秒）；尚未收到 MSI 启动确认`
  }
}

export function createMsiLogReader(logPath) {
  let offset = 0
  let decoder
  let pending = ''
  return (flush = false) => {
    let descriptor
    try { descriptor = openSync(logPath, 'r') } catch (error) {
      if (['ENOENT', 'EACCES', 'EBUSY'].includes(error.code)) return []
      throw error
    }
    try {
      if (!decoder) {
        const header = Buffer.alloc(2)
        if (readSync(descriptor, header, 0, 2, 0) < 2) return []
        const utf16 = header[0] === 0xff && header[1] === 0xfe
        decoder = new StringDecoder(utf16 ? 'utf16le' : 'utf8')
        offset = utf16 ? 2 : 0
      }
      const buffer = Buffer.alloc(65536)
      let bytes
      while ((bytes = readSync(descriptor, buffer, 0, buffer.length, offset)) > 0) {
        offset += bytes
        pending += decoder.write(buffer.subarray(0, bytes))
      }
    } finally { closeSync(descriptor) }
    const lines = pending.split(/\r?\n/)
    pending = lines.pop() || ''
    if (flush && pending) { lines.push(pending); pending = '' }
    // Forward installation actions/results/errors, not MSI property dumps or command lines containing credentials.
    return lines.filter(line => !/Property\([CS]\):|PROPERTY CHANGE:|Command Line:|\b(?:PASSWORD|TOKEN|SECRET)\s*=/i.test(line)
      && /^(?:Action start|Action ended|Error\s|Warning\s)|Doing action:|Product:|Windows Installer |MainEngineThread is returning|\b(?:error|failed|failure)\b/i.test(line))
  }
}

async function installMariaDB(installRoot) {
  const installer = await prepareMariaDBInstaller({ cacheDirectory: resolve(windowsInstallPaths(installRoot).cache, 'MariaDB') })
  const directory = mkdtempSync(resolve(tmpdir(), 'aster-mariadb-'))
  const logPath = resolve(directory, 'install.log')
  const startedPath = resolve(directory, 'installer-started.txt')
  const needsElevation = !administrator()
  const progress = createMsiProgress(logPath, startedPath, needsElevation)
  const readLogLines = createMsiLogReader(logPath)
  console.log(`MariaDB 安装日志：${logPath}`)
  const script = mariaDBElevatedScript(installer, logPath, startedPath, { installRoot, elevate: needsElevation })
  const result = await executeWithProgress('powershell.exe', ['-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(script, 'utf16le').toString('base64')], { label: '安装 MariaDB Server', progress, readLogLines })
  if (!installerSucceeded(result)) throw new Error(`MariaDB Server 安装失败（${result.error?.message || `退出码 ${result.status}`}）；日志：${logPath}；安装包已保留：${installer}`)
  if (installerNeedsRestart(result)) throw new Error('MariaDB 安装需要重启 Windows，已停止后续步骤；目录、缓存及待设置密码状态已保留，请重启后重新运行 npm run setup')
}

function successful(command, args = []) {
  const result = execute(command, args)
  return !result.error && result.status === 0
}

function commandPath(name) {
  const result = execute('where.exe', [name])
  if (result.error || result.status !== 0) return null
  return String(result.stdout || '').split(/\r?\n/).map(value => value.trim()).find(Boolean) || null
}

function firstExisting(paths) {
  return paths.find(path => path && existsSync(path)) || null
}

function isGitBash(path) {
  if (!path || !existsSync(path)) return false
  const result = execute(path, ['--noprofile', '--norc', '-c', 'uname -s'])
  return result.status === 0 && /^(?:MINGW|MSYS|CYGWIN)/.test(String(result.stdout || '').trim())
}

export function gitBashPath() {
  const gitPaths = String(execute('where.exe', ['git.exe']).stdout || '').split(/\r?\n/).map(value => value.trim()).filter(Boolean)
  const candidates = [
    configuredPaths() && resolve(configuredPaths().git, 'bin', 'bash.exe'),
    ...gitPaths.flatMap(path => [resolve(dirname(dirname(path)), 'bin', 'bash.exe'), resolve(dirname(dirname(path)), 'usr', 'bin', 'bash.exe')]),
    resolve(process.env.ProgramFiles || 'C:\\Program Files', 'Git', 'bin', 'bash.exe'),
    resolve(process.env.ProgramFiles || 'C:\\Program Files', 'Git', 'usr', 'bin', 'bash.exe'),
    commandPath('bash.exe'),
  ]
  return candidates.find(isGitBash) || null
}

function pythonCandidates() {
  const values = []
  if (configuredPaths()) values.push([resolve(configuredPaths().python, 'python.exe')])
  const launcher = commandPath('py.exe')
  if (launcher) values.push([launcher, '-3'])
  const python = commandPath('python.exe')
  if (python) values.push([python])
  const programs = resolve(process.env.LOCALAPPDATA || '', 'Programs', 'Python')
  if (existsSync(programs)) {
    for (const entry of readdirSync(programs).sort().reverse()) {
      const candidate = resolve(programs, entry, 'python.exe')
      if (existsSync(candidate)) values.push([candidate])
    }
  }
  return values
}

function workingPython() {
  for (const [command, ...prefix] of pythonCandidates()) {
    if (successful(command, [...prefix, '-c', 'import tkinter; print(tkinter.TkVersion)'])) return [command, ...prefix]
  }
  return null
}

function cargoTool(name) {
  return commandPath(`${name}.exe`) || firstExisting([
    setupSettings.managedRust && resolve(configuredPaths().cargo, 'bin', `${name}.exe`),
    process.env.CARGO_HOME && resolve(process.env.CARGO_HOME, 'bin', `${name}.exe`),
    resolve(process.env.USERPROFILE || '', '.cargo', 'bin', `${name}.exe`),
  ])
}

function githubCLIPath() {
  const found = commandPath('gh.exe') || firstExisting([
    configuredPaths() && resolve(configuredPaths().gh, 'gh.exe'),
    resolve(process.env.ProgramFiles || 'C:\\Program Files', 'GitHub CLI', 'gh.exe'),
    resolve(process.env['ProgramFiles(x86)'] || 'C:\\Program Files (x86)', 'GitHub CLI', 'gh.exe'),
  ])
  if (found) return found
  const registration = powerShell("(Get-ItemProperty 'HKLM:\\SOFTWARE\\GitHub\\CLI' -ErrorAction SilentlyContinue).InstallDir")
  const path = String(registration.stdout || '').trim()
  return registration.status === 0 && path ? firstExisting([resolve(path, 'gh.exe')]) : null
}

function gitExecutable() {
  const bash = gitBashPath()
  return commandPath('git.exe') || (bash && firstExisting([resolve(dirname(bash), '..', 'cmd', 'git.exe')]))
}

function msvcInstallation() {
  const vswhere = firstExisting([
    resolve(process.env['ProgramFiles(x86)'] || 'C:\\Program Files (x86)', 'Microsoft Visual Studio', 'Installer', 'vswhere.exe'),
    resolve(process.env.ProgramFiles || 'C:\\Program Files', 'Microsoft Visual Studio', 'Installer', 'vswhere.exe'),
  ])
  if (!vswhere) return null
  const result = execute(vswhere, [
    '-latest', '-products', '*',
    '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64',
    '-requires', 'Microsoft.VisualStudio.Component.Windows11SDK.26100',
    '-property', 'installationPath',
  ])
  if (result.status !== 0) return null
  return String(result.stdout || '').trim() || null
}

function mariaDBState() {
  const service = execute('sc.exe', ['query', 'state=', 'all'])
  const source = String(service.stdout || '')
  const match = source.match(/SERVICE_NAME:\s*((?:MariaDB|MySQL)[^\r\n]*)[\s\S]{0,500}?STATE\s*:\s*(\d+)\s+([^\r\n]+)/i)
  if (match) return { installed: true, running: match[2] === '4', detail: `${match[1].trim()}（${match[3].trim()}）` }
  if (configuredPaths() && existsSync(resolve(configuredPaths().mariaDB, 'bin'))) return { installed: true, running: false, detail: `${configuredPaths().mariaDB}（服务未运行）` }
  const programFiles = process.env.ProgramFiles || 'C:\\Program Files'
  if (existsSync(programFiles)) {
    for (const entry of readdirSync(programFiles)) {
      if (/^(?:MariaDB|MySQL)/i.test(entry) && existsSync(resolve(programFiles, entry, 'bin'))) {
        return { installed: true, running: false, detail: `${resolve(programFiles, entry)}（服务未运行）` }
      }
    }
  }
  return { installed: false, running: false, detail: '未找到本地数据库服务' }
}

export function versionAtLeast(actual, minimum) {
  const values = String(actual).replace(/^v/, '').split('.').map(value => Number.parseInt(value, 10))
  for (let index = 0; index < minimum.length; index += 1) {
    const part = values[index] || 0
    if (part > minimum[index]) return true
    if (part < minimum[index]) return false
  }
  return true
}

export function nativeReadiness({ checkAuth = false } = {}) {
  const bash = gitBashPath()
  const python = workingPython()
  const rustup = cargoTool('rustup')
  const cargo = cargoTool('cargo')
  const rustc = cargoTool('rustc')
  const msvc = msvcInstallation()
  const mariadb = mariaDBState()
  const gh = githubCLIPath()
  const localGo = resolve(preferredGoRoot(root), 'bin', 'go.exe')
  const goVersion = existsSync(localGo) ? execute(localGo, ['version']) : null
  const goReady = goVersion?.status === 0 && /\bgo1\.25\.14\b/.test(String(goVersion.stdout || ''))
  const entries = [
    { key: 'node', name: `Node.js ${minimumNode.join('.')}+`, ok: versionAtLeast(process.versions.node, minimumNode), detail: process.version },
    { key: 'bash', name: 'Git Bash', ok: Boolean(bash), detail: bash || '未找到 bash.exe' },
    { key: 'gh', name: 'GitHub CLI', ok: Boolean(gh), detail: gh || '未找到 gh.exe' },
    { key: 'python', name: 'Python 3 + tkinter', ok: Boolean(python), detail: python?.join(' ') || '未找到可导入 tkinter 的 Python 3' },
    { key: 'rustup', name: 'Rustup、Cargo 与 Rust 编译器', ok: Boolean(rustup && cargo && rustc), detail: rustup || '未找到 Rustup' },
    { key: 'msvc', name: 'MSVC x64/x86 Build Tools + Windows 11 SDK 26100', ok: Boolean(msvc), detail: msvc || '未找到完整的 C++ 编译组件' },
    { key: 'mariadb', name: 'MariaDB/MySQL 服务', ok: mariadb.running, installed: mariadb.installed, detail: mariadb.detail },
    { key: 'go', name: 'Go 1.25.14', ok: goReady, detail: goReady ? `${localGo}（go1.25.14）` : existsSync(localGo) ? `${localGo}（版本不匹配）` : `尚未安装：${localGo}` },
    { key: 'node_modules', name: '锁定的 Node.js 依赖', ok: existsSync(resolve(root, 'node_modules', '.package-lock.json')), detail: 'node_modules/.package-lock.json' },
  ]
  if (checkAuth) {
    let auth = { ok: false, reason: '需要先安装 Git 和 gh' }
    if (gh && gitExecutable()) {
      try {
        const remote = execute(gitExecutable(), ['remote', 'get-url', 'origin'])
        const target = githubRepository(remote.stdout)
        auth = readGitHubAuth(gh, target.host, { run: execute, cwd: root })
      } catch (error) { auth = { ok: false, reason: error.message } }
    }
    entries.push({ key: 'github-auth', name: 'GitHub 当前账号认证', ok: auth.ok, detail: auth.ok ? `${auth.login}（${auth.source}）` : auth.reason })
  }
  return entries
}

function powerShell(script) {
  const executable = commandPath('powershell.exe') || commandPath('pwsh.exe')
  if (!executable) return { error: new Error('未找到 PowerShell'), status: 1, stdout: '', stderr: '' }
  return execute(executable, ['-NoProfile', '-NonInteractive', '-Command', script])
}

function windowsFeatureState(name) {
  const escaped = name.replaceAll("'", "''")
  const result = powerShell(`(Get-WindowsOptionalFeature -Online -FeatureName '${escaped}').State`)
  if (result.status !== 0) return null
  return String(result.stdout || '').trim() === 'Enabled'
}

function wslReady() {
  const wsl = commandPath('wsl.exe') || resolve(process.env.SystemRoot || 'C:\\Windows', 'System32', 'wsl.exe')
  return existsSync(wsl) && successful(wsl, ['--version']) && successful(wsl, ['--status'])
}

function dockerState() {
  try {
    const runtime = resolveWindowsDockerRuntime()
    const helper = runtime.credentialHelpers.length > 0 ? `；凭据助手 ${runtime.credentialHelpers.join('、')}` : ''
    return { installed: true, running: true, detail: `${runtime.executable}${helper}` }
  } catch (error) {
    const installed = !/Docker Desktop CLI was not found/.test(error.message)
    return { installed, running: false, detail: error.message }
  }
}

export function linuxLabReadiness() {
  const docker = dockerState()
  const wsl = wslReady()
  const wslFeature = windowsFeatureState('Microsoft-Windows-Subsystem-Linux')
  const vmFeature = windowsFeatureState('VirtualMachinePlatform')
  const inferredDetail = 'WSL 2 已实际运行；非管理员检查不读取可选功能状态'
  return [
    { key: 'wsl-feature', name: 'Windows Subsystem for Linux 可选功能', ok: wslFeature === true || (wslFeature === null && wsl), detail: wslFeature === null && wsl ? inferredDetail : 'Microsoft-Windows-Subsystem-Linux' },
    { key: 'vm-feature', name: 'Virtual Machine Platform 可选功能', ok: vmFeature === true || (vmFeature === null && wsl), detail: vmFeature === null && wsl ? inferredDetail : 'VirtualMachinePlatform' },
    { key: 'wsl', name: 'WSL 2 运行时', ok: wsl, detail: 'wsl.exe --version / --status' },
    { key: 'docker', name: 'Docker Desktop', ok: docker.installed, detail: docker.detail },
    { key: 'docker-running', name: 'Docker Engine', ok: docker.running, detail: docker.running ? 'docker info 已通过' : '启动 Docker Desktop 后重新检查' },
  ]
}

function printReadiness(title, entries) {
  console.log(`\n${title}`)
  for (const entry of entries) console.log(`${entry.ok ? '[PASS]' : '[MISS]'} ${entry.name}: ${entry.detail}`)
}

async function wingetInstall(entry, paths) {
  const args = [
    'install', '--exact', '--id', entry.id, '--source', 'winget', '--silent', '--disable-interactivity',
    '--accept-package-agreements', '--accept-source-agreements',
  ]
  const override = paths ? nativePackageOverride(entry, paths) : entry.override
  if (override) args.push('--override', override)
  console.log(`\n正在安装 ${entry.name}…`)
  const progress = entry.key === 'msvc' ? createSetupLogProgress() : undefined
  if (progress) console.log(`每 10 秒输出等待时间和本次组件进度；详细日志：${tmpdir()}\\dd_setup_*.log`)
  const result = await executeWithProgress('winget.exe', args, { label: entry.name, progress })
  if (installerNeedsRestart(result)) throw new Error(`${entry.name} 提示需要重启 Windows，已停止后续步骤；请重启后重新执行原 setup 命令，已安装工具会被复用`)
  if (result.error || result.status !== 0) throw new Error(`${entry.name} 安装失败（退出码 ${result.status ?? 'unknown'}）`)
}

function startDatabaseService() {
  const script = "Get-Service | Where-Object { $_.Name -match '^(MariaDB|MySQL)' } | Where-Object Status -ne 'Running' | Start-Service"
  const result = powerShell(script)
  if (result.status !== 0) throw new Error('MariaDB/MySQL 服务启动失败')
}

function administrator() {
  const result = powerShell("[Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)")
  return result.status === 0 && String(result.stdout || '').trim().toLowerCase() === 'true'
}

function checkSetupUser() {
  const result = powerShell("$ErrorActionPreference = 'Stop'; $identity = [Security.Principal.WindowsIdentity]::GetCurrent(); $session = (Get-Process -Id $PID).SessionId; $owners = @(Get-CimInstance Win32_Process -Filter \"Name='explorer.exe'\" | Where-Object SessionId -eq $session | ForEach-Object { (Invoke-CimMethod -InputObject $_ -MethodName GetOwnerSid).Sid }); if ($owners.Count -gt 0 -and $identity.User.Value -notin $owners) { throw '请使用日常桌面账号的管理员终端' }; Write-Output $identity.Name")
  if (result.error || result.status !== 0) throw new Error('无法确认安装用户与当前桌面用户一致，请使用日常开发账号的管理员 PowerShell')
  console.log(`安装与 GitHub 认证用户：${String(result.stdout).trim()}`)
}

async function confirmPlan(lines, assumeYes) {
  console.log('\n将执行：')
  for (const line of lines) console.log(`  - ${line}`)
  if (assumeYes) return true
  if (!process.stdin.isTTY) throw new Error('非交互终端必须显式增加 --yes')
  const prompt = createInterface({ input: process.stdin, output: process.stdout })
  try { return ['y', 'yes'].includes((await prompt.question('继续安装？[y/N] ')).trim().toLowerCase()) }
  finally { prompt.close() }
}

export function initializeDatabaseRootPassword(client, password, expectedDataDirectory, { run = execute, log = console.log } = {}) {
  const args = ['--no-defaults', '--protocol=TCP', '--host=127.0.0.1', '--port=3306', '--user=root', '--skip-password', '--connect-timeout=10', '--batch', '--skip-column-names']
  const location = run(client, args, { input: 'SELECT @@datadir;\n' })
  const actual = String(location.stdout || '').trim()
  const normalize = value => win32.normalize(value).replace(/[\\/]+$/, '').toLowerCase()
  if (location.error || location.status !== 0 || normalize(actual) !== normalize(expectedDataDirectory)) {
    throw new Error('无法确认新安装的本地数据库实例（目录或空密码登录不匹配）；未修改现有账号。请检查 MariaDB 服务与 3306 端口后重试。')
  }
  const result = run(client, args, { input: rootPasswordSql(password) })
  if (result.error || result.status !== 0) throw new Error('新数据库 root 密码初始化失败；安装包已缓存，再次运行 setup 可重新设置。')
  log('\nMariaDB 管理员连接（请保存，密码不写入凭据文件）：')
  log('  地址：127.0.0.1:3306')
  log('  用户：root')
  log(`  密码：${password}`)
}

function persistToolEnvironment(settings) {
  const paths = windowsInstallPaths(settings.installRoot)
  const gh = githubCLIPath()
  const additions = [resolve(paths.git, 'cmd'), gh ? dirname(gh) : paths.gh, paths.python, resolve(paths.python, 'Scripts')].filter(existsSync)
  const variables = { ASTER_TOOLS_ROOT: paths.root }
  if (settings.managedRust) {
    variables.CARGO_HOME = paths.cargo
    variables.RUSTUP_HOME = paths.rustup
    additions.push(resolve(paths.cargo, 'bin'))
  }
  if (settings.managedGo) additions.push(resolve(paths.go, 'bin'))
  const literal = value => `'${value.replaceAll("'", "''")}'`
  const commands = Object.entries(variables).map(([key, value]) => `[Environment]::SetEnvironmentVariable(${literal(key)}, ${literal(value)}, 'User')`)
  if (additions.length) commands.push(`$setupPaths = @(${additions.map(literal).join(',')}); $oldPath = [Environment]::GetEnvironmentVariable('Path','User'); $newPath = (@($setupPaths) + @($oldPath -split ';') | Where-Object { $_ } | Select-Object -Unique) -join ';'; [Environment]::SetEnvironmentVariable('Path',$newPath,'User')`)
  const result = powerShell(`$ErrorActionPreference = 'Stop'; ${commands.join('; ')}`)
  if (result.error || result.status !== 0) throw new Error('保存工具路径环境变量失败，请检查当前用户权限后重试')
}

async function installNative({ assumeYes, installRoot }) {
  if (!administrator()) throw new Error('请先在“以管理员身份运行”的 PowerShell 中执行 npm run setup。权限在下载前检查，后续安装不再逐项请求 UAC。')
  checkSetupUser()
  let readiness = nativeReadiness()
  printReadiness('Windows 原生开发环境', readiness)
  if (!readiness.find(entry => entry.key === 'node')?.ok) {
    throw new Error('当前 Node.js 低于 22.19.0。npm 无法在运行自身时安全替换 Node.js；请先升级 Node.js 后重新执行。')
  }
  const missingPackageKeys = new Set(readiness.filter(entry => !entry.ok && !entry.installed).map(entry => entry.key))
  const packages = nativePackages.filter(entry => missingPackageKeys.has(entry.key))
  const newDatabase = packages.some(entry => entry.key === 'mariadb')
  const needsDatabasePassword = newDatabase || setupSettings.pendingDatabasePassword
  const disks = powerShell("Get-CimInstance Win32_LogicalDisk -Filter 'DriveType=3' | Select-Object DeviceID,FreeSpace | ConvertTo-Json -Compress")
  const drives = disks.status === 0 && String(disks.stdout || '').trim() ? [].concat(JSON.parse(disks.stdout)) : []
  const defaultRoot = installRoot || setupSettings.installRoot || selectInstallRoot(drives, root, process.env.SystemDrive)
  if (!assumeYes && !process.stdin.isTTY) throw new Error('非交互终端必须显式增加 --yes')
  const ask = async question => {
    const prompt = createInterface({ input: process.stdin, output: process.stdout })
    try { return await prompt.question(question) } finally { prompt.close() }
  }
  const choices = await collectNativeChoices({ defaultRoot, needsDatabasePassword, assumeYes, ask })
  const paths = windowsInstallPaths(choices.installRoot)
  if (!existsSync(win32.parse(paths.root).root) || (existsSync(paths.root) && !statSync(paths.root).isDirectory())) throw new Error('所选安装根目录不可用，请选择已挂载的本地磁盘目录')
  if (setupSettings.installRoot && paths.root.toLowerCase() !== setupSettings.installRoot.toLowerCase()
    && (setupSettings.managedRust || setupSettings.managedGo || setupSettings.pendingDatabasePassword)) {
    throw new Error('该仓库已记录统一工具目录；setup 不迁移已管理的工具或待初始化的数据库，请继续使用原安装根目录')
  }
  const destinations = { bash: paths.git, gh: paths.gh, python: paths.python, rustup: paths.cargo, msvc: paths.msvc, mariadb: paths.mariaDB }
  const actions = [
    `统一安装根目录：${paths.root}；记录目录与安装状态，不保存密码`,
    ...packages.map(entry => `${entry.name} → ${destinations[entry.key]}`),
    '优先准备 Git 和 gh；完成浏览器授权、仓库 PR 权限与 Git 远程读取检查后，才安装其余工具',
    '复用有效登录和 Git 认证；HTTPS 认证不可用时为当前 GitHub 主机配置 gh 凭据助手，SSH 配置保留',
    newDatabase ? `MariaDB 数据目录：${paths.mariaDBData}；下载缓存：${resolve(paths.cache, 'MariaDB')}` : '复用现有数据库安装',
    needsDatabasePassword ? '设置 root 密码，成功后仅在控制台打印（不传入 MSI 或日志文件）' : '保留现有数据库账号密码，不自动重置',
    '已安装工具继续使用原位置；Windows Installer、启动器和部分微软共享组件仍由系统管理',
    '安装 rust-toolchain.toml 锁定的 Rust 组件',
    '执行 npm ci 安装锁定的项目依赖',
    readiness.find(entry => entry.key === 'go')?.ok ? '复用已有 Go 1.25.14 并预下载模块' : `安装 Go 1.25.14 → ${paths.go} 并预下载模块`,
  ]
  if (!(await confirmPlan(actions, assumeYes))) { console.log('已取消，未修改系统或项目环境。'); return }
  setupSettings = {
    installRoot: choices.installRoot,
    managedRust: setupSettings.managedRust || missingPackageKeys.has('rustup'),
    managedGo: setupSettings.managedGo || !readiness.find(entry => entry.key === 'go')?.ok,
    pendingDatabasePassword: Boolean(needsDatabasePassword),
  }
  writeSetupSettings(root, setupSettings)
  Object.assign(process.env, setupEnvironment(setupSettings))
  if (packages.some(entry => entry.key !== 'mariadb') && !commandPath('winget.exe')) throw new Error('未找到 winget.exe，无法安装缺失的 Windows 组件')
  let passwordApplied = false
  let gitSshCommand
  const reportPassword = () => {
    if (!passwordApplied || !choices.password) return
    console.log('\n本次已设置的 MariaDB 管理员连接（请保存，密码不写入文件）：')
    console.log('  地址：127.0.0.1:3306；用户：root')
    console.log(`  密码：${choices.password}`)
    choices.password = undefined
  }
  process.once('exit', reportPassword)
  try {
    await runNativeSetupPhases({
      packages,
      install: entry => wingetInstall(entry, paths),
      authenticate: async () => {
        const gh = githubCLIPath()
        const git = gitExecutable()
        if (gh) {
          const pathKey = Object.keys(process.env).find(key => key.toLowerCase() === 'path') || 'Path'
          process.env[pathKey] = `${dirname(gh)};${process.env[pathKey] || ''}`
        }
        const github = await prepareGitHubAuthentication({ gh, git, cwd: root, assumeYes, ask, run: execute })
        gitSshCommand = github.gitSshCommand
        persistToolEnvironment({ ...setupSettings, managedGo: false, managedRust: false })
      },
      automatic: async remainingPackages => {
        Object.assign(process.env, githubEnvironment({ ...process.env, GIT_SSH_COMMAND: gitSshCommand }))
        for (const entry of remainingPackages) {
          if (entry.key === 'mariadb') await installMariaDB(choices.installRoot)
          else await wingetInstall(entry, paths)
        }
        startDatabaseService()
        if (needsDatabasePassword) {
          initializeDatabaseRootPassword(resolve(paths.mariaDB, 'bin', 'mariadb.exe'), choices.password, paths.mariaDBData, { log: () => {} })
          passwordApplied = true
          setupSettings.pendingDatabasePassword = false
          writeSetupSettings(root, setupSettings)
        }
        persistToolEnvironment(setupSettings)

        const rustup = cargoTool('rustup')
        if (!rustup) throw new Error('Rustup 安装后仍未找到。请重新打开终端，再次执行 npm run setup')
        console.log('\n正在安装仓库锁定的 Rust 工具链…')
        let result = await executeWithProgress(rustup, ['toolchain', 'install', '1.95.0', '--profile', 'minimal', '--component', 'clippy', '--component', 'rustfmt'], { label: '安装 Rust 工具链' })
        if (result.error || result.status !== 0) throw new Error('Rust 1.95.0 工具链安装失败')

        console.log('\n正在安装锁定的 Node.js 项目依赖…')
        const npmCommand = commandPath('npm.cmd')
        const npmCli = firstExisting([process.env.npm_execpath, resolve(dirname(process.execPath), 'node_modules', 'npm', 'bin', 'npm-cli.js'), npmCommand && resolve(dirname(npmCommand), 'node_modules', 'npm', 'bin', 'npm-cli.js')])
        if (!npmCli) throw new Error('未找到 npm-cli.js，请通过 npm run setup 运行安装')
        result = await executeWithProgress(process.execPath, [npmCli, 'ci'], { label: 'npm ci' })
        if (result.error || result.status !== 0) throw new Error('npm ci 执行失败')

        const bash = gitBashPath()
        if (!bash) throw new Error('Git Bash 安装后仍未找到。请重新打开终端，再次执行 npm run setup')
        console.log('\n正在安装统一 Go 工具链…')
        const goRoot = preferredGoRoot(root)
        const goInstallPath = goRoot.replace(/^([a-z]):/i, (_match, drive) => `/${drive.toLowerCase()}`).replaceAll('\\', '/')
        result = await executeWithProgress(bash, ['./scripts/install-go-toolchain.sh', '--yes'], { label: '安装统一 Go 工具链', env: { ...process.env, INIT_CWD: root, ASTER_GO_INSTALL_DIR: goInstallPath } })
        if (result.error || result.status !== 0) throw new Error('统一 Go 工具链安装失败')
        console.log('\n正在预下载 Go 模块…')
        result = await executeWithProgress(process.execPath, ['./scripts/run-go.mjs', 'mod', 'download'], { label: '预下载 Go 模块' })
        if (result.error || result.status !== 0) throw new Error('Go 模块下载失败')

        readiness = nativeReadiness()
        printReadiness('安装后检查', readiness)
        const missing = readiness.filter(entry => !entry.ok)
        if (missing.length) throw new Error(`仍有 ${missing.length} 项未就绪；重新打开终端后执行 npm run setup:check`)
        console.log('\nWindows 原生开发环境已就绪。下一步：npm run dev:manager')
      },
    })
  } finally {
    reportPassword()
    process.removeListener('exit', reportPassword)
    choices.password = undefined
  }
}

export function enableWindowsFeature(name, {
  featureState = windowsFeatureState, isWslReady = wslReady, run = execute, log = console.log,
} = {}) {
  const state = featureState(name)
  if (state === true || (state === null && isWslReady())) return false
  log(`\n正在启用 Windows 可选功能 ${name}…`)
  const result = run('dism.exe', ['/Online', '/Enable-Feature', `/FeatureName:${name}`, '/All', '/NoRestart'], { capture: false })
  // ERROR_SUCCESS_REBOOT_REQUIRED (3010) is successful servicing with a pending reboot.
  if (result.error || ![0, 3010].includes(result.status)) {
    throw new Error(`启用 ${name} 失败（${result.error?.message || `退出码 ${result.status ?? 'unknown'}`}）`)
  }
  if (result.status === 3010) log(`[PASS] ${name} 已启用；需要重启 Windows 后生效（退出码 3010）。`)
  return true
}

async function installLinuxLab(assumeYes) {
  let readiness = linuxLabReadiness()
  printReadiness('可选 Linux 发布验证环境', readiness)
  const missingKeys = new Set(readiness.filter(entry => !entry.ok).map(entry => entry.key))
  if (!missingKeys.size) { console.log('\nLinux 发布验证环境已经就绪。下一步：npm run test:linux:doctor'); return }
  if (!missingKeys.has('wsl-feature') && !missingKeys.has('vm-feature') && !missingKeys.has('wsl') && !missingKeys.has('docker')) {
    console.log('\nWSL 2 与 Docker Desktop 已安装。请启动 Docker Desktop，等待 Engine 就绪后执行 npm run test:linux:doctor。')
    return
  }
  const packages = linuxLabPackages.filter(entry => missingKeys.has(entry.key))
  const actions = []
  if (missingKeys.has('wsl-feature') || missingKeys.has('vm-feature')) actions.push('启用 Windows Subsystem for Linux 与 Virtual Machine Platform')
  actions.push(...packages.map(entry => `通过 winget 静默安装 ${entry.name}`))
  if (!(await confirmPlan(actions, assumeYes))) { console.log('已取消，未修改系统。'); return }
  if (!administrator()) throw new Error('该命令需要管理员权限。请在“以管理员身份运行”的终端中重新执行。')
  if (!commandPath('winget.exe')) throw new Error('未找到 winget.exe，无法安装 WSL 与 Docker Desktop')
  const enabledWSL = enableWindowsFeature('Microsoft-Windows-Subsystem-Linux')
  const enabledVirtualization = enableWindowsFeature('VirtualMachinePlatform')
  const restartRequired = enabledWSL || enabledVirtualization
  for (const entry of packages) await wingetInstall(entry)
  const wsl = commandPath('wsl.exe') || resolve(process.env.SystemRoot || 'C:\\Windows', 'System32', 'wsl.exe')
  if (!restartRequired && existsSync(wsl)) execute(wsl, ['--set-default-version', '2'], { capture: false })
  readiness = linuxLabReadiness()
  printReadiness('安装后检查', readiness)
  if (restartRequired) {
    console.log('\nWindows 可选功能已启用，需要重启系统。重启后启动 Docker Desktop，再执行：')
    console.log('  npm run setup:linux-lab:check')
    console.log('  npm run test:linux:doctor')
    return
  }
  if (!readiness.find(entry => entry.key === 'docker-running')?.ok) {
    console.log('\nWSL 2 与 Docker Desktop 已安装。请启动 Docker Desktop，等待 Engine 就绪后执行：')
    console.log('  npm run test:linux:doctor')
    return
  }
  console.log('\nLinux 发布验证环境已就绪。下一步：npm run test:linux:doctor')
}

async function main() {
  if (process.platform !== 'win32') throw new Error('此安装命令仅用于 Windows；Linux 主机直接安装 Docker 后运行 npm run test:linux:doctor')
  const [profile, ...args] = process.argv.slice(2)
  if (!['native', 'linux-lab'].includes(profile)) {
    usage(); process.exitCode = 2; return
  }
  const options = parseSetupArguments(args)
  const { checkOnly, assumeYes } = options
  if (profile === 'linux-lab' && options.installRoot) throw new Error('--install-root 只用于 Windows 原生开发工具')
  Object.assign(process.env, setupEnvironment(setupSettings))
  if (checkOnly) {
    const readiness = profile === 'native' ? nativeReadiness({ checkAuth: true }) : linuxLabReadiness()
    printReadiness(profile === 'native' ? 'Windows 原生开发环境' : '可选 Linux 发布验证环境', readiness)
    process.exitCode = readiness.every(entry => entry.ok) ? 0 : 1
    return
  }
  const lock = await acquireSetupLock()
  try {
    if (profile === 'native') await installNative(options)
    else await installLinuxLab(assumeYes)
  } finally { await lock.release() }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(`Error: ${error.message}`); process.exitCode = 1 })
}
