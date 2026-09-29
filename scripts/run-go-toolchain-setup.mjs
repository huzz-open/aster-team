import { spawnSync } from 'node:child_process'
import { resolve } from 'node:path'
import { gitBashPath } from './setup-windows-environment.mjs'
import { preferredGoRoot } from '../tools/toolchains/go-toolchain.mjs'

const root = resolve('.')
const bash = process.platform === 'win32' ? gitBashPath() : 'bash'
if (!bash) {
  console.error('未找到 Git Bash。Windows 原生 Go 安装不使用 WSL bash；请先执行 npm run setup。')
  process.exit(1)
}
const goRoot = preferredGoRoot(root)
const installDirectory = process.platform === 'win32'
  ? goRoot.replace(/^([a-z]):/i, (_match, drive) => `/${drive.toLowerCase()}`).replaceAll('\\', '/')
  : goRoot

const result = spawnSync(bash, ['./scripts/install-go-toolchain.sh', ...process.argv.slice(2)], {
  cwd: root, stdio: 'inherit', windowsHide: true,
  env: { ...process.env, INIT_CWD: root, ASTER_GO_INSTALL_DIR: installDirectory },
})
if (result.error) throw result.error
process.exit(result.status ?? 1)
