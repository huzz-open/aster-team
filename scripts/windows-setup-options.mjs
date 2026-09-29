import { createHash, randomBytes } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, resolve, win32 } from 'node:path'
import { createInterface } from 'node:readline/promises'
import { Writable } from 'node:stream'

export function validateInstallRoot(value) {
  if (!/^[a-z]:[\\/]/i.test(value) || /[<>"|?*\x00-\x1f]/.test(value) || value.slice(2).includes(':')) {
    throw new Error('安装根目录必须是本地盘符下的绝对路径，例如 D:\\AsterDev')
  }
  const path = win32.normalize(value).replace(/[\\/]+$/, '')
  if (path.length <= 2 || path.split('\\').some(part => /[. ]$/.test(part) || /^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)/i.test(part))) throw new Error('请指定盘符下的有效子目录，不要直接使用磁盘根目录或保留名称')
  return path
}

export function selectInstallRoot(drives, repositoryRoot, systemDrive = 'C:') {
  const repoDrive = win32.parse(repositoryRoot).root.slice(0, 2).toUpperCase()
  const candidates = drives.filter(drive => (!drive.DriveType || drive.DriveType === 3) && /^[a-z]:$/i.test(drive.DeviceID) && Number(drive.FreeSpace) > 0)
  if (!candidates.length) throw new Error('未找到有可用空间的本地固定磁盘，请通过 --install-root 指定安装目录')
  const rank = drive => {
    const id = drive.DeviceID.toUpperCase()
    return (id !== 'C:' ? 4 : 0) + (id !== systemDrive.toUpperCase() ? 2 : 0) + (id === repoDrive ? 1 : 0)
  }
  candidates.sort((a, b) => rank(b) - rank(a) || Number(b.FreeSpace) - Number(a.FreeSpace))
  return `${candidates[0].DeviceID.toUpperCase()}\\AsterDev`
}

export function windowsInstallPaths(installRoot) {
  const root = validateInstallRoot(installRoot)
  return {
    root, git: win32.join(root, 'Git'), gh: win32.join(root, 'GitHubCLI'), python: win32.join(root, 'Python'),
    cargo: win32.join(root, 'Rust', 'cargo'), rustup: win32.join(root, 'Rust', 'rustup'),
    msvc: win32.join(root, 'BuildTools'), shared: win32.join(root, 'VisualStudioShared'),
    mariaDB: win32.join(root, 'MariaDB'), mariaDBData: win32.join(root, 'MariaDB', 'data'),
    go: win32.join(root, 'Go'), cache: win32.join(root, 'Cache'),
  }
}

export function readSetupSettings(repositoryRoot) {
  const file = resolve(repositoryRoot, '.aster-tools', 'windows-setup.json')
  if (!existsSync(file)) return {}
  const value = JSON.parse(readFileSync(file, 'utf8'))
  return {
    installRoot: validateInstallRoot(value.installRoot),
    managedRust: value.managedRust === true, managedGo: value.managedGo === true,
    pendingDatabasePassword: value.pendingDatabasePassword === true,
  }
}

export function writeSetupSettings(repositoryRoot, settings) {
  const file = resolve(repositoryRoot, '.aster-tools', 'windows-setup.json')
  mkdirSync(dirname(file), { recursive: true })
  // An explicit allowlist prevents a password or other transient prompt answer from being persisted.
  writeFileSync(file, JSON.stringify({
    installRoot: validateInstallRoot(settings.installRoot),
    managedRust: settings.managedRust === true, managedGo: settings.managedGo === true,
    pendingDatabasePassword: settings.pendingDatabasePassword === true,
  }, null, 2) + '\n')
}

export function setupEnvironment(settings, environment = process.env) {
  if (!settings.installRoot) return { ...environment }
  const paths = windowsInstallPaths(settings.installRoot)
  const result = { ...environment }
  result.ASTER_TOOLS_ROOT = paths.root
  const pathKey = Object.keys(result).find(key => key.toLowerCase() === 'path') || 'Path'
  const bins = [win32.join(paths.git, 'cmd'), paths.gh, paths.python, win32.join(paths.python, 'Scripts')]
  if (settings.managedRust) {
    result.CARGO_HOME = paths.cargo
    result.RUSTUP_HOME = paths.rustup
    bins.push(win32.join(paths.cargo, 'bin'))
  }
  if (settings.managedGo) {
    bins.push(win32.join(paths.go, 'bin'))
  }
  result[pathKey] = [...new Set([...bins, ...(result[pathKey] || '').split(';').filter(Boolean)])].join(';')
  return result
}

export function resolveRootPassword(value = '') {
  if (value === '') return `Aa9!${randomBytes(21).toString('base64url')}`
  if (value.length < 12 || value.length > 128 || /[\x00-\x1f\x7f]/.test(value)) throw new Error('密码需要 12–128 个字符，且不能包含控制字符')
  return value
}

export function rootPasswordSql(password) {
  const first = createHash('sha1').update(password, 'utf8').digest()
  const verifier = createHash('sha1').update(first).digest('hex').toUpperCase()
  // MariaDB mysql_native_password accepts this verifier. Plaintext never enters MSI arguments, SQL, or logs.
  return `ALTER USER 'root'@'localhost' IDENTIFIED BY PASSWORD '*${verifier}';\n`
}

export async function askSecret(question, { input = process.stdin, output = process.stdout } = {}) {
  let muted = false
  const hidden = new Writable({ write(chunk, _encoding, callback) { if (!muted) output.write(chunk); callback() } })
  const prompt = createInterface({ input, output: hidden, terminal: true })
  try {
    const answer = prompt.question(question)
    muted = true
    return await answer
  } finally { prompt.close(); output.write('\n') }
}

export async function collectNativeChoices({ defaultRoot, needsDatabasePassword, assumeYes = false, ask, secret = askSecret }) {
  const rootAnswer = assumeYes ? '' : await ask(`统一安装根目录 [${defaultRoot}]：`)
  const installRoot = validateInstallRoot(rootAnswer.trim() || defaultRoot)
  let password
  if (needsDatabasePassword) {
    const answer = assumeYes ? '' : await secret('MariaDB root 密码（输入不回显；至少 12 字符；留空随机生成）：')
    password = resolveRootPassword(answer)
    if (answer && await secret('再次输入 root 密码：') !== answer) throw new Error('两次密码不一致，尚未开始安装')
  }
  return { installRoot, password }
}

export function nativePackageOverride(entry, paths) {
  if (entry.key === 'bash') return `/VERYSILENT /NORESTART /SP- /DIR="${paths.git}"`
  if (entry.key === 'gh') return `/qn /norestart INSTALLDIR="${paths.gh}"`
  if (entry.key === 'python') return `/quiet InstallAllUsers=0 PrependPath=1 Include_launcher=1 Include_test=0 TargetDir="${paths.python}"`
  if (entry.key === 'rustup') return '-y --default-toolchain none --no-modify-path'
  if (entry.key === 'msvc') return `${entry.override} --installPath "${paths.msvc}" --path cache="${win32.join(paths.cache, 'VisualStudio')}" --path shared="${paths.shared}"`
  return entry.override
}

export function parseSetupArguments(args) {
  let installRoot
  const flags = new Set()
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    if (arg === '--install-root') {
      if (installRoot || !args[index + 1]) throw new Error('--install-root 需要一个目录')
      installRoot = validateInstallRoot(args[++index])
    } else if (['--yes', '--check'].includes(arg)) flags.add(arg)
    else throw new Error(`未知参数：${arg}`)
  }
  return { installRoot, assumeYes: flags.has('--yes'), checkOnly: flags.has('--check') }
}
