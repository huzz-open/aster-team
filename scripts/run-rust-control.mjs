import { existsSync, readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { parseEnv } from 'node:util'
import { fileURLToPath } from 'node:url'
import { localDevelopmentBindHost, localLanAccessEnabled } from './local-lan-development.mjs'
import { prepareLocalOfficialPlugin } from './prepare-local-official-plugin.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const controlBuildReadyMarker = '@@ASTER_CUSTOMER_CONTROL_BUILD_READY@@'
const envFile = resolve(root, 'data/local/customer.env')
if (!existsSync(envFile)) throw new Error('缺少 data/local/customer.env，请先执行 npm run setup:local')
const local = parseEnv(readFileSync(envFile, 'utf8'))
const environment = { ...process.env, ...local }
if (process.env.ASTER_CONTROL_PORT) environment.ASTER_CONTROL_PORT = process.env.ASTER_CONTROL_PORT
const bindHost = localDevelopmentBindHost(environment)
const required = [
  'ASTER_LICENSE_TRUSTED_KEYS_JSON', 'ASTER_RELEASE_TRUSTED_KEYS_JSON',
  'ASTER_INSTALLATION_PROFILE_PATH', 'ASTER_INSTALLATION_KEY_PATH',
  'ASTER_LICENSE_FILE_PATH', 'ASTER_LICENSE_STATE_PATH',
  'ASTER_CONTROL_DB_HOST', 'ASTER_CONTROL_DB_PORT', 'ASTER_CONTROL_DB_NAME',
  'ASTER_CONTROL_DB_USER', 'ASTER_CONTROL_DB_PASSWORD_FILE',
  'ASTER_RUNNER_TASK_KEY_ID', 'ASTER_RUNNER_TASK_KEY_FILE',
]
const missing = required.filter(name => !environment[name]?.trim())
if (missing.length) throw new Error(`本地 Rust Control 配置不完整：${missing.join(', ')}`)

const databaseArguments = [
  '--database-driver', 'mariadb',
  '--database-host', environment.ASTER_CONTROL_DB_HOST,
  '--database-port', environment.ASTER_CONTROL_DB_PORT,
  '--database-name', environment.ASTER_CONTROL_DB_NAME,
  '--database-user', environment.ASTER_CONTROL_DB_USER,
  '--database-password-file', resolve(root, environment.ASTER_CONTROL_DB_PASSWORD_FILE),
]
if (environment.ASTER_CONTROL_DB_TLS === 'true') databaseArguments.push('--database-tls')

let command
if (process.argv.includes('--initialize-installation')) {
  command = ['initialize-installation', '--installation-profile', resolve(root, environment.ASTER_INSTALLATION_PROFILE_PATH), '--installation-key', resolve(root, environment.ASTER_INSTALLATION_KEY_PATH)]
} else if (process.argv.includes('--initialize-database')) {
  command = ['initialize-database', ...databaseArguments]
} else if (process.argv.includes('--initialize-owner')) {
  if (!environment.BOOTSTRAP_ADMIN_EMAIL || !environment.BOOTSTRAP_ADMIN_PASSWORD_FILE) throw new Error('本地 owner 初始化配置缺失')
  command = [
    'initialize-owner', '--installation-profile', resolve(root, environment.ASTER_INSTALLATION_PROFILE_PATH),
    '--installation-key', resolve(root, environment.ASTER_INSTALLATION_KEY_PATH), ...databaseArguments,
    '--email', environment.BOOTSTRAP_ADMIN_EMAIL, '--password-file', resolve(root, environment.BOOTSTRAP_ADMIN_PASSWORD_FILE),
  ]
} else if (process.argv.includes('--initialize-runner-key')) {
  command = ['initialize-runner-task-key', '--key-file', resolve(root, environment.ASTER_RUNNER_TASK_KEY_FILE)]
} else if (process.argv.includes('--export-runner-keys')) {
  command = [
    'export-runner-task-keys', '--key-id', environment.ASTER_RUNNER_TASK_KEY_ID,
    '--key-file', resolve(root, environment.ASTER_RUNNER_TASK_KEY_FILE),
    '--target', resolve(root, environment.ASTER_RUNNER_TASK_PUBLIC_KEYS_PATH || 'data/control/keys/runner-task-public.json'),
  ]
} else if (process.argv.includes('--generate-license-request')) {
  command = [
    'generate-license-request', '--installation-profile', resolve(root, environment.ASTER_INSTALLATION_PROFILE_PATH),
    '--output', resolve(root, environment.ASTER_LICENSE_REQUEST_PATH || 'data/control/license/request.json'),
  ]
} else if (process.argv.some(value => value.startsWith('--install-license='))) {
  const source = process.argv.find(value => value.startsWith('--install-license='))?.slice('--install-license='.length)
  if (!source) throw new Error('--install-license requires a source path')
  command = [
    'install-license', '--source', resolve(root, source),
    '--license-file', resolve(root, environment.ASTER_LICENSE_FILE_PATH),
    '--installation-profile', resolve(root, environment.ASTER_INSTALLATION_PROFILE_PATH),
    '--installation-key', resolve(root, environment.ASTER_INSTALLATION_KEY_PATH),
    '--license-state', resolve(root, environment.ASTER_LICENSE_STATE_PATH),
  ]
} else {
  if (process.platform === 'win32' && process.arch === 'x64') {
    const asterctl = resolve(root, 'target/client-tools/asterctl-windows-x86_64.exe')
    const asterctlBuild = spawnSync(process.execPath, [
      resolve(root, 'scripts/build-asterctl-windows.mjs'),
      `--output=${asterctl}`,
    ], {
      cwd: root, env: environment, stdio: 'inherit', windowsHide: true,
    })
    if (asterctlBuild.error) throw asterctlBuild.error
    if (asterctlBuild.status !== 0) process.exit(asterctlBuild.status ?? 1)
    environment.ASTER_DEV_ASTERCTL_WINDOWS_X64 = asterctl
  }
  command = [
    'serve', '--listen', `${bindHost}:${environment.ASTER_CONTROL_PORT || '11080'}`,
    '--install-root', resolve(root, 'data'),
    '--admin-listen', '127.0.0.1:11182', '--member-listen', '127.0.0.1:11181',
    '--admin-assets', resolve(root, 'customer/admin'), '--member-assets', resolve(root, 'customer/member'),
    '--license-file', resolve(root, environment.ASTER_LICENSE_FILE_PATH),
    '--installation-profile', resolve(root, environment.ASTER_INSTALLATION_PROFILE_PATH),
    '--installation-key', resolve(root, environment.ASTER_INSTALLATION_KEY_PATH),
    '--license-state', resolve(root, environment.ASTER_LICENSE_STATE_PATH),
    '--runner-task-key-id', environment.ASTER_RUNNER_TASK_KEY_ID,
    '--runner-task-key-file', resolve(root, environment.ASTER_RUNNER_TASK_KEY_FILE),
    ...databaseArguments,
  ]
  if (localLanAccessEnabled(environment)) {
    command.push('--allow-insecure-http', 'true', '--secure-cookies', 'false')
  }
}

const cargoPackageArguments = [
  '-p', 'aster-control', '--no-default-features', '--features', 'local-demo,mariadb',
]
if (command[0] === 'serve') prepareLocalOfficialPlugin(root, environment)
const cargoOptions = {
  cwd: root, env: environment, stdio: 'inherit', shell: process.platform === 'win32',
}

const managedServe = process.env.ASTER_LOCAL_DEV_MANAGER === 'true' && command[0] === 'serve'
let result
if (managedServe) {
  const build = spawnSync('cargo', ['build', ...cargoPackageArguments], cargoOptions)
  if (build.error) throw build.error
  if (build.status !== 0) process.exit(build.status ?? 1)
  console.log(controlBuildReadyMarker)
  const cargoTargetDirectory = resolve(root, environment.CARGO_TARGET_DIR || 'target')
  const controlBinary = resolve(cargoTargetDirectory, 'debug', process.platform === 'win32' ? 'aster-control.exe' : 'aster-control')
  result = spawnSync(controlBinary, command, {
    cwd: root, env: environment, stdio: 'inherit', windowsHide: true,
  })
} else {
  result = spawnSync('cargo', [
    'run', ...cargoPackageArguments, '--', ...command,
  ], cargoOptions)
}
if (result.error) throw result.error
process.exit(result.status ?? 1)
