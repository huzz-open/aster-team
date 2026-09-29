import { spawnSync } from 'node:child_process'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { resolveGoCommand } from '../tools/toolchains/go-toolchain.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const args = process.argv.slice(2)
const go = resolveGoCommand('go', root)
const dependenciesReadyMarker = '@@ASTER_GO_DEPENDENCIES_READY@@'

if (!args.length) throw new Error('Usage: node scripts/run-go-service.mjs <go arguments...>')

function run(arguments_) {
  const result = spawnSync(go, arguments_, { cwd: root, env: process.env, stdio: 'inherit' })
  if (result.error?.code === 'ENOENT') {
    console.error('Missing required command: go. From the repository directory, run: npm run setup')
    process.exit(1)
  }
  if (result.error) throw result.error
  if (result.status !== 0) process.exit(result.status ?? 1)
}

console.log('正在下载并校验 Go 模块依赖；此阶段不计入服务健康检查时间。')
run(['mod', 'download'])
if (process.env.ASTER_LOCAL_DEV_MANAGER === 'true') console.log(dependenciesReadyMarker)
run(args)
